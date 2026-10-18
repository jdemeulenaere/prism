//! Drop, clone and equality glue (D9 and D10 of `runtime-strings-and-drops`, D12 to D14 of
//! `heap-boxes`).
//!
//! What dropping or cloning a value does depends only on its type. A type that owns no storage
//! (no `String`, `Heap<T>` or function value in it, through any field, payload or nullable) has
//! no glue: its drop runs no code and its clone is a bitwise copy. A `String` is dropped and
//! cloned by the runtime (`prism_rt_string_drop`, `prism_rt_string_clone`). Every box, and every
//! other struct, enum and nullable that owns storage, gets generated glue functions:
//!
//! - `<unit module>#drop(<type>)`, taking the value's address, which drops a struct's owning
//!   fields in declaration order, an enum's owning payloads of its current variant in order, or
//!   a nullable's payload when it is present; a box's drop glue drops its contents and then frees
//!   their allocation;
//! - `<unit module>#clone(<type>)`, taking the destination's and the source's addresses, which
//!   copies the value bitwise and then replaces each owning part, in the same order, by a clone
//!   of the source's; a box's clone glue allocates a new box and clones the contents into it.
//!
//! A function value's drop glue calls the drop entry of its descriptor with the value's
//! environment when the entry is not null, and does nothing otherwise (D9 of `closures`); every
//! function type's has the same code. Function types are neither `Copy` nor `Clone`, so they
//! have no clone glue.
//!
//! Comparing the contents of two boxes of a struct, an enum or a nullable of one, or a struct,
//! enum or nullable part beyond the inline budget of a comparison, calls the equality glue of
//! its type, `<unit module>#equals(<type>)`, which takes the two values' addresses and returns a
//! `Bool` ([`crate::equality`]).
//!
//! Each part is handled by calling its own glue, so glue is a set of small functions, one per
//! type, recursive for a type that reaches itself through boxes. Each code unit generates the
//! glue of the types its bodies drop, clone and compare through boxes or beyond the inline
//! budget, and of the parts those reach, as functions local to the unit and called directly, never through the function table:
//! a unit's code depends only on its own MIR, and instances of generic types get glue where they
//! are used. Glue is queued when first asked for and lowered after the unit's bodies. It runs no
//! user code. It checks the stack on entry, as bodies do, raising "stack overflow" without a
//! location when a structure is deeper than the stack allows.

use crate::equality::Comparison;
use crate::error::BackendError;
use crate::layout::{FunctionField, Layout, Member, Shape};
use crate::symbols::{environment_drop_symbol, symbol_of};
use crate::unit::{data_global, LoweredFunction, UnitContext};
use cranelift_codegen::ir::{
    self, condcodes::IntCC, types, AbiParam, Function, InstBuilder, MemFlagsData, TrapCode,
    UserFuncName, Value,
};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Switch};
use cranelift_module::{FuncId, Linkage, Module};
use prism_mir::BodyId;
use prism_runtime::PanicKind;
use prism_typed_tree::{Type, TypeTable};
use std::collections::HashMap;
use std::fmt::Write;
use std::rc::Rc;

/// How deep [`owns_storage`] looks into nested structs and enums; only a broken type table, with
/// a type containing itself by value, reaches it.
const MAXIMUM_DEPTH: usize = 256;

/// Whether a value of `ty` owns storage that a drop must free: a `String`, a `Heap<T>` or a
/// function value, or a struct, enum or nullable holding one through its fields or payloads. A
/// reference owns nothing.
pub fn owns_storage(ty: &Type, types: &TypeTable) -> bool {
    owns_storage_at(ty, types, 0)
}

fn owns_storage_at(ty: &Type, types: &TypeTable, depth: usize) -> bool {
    if depth > MAXIMUM_DEPTH {
        return false;
    }
    match ty {
        Type::String | Type::Heap(_) | Type::List(_) | Type::Fn { .. } => true,
        Type::Nullable(inner) => owns_storage_at(inner, types, depth + 1),
        Type::Struct { id, arguments } => types
            .instantiated_fields(id, arguments)
            .unwrap_or_default()
            .iter()
            .any(|(_, _, field)| owns_storage_at(field, types, depth + 1)),
        Type::Enum { id, arguments } => types
            .instantiated_variants(id, arguments)
            .unwrap_or_default()
            .iter()
            .flat_map(|(_, payloads)| payloads)
            .any(|payload| owns_storage_at(payload, types, depth + 1)),
        Type::Int
        | Type::Float
        | Type::Bool
        | Type::Char
        | Type::Unit
        | Type::Nothing
        | Type::Reference { .. }
        | Type::Parameter(_)
        | Type::Opaque(_)
        | Type::Error => false,
    }
}

/// What glue does: drop a value, clone one, or compare two.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GlueKind {
    Drop,
    Clone,
    Equals,
}

impl GlueKind {
    fn name(self) -> &'static str {
        match self {
            GlueKind::Drop => "drop",
            GlueKind::Clone => "clone",
            GlueKind::Equals => "equals",
        }
    }

    /// The runtime function that is the glue of a `String`.
    fn string_function(self) -> &'static str {
        match self {
            GlueKind::Drop => "prism_rt_string_drop",
            GlueKind::Clone => "prism_rt_string_clone",
            GlueKind::Equals => "prism_rt_string_equals",
        }
    }
}

/// The function that drops or clones a value of some type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Glue {
    /// A runtime function: the glue of a `String`.
    Runtime(&'static str),
    /// A glue function of the unit.
    Generated(FuncId),
}

/// The glue functions of one code unit, each declared once, and those still to lower.
#[derive(Default)]
pub struct GlueTable {
    declared: HashMap<(GlueKind, Type), FuncId>,
    queued: Vec<(GlueKind, Type, String, FuncId)>,
    /// The drop function of each closure's allocated environment (D8 of `escaping-closures`).
    environment_drops: HashMap<BodyId, FuncId>,
    queued_environment_drops: Vec<(BodyId, String, FuncId)>,
}

/// The symbol of the glue of `kind` for `ty` in the unit named `module` (a file's module or an
/// instance's symbol): `<module>#drop(<type>)`,
/// `<module>#clone(<type>)` or `<module>#equals(<type>)`, the type written with every struct and enum qualified by its
/// module, so that two types never share a name.
pub fn glue_symbol(module: &str, kind: GlueKind, ty: &Type) -> String {
    format!("{}#{}({})", module, kind.name(), qualified_name(ty))
}

/// `ty` as written, with every struct and enum qualified by its module.
pub(crate) fn qualified_name(ty: &Type) -> String {
    match ty {
        Type::Struct { id, arguments } | Type::Enum { id, arguments } => {
            let mut name = format!("{}::{}", id.module, id.name);
            if !arguments.is_empty() {
                let arguments: Vec<String> = arguments.iter().map(qualified_name).collect();
                write!(name, "<{}>", arguments.join(", ")).unwrap();
            }
            name
        }
        Type::Nullable(inner) => match inner.as_ref() {
            Type::Reference { .. } | Type::Fn { .. } => format!("({})?", qualified_name(inner)),
            inner => format!("{}?", qualified_name(inner)),
        },
        Type::Reference { mutable, inner } => format!(
            "&{}{}",
            if *mutable { "mut " } else { "" },
            qualified_name(inner)
        ),
        Type::Heap(inner) => format!("Heap<{}>", qualified_name(inner)),
        Type::Fn {
            is_compose,
            params,
            escaping,
            ret,
        } => {
            let params: Vec<String> = params
                .iter()
                .zip(escaping)
                .map(|(param, &flag)| {
                    let name = qualified_name(param);
                    if flag {
                        format!("escaping {}", name)
                    } else {
                        name
                    }
                })
                .collect();
            format!(
                "{}({}) -> {}",
                if *is_compose { "compose " } else { "" },
                params.join(", "),
                qualified_name(ret)
            )
        }
        other => other.to_string(),
    }
}

impl<M: Module> UnitContext<'_, M> {
    /// The glue of `kind` for `ty`, or `None` when `ty` owns no storage, so that its drop does
    /// nothing and its clone is a copy. A generated function is declared, and queued for
    /// lowering, the first time it is asked for.
    pub fn glue(&mut self, kind: GlueKind, ty: &Type) -> Result<Option<Glue>, BackendError> {
        if !owns_storage(ty, &self.file.types) {
            return Ok(None);
        }
        if *ty == Type::String {
            return Ok(Some(Glue::Runtime(kind.string_function())));
        }
        Ok(Some(Glue::Generated(self.generated_glue(kind, ty)?)))
    }

    /// The equality glue of `ty`, a struct, an enum or a nullable in memory, declared and queued
    /// the first time it is asked for (D13 of `heap-boxes`).
    pub fn equality_glue(&mut self, ty: &Type) -> Result<FuncId, BackendError> {
        self.generated_glue(GlueKind::Equals, ty)
    }

    /// The generated glue function of `kind` for `ty`, declared and queued once.
    fn generated_glue(&mut self, kind: GlueKind, ty: &Type) -> Result<FuncId, BackendError> {
        let key = (kind, ty.clone());
        if let Some(id) = self.glue.declared.get(&key) {
            return Ok(*id);
        }
        let symbol = glue_symbol(&self.name, kind, ty);
        let signature = self.glue_signature(kind);
        let id = self
            .module
            .declare_function(&symbol, Linkage::Local, &signature)?;
        self.glue.declared.insert(key, id);
        self.glue.queued.push((kind, ty.clone(), symbol, id));
        Ok(id)
    }

    /// The drop function of the allocated environment of the closure `body`,
    /// `<closure symbol>#drop-environment`, declared and queued the first time it is asked for
    /// (D8 of `escaping-closures`): it takes the environment's address, like drop glue.
    pub fn environment_drop(&mut self, body: BodyId) -> Result<FuncId, BackendError> {
        if let Some(id) = self.glue.environment_drops.get(&body) {
            return Ok(*id);
        }
        let symbol = environment_drop_symbol(&symbol_of(&self.file.bodies[body], self.file));
        let signature = self.glue_signature(GlueKind::Drop);
        let id = self
            .module
            .declare_function(&symbol, Linkage::Local, &signature)?;
        self.glue.environment_drops.insert(body, id);
        self.glue.queued_environment_drops.push((body, symbol, id));
        Ok(id)
    }

    /// The signature of glue of `kind`: the value's address for a drop, the destination's and
    /// the source's for a clone, returning nothing; the two values' addresses for equality,
    /// returning a `Bool`.
    fn glue_signature(&self, kind: GlueKind) -> ir::Signature {
        let mut signature = ir::Signature::new(self.call_conv);
        let parameters = match kind {
            GlueKind::Drop => 1,
            GlueKind::Clone | GlueKind::Equals => 2,
        };
        for _ in 0..parameters {
            signature.params.push(AbiParam::new(self.pointer));
        }
        if kind == GlueKind::Equals {
            signature.returns.push(AbiParam::new(types::I8));
        }
        signature
    }

    /// A reference to `glue` in `function`, called directly. A generated function is not
    /// colocated, like data: the JIT does not promise that two functions are within reach of a
    /// PC-relative call.
    pub fn glue_reference(
        &mut self,
        glue: Glue,
        function: &mut Function,
    ) -> Result<ir::FuncRef, BackendError> {
        let id = match glue {
            Glue::Runtime(name) => self.runtime_function(name)?,
            Glue::Generated(id) => id,
        };
        let reference = self.module.declare_func_in_func(id, function);
        function.dfg.ext_funcs[reference].colocated = false;
        Ok(reference)
    }

    /// Lowers every queued glue function, and those their parts queue in turn, in order of
    /// their symbols.
    pub fn lower_glue(
        &mut self,
        builder_context: &mut FunctionBuilderContext,
    ) -> Result<Vec<LoweredFunction>, BackendError> {
        let mut lowered = Vec::new();
        loop {
            if let Some((kind, ty, symbol, id)) = self.glue.queued.pop() {
                let signature = self.glue_signature(kind);
                let mut function =
                    Function::with_name_signature(UserFuncName::user(0, id.as_u32()), signature);
                GlueLowering::lower(self, kind, &ty, &mut function, builder_context)?;
                lowered.push(LoweredFunction {
                    symbol,
                    id,
                    function,
                });
            } else if let Some((body, symbol, id)) = self.glue.queued_environment_drops.pop() {
                let signature = self.glue_signature(GlueKind::Drop);
                let mut function =
                    Function::with_name_signature(UserFuncName::user(0, id.as_u32()), signature);
                GlueLowering::lower_environment_drop(self, body, &mut function, builder_context)?;
                lowered.push(LoweredFunction {
                    symbol,
                    id,
                    function,
                });
            } else {
                break;
            }
        }
        lowered.sort_by(|a, b| a.symbol.cmp(&b.symbol));
        Ok(lowered)
    }
}

/// The lowering of one glue function.
struct GlueLowering<'a, 'c, 'm, M: Module> {
    context: &'c mut UnitContext<'m, M>,
    builder: FunctionBuilder<'a>,
}

impl<'a, 'c, 'm, M: Module> GlueLowering<'a, 'c, 'm, M> {
    fn lower(
        context: &'c mut UnitContext<'m, M>,
        kind: GlueKind,
        ty: &Type,
        function: &'a mut Function,
        builder_context: &'a mut FunctionBuilderContext,
    ) -> Result<(), BackendError> {
        let builder = FunctionBuilder::new(function, builder_context);
        let mut lowering = GlueLowering { context, builder };
        let entry = lowering.builder.create_block();
        lowering
            .builder
            .append_block_params_for_function_params(entry);
        lowering.builder.switch_to_block(entry);
        let parameters = lowering.builder.block_params(entry).to_vec();
        lowering.check_stack()?;
        let layout = lowering.layout(ty)?;
        match (kind, ty) {
            (GlueKind::Drop, Type::Heap(contents)) => lowering.drop_box(contents, parameters[0])?,
            (GlueKind::Drop, Type::Fn { .. }) => lowering.drop_function_value(parameters[0])?,
            (GlueKind::Clone | GlueKind::Equals, Type::Fn { .. }) => {
                return Err(BackendError::new(format!(
                    "{} glue for the function type {}",
                    kind.name(),
                    ty
                )))
            }
            (GlueKind::Clone, Type::Heap(contents)) => {
                lowering.clone_box(contents, parameters[0], parameters[1])?
            }
            (GlueKind::Drop, _) => lowering.drop_parts(ty, &layout, parameters[0])?,
            (GlueKind::Clone, _) => {
                let (destination, source) = (parameters[0], parameters[1]);
                // Every byte first, then each owning part replaced by a clone of its own.
                lowering.copy(destination, source, &layout);
                lowering.clone_parts(ty, &layout, destination, source)?;
            }
            (GlueKind::Equals, _) => {
                let mut functions = HashMap::new();
                let mut comparison = Comparison::new(
                    &mut lowering.builder,
                    &mut *lowering.context,
                    &mut functions,
                );
                let equal = comparison.glue_body(ty, parameters[0], parameters[1])?;
                lowering.builder.ins().return_(&[equal]);
            }
        }
        if kind != GlueKind::Equals {
            lowering.builder.ins().return_(&[]);
        }
        let config = lowering.context.module.target_config();
        lowering.builder.seal_all_blocks();
        lowering.builder.finalize(config);
        Ok(())
    }

    /// The drop function of the allocated environment of the closure `body`, at the address it
    /// takes (D8 of `escaping-closures`): each owned capture needing a drop is dropped by its
    /// glue, in capture order, then the allocation is freed.
    fn lower_environment_drop(
        context: &'c mut UnitContext<'m, M>,
        body: BodyId,
        function: &'a mut Function,
        builder_context: &'a mut FunctionBuilderContext,
    ) -> Result<(), BackendError> {
        let builder = FunctionBuilder::new(function, builder_context);
        let mut lowering = GlueLowering { context, builder };
        let entry = lowering.builder.create_block();
        lowering
            .builder
            .append_block_params_for_function_params(entry);
        lowering.builder.switch_to_block(entry);
        let environment = lowering.builder.block_params(entry)[0];
        lowering.check_stack()?;
        let file = lowering.context.file;
        let layout = lowering
            .context
            .layouts
            .environment_layout(&file.bodies[body])
            .map_err(|construct| {
                BackendError::new(format!(
                    "no layout for a closure's environment: {}",
                    construct.description()
                ))
            })?;
        for field in layout.captures.iter().filter(|field| field.owned) {
            let address = if field.offset == 0 {
                environment
            } else {
                lowering
                    .builder
                    .ins()
                    .iadd_imm_s(environment, i64::from(field.offset))
            };
            lowering.call_glue(GlueKind::Drop, &field.ty, &[address])?;
        }
        let allocation = Layout {
            size: layout.size,
            align: layout.align,
            shape: Shape::Empty,
        };
        lowering.free(environment, &allocation)?;
        lowering.builder.ins().return_(&[]);
        let config = lowering.context.module.target_config();
        lowering.builder.seal_all_blocks();
        lowering.builder.finalize(config);
        Ok(())
    }

    fn layout(&self, ty: &Type) -> Result<Rc<Layout>, BackendError> {
        self.context.layouts.layout(ty).map_err(|construct| {
            BackendError::new(format!("no layout for {}: {}", ty, construct.description()))
        })
    }

    /// Raises "stack overflow", without a location, when the stack pointer is below the
    /// runtime's stack limit, as a body does on entry (D14 of `heap-boxes`).
    fn check_stack(&mut self) -> Result<(), BackendError> {
        let pointer = self.context.pointer;
        let limit = self.context.stack_limit()?;
        let global = data_global(self.builder.func, limit);
        let limit_address = self.builder.ins().symbol_value(pointer, global);
        let limit = self
            .builder
            .ins()
            .load(pointer, MemFlagsData::trusted(), limit_address, 0);
        let stack_pointer = self.builder.ins().get_stack_pointer(pointer);
        let overflow = self
            .builder
            .ins()
            .icmp(IntCC::UnsignedLessThan, stack_pointer, limit);
        let failure = self.builder.create_block();
        let success = self.builder.create_block();
        self.builder.set_cold_block(failure);
        self.builder
            .ins()
            .brif(overflow, failure, &[], success, &[]);
        self.builder.switch_to_block(failure);
        let id = self
            .context
            .runtime_function("prism_rt_panic_without_location")?;
        let function = self
            .context
            .module
            .declare_func_in_func(id, self.builder.func);
        let kind = self
            .builder
            .ins()
            .iconst(types::I32, i64::from(PanicKind::StackOverflow as u32));
        self.builder.ins().call(function, &[kind]);
        self.builder.ins().trap(UNREACHABLE);
        self.builder.switch_to_block(success);
        Ok(())
    }

    /// The address of a new allocation for the contents of a box, of `layout`: the runtime's,
    /// or the alignment itself, a non-null aligned address, for a zero-sized value, which
    /// allocates nothing (D3 of `heap-boxes`).
    fn allocate(&mut self, layout: &Layout) -> Result<Value, BackendError> {
        allocate_box(self.context, &mut self.builder, layout)
    }

    /// Frees the allocation of the contents of a box, of `layout`, at `address`.
    fn free(&mut self, address: Value, layout: &Layout) -> Result<(), BackendError> {
        free_box(self.context, &mut self.builder, address, layout)
    }

    /// The drop glue of a `Heap<contents>` at `address`: the contents are dropped, then their
    /// allocation is freed (D12 of `heap-boxes`).
    fn drop_box(&mut self, contents: &Type, address: Value) -> Result<(), BackendError> {
        let pointer = self.context.pointer;
        let boxed = self
            .builder
            .ins()
            .load(pointer, MemFlagsData::trusted(), address, 0);
        self.call_glue(GlueKind::Drop, contents, &[boxed])?;
        let layout = self.layout(contents)?;
        self.free(boxed, &layout)
    }

    /// The drop glue of a function value at `address` (D9 of `closures`): the drop entry of its
    /// descriptor, when not null, is called with the value's environment.
    fn drop_function_value(&mut self, address: Value) -> Result<(), BackendError> {
        let pointer = self.context.pointer;
        let flags = MemFlagsData::trusted();
        let descriptor = self.builder.ins().load(
            pointer,
            flags,
            address,
            FunctionField::Descriptor.offset(pointer) as i32,
        );
        let entry = self
            .builder
            .ins()
            .load(pointer, flags, descriptor, pointer.bytes() as i32);
        let present = self.builder.create_block();
        let done = self.builder.create_block();
        self.builder.ins().brif(entry, present, &[], done, &[]);
        self.builder.switch_to_block(present);
        let environment = self.builder.ins().load(
            pointer,
            flags,
            address,
            FunctionField::Environment.offset(pointer) as i32,
        );
        let mut signature = ir::Signature::new(self.context.call_conv);
        signature.params.push(AbiParam::new(pointer));
        let signature = self.builder.import_signature(signature);
        self.builder
            .ins()
            .call_indirect(signature, entry, &[environment]);
        self.builder.ins().jump(done, &[]);
        self.builder.switch_to_block(done);
        Ok(())
    }

    /// The clone glue of a `Heap<contents>`: a new allocation holding a clone of the source's
    /// contents, by their clone glue or bitwise, written to the destination.
    fn clone_box(
        &mut self,
        contents: &Type,
        destination: Value,
        source: Value,
    ) -> Result<(), BackendError> {
        let pointer = self.context.pointer;
        let flags = MemFlagsData::trusted();
        let original = self.builder.ins().load(pointer, flags, source, 0);
        let layout = self.layout(contents)?;
        let copy = self.allocate(&layout)?;
        if owns_storage(contents, &self.context.file.types) {
            self.call_glue(GlueKind::Clone, contents, &[copy, original])?;
        } else if layout.size > 0 {
            self.copy(copy, original, &layout);
        }
        self.builder.ins().store(flags, copy, destination, 0);
        Ok(())
    }

    fn add(&mut self, address: Value, offset: u32) -> Value {
        if offset == 0 {
            address
        } else {
            self.builder.ins().iadd_imm_s(address, i64::from(offset))
        }
    }

    fn copy(&mut self, destination: Value, source: Value, layout: &Layout) {
        let config = self.context.module.target_config();
        let align = layout.align.min(u32::from(u8::MAX)) as u8;
        let mut flags = MemFlagsData::new();
        flags.set_notrap();
        self.builder.emit_small_memory_copy(
            config,
            destination,
            source,
            u64::from(layout.size),
            align,
            align,
            false,
            flags,
        );
    }

    /// Calls the glue of `kind` for `ty` on `arguments`, if `ty` has any.
    fn call_glue(
        &mut self,
        kind: GlueKind,
        ty: &Type,
        arguments: &[Value],
    ) -> Result<(), BackendError> {
        if let Some(glue) = self.context.glue(kind, ty)? {
            let reference = self.context.glue_reference(glue, self.builder.func)?;
            self.builder.ins().call(reference, arguments);
        }
        Ok(())
    }

    /// The members whose glue a value of `ty` with `layout` runs, by case: for a struct, its
    /// fields; for an enum, the payloads of each variant, keyed by its index; for a nullable, its
    /// payload when the tag is 1.
    fn parts(&self, ty: &Type, layout: &Layout) -> Result<Parts, BackendError> {
        Ok(match &layout.shape {
            Shape::Struct { fields } => Parts::Always(fields.clone()),
            Shape::Enum { tag, variants } => Parts::ByTag(
                *tag,
                variants
                    .iter()
                    .enumerate()
                    .map(|(index, variant)| (index as u128, variant.payloads.clone()))
                    .collect(),
            ),
            Shape::Nullable { payload } => {
                Parts::ByTag(types::I8, vec![(1, vec![payload.clone()])])
            }
            // A nullable box is its pointer, present when it is not 0.
            Shape::NullPointer => match ty {
                Type::Nullable(inner) => Parts::IfNotNull(Member {
                    name: String::new(),
                    ty: inner.as_ref().clone(),
                    offset: 0,
                }),
                _ => return Err(BackendError::new(format!("glue for {}", ty))),
            },
            other => {
                return Err(BackendError::new(format!(
                    "glue for {} of shape {:?}",
                    ty, other
                )))
            }
        })
    }

    /// Runs `each` on the members of the value at `address` that its current case holds and that
    /// own storage, in order.
    fn for_owning_parts(
        &mut self,
        ty: &Type,
        layout: &Layout,
        address: Value,
        mut each: impl FnMut(&mut Self, &Member) -> Result<(), BackendError>,
    ) -> Result<(), BackendError> {
        let owning = |lowering: &Self, members: Vec<Member>| -> Vec<Member> {
            members
                .into_iter()
                .filter(|member| owns_storage(&member.ty, &lowering.context.file.types))
                .collect()
        };
        match self.parts(ty, layout)? {
            Parts::Always(members) => {
                for member in owning(self, members) {
                    each(self, &member)?;
                }
            }
            Parts::IfNotNull(member) => {
                let pointer = self.context.pointer;
                let value = self
                    .builder
                    .ins()
                    .load(pointer, MemFlagsData::trusted(), address, 0);
                let present = self.builder.create_block();
                let done = self.builder.create_block();
                self.builder.ins().brif(value, present, &[], done, &[]);
                self.builder.switch_to_block(present);
                each(self, &member)?;
                self.builder.ins().jump(done, &[]);
                self.builder.switch_to_block(done);
            }
            Parts::ByTag(tag_type, cases) => {
                let cases: Vec<(u128, Vec<Member>)> = cases
                    .into_iter()
                    .map(|(tag, members)| (tag, owning(self, members)))
                    .filter(|(_, members)| !members.is_empty())
                    .collect();
                let tag = self
                    .builder
                    .ins()
                    .load(tag_type, MemFlagsData::trusted(), address, 0);
                let done = self.builder.create_block();
                let mut switch = Switch::new();
                let mut blocks = Vec::new();
                for (value, members) in cases {
                    let block = self.builder.create_block();
                    switch.set_entry(value, block);
                    blocks.push((block, members));
                }
                switch.emit(&mut self.builder, tag, done);
                for (block, members) in blocks {
                    self.builder.switch_to_block(block);
                    for member in members {
                        each(self, &member)?;
                    }
                    self.builder.ins().jump(done, &[]);
                }
                self.builder.switch_to_block(done);
            }
        }
        Ok(())
    }

    fn drop_parts(
        &mut self,
        ty: &Type,
        layout: &Layout,
        address: Value,
    ) -> Result<(), BackendError> {
        self.for_owning_parts(ty, layout, address, |lowering, member| {
            let part = lowering.add(address, member.offset);
            lowering.call_glue(GlueKind::Drop, &member.ty, &[part])
        })
    }

    fn clone_parts(
        &mut self,
        ty: &Type,
        layout: &Layout,
        destination: Value,
        source: Value,
    ) -> Result<(), BackendError> {
        self.for_owning_parts(ty, layout, source, |lowering, member| {
            let to = lowering.add(destination, member.offset);
            let from = lowering.add(source, member.offset);
            lowering.call_glue(GlueKind::Clone, &member.ty, &[to, from])
        })
    }
}

/// The members a glue function handles: always, by the value of a tag at offset 0, or the
/// payload of a nullable pointer when it is not 0.
enum Parts {
    Always(Vec<Member>),
    ByTag(ir::Type, Vec<(u128, Vec<Member>)>),
    IfNotNull(Member),
}

/// The trap that ends a block after a call that never returns; it is never reached.
const UNREACHABLE: TrapCode = match TrapCode::user(1) {
    Some(code) => code,
    None => panic!("1 is a user trap code"),
};

/// The address of a new allocation for the contents of a box, of `layout`, in the function
/// `builder` builds: a call of the runtime's allocator, which reports "out of memory" itself, or
/// for a zero-sized value the alignment itself, which is non-null, aligned and never read or
/// written (D3 of `heap-boxes`).
pub(crate) fn allocate_box<M: Module>(
    context: &mut UnitContext<'_, M>,
    builder: &mut FunctionBuilder<'_>,
    layout: &Layout,
) -> Result<Value, BackendError> {
    let pointer = context.pointer;
    let align = builder.ins().iconst(pointer, i64::from(layout.align));
    if layout.size == 0 {
        return Ok(align);
    }
    let size = builder.ins().iconst(pointer, i64::from(layout.size));
    let id = context.runtime_function("prism_rt_allocate")?;
    let function = context.module.declare_func_in_func(id, builder.func);
    let call = builder.ins().call(function, &[size, align]);
    Ok(builder.inst_results(call)[0])
}

/// Frees the allocation at `address` of the contents of a box, of `layout`, unless the value is
/// zero-sized, which allocated nothing.
pub(crate) fn free_box<M: Module>(
    context: &mut UnitContext<'_, M>,
    builder: &mut FunctionBuilder<'_>,
    address: Value,
    layout: &Layout,
) -> Result<(), BackendError> {
    if layout.size == 0 {
        return Ok(());
    }
    let pointer = context.pointer;
    let size = builder.ins().iconst(pointer, i64::from(layout.size));
    let align = builder.ins().iconst(pointer, i64::from(layout.align));
    let id = context.runtime_function("prism_rt_free")?;
    let function = context.module.declare_func_in_func(id, builder.func);
    builder.ins().call(function, &[address, size, align]);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::lower_source;
    use prism_typed_tree::NominalId;

    #[test]
    fn types_owning_storage() {
        let file = lower_source(
            "struct User { val name: String }\n\
             struct Point : Copy { val x: Int }\n\
             struct Line { val from: Point, val to: Point }\n\
             struct Team { val lead: User?, val size: Int }\n\
             enum Message { Text(String, Int), Empty }\n\
             enum Shape { Dot(Point), Empty }\n",
        );
        let types = &file.types;
        let main = |name: &str| NominalId::new("main", name);
        for ty in [
            Type::String,
            Type::String.nullable(),
            Type::plain_struct(main("User")),
            Type::plain_struct(main("Team")),
            Type::plain_enum(main("Message")),
            Type::Heap(Box::new(Type::Int)),
        ] {
            assert!(owns_storage(&ty, types), "{} owns storage", ty);
        }
        for ty in [
            Type::Int,
            Type::Int.nullable(),
            Type::plain_struct(main("Point")),
            Type::plain_struct(main("Line")),
            Type::plain_enum(main("Shape")),
            Type::Reference {
                mutable: false,
                inner: Box::new(Type::String),
            },
        ] {
            assert!(!owns_storage(&ty, types), "{} owns nothing", ty);
        }
    }

    #[test]
    fn glue_symbols_qualify_every_type() {
        let user = Type::plain_struct(NominalId::new("lib/user", "User"));
        assert_eq!(
            glue_symbol("app/main", GlueKind::Drop, &user),
            "app/main#drop(lib/user::User)"
        );
        let pair = Type::Struct {
            id: NominalId::new("main", "Pair"),
            arguments: vec![user.clone().nullable(), Type::String],
        };
        assert_eq!(
            glue_symbol("main", GlueKind::Clone, &pair),
            "main#clone(main::Pair<lib/user::User?, String>)"
        );
        assert_eq!(
            glue_symbol("main", GlueKind::Drop, &Type::String.nullable()),
            "main#drop(String?)"
        );
        let expr = Type::plain_enum(NominalId::new("main", "Expr"));
        assert_eq!(
            glue_symbol("main", GlueKind::Drop, &Type::Heap(Box::new(expr.clone()))),
            "main#drop(Heap<main::Expr>)"
        );
        assert_eq!(
            glue_symbol("main", GlueKind::Equals, &expr),
            "main#equals(main::Expr)"
        );
    }
}

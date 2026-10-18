//! Drop and clone glue (D9 and D10 of `runtime-strings-and-drops`).
//!
//! What dropping or cloning a value does depends only on its type. A type that owns no storage
//! (no `String`, `Heap<T>` or function value in it, through any field, payload or nullable) has
//! no glue: its drop runs no code and its clone is a bitwise copy. A `String` is dropped and
//! cloned by the runtime (`prism_rt_string_drop`, `prism_rt_string_clone`). Every other struct,
//! enum and nullable that owns storage gets generated glue functions:
//!
//! - `<unit module>#drop(<type>)`, taking the value's address, which drops a struct's owning
//!   fields in declaration order, an enum's owning payloads of its current variant in order, or
//!   a nullable's payload when it is present;
//! - `<unit module>#clone(<type>)`, taking the destination's and the source's addresses, which
//!   copies the value bitwise and then replaces each owning part, in the same order, by a clone
//!   of the source's.
//!
//! Each part is handled by calling its own glue, so glue is a set of small functions, one per
//! type. Each code unit generates the glue of the types its bodies drop and clone, and of the
//! parts those reach, as functions local to the unit and called directly, never through the
//! function table: a unit's code depends only on its own MIR, and instances of generic types get
//! glue where they are used. Glue is queued when first asked for and lowered after the unit's
//! bodies. It runs no user code and does not check the stack, since nothing recursive can be
//! dropped or cloned yet (`heap-boxes` must add the check).

use crate::error::BackendError;
use crate::layout::{Layout, Member, Shape};
use crate::unit::{LoweredFunction, UnitContext};
use cranelift_codegen::ir::{
    self, types, AbiParam, Function, InstBuilder, MemFlagsData, UserFuncName, Value,
};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Switch};
use cranelift_module::{FuncId, Linkage, Module};
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
        Type::String | Type::Heap(_) | Type::Fn { .. } => true,
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

/// What glue does: drop a value, or clone one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GlueKind {
    Drop,
    Clone,
}

impl GlueKind {
    fn name(self) -> &'static str {
        match self {
            GlueKind::Drop => "drop",
            GlueKind::Clone => "clone",
        }
    }

    /// The runtime function that is the glue of a `String`.
    fn string_function(self) -> &'static str {
        match self {
            GlueKind::Drop => "prism_rt_string_drop",
            GlueKind::Clone => "prism_rt_string_clone",
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
}

/// The symbol of the glue of `kind` for `ty` in the unit of `module`: `<module>#drop(<type>)`
/// or `<module>#clone(<type>)`, the type written with every struct and enum qualified by its
/// module, so that two types never share a name.
pub fn glue_symbol(module: &str, kind: GlueKind, ty: &Type) -> String {
    format!("{}#{}({})", module, kind.name(), qualified_name(ty))
}

/// `ty` as written, with every struct and enum qualified by its module.
fn qualified_name(ty: &Type) -> String {
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
        let key = (kind, ty.clone());
        if let Some(id) = self.glue.declared.get(&key) {
            return Ok(Some(Glue::Generated(*id)));
        }
        let symbol = glue_symbol(&self.file.module, kind, ty);
        let signature = self.glue_signature(kind);
        let id = self
            .module
            .declare_function(&symbol, Linkage::Local, &signature)?;
        self.glue.declared.insert(key, id);
        self.glue.queued.push((kind, ty.clone(), symbol, id));
        Ok(Some(Glue::Generated(id)))
    }

    /// The signature of glue of `kind`: the value's address for a drop, the destination's and
    /// the source's for a clone; nothing returned.
    fn glue_signature(&self, kind: GlueKind) -> ir::Signature {
        let mut signature = ir::Signature::new(self.call_conv);
        let parameters = match kind {
            GlueKind::Drop => 1,
            GlueKind::Clone => 2,
        };
        for _ in 0..parameters {
            signature.params.push(AbiParam::new(self.pointer));
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
        while let Some((kind, ty, symbol, id)) = self.glue.queued.pop() {
            let signature = self.glue_signature(kind);
            let mut function =
                Function::with_name_signature(UserFuncName::user(0, id.as_u32()), signature);
            GlueLowering::lower(self, kind, &ty, &mut function, builder_context)?;
            lowered.push(LoweredFunction {
                symbol,
                id,
                function,
            });
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
        let layout = lowering.layout(ty)?;
        match kind {
            GlueKind::Drop => lowering.drop_parts(ty, &layout, parameters[0])?,
            GlueKind::Clone => {
                let (destination, source) = (parameters[0], parameters[1]);
                // Every byte first, then each owning part replaced by a clone of its own.
                lowering.copy(destination, source, &layout);
                lowering.clone_parts(ty, &layout, destination, source)?;
            }
        }
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

/// The members a glue function handles: always, or by the value of a tag at offset 0.
enum Parts {
    Always(Vec<Member>),
    ByTag(ir::Type, Vec<(u128, Vec<Member>)>),
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
    }
}

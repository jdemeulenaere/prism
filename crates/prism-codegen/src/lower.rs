//! Lowering one body of elaborated MIR to Cranelift IR (D6 to D11 and D18 of `codegen-core`, D9
//! to D17 of `codegen-aggregates`).
//!
//! A scalar local is a Cranelift variable, from which Cranelift builds SSA, unless it is
//! borrowed, in which case it lives in a stack slot. A struct, an enum or a nullable with a tag
//! lives in memory: a stack slot sized by its layout, or, for an argument and for the result, the
//! memory whose address the caller passed (D9, D10). `Unit` and `Nothing` have no storage.
//! Reading a place in memory yields its address, and writing one copies its bytes (D12);
//! aggregates, nullables, discriminants and `==` over them are expanded inline over their
//! layouts (D11, D13). Operators check `Int` overflow and division by zero in cold blocks that
//! call `prism_rt_panic` at the operator's span (D10 of `codegen-core`); `Double` follows IEEE
//! 754, its `%` calling the runtime (D11). Every call of a generated function loads its target
//! from its slot of the function table (D4), and a read of a top-level value calls its accessor
//! the same way, panicking on the null pointer of a value read during its own initialization
//! (D18). Each body starts by comparing the stack pointer with the runtime's stack limit, raising
//! "stack overflow" below it (D27). A drop of a value that owns storage, and a clone of one, call
//! its type's glue directly ([`crate::glue`], D9 and D10 of `runtime-strings-and-drops`); a drop
//! of any other value does nothing (D14 of `codegen-aggregates`), and its clone is a copy. Strings
//! are built inline from literals and otherwise by the runtime's string functions (D7).

use crate::equality::{Compared, Comparison};
use crate::error::BackendError;
use crate::glue::{allocate_box, free_box, Glue, GlueKind};
use crate::layout::{Layout, Member, Shape, StringField};
use crate::representation::{is_pointer, representation, signature, Representation};
use crate::symbols::{accessor_symbol, callee_symbol};
use crate::unit::{data_global, UnitContext};
use cranelift_codegen::ir::{
    self,
    condcodes::{FloatCC, IntCC},
    types, Block, Function, InstBuilder, MemFlagsData, StackSlotData, StackSlotKind, TrapCode,
    Value,
};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Switch, Variable};
use cranelift_module::{DataId, FuncId, Module};
use prism_ast::Span;
use prism_mir::{
    AggregateKind, BasicBlock, Body, Callee, Constant, LocalId, LocalKind, Operand, Place,
    Projection, Rvalue, StatementKind, TemplatePart, TerminatorKind,
};
use prism_runtime::{
    PanicKind, TEMPLATE_BOOL, TEMPLATE_CHAR, TEMPLATE_DOUBLE, TEMPLATE_INT, TEMPLATE_STRING,
    TEMPLATE_TEXT,
};
use prism_typed_tree::{BinaryOperator, BuiltinFunction, DeclarationPath, Type, UnaryOperator};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

/// The trap that ends a block after a call that never returns; it is never reached.
const UNREACHABLE: TrapCode = match TrapCode::user(1) {
    Some(code) => code,
    None => panic!("1 is a user trap code"),
};

/// Where a local lives.
#[derive(Debug, Clone, Copy)]
enum Storage {
    /// `Unit`, `Nothing`, or a type no use of which reaches the backend.
    Empty,
    Variable(Variable),
    /// A scalar temporary assigned once and used only after that, in the block that assigns
    /// it: its value is kept as is, with no variable for Cranelift's SSA construction to track.
    Value(LocalId),
    /// A value in memory, or a borrowed scalar.
    Slot(ir::StackSlot),
    /// Memory whose address the function received: an argument in memory, or the result.
    Address(Value),
}

/// A value of some MIR type, lowered.
#[derive(Debug, Clone, Copy)]
enum Lowered {
    Empty,
    Scalar(Value),
    /// A value in memory, at the address: what reading a place in memory gives.
    Memory(Value),
}

impl Lowered {
    /// The value as [`crate::equality`] compares it.
    fn compared(self) -> Compared {
        match self {
            Lowered::Empty => Compared::Empty,
            Lowered::Scalar(value) => Compared::Scalar(value),
            Lowered::Memory(address) => Compared::Memory(address),
        }
    }

    fn scalar(self) -> Result<Value, BackendError> {
        match self {
            Lowered::Scalar(value) => Ok(value),
            other => Err(BackendError::new(format!(
                "expected a scalar value, found {:?}",
                other
            ))),
        }
    }
}

/// Where a place is.
#[derive(Debug, Clone, Copy)]
enum Location {
    Empty,
    Variable(Variable),
    /// A local of [`Storage::Value`].
    Value(LocalId),
    /// In memory, at the address.
    Memory(Value),
}

/// Whether two places may share bytes: when they have one root local, or either is reached
/// through a dereference (D12 of `codegen-aggregates`).
fn may_overlap(a: &Place, b: &Place) -> bool {
    a.local == b.local
        || a.projections.contains(&Projection::Deref)
        || b.projections.contains(&Projection::Deref)
}

/// The flags of a load or store of a scalar at its natural alignment, which every field, payload
/// and tag has.
fn aligned() -> MemFlagsData {
    MemFlagsData::trusted()
}

/// The lowering of one body into one function.
pub(crate) struct BodyLowering<'a, 'm, M: Module> {
    context: &'a mut UnitContext<'m, M>,
    body: &'a Body,
    builder: FunctionBuilder<'a>,
    blocks: Vec<Block>,
    /// The MIR blocks lowered into their predecessor's Cranelift block, after it: each is the
    /// only successor of its only predecessor, through a `Goto`, a `Call` or a `Drop`.
    continued: Vec<bool>,
    /// The block a terminator just continued into, lowered next in the same Cranelift block.
    continuation: Option<prism_mir::BlockId>,
    locals: Vec<Storage>,
    /// The value of each local of [`Storage::Value`], once assigned.
    values: Vec<Option<Value>>,
    /// The temporaries that can be of [`Storage::Value`] ([`block_local_temporaries`]).
    block_local: Vec<bool>,
    /// The locals whose address is taken.
    borrowed: HashSet<LocalId>,
    runtime: HashMap<&'static str, ir::FuncRef>,
    /// The glue functions the function calls.
    glue: HashMap<Glue, ir::FuncRef>,
    /// The global value of each data object the function addresses.
    globals: HashMap<DataId, ir::GlobalValue>,
    /// The functions comparisons call: the runtime's string equality and equality glue.
    compared_functions: HashMap<FuncId, ir::FuncRef>,
}

impl<'a, 'm, M: Module> BodyLowering<'a, 'm, M> {
    /// Lowers `body` into `function`, whose signature is the body's.
    pub fn lower(
        context: &'a mut UnitContext<'m, M>,
        body: &'a Body,
        function: &'a mut Function,
        builder_context: &'a mut FunctionBuilderContext,
    ) -> Result<(), BackendError> {
        let builder = FunctionBuilder::new(function, builder_context);
        let mut lowering = BodyLowering {
            context,
            body,
            builder,
            blocks: Vec::new(),
            continued: continued_blocks(body),
            continuation: None,
            locals: Vec::new(),
            values: vec![None; body.locals.len()],
            block_local: block_local_temporaries(body),
            borrowed: borrowed_locals(body),
            runtime: HashMap::new(),
            glue: HashMap::new(),
            globals: HashMap::new(),
            compared_functions: HashMap::new(),
        };
        lowering.lower_body()?;
        let config = lowering.context.module.target_config();
        lowering.builder.seal_all_blocks();
        lowering.builder.finalize(config);
        Ok(())
    }

    fn pointer(&self) -> ir::Type {
        self.context.pointer
    }

    fn types(&self) -> &prism_typed_tree::TypeTable {
        &self.context.file.types
    }

    fn representation(&self, ty: &Type) -> Option<Representation> {
        representation(ty, self.pointer())
    }

    fn is_memory(&self, ty: &Type) -> bool {
        self.representation(ty) == Some(Representation::Memory)
    }

    fn layout(&self, ty: &Type) -> Result<Rc<Layout>, BackendError> {
        self.context.layouts.layout(ty).map_err(|construct| {
            BackendError::new(format!("no layout for {}: {}", ty, construct.description()))
        })
    }

    fn lower_body(&mut self) -> Result<(), BackendError> {
        let entry = self.builder.create_block();
        self.builder.append_block_params_for_function_params(entry);
        self.blocks = self
            .body
            .blocks
            .iter()
            .map(|_| self.builder.create_block())
            .collect();
        self.builder.switch_to_block(entry);
        // The parameters are the result's address when it is in memory, then the arguments in
        // order, those without a representation left out (D10).
        let parameters = self.builder.block_params(entry).to_vec();
        let mut parameters = parameters.into_iter();
        let mut next = || {
            parameters
                .next()
                .ok_or_else(|| BackendError::new("fewer parameters than arguments"))
        };
        let mut received = HashMap::new();
        if self.is_memory(self.body.return_type()) {
            received.insert(LocalId::RETURN, next()?);
        }
        let mut scalars = Vec::new();
        for local in self.body.arguments() {
            match self.representation(&self.body.locals[local].ty) {
                Some(Representation::Memory) => {
                    received.insert(local, next()?);
                }
                Some(Representation::Scalar(_)) => scalars.push((local, next()?)),
                Some(Representation::Empty) | None => {}
            }
        }
        self.declare_locals(&received)?;
        for (local, value) in scalars {
            self.write_place(&Place::local(local), Lowered::Scalar(value), false)?;
        }
        self.check_stack()?;
        self.builder.ins().jump(self.blocks[0], &[]);
        // Each block that starts a Cranelift block, then the blocks it continues into: a
        // straight-line run of calls is one Cranelift block, so Cranelift's SSA construction
        // finds a variable's value without walking a chain of blocks.
        for (index, (_, block)) in self.body.blocks.iter().enumerate() {
            if self.continued[index] {
                continue;
            }
            self.builder.switch_to_block(self.blocks[index]);
            self.lower_block(block)?;
            while let Some(next) = self.continuation.take() {
                self.lower_block(&self.body.blocks[next])?;
            }
        }
        Ok(())
    }

    /// Ends the current block by going to `target`: a jump, or nothing when `target` is
    /// lowered next in the same Cranelift block.
    fn go_to(&mut self, target: prism_mir::BlockId) {
        if self.continued[target.0 as usize] {
            self.continuation = Some(target);
        } else {
            self.builder.ins().jump(self.blocks[target.0 as usize], &[]);
        }
    }

    /// Gives each local its storage: the memory the caller passed for an argument or result in
    /// memory (`received`), a stack slot for any other value in memory and for a borrowed
    /// scalar, otherwise a variable.
    fn declare_locals(&mut self, received: &HashMap<LocalId, Value>) -> Result<(), BackendError> {
        for (local, declaration) in self.body.locals.iter() {
            let storage = match self.representation(&declaration.ty) {
                None | Some(Representation::Empty) => Storage::Empty,
                Some(Representation::Memory) => match received.get(&local) {
                    Some(address) => Storage::Address(*address),
                    None => {
                        let layout = self.layout(&declaration.ty)?;
                        Storage::Slot(self.slot(&layout))
                    }
                },
                // A drop flag lives in memory: it is live from its value's initialization to
                // the end of its scope, and a variable live across many branches would make
                // Cranelift's SSA construction visit every merge once per flag.
                Some(Representation::Scalar(_))
                    if self.borrowed.contains(&local)
                        || declaration.kind == LocalKind::DropFlag =>
                {
                    let layout = self.layout(&declaration.ty)?;
                    Storage::Slot(self.slot(&layout))
                }
                Some(Representation::Scalar(_)) if self.block_local[local.0 as usize] => {
                    Storage::Value(local)
                }
                Some(Representation::Scalar(machine)) => {
                    Storage::Variable(self.builder.declare_var(machine))
                }
            };
            self.locals.push(storage);
        }
        Ok(())
    }

    /// A new stack slot for a value of `layout`.
    fn slot(&mut self, layout: &Layout) -> ir::StackSlot {
        self.builder.create_sized_stack_slot(StackSlotData::new(
            StackSlotKind::ExplicitSlot,
            layout.storage_size(),
            layout.align.trailing_zeros() as u8,
        ))
    }

    /// The address of a new stack slot for a value of `ty`.
    fn temporary(&mut self, ty: &Type) -> Result<Value, BackendError> {
        let layout = self.layout(ty)?;
        let slot = self.slot(&layout);
        let pointer = self.pointer();
        Ok(self.builder.ins().stack_addr(pointer, slot, 0))
    }

    fn lower_block(&mut self, block: &BasicBlock) -> Result<(), BackendError> {
        for statement in &block.statements {
            match &statement.kind {
                StatementKind::Assign(place, rvalue) => {
                    self.assign(place, rvalue, statement.span)?
                }
                // Storage markers and reads only matter to the borrow checker.
                StatementKind::StorageLive(_)
                | StatementKind::StorageDead(_)
                | StatementKind::Read(_) => {}
                // Each listed part is moved into its target: a move is a byte copy, and the
                // source is never read again (D17 of `payload-patterns`).
                StatementKind::Destructure {
                    source,
                    variant,
                    parts,
                } => {
                    for (index, target) in parts {
                        let projection = match variant {
                            Some(variant) => Projection::VariantField {
                                variant: *variant,
                                field: *index,
                            },
                            None => Projection::Field(*index),
                        };
                        let part = Place::local(*source).project(projection);
                        let rvalue = Rvalue::Use(Operand::Move(part, statement.span));
                        self.assign(&Place::local(*target), &rvalue, statement.span)?;
                    }
                }
            }
        }
        self.terminator(&block.terminator.kind, block.terminator.span)
    }

    // Places.

    fn location(&mut self, place: &Place) -> Result<Location, BackendError> {
        let mut ty = self.body.locals[place.local].ty.clone();
        let pointer = self.pointer();
        let mut location = match self.locals[place.local.0 as usize] {
            Storage::Empty => Location::Empty,
            Storage::Variable(variable) => Location::Variable(variable),
            Storage::Value(local) => Location::Value(local),
            Storage::Slot(slot) => {
                Location::Memory(self.builder.ins().stack_addr(pointer, slot, 0))
            }
            Storage::Address(address) => Location::Memory(address),
        };
        for projection in &place.projections {
            match projection {
                Projection::Deref => {
                    let address = self.read_location(location, &ty)?.scalar()?;
                    location = Location::Memory(address);
                    ty = match ty {
                        Type::Reference { inner, .. } | Type::Heap(inner) => *inner,
                        other => {
                            return Err(BackendError::new(format!("dereference of {}", other)))
                        }
                    };
                }
                Projection::Field(index) => {
                    let layout = self.layout(&ty)?;
                    let field = match &layout.shape {
                        Shape::Struct { fields } => fields.get(*index).cloned(),
                        _ => None,
                    }
                    .ok_or_else(|| BackendError::new(format!("no field {} in {}", index, ty)))?;
                    location = self.offset(location, field.offset)?;
                    ty = field.ty;
                }
                Projection::Payload => {
                    let inner = match &ty {
                        Type::Nullable(inner) => inner.as_ref().clone(),
                        other => return Err(BackendError::new(format!("payload of {}", other))),
                    };
                    // A nullable pointer is its payload; a tagged nullable holds it after the tag.
                    if !is_pointer(&inner) {
                        let layout = self.layout(&ty)?;
                        let Shape::Nullable { payload } = &layout.shape else {
                            return Err(BackendError::new(format!("payload of {}", ty)));
                        };
                        location = self.offset(location, payload.offset)?;
                    }
                    ty = inner;
                }
                // Payload `field` of variant `variant`, at its offset in the variant's payload
                // layout, after the tag (D17 of `payload-patterns`).
                Projection::VariantField { variant, field } => {
                    let layout = self.layout(&ty)?;
                    let member = match &layout.shape {
                        Shape::Enum { variants, .. } => variants
                            .get(*variant)
                            .and_then(|v| v.payloads.get(*field))
                            .cloned(),
                        _ => None,
                    }
                    .ok_or_else(|| {
                        BackendError::new(format!(
                            "no payload {} of variant {} in {}",
                            field, variant, ty
                        ))
                    })?;
                    location = self.offset(location, member.offset)?;
                    ty = member.ty;
                }
                Projection::Index(_) => {
                    return Err(BackendError::new("an index projection".to_string()))
                }
            }
        }
        Ok(location)
    }

    /// The location `offset` bytes into the memory location `location`.
    fn offset(&mut self, location: Location, offset: u32) -> Result<Location, BackendError> {
        match location {
            Location::Memory(address) => Ok(Location::Memory(self.add(address, offset))),
            other => Err(BackendError::new(format!(
                "a member of {:?}, which is not in memory",
                other
            ))),
        }
    }

    /// `address + offset`.
    fn add(&mut self, address: Value, offset: u32) -> Value {
        if offset == 0 {
            address
        } else {
            self.builder.ins().iadd_imm_s(address, i64::from(offset))
        }
    }

    fn read_location(&mut self, location: Location, ty: &Type) -> Result<Lowered, BackendError> {
        Ok(match location {
            Location::Empty => Lowered::Empty,
            Location::Variable(variable) => Lowered::Scalar(self.builder.use_var(variable)),
            Location::Value(local) => {
                Lowered::Scalar(self.values[local.0 as usize].ok_or_else(|| {
                    BackendError::new(format!("_{} is read before it is assigned", local.0))
                })?)
            }
            Location::Memory(address) => match self.representation(ty) {
                Some(Representation::Scalar(machine)) => {
                    Lowered::Scalar(self.builder.ins().load(machine, aligned(), address, 0))
                }
                Some(Representation::Memory) => Lowered::Memory(address),
                // A `Unit` referent has nothing to read.
                Some(Representation::Empty) | None => Lowered::Empty,
            },
        })
    }

    fn read_place(&mut self, place: &Place) -> Result<Lowered, BackendError> {
        let ty = place.ty(self.body, self.types());
        let location = self.location(place)?;
        self.read_location(location, &ty)
    }

    /// Writes `value`, of type `ty`, to `location`; a value in memory is copied, with a copy
    /// that allows overlap when `may_overlap` says source and destination may share bytes.
    fn write_location(
        &mut self,
        location: Location,
        ty: &Type,
        value: Lowered,
        may_overlap: bool,
    ) -> Result<(), BackendError> {
        match (location, value) {
            (Location::Empty, _) | (_, Lowered::Empty) => {}
            (Location::Variable(variable), Lowered::Scalar(value)) => {
                self.builder.def_var(variable, value)
            }
            (Location::Value(local), Lowered::Scalar(value)) => {
                self.values[local.0 as usize] = Some(value)
            }
            (Location::Memory(address), Lowered::Scalar(value)) => {
                self.builder.ins().store(aligned(), value, address, 0);
            }
            (Location::Memory(destination), Lowered::Memory(source)) => {
                let layout = self.layout(ty)?;
                self.copy(destination, source, &layout, may_overlap);
            }
            (location, value) => {
                return Err(BackendError::new(format!(
                    "cannot write {:?} to {:?}",
                    value, location
                )))
            }
        }
        Ok(())
    }

    fn write_place(
        &mut self,
        place: &Place,
        value: Lowered,
        may_overlap: bool,
    ) -> Result<(), BackendError> {
        let ty = place.ty(self.body, self.types());
        let location = self.location(place)?;
        self.write_location(location, &ty, value, may_overlap)
    }

    /// Copies the bytes of a value of `layout` from `source` to `destination`: inline loads and
    /// stores for a small value, `memcpy` or, when they may overlap, `memmove` above (D12).
    fn copy(&mut self, destination: Value, source: Value, layout: &Layout, may_overlap: bool) {
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
            !may_overlap,
            flags,
        );
    }

    /// The address of `place`, which is in memory: a value in memory, a borrowed local or a
    /// dereference.
    fn address(&mut self, place: &Place) -> Result<Value, BackendError> {
        match self.location(place)? {
            Location::Memory(address) => Ok(address),
            other => Err(BackendError::new(format!(
                "address of {:?}, which is not in memory",
                other
            ))),
        }
    }

    // Values.

    fn operand_type(&self, operand: &Operand) -> Option<Type> {
        operand.try_ty(self.body, self.types())
    }

    fn operand(&mut self, operand: &Operand, span: Span) -> Result<Lowered, BackendError> {
        Ok(match operand {
            Operand::Copy(place, _) | Operand::Move(place, _) => self.read_place(place)?,
            Operand::Constant(constant) => match constant {
                Constant::Int(value) => {
                    Lowered::Scalar(self.builder.ins().iconst(types::I64, *value))
                }
                Constant::Float(value) => Lowered::Scalar(self.builder.ins().f64const(*value)),
                Constant::Bool(value) => {
                    Lowered::Scalar(self.builder.ins().iconst(types::I8, i64::from(*value)))
                }
                Constant::Char(value) => Lowered::Scalar(
                    self.builder
                        .ins()
                        .iconst(types::I32, i64::from(*value as u32)),
                ),
                Constant::Unit => Lowered::Empty,
                // A literal used as an operand is built in a temporary.
                Constant::String(text) => {
                    let address = self.temporary(&Type::String)?;
                    self.write_string_literal(address, text)?;
                    Lowered::Memory(address)
                }
                Constant::TopLevelValue(path) => Lowered::Scalar(self.top_level_value(path, span)?),
                // A `null` in memory is built in a temporary.
                Constant::Null(ty) if self.is_memory(ty) => {
                    let address = self.temporary(ty)?;
                    self.write_null(Location::Memory(address), ty)?;
                    Lowered::Memory(address)
                }
                Constant::Null(_) => {
                    let pointer = self.pointer();
                    Lowered::Scalar(self.builder.ins().iconst(pointer, 0))
                }
                other => {
                    return Err(BackendError::new(format!(
                        "unsupported constant {:?}",
                        other
                    )))
                }
            },
        })
    }

    /// The operand's value, copied into a temporary when it is in memory and may share bytes
    /// with `destination`, which is written before the value is used up (D12).
    fn operand_apart_from(
        &mut self,
        operand: &Operand,
        destination: &Place,
        span: Span,
    ) -> Result<Lowered, BackendError> {
        let value = self.operand(operand, span)?;
        match (value, operand.place()) {
            (Lowered::Memory(source), Some(place)) if may_overlap(place, destination) => {
                let ty = place.ty(self.body, self.types());
                let temporary = self.temporary(&ty)?;
                let layout = self.layout(&ty)?;
                self.copy(temporary, source, &layout, true);
                Ok(Lowered::Memory(temporary))
            }
            _ => Ok(value),
        }
    }

    /// Writes `null` of the nullable type `ty` to `location`: tag 0, or the null pointer.
    fn write_null(&mut self, location: Location, ty: &Type) -> Result<(), BackendError> {
        if self.is_memory(ty) {
            let zero = self.builder.ins().iconst(types::I8, 0);
            self.write_location(location, &Type::Bool, Lowered::Scalar(zero), false)
        } else {
            let pointer = self.pointer();
            let zero = self.builder.ins().iconst(pointer, 0);
            self.write_location(location, ty, Lowered::Scalar(zero), false)
        }
    }

    /// Writes the string literal `text` to the `String` at `address`: a pointer to the literal's
    /// bytes in the unit's read-only data, its lengths in bytes and in characters, counted here,
    /// and a capacity of 0, since the value owns no buffer (D3 of `runtime-strings-and-drops`).
    fn write_string_literal(&mut self, address: Value, text: &str) -> Result<(), BackendError> {
        let pointer = self.pointer();
        let data = self.context.text(text)?;
        let words = [
            (StringField::Pointer, self.data(data)),
            (
                StringField::ByteLength,
                self.builder.ins().iconst(pointer, text.len() as i64),
            ),
            (
                StringField::CharacterCount,
                self.builder
                    .ins()
                    .iconst(pointer, text.chars().count() as i64),
            ),
            (StringField::Capacity, self.builder.ins().iconst(pointer, 0)),
        ];
        for (field, value) in words {
            let offset = field.offset(pointer) as i32;
            self.builder.ins().store(aligned(), value, address, offset);
        }
        Ok(())
    }

    /// The address of the `String` an operand of type `String`, `&String`, `&&String`, ... holds
    /// or refers to.
    fn string_address(&mut self, operand: &Operand, span: Span) -> Result<Value, BackendError> {
        let ty = self
            .operand_type(operand)
            .ok_or_else(|| BackendError::new("operand of unknown type"))?;
        let pointer = self.pointer();
        let mut address = match self.operand(operand, span)? {
            Lowered::Memory(address) | Lowered::Scalar(address) => address,
            Lowered::Empty => return Err(BackendError::new("a string without a value")),
        };
        // A `&String` is the string's address; each further layer is loaded.
        let mut referent = &ty;
        let mut layers = 0;
        while let Type::Reference { inner, .. } = referent {
            layers += 1;
            referent = inner;
        }
        if *referent != Type::String {
            return Err(BackendError::new(format!("{} is not a string", ty)));
        }
        for _ in 1..layers.max(1) {
            address = self.builder.ins().load(pointer, aligned(), address, 0);
        }
        Ok(address)
    }

    fn assign(&mut self, place: &Place, rvalue: &Rvalue, span: Span) -> Result<(), BackendError> {
        let ty = place.ty(self.body, self.types());
        match rvalue {
            Rvalue::Aggregate(kind, operands) => self.aggregate(place, &ty, kind, operands, span),
            Rvalue::WrapNullable(operand) => {
                let inner = match &ty {
                    Type::Nullable(inner) => inner.as_ref().clone(),
                    other => return Err(BackendError::new(format!("wrap into {}", other))),
                };
                let value = self.operand_apart_from(operand, place, span)?;
                if !self.is_memory(&ty) {
                    // A nullable pointer is its payload.
                    return self.write_place(place, value, false);
                }
                let layout = self.layout(&ty)?;
                let Shape::Nullable { payload } = &layout.shape else {
                    return Err(BackendError::new(format!("wrap into {}", ty)));
                };
                let address = self.address(place)?;
                let one = self.builder.ins().iconst(types::I8, 1);
                self.builder.ins().store(aligned(), one, address, 0);
                let payload_address = self.add(address, payload.offset);
                self.write_location(Location::Memory(payload_address), &inner, value, false)
            }
            Rvalue::Use(Operand::Constant(Constant::Null(_))) => {
                let location = self.location(place)?;
                self.write_null(location, &ty)
            }
            Rvalue::Use(Operand::Constant(Constant::String(text))) => {
                let address = self.address(place)?;
                self.write_string_literal(address, text)
            }
            Rvalue::Binary(BinaryOperator::Add, left, right) if ty == Type::String => {
                // The runtime reads both operands before it writes the result, so the
                // destination may be either of them.
                let left = self.string_address(left, span)?;
                let right = self.string_address(right, span)?;
                let destination = self.address(place)?;
                let function = self.runtime_function("prism_rt_string_concatenate")?;
                self.builder
                    .ins()
                    .call(function, &[destination, left, right]);
                Ok(())
            }
            Rvalue::Template(parts) => self.template(place, parts, span),
            Rvalue::Subtype(operand) => match self.operand_type(operand) {
                // `Nothing?` to any nullable is `null`.
                Some(Type::Nullable(inner)) if *inner == Type::Nothing => {
                    let location = self.location(place)?;
                    self.write_null(location, &ty)
                }
                // `Nothing` has no value.
                Some(Type::Nothing) => Ok(()),
                _ => self.copy_operand(place, operand, span),
            },
            Rvalue::Use(operand) => self.copy_operand(place, operand, span),
            other => {
                let value = self.rvalue(place, other, span)?;
                self.write_place(place, value, false)
            }
        }
    }

    /// A string template into `place` (D13 of `string-templates`): a table of its parts in a
    /// stack slot, 32 bytes each (a kind word, then text's address, byte length and character
    /// count, a string's address, or a scalar's value), then one call of
    /// `prism_rt_string_template`, which measures the parts and allocates the result once.
    fn template(
        &mut self,
        place: &Place,
        parts: &[TemplatePart],
        span: Span,
    ) -> Result<(), BackendError> {
        const PART_SIZE: u32 = 32;
        let pointer = self.pointer();
        let size = u32::try_from(parts.len())
            .ok()
            .and_then(|count| count.checked_mul(PART_SIZE))
            .ok_or_else(|| BackendError::new("a template with too many parts"))?;
        let slot = self.builder.create_sized_stack_slot(StackSlotData::new(
            StackSlotKind::ExplicitSlot,
            size,
            3,
        ));
        let table = self.builder.ins().stack_addr(pointer, slot, 0);
        for (index, part) in parts.iter().enumerate() {
            let offset = (index as u32 * PART_SIZE) as i32;
            let (kind, words): (u64, Vec<Value>) = match part {
                TemplatePart::Text(text) => {
                    let data = self.context.text(text)?;
                    let address = self.data(data);
                    let bytes = self.builder.ins().iconst(types::I64, text.len() as i64);
                    let characters = self
                        .builder
                        .ins()
                        .iconst(types::I64, text.chars().count() as i64);
                    (TEMPLATE_TEXT, vec![address, bytes, characters])
                }
                TemplatePart::Value(operand) => {
                    let ty = self
                        .operand_type(operand)
                        .ok_or_else(|| BackendError::new("template part of unknown type"))?;
                    if matches!(ty, Type::Reference { .. } | Type::String) {
                        let address = self.string_address(operand, span)?;
                        (TEMPLATE_STRING, vec![address])
                    } else {
                        let kind = match ty {
                            Type::Int => TEMPLATE_INT,
                            Type::Float => TEMPLATE_DOUBLE,
                            Type::Bool => TEMPLATE_BOOL,
                            Type::Char => TEMPLATE_CHAR,
                            other => {
                                return Err(BackendError::new(format!(
                                    "a template part of type {}",
                                    other
                                )))
                            }
                        };
                        let Lowered::Scalar(value) = self.operand(operand, span)? else {
                            return Err(BackendError::new("a scalar template part in memory"));
                        };
                        // A `Bool` and a `Char` fill their word, zero-extended.
                        let value = match self.builder.func.dfg.value_type(value) {
                            types::I8 | types::I32 => self.builder.ins().uextend(types::I64, value),
                            _ => value,
                        };
                        (kind, vec![value])
                    }
                }
            };
            let kind = self.builder.ins().iconst(types::I64, kind as i64);
            self.builder.ins().store(aligned(), kind, table, offset);
            for (word, value) in words.into_iter().enumerate() {
                let word_offset = offset + 8 * (word as i32 + 1);
                self.builder
                    .ins()
                    .store(aligned(), value, table, word_offset);
            }
        }
        let count = self.builder.ins().iconst(pointer, parts.len() as i64);
        let destination = self.address(place)?;
        let function = self.runtime_function("prism_rt_string_template")?;
        self.builder
            .ins()
            .call(function, &[destination, table, count]);
        Ok(())
    }

    /// Writes the value of `operand` to `place`.
    fn copy_operand(
        &mut self,
        place: &Place,
        operand: &Operand,
        span: Span,
    ) -> Result<(), BackendError> {
        let value = self.operand(operand, span)?;
        let overlap = operand
            .place()
            .is_some_and(|source| may_overlap(source, place));
        self.write_place(place, value, overlap)
    }

    /// Builds the struct or variant `kind` of type `ty` from `operands` in `place`, every operand
    /// evaluated before any byte of `place` is written (D11, D12).
    fn aggregate(
        &mut self,
        place: &Place,
        ty: &Type,
        kind: &AggregateKind,
        operands: &[Operand],
        span: Span,
    ) -> Result<(), BackendError> {
        let mut values = Vec::new();
        for operand in operands {
            values.push(self.operand_apart_from(operand, place, span)?);
        }
        let layout = self.layout(ty)?;
        let address = self.address(place)?;
        let members: Vec<Member> = match (kind, &layout.shape) {
            (AggregateKind::Struct { .. }, Shape::Struct { fields }) => fields.clone(),
            (AggregateKind::Variant { index, .. }, Shape::Enum { tag, variants }) => {
                let tag = self.builder.ins().iconst(*tag, *index as i64);
                self.builder.ins().store(aligned(), tag, address, 0);
                variants
                    .get(*index)
                    .ok_or_else(|| BackendError::new(format!("no variant {} of {}", index, ty)))?
                    .payloads
                    .clone()
            }
            _ => return Err(BackendError::new(format!("an aggregate of {}", ty))),
        };
        if members.len() != values.len() {
            return Err(BackendError::new(format!(
                "{} operands for the {} members of {}",
                values.len(),
                members.len(),
                ty
            )));
        }
        for (member, value) in members.iter().zip(values) {
            let member_address = self.add(address, member.offset);
            self.write_location(Location::Memory(member_address), &member.ty, value, false)?;
        }
        Ok(())
    }

    fn rvalue(
        &mut self,
        destination: &Place,
        rvalue: &Rvalue,
        span: Span,
    ) -> Result<Lowered, BackendError> {
        match rvalue {
            Rvalue::Use(operand) | Rvalue::Subtype(operand) => self.operand(operand, span),
            Rvalue::Borrow { place, .. } => Ok(Lowered::Scalar(self.address(place)?)),
            Rvalue::Discriminant(place) => Ok(Lowered::Scalar(self.discriminant(place)?)),
            Rvalue::Binary(operator, left, right) => {
                let ty = self
                    .operand_type(left)
                    .ok_or_else(|| BackendError::new("operand of unknown type"))?;
                let left = self.operand(left, span)?;
                let right = self.operand(right, span)?;
                let structural = !matches!(
                    ty,
                    Type::Int | Type::Float | Type::Bool | Type::Char | Type::Unit
                );
                match operator {
                    BinaryOperator::Equal if structural => {
                        Ok(Lowered::Scalar(self.equality(&ty, left, right)?))
                    }
                    BinaryOperator::NotEqual if structural => {
                        let equal = self.equality(&ty, left, right)?;
                        Ok(Lowered::Scalar(self.builder.ins().bxor_imm_s(equal, 1)))
                    }
                    _ => self.binary(*operator, &ty, left, right, span),
                }
            }
            Rvalue::Unary(operator, operand) => {
                let ty = self
                    .operand_type(operand)
                    .ok_or_else(|| BackendError::new("operand of unknown type"))?;
                let value = self.operand(operand, span)?.scalar()?;
                Ok(Lowered::Scalar(match (operator, ty) {
                    (UnaryOperator::Not, _) => self.builder.ins().bxor_imm_s(value, 1),
                    (UnaryOperator::Negate, Type::Float) => self.builder.ins().fneg(value),
                    (UnaryOperator::Negate, _) => {
                        let zero = self.builder.ins().iconst(types::I64, 0);
                        let (result, overflow) = self.builder.ins().ssub_overflow(zero, value);
                        self.check(overflow, PanicKind::IntegerOverflow, "-", span)?;
                        result
                    }
                }))
            }
            other => Err(BackendError::new(format!(
                "unsupported rvalue {:?} for {:?}",
                other, destination
            ))),
        }
    }

    /// The variant index of an enum place, or the tag of a nullable one, as an `Int` (D11).
    fn discriminant(&mut self, place: &Place) -> Result<Value, BackendError> {
        let ty = place.ty(self.body, self.types());
        let tag = match &ty {
            Type::Nullable(inner) if is_pointer(inner) => {
                let value = self.read_place(place)?.scalar()?;
                self.builder.ins().icmp_imm_s(IntCC::NotEqual, value, 0)
            }
            Type::Nullable(_) => {
                let address = self.address(place)?;
                self.builder.ins().load(types::I8, aligned(), address, 0)
            }
            Type::Enum { .. } => {
                let layout = self.layout(&ty)?;
                let Shape::Enum { tag, .. } = layout.shape else {
                    return Err(BackendError::new(format!("discriminant of {}", ty)));
                };
                let address = self.address(place)?;
                self.builder.ins().load(tag, aligned(), address, 0)
            }
            other => return Err(BackendError::new(format!("discriminant of {}", other))),
        };
        Ok(self.builder.ins().uextend(types::I64, tag))
    }

    // Equality.

    /// `left == right` over values of `ty`, as a `Bool`, expanded inline up to the boxes whose
    /// contents compare in equality glue ([`crate::equality`]).
    fn equality(
        &mut self,
        ty: &Type,
        left: Lowered,
        right: Lowered,
    ) -> Result<Value, BackendError> {
        let mut comparison = Comparison::new(
            &mut self.builder,
            &mut *self.context,
            &mut self.compared_functions,
        );
        comparison.equality(ty, left.compared(), right.compared())
    }

    fn binary(
        &mut self,
        operator: BinaryOperator,
        ty: &Type,
        left: Lowered,
        right: Lowered,
        span: Span,
    ) -> Result<Lowered, BackendError> {
        if matches!(ty, Type::Unit) {
            // `()` equals `()`.
            let equal = matches!(
                operator,
                BinaryOperator::Equal
                    | BinaryOperator::LessOrEqual
                    | BinaryOperator::GreaterOrEqual
            );
            return Ok(Lowered::Scalar(
                self.builder.ins().iconst(types::I8, i64::from(equal)),
            ));
        }
        let (left, right) = (left.scalar()?, right.scalar()?);
        let ins = self.builder.ins();
        let value = match (ty, operator) {
            (Type::Float, BinaryOperator::Add) => ins.fadd(left, right),
            (Type::Float, BinaryOperator::Subtract) => ins.fsub(left, right),
            (Type::Float, BinaryOperator::Multiply) => ins.fmul(left, right),
            (Type::Float, BinaryOperator::Divide) => ins.fdiv(left, right),
            (Type::Float, BinaryOperator::Remainder) => {
                let function = self.runtime_function("prism_rt_double_remainder")?;
                let call = self.builder.ins().call(function, &[left, right]);
                self.builder.inst_results(call)[0]
            }
            (Type::Float, comparison) => ins.fcmp(float_condition(comparison)?, left, right),
            (Type::Int, BinaryOperator::Add) => {
                let (result, overflow) = ins.sadd_overflow(left, right);
                self.check(overflow, PanicKind::IntegerOverflow, "+", span)?;
                result
            }
            (Type::Int, BinaryOperator::Subtract) => {
                let (result, overflow) = ins.ssub_overflow(left, right);
                self.check(overflow, PanicKind::IntegerOverflow, "-", span)?;
                result
            }
            (Type::Int, BinaryOperator::Multiply) => {
                let (result, overflow) = ins.smul_overflow(left, right);
                self.check(overflow, PanicKind::IntegerOverflow, "*", span)?;
                result
            }
            (Type::Int, BinaryOperator::Divide) => {
                let zero = ins.icmp_imm_s(IntCC::Equal, right, 0);
                self.check(zero, PanicKind::DivisionByZero, "", span)?;
                let smallest = self.builder.ins().icmp_imm_s(IntCC::Equal, left, i64::MIN);
                let minus_one = self.builder.ins().icmp_imm_s(IntCC::Equal, right, -1);
                let overflow = self.builder.ins().band(smallest, minus_one);
                self.check(overflow, PanicKind::IntegerOverflow, "/", span)?;
                self.builder.ins().sdiv(left, right)
            }
            (Type::Int, BinaryOperator::Remainder) => {
                let zero = ins.icmp_imm_s(IntCC::Equal, right, 0);
                self.check(zero, PanicKind::DivisionByZero, "", span)?;
                // `x % -1` is 0, which `x % 1` also gives without the hardware trap of the
                // smallest `Int` divided by -1.
                let minus_one = self.builder.ins().icmp_imm_s(IntCC::Equal, right, -1);
                let one = self.builder.ins().iconst(types::I64, 1);
                let divisor = self.builder.ins().select(minus_one, one, right);
                self.builder.ins().srem(left, divisor)
            }
            (Type::Int | Type::Char, comparison) => {
                ins.icmp(signed_condition(comparison)?, left, right)
            }
            (Type::Bool, comparison) => ins.icmp(unsigned_condition(comparison)?, left, right),
            (other, operator) => {
                return Err(BackendError::new(format!(
                    "unsupported operator {} on {}",
                    operator.as_str(),
                    other
                )))
            }
        };
        Ok(Lowered::Scalar(value))
    }

    /// The address of the data object `id`.
    fn data(&mut self, id: DataId) -> Value {
        let pointer = self.pointer();
        let global = *self
            .globals
            .entry(id)
            .or_insert_with(|| data_global(self.builder.func, id));
        self.builder.ins().symbol_value(pointer, global)
    }

    // Runtime calls and errors.

    fn runtime_function(&mut self, name: &'static str) -> Result<ir::FuncRef, BackendError> {
        if let Some(function) = self.runtime.get(name) {
            return Ok(*function);
        }
        let id: FuncId = self.context.runtime_function(name)?;
        let function = self
            .context
            .module
            .declare_func_in_func(id, self.builder.func);
        self.runtime.insert(name, function);
        Ok(function)
    }

    /// Calls the glue of `kind` for `ty`, which owns storage, on `arguments`.
    fn call_glue(
        &mut self,
        kind: GlueKind,
        ty: &Type,
        arguments: &[Value],
    ) -> Result<(), BackendError> {
        let glue = self
            .context
            .glue(kind, ty)?
            .ok_or_else(|| BackendError::new(format!("no glue for {}", ty)))?;
        let reference = match self.glue.get(&glue) {
            Some(reference) => *reference,
            None => {
                let reference = self.context.glue_reference(glue, self.builder.func)?;
                self.glue.insert(glue, reference);
                reference
            }
        };
        self.builder.ins().call(reference, arguments);
        Ok(())
    }

    /// Continues in a new block when `condition` is false, and raises the runtime error `kind`
    /// at `span` in a cold block when it is true.
    fn check(
        &mut self,
        condition: Value,
        kind: PanicKind,
        detail: &str,
        span: Span,
    ) -> Result<(), BackendError> {
        let failure = self.builder.create_block();
        let success = self.builder.create_block();
        self.builder.set_cold_block(failure);
        self.builder
            .ins()
            .brif(condition, failure, &[], success, &[]);
        self.builder.switch_to_block(failure);
        self.panic(kind, detail, span)?;
        self.builder.switch_to_block(success);
        Ok(())
    }

    /// Calls `prism_rt_panic` with `kind`, `detail` and the location `span`, which ends the
    /// block.
    fn panic(&mut self, kind: PanicKind, detail: &str, span: Span) -> Result<(), BackendError> {
        let pointer = self.pointer();
        let function = self.runtime_function("prism_rt_panic")?;
        let detail_data = self.context.text(detail)?;
        let (file_data, file_length) = self.context.file_name();
        let kind = self
            .builder
            .ins()
            .iconst(types::I32, i64::from(kind as u32));
        let detail_address = self.data(detail_data);
        let detail_length = self.builder.ins().iconst(pointer, detail.len() as i64);
        let file_address = self.data(file_data);
        let file_length = self.builder.ins().iconst(pointer, file_length as i64);
        let line = self.builder.ins().iconst(types::I32, i64::from(span.line));
        let column = self
            .builder
            .ins()
            .iconst(types::I32, i64::from(span.column));
        self.builder.ins().call(
            function,
            &[
                kind,
                detail_address,
                detail_length,
                file_address,
                file_length,
                line,
                column,
            ],
        );
        self.builder.ins().trap(UNREACHABLE);
        Ok(())
    }

    /// Raises "stack overflow" at the body's declaration when the stack pointer is below the
    /// runtime's stack limit (D27), before the body runs.
    fn check_stack(&mut self) -> Result<(), BackendError> {
        let pointer = self.pointer();
        let limit = self.context.stack_limit()?;
        let limit_address = self.data(limit);
        let limit = self
            .builder
            .ins()
            .load(pointer, MemFlagsData::trusted(), limit_address, 0);
        let stack_pointer = self.builder.ins().get_stack_pointer(pointer);
        let overflow = self
            .builder
            .ins()
            .icmp(IntCC::UnsignedLessThan, stack_pointer, limit);
        self.check(overflow, PanicKind::StackOverflow, "", self.body.span)
    }

    // Calls.

    /// Loads the address in the slot of `symbol`.
    fn slot_target(&mut self, symbol: &str) -> Result<Value, BackendError> {
        let pointer = self.pointer();
        let slot = self.context.slot(symbol)?;
        let address = self.data(slot);
        Ok(self
            .builder
            .ins()
            .load(pointer, MemFlagsData::trusted(), address, 0))
    }

    /// The address of the top-level value `path`, through its accessor, raising "read during
    /// its own initialization" at `span` when the accessor returns null.
    fn top_level_value(
        &mut self,
        path: &DeclarationPath,
        span: Span,
    ) -> Result<Value, BackendError> {
        let pointer = self.pointer();
        let target = self.slot_target(&accessor_symbol(path))?;
        let mut accessor = ir::Signature::new(self.context.call_conv);
        accessor.returns.push(ir::AbiParam::new(pointer));
        let signature = self.builder.import_signature(accessor);
        let call = self.builder.ins().call_indirect(signature, target, &[]);
        let address = self.builder.inst_results(call)[0];
        let null = self.builder.ins().icmp_imm_s(IntCC::Equal, address, 0);
        self.check(
            null,
            PanicKind::ValueReadDuringInitialization,
            &path.name,
            span,
        )?;
        Ok(address)
    }

    #[allow(clippy::too_many_arguments)]
    fn call(
        &mut self,
        callee: &Callee,
        arguments: &[Operand],
        parameters: &[Type],
        destination: &Place,
        target: Option<prism_mir::BlockId>,
        span: Span,
    ) -> Result<(), BackendError> {
        let result_type = destination.ty(self.body, self.types());
        match callee {
            Callee::Function(_) | Callee::Default { .. } => {
                let mut values = Vec::new();
                // A result in memory is written through an address passed first: the
                // destination's own when the callee cannot reach it otherwise, else a temporary
                // copied to the destination after the call (D10).
                let mut result_temporary = None;
                if self.is_memory(&result_type) {
                    let direct = destination.projections.is_empty()
                        && !self.borrowed.contains(&destination.local)
                        && !arguments.iter().any(|argument| {
                            argument.place().map(|p| p.local) == Some(destination.local)
                        });
                    let address = if direct {
                        self.address(destination)?
                    } else {
                        let address = self.temporary(&result_type)?;
                        result_temporary = Some(address);
                        address
                    };
                    values.push(address);
                }
                for argument in arguments {
                    match self.argument(argument, span)? {
                        Lowered::Empty => {}
                        Lowered::Scalar(value) | Lowered::Memory(value) => values.push(value),
                    }
                }
                let symbol = callee_symbol(callee).expect("a generated function has a symbol");
                let target = self.slot_target(&symbol)?;
                let signature = signature(
                    parameters,
                    &result_type,
                    self.context.call_conv,
                    self.pointer(),
                );
                let signature = self.builder.import_signature(signature);
                let call = self.builder.ins().call_indirect(signature, target, &values);
                if let Some(result) = self.builder.inst_results(call).first().copied() {
                    self.write_place(destination, Lowered::Scalar(result), false)?;
                }
                if let Some(temporary) = result_temporary {
                    self.write_place(destination, Lowered::Memory(temporary), false)?;
                }
            }
            Callee::Builtin(BuiltinFunction::Clone)
                if self.context.glue(GlueKind::Clone, &result_type)?.is_some() =>
            {
                // A value that owns storage is cloned by its glue, into the destination, or into
                // memory copied to it when it is a variable (a box).
                let receiver = self.receiver(arguments, span)?;
                match self.location(destination)? {
                    Location::Memory(address) => {
                        self.call_glue(GlueKind::Clone, &result_type, &[address, receiver])?;
                    }
                    _ => {
                        let temporary = self.temporary(&result_type)?;
                        self.call_glue(GlueKind::Clone, &result_type, &[temporary, receiver])?;
                        let clone =
                            self.read_location(Location::Memory(temporary), &result_type)?;
                        self.write_place(destination, clone, false)?;
                    }
                }
            }
            Callee::Builtin(BuiltinFunction::Clone) => {
                // The clone of a value that owns nothing is a copy of the receiver's referent
                // (D15), which may be the destination itself.
                let receiver = self.receiver(arguments, span)?;
                let referent = self.read_location(Location::Memory(receiver), &result_type)?;
                self.write_place(destination, referent, true)?;
            }
            Callee::Builtin(BuiltinFunction::StringLength) => {
                // The stored count of characters (D2 and D4 of `runtime-strings-and-drops`).
                let string = self.receiver(arguments, span)?;
                let offset = StringField::CharacterCount.offset(self.pointer()) as i32;
                let length = self
                    .builder
                    .ins()
                    .load(types::I64, aligned(), string, offset);
                self.write_place(destination, Lowered::Scalar(length), false)?;
            }
            Callee::Builtin(BuiltinFunction::StringCharacter) => {
                let string = self.receiver(arguments, span)?;
                let index = arguments
                    .get(1)
                    .ok_or_else(|| BackendError::new("a string index without an index"))?;
                let index = self.operand(index, span)?.scalar()?;
                let pointer = self.pointer();
                let (file_data, file_length) = self.context.file_name();
                let file = self.data(file_data);
                let file_length = self.builder.ins().iconst(pointer, file_length as i64);
                let line = self.builder.ins().iconst(types::I32, i64::from(span.line));
                let column = self
                    .builder
                    .ins()
                    .iconst(types::I32, i64::from(span.column));
                let function = self.runtime_function("prism_rt_string_character")?;
                let call = self
                    .builder
                    .ins()
                    .call(function, &[string, index, file, file_length, line, column]);
                let character = self.builder.inst_results(call)[0];
                self.write_place(destination, Lowered::Scalar(character), false)?;
            }
            Callee::Builtin(BuiltinFunction::ToString) => {
                let receiver = arguments
                    .first()
                    .ok_or_else(|| BackendError::new("toString without a receiver"))?;
                let name = match parameters.first() {
                    Some(Type::Int) => "prism_rt_int_to_string",
                    Some(Type::Float) => "prism_rt_double_to_string",
                    Some(Type::Bool) => "prism_rt_bool_to_string",
                    Some(Type::Char) => "prism_rt_char_to_string",
                    other => return Err(BackendError::new(format!("toString of {:?}", other))),
                };
                let value = self.operand(receiver, span)?.scalar()?;
                let destination = self.address(destination)?;
                let function = self.runtime_function(name)?;
                self.builder.ins().call(function, &[destination, value]);
            }
            Callee::Builtin(builtin @ (BuiltinFunction::Print | BuiltinFunction::PrintLine)) => {
                let mut values = Vec::new();
                for argument in arguments {
                    if let Lowered::Scalar(value) = self.operand(argument, span)? {
                        values.push(value);
                    }
                }
                if let (Some(value), Some(ty)) = (values.first(), parameters.first()) {
                    let name = match ty {
                        Type::Int => "prism_rt_print_int",
                        Type::Float => "prism_rt_print_double",
                        Type::Bool => "prism_rt_print_bool",
                        Type::Char => "prism_rt_print_char",
                        // A `String` is printed through a shared borrow of it.
                        Type::Reference { inner, .. } if **inner == Type::String => {
                            "prism_rt_print_string"
                        }
                        other => return Err(BackendError::new(format!("print of {}", other))),
                    };
                    let function = self.runtime_function(name)?;
                    self.builder.ins().call(function, &[*value]);
                }
                if *builtin == BuiltinFunction::PrintLine {
                    let function = self.runtime_function("prism_rt_print_newline")?;
                    self.builder.ins().call(function, &[]);
                }
            }
            // The argument is evaluated, then the box allocated and the value moved into it
            // (D3 of `heap-boxes`).
            Callee::Builtin(BuiltinFunction::HeapAllocate) => {
                let Type::Heap(contents) = &result_type else {
                    return Err(BackendError::new(format!(
                        "Heap(...) of type {}",
                        result_type
                    )));
                };
                let argument = arguments
                    .first()
                    .ok_or_else(|| BackendError::new("Heap(...) without an argument"))?;
                let value = self.operand(argument, span)?;
                let layout = self.layout(contents)?;
                let address = allocate_box(self.context, &mut self.builder, &layout)?;
                self.write_location(Location::Memory(address), contents, value, false)?;
                self.write_place(destination, Lowered::Scalar(address), false)?;
            }
            // The contents move out, and their allocation is freed without dropping them.
            Callee::Builtin(BuiltinFunction::Unbox) => {
                let boxed = arguments
                    .first()
                    .ok_or_else(|| BackendError::new("unbox without a receiver"))?;
                let address = self.operand(boxed, span)?.scalar()?;
                let contents = self.read_location(Location::Memory(address), &result_type)?;
                self.write_place(destination, contents, false)?;
                let layout = self.layout(&result_type)?;
                free_box(self.context, &mut self.builder, address, &layout)?;
            }
            Callee::Value(..) => return Err(BackendError::new("a call of a function value")),
        }
        match target {
            Some(target) => self.go_to(target),
            // The callee returns `Nothing`.
            None => {
                self.builder.ins().trap(UNREACHABLE);
            }
        }
        Ok(())
    }

    /// The address of the referent of the receiver borrow that a built-in member receives
    /// first.
    fn receiver(&mut self, arguments: &[Operand], span: Span) -> Result<Value, BackendError> {
        let receiver = arguments
            .first()
            .ok_or_else(|| BackendError::new("a built-in member without a receiver"))?;
        self.operand(receiver, span)?.scalar()
    }

    /// The value passed for `argument`: a scalar, or the address of memory the callee owns for
    /// the call (D10). A moved whole local is passed in place, being dead after the move; any
    /// other value in memory is copied into a fresh temporary, so a write through another
    /// argument cannot change it.
    fn argument(&mut self, argument: &Operand, span: Span) -> Result<Lowered, BackendError> {
        let value = self.operand(argument, span)?;
        match (value, argument) {
            (Lowered::Memory(_), Operand::Move(place, _)) if place.projections.is_empty() => {
                Ok(value)
            }
            (Lowered::Memory(source), Operand::Copy(place, _) | Operand::Move(place, _)) => {
                let ty = place.ty(self.body, self.types());
                let temporary = self.temporary(&ty)?;
                let layout = self.layout(&ty)?;
                self.copy(temporary, source, &layout, false);
                Ok(Lowered::Memory(temporary))
            }
            _ => Ok(value),
        }
    }

    fn terminator(&mut self, terminator: &TerminatorKind, span: Span) -> Result<(), BackendError> {
        match terminator {
            TerminatorKind::Goto(target) => self.go_to(*target),
            TerminatorKind::SwitchInt {
                value,
                targets,
                otherwise,
            } => {
                let value = self.operand(value, span)?.scalar()?;
                let otherwise = self.blocks[otherwise.0 as usize];
                match targets.as_slice() {
                    [(0, target)] => {
                        let target = self.blocks[target.0 as usize];
                        self.builder.ins().brif(value, otherwise, &[], target, &[]);
                    }
                    [(constant, target)] => {
                        let target = self.blocks[target.0 as usize];
                        let equal =
                            self.builder
                                .ins()
                                .icmp_imm_s(IntCC::Equal, value, *constant as i64);
                        self.builder.ins().brif(equal, target, &[], otherwise, &[]);
                    }
                    targets => {
                        let mut switch = Switch::new();
                        for (constant, target) in targets {
                            let width = self.builder.func.dfg.value_type(value).bits();
                            let mask = if width >= 64 {
                                u64::MAX
                            } else {
                                (1 << width) - 1
                            };
                            switch.set_entry(
                                u128::from(*constant & mask),
                                self.blocks[target.0 as usize],
                            );
                        }
                        switch.emit(&mut self.builder, value, otherwise);
                    }
                }
            }
            TerminatorKind::Call {
                callee,
                arguments,
                parameters,
                destination,
                target,
                ..
            } => self.call(callee, arguments, parameters, destination, *target, span)?,
            TerminatorKind::Return => {
                // A result in memory is already in the caller's memory.
                match self.representation(self.body.return_type()) {
                    Some(Representation::Scalar(_)) => {
                        let value = self.read_place(&Place::return_place())?.scalar()?;
                        self.builder.ins().return_(&[value]);
                    }
                    _ => {
                        self.builder.ins().return_(&[]);
                    }
                }
            }
            TerminatorKind::Unreachable => self.panic(PanicKind::Unreachable, "", span)?,
            // A value that owns storage runs its drop glue; any other drop does nothing (D14 of
            // `codegen-aggregates`). Drop flags are tested before the drop, in the MIR.
            TerminatorKind::Drop { place, target } => {
                let ty = place.ty(self.body, self.types());
                if self.context.glue(GlueKind::Drop, &ty)?.is_some() {
                    // Glue takes the value's address: a box in a variable is spilled first.
                    let address = match self.location(place)? {
                        Location::Memory(address) => address,
                        location => {
                            let value = self.read_location(location, &ty)?;
                            let temporary = self.temporary(&ty)?;
                            self.write_location(Location::Memory(temporary), &ty, value, false)?;
                            temporary
                        }
                    };
                    self.call_glue(GlueKind::Drop, &ty, &[address])?;
                }
                self.go_to(*target);
            }
        }
        Ok(())
    }
}

/// The blocks of `body` reached from `bb0` that are the only successor of their only
/// predecessor, through a `Goto`, a `Call` that returns or a `Drop`: those the lowering continues
/// into from their predecessor's Cranelift block. Each continued block's chain starts at a block
/// that is not continued, since a cycle of them would be unreachable.
fn continued_blocks(body: &Body) -> Vec<bool> {
    let count = body.blocks.len();
    // Every block's predecessors, reached or not, once per edge.
    let mut predecessors: Vec<Vec<usize>> = vec![Vec::new(); count];
    for (block, data) in body.blocks.iter() {
        for successor in data.terminator.kind.successors() {
            predecessors[successor.0 as usize].push(block.0 as usize);
        }
    }
    let mut reached = vec![false; count];
    let mut stack = if count == 0 { Vec::new() } else { vec![0usize] };
    while let Some(block) = stack.pop() {
        if reached[block] {
            continue;
        }
        reached[block] = true;
        for successor in body.blocks[prism_mir::BlockId(block as u32)]
            .terminator
            .kind
            .successors()
        {
            stack.push(successor.0 as usize);
        }
    }
    (0..count)
        .map(|block| {
            let [predecessor] = predecessors[block][..] else {
                return false;
            };
            let only_exit = match &body.blocks[prism_mir::BlockId(predecessor as u32)]
                .terminator
                .kind
            {
                TerminatorKind::Goto(target)
                | TerminatorKind::Drop { target, .. }
                | TerminatorKind::Call {
                    target: Some(target),
                    ..
                } => target.0 as usize == block,
                _ => false,
            };
            block != 0 && predecessor != block && reached[block] && only_exit
        })
        .collect()
}

/// The temporaries of `body` that are wholly assigned once, by a statement, and mentioned
/// otherwise only after that assignment and in its block: their value never crosses a block, so
/// they need no Cranelift variable. A temporary read by a later block, written by a call or
/// assigned twice (a loop's) is not one.
fn block_local_temporaries(body: &Body) -> Vec<bool> {
    #[derive(Clone, Copy, PartialEq)]
    enum Seen {
        Never,
        /// Assigned in this block, and maybe used after it there.
        Assigned(usize),
        Disqualified,
    }
    let mut seen = vec![Seen::Never; body.locals.len()];
    fn mention(seen: &mut [Seen], local: LocalId, block: usize, whole_assignment: bool) {
        let entry = &mut seen[local.0 as usize];
        *entry = match *entry {
            Seen::Never if whole_assignment => Seen::Assigned(block),
            Seen::Assigned(assigned) if assigned == block && !whole_assignment => *entry,
            _ => Seen::Disqualified,
        };
    }
    fn operand_place(operand: &Operand) -> Option<&Place> {
        match operand {
            Operand::Copy(place, _) | Operand::Move(place, _) => Some(place),
            Operand::Constant(_) => None,
        }
    }
    for (block, data) in body.blocks.iter() {
        let block = block.0 as usize;
        for statement in &data.statements {
            match &statement.kind {
                StatementKind::Assign(destination, rvalue) => {
                    let mut used: Vec<&Place> = Vec::new();
                    match rvalue.as_ref() {
                        Rvalue::Use(operand)
                        | Rvalue::Unary(_, operand)
                        | Rvalue::WrapNullable(operand)
                        | Rvalue::Subtype(operand) => used.extend(operand_place(operand)),
                        Rvalue::Binary(_, left, right) => {
                            used.extend(operand_place(left));
                            used.extend(operand_place(right));
                        }
                        Rvalue::Aggregate(_, operands) => {
                            used.extend(operands.iter().filter_map(operand_place))
                        }
                        Rvalue::Borrow { place, .. } | Rvalue::Discriminant(place) => {
                            used.push(place)
                        }
                        Rvalue::Closure { captures, .. } => used.extend(captures),
                        Rvalue::Template(_) => {
                            used.extend(rvalue.template_operands().filter_map(operand_place))
                        }
                    }
                    for place in used {
                        mention(&mut seen, place.local, block, false);
                    }
                    mention(
                        &mut seen,
                        destination.local,
                        block,
                        destination.projections.is_empty(),
                    );
                }
                StatementKind::Read(place) => mention(&mut seen, place.local, block, false),
                StatementKind::Destructure { source, parts, .. } => {
                    mention(&mut seen, *source, block, false);
                    for (_, target) in parts {
                        mention(&mut seen, *target, block, true);
                    }
                }
                StatementKind::StorageLive(_) | StatementKind::StorageDead(_) => {}
            }
        }
        match &data.terminator.kind {
            TerminatorKind::SwitchInt { value, .. } => {
                if let Some(place) = operand_place(value) {
                    mention(&mut seen, place.local, block, false);
                }
            }
            TerminatorKind::Call {
                callee,
                arguments,
                destination,
                ..
            } => {
                if let Callee::Value(place, _) = callee {
                    mention(&mut seen, place.local, block, false);
                }
                for place in arguments.iter().filter_map(operand_place) {
                    mention(&mut seen, place.local, block, false);
                }
                seen[destination.local.0 as usize] = Seen::Disqualified;
            }
            TerminatorKind::Drop { place, .. } => mention(&mut seen, place.local, block, false),
            TerminatorKind::Goto(_) | TerminatorKind::Return | TerminatorKind::Unreachable => {}
        }
    }
    body.locals
        .iter()
        .map(|(local, declaration)| {
            declaration.kind == LocalKind::Temporary
                && matches!(seen[local.0 as usize], Seen::Assigned(_))
        })
        .collect()
}

/// The locals whose address is taken: a borrow of the local itself, rather than of what it
/// points at.
fn borrowed_locals(body: &Body) -> HashSet<LocalId> {
    let mut borrowed = HashSet::new();
    for (_, block) in body.blocks.iter() {
        for statement in &block.statements {
            if let StatementKind::Assign(_, rvalue) = &statement.kind {
                if let Rvalue::Borrow { place, .. } = rvalue.as_ref() {
                    if place.projections.first() != Some(&Projection::Deref) {
                        borrowed.insert(place.local);
                    }
                }
            }
        }
    }
    borrowed
}

fn signed_condition(operator: BinaryOperator) -> Result<IntCC, BackendError> {
    Ok(match operator {
        BinaryOperator::Equal => IntCC::Equal,
        BinaryOperator::NotEqual => IntCC::NotEqual,
        BinaryOperator::Less => IntCC::SignedLessThan,
        BinaryOperator::LessOrEqual => IntCC::SignedLessThanOrEqual,
        BinaryOperator::Greater => IntCC::SignedGreaterThan,
        BinaryOperator::GreaterOrEqual => IntCC::SignedGreaterThanOrEqual,
        other => {
            return Err(BackendError::new(format!(
                "{} is not a comparison",
                other.as_str()
            )))
        }
    })
}

fn unsigned_condition(operator: BinaryOperator) -> Result<IntCC, BackendError> {
    Ok(match operator {
        BinaryOperator::Equal => IntCC::Equal,
        BinaryOperator::NotEqual => IntCC::NotEqual,
        BinaryOperator::Less => IntCC::UnsignedLessThan,
        BinaryOperator::LessOrEqual => IntCC::UnsignedLessThanOrEqual,
        BinaryOperator::Greater => IntCC::UnsignedGreaterThan,
        BinaryOperator::GreaterOrEqual => IntCC::UnsignedGreaterThanOrEqual,
        other => {
            return Err(BackendError::new(format!(
                "{} is not a comparison",
                other.as_str()
            )))
        }
    })
}

/// The IEEE comparison of `operator`: ordered, except `!=`, which is true for NaN (D11).
fn float_condition(operator: BinaryOperator) -> Result<FloatCC, BackendError> {
    Ok(match operator {
        BinaryOperator::Equal => FloatCC::Equal,
        BinaryOperator::NotEqual => FloatCC::NotEqual,
        BinaryOperator::Less => FloatCC::LessThan,
        BinaryOperator::LessOrEqual => FloatCC::LessThanOrEqual,
        BinaryOperator::Greater => FloatCC::GreaterThan,
        BinaryOperator::GreaterOrEqual => FloatCC::GreaterThanOrEqual,
        other => {
            return Err(BackendError::new(format!(
                "{} is not a comparison",
                other.as_str()
            )))
        }
    })
}

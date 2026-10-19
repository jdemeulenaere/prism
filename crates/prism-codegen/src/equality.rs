//! Structural equality (D13 of `codegen-aggregates`, D13 of `heap-boxes`, and the linear-time
//! lowering of the `code-generation` spec's Design Rationale).
//!
//! `==` compares the layouts of the two values: fields in declaration order, enum tags then the
//! current variant's payloads, nullables by presence then payload, references by referent, each
//! scalar as its type compares, and strings by the runtime. The parts of a value are compared
//! without branching between them, their results joined by `band`: only an enum's tag and a
//! nullable's presence are branched on, since the inactive payload must not be read. Addresses
//! are never compared.
//!
//! A struct, an enum or a nullable in memory is compared inline only while the function's
//! inline budget lasts: [`INLINE_BUDGET`] comparisons of scalars, strings, tags and presences
//! per `==` and per equality glue function. A part that does not fit is compared by the
//! equality glue of its type, `<module>#equals(<type>)`, a function of the unit whose body
//! expands the type's own shape the same way ([`crate::glue`]). The contents of two boxes
//! holding a struct, an enum or a nullable of one are always compared by glue, since they may
//! reach the box's own type. A function's code is then bounded by the budget and the type's
//! own fields and variants, and its branches by the budget, whatever the size of the types its
//! parts hold, so equality compiles in time linear in the fields and payloads of the compared
//! types. Cranelift's optimizer spends time proportional to the dominator tree's depth on each
//! pure instruction, which made the earlier expansion, one branch per scalar, quadratic; the
//! conjunction of the results is kept short-lived for its register allocator (see
//! `Comparison::join`).
//!
//! The expansion is shared by the bodies' lowering and by equality glue, so it works over a
//! function builder and the unit's context rather than over either lowering.

use crate::error::BackendError;
use crate::layout::{Layout, Member, Shape};
use crate::representation::{is_pointer, representation, Representation};
use crate::unit::UnitContext;
use cranelift_codegen::ir::{self, condcodes::FloatCC, condcodes::IntCC, types, InstBuilder};
use cranelift_codegen::ir::{Block, MemFlagsData, Value};
use cranelift_frontend::{FunctionBuilder, Switch};
use cranelift_module::{FuncId, Module};
use prism_typed_tree::Type;
use std::collections::HashMap;
use std::rc::Rc;

/// How many comparisons of scalars, strings, enum tags and nullable presences one `==`, or one
/// equality glue function, expands inline for its struct, enum and nullable parts; the parts
/// beyond it are compared by calling their equality glue.
pub(crate) const INLINE_BUDGET: usize = 32;

/// How many results are joined in registers before the conjunction is written to the frame.
const FLUSH_INTERVAL: usize = 8;

/// A compared value: a scalar, the address of a value in memory, or nothing (`Unit`).
#[derive(Debug, Clone, Copy)]
pub(crate) enum Compared {
    Empty,
    Scalar(Value),
    Memory(Value),
}

/// A compared value within the expansion: a value in memory is a base address and an offset
/// from it, so that a part is read with the load's offset rather than with address arithmetic.
#[derive(Debug, Clone, Copy)]
enum Operand {
    Empty,
    Scalar(Value),
    Memory { base: Value, offset: i32 },
}

impl From<Compared> for Operand {
    fn from(compared: Compared) -> Self {
        match compared {
            Compared::Empty => Operand::Empty,
            Compared::Scalar(value) => Operand::Scalar(value),
            Compared::Memory(address) => Operand::Memory {
                base: address,
                offset: 0,
            },
        }
    }
}

impl Operand {
    fn scalar(self) -> Result<Value, BackendError> {
        match self {
            Operand::Scalar(value) => Ok(value),
            other => Err(BackendError::new(format!(
                "expected a scalar value, found {:?}",
                other
            ))),
        }
    }

    fn memory(self) -> Result<(Value, i32), BackendError> {
        match self {
            Operand::Memory { base, offset } => Ok((base, offset)),
            other => Err(BackendError::new(format!(
                "expected a value in memory, found {:?}",
                other
            ))),
        }
    }
}

/// Whether the contents of two boxes of `contents` are compared by equality glue rather than
/// inline: a struct, an enum, or a nullable of one, which may reach the box's type again.
pub(crate) fn needs_equality_glue(contents: &Type) -> bool {
    match contents {
        Type::Struct { .. } | Type::Enum { .. } => true,
        Type::Nullable(inner) => matches!(**inner, Type::Struct { .. } | Type::Enum { .. }),
        _ => false,
    }
}

/// Whether `ty` is an aggregate that equality glue can compare: a struct, an enum or a
/// nullable in memory, whose values are passed by address.
fn is_aggregate(ty: &Type) -> bool {
    match ty {
        Type::Struct { .. } | Type::Enum { .. } => true,
        Type::Nullable(inner) => !is_pointer(inner),
        _ => false,
    }
}

/// The comparison of values in one function.
pub(crate) struct Comparison<'r, 'b, 'm, M: Module> {
    builder: &'r mut FunctionBuilder<'b>,
    context: &'r mut UnitContext<'m, M>,
    /// The functions the comparison calls, declared once in the function.
    functions: &'r mut HashMap<FuncId, ir::FuncRef>,
    /// The comparisons the current `==` or glue function may still expand inline.
    budget: usize,
    /// Whether the aggregate being expanded was charged to the budget as a whole, parts
    /// included, so that its parts are expanded inline without charging them again.
    charged: bool,
    /// The conjunction of the results since the last flush, and how many results it joins.
    pending: Option<(Value, usize)>,
    /// The byte holding the conjunction of the flushed results, once there is one.
    flushed: Option<ir::StackSlot>,
    /// Whether the conjunction stays in registers, crossing branches as a block parameter: for
    /// a small comparison without calls, which never flushes.
    in_registers: bool,
}

impl<'r, 'b, 'm, M: Module> Comparison<'r, 'b, 'm, M> {
    pub(crate) fn new(
        builder: &'r mut FunctionBuilder<'b>,
        context: &'r mut UnitContext<'m, M>,
        functions: &'r mut HashMap<FuncId, ir::FuncRef>,
    ) -> Self {
        Comparison {
            builder,
            context,
            functions,
            budget: INLINE_BUDGET,
            charged: false,
            pending: None,
            flushed: None,
            in_registers: false,
        }
    }

    fn layout(&self, ty: &Type) -> Result<Rc<Layout>, BackendError> {
        self.context.layouts.layout(ty).map_err(|construct| {
            BackendError::new(format!("no layout for {}: {}", ty, construct.description()))
        })
    }

    /// A reference to `id` in the function, which is not colocated with it (see
    /// `UnitContext::glue_reference`).
    fn function(&mut self, id: FuncId) -> ir::FuncRef {
        if let Some(reference) = self.functions.get(&id) {
            return *reference;
        }
        let reference = self
            .context
            .module
            .declare_func_in_func(id, self.builder.func);
        self.builder.func.dfg.ext_funcs[reference].colocated = false;
        self.functions.insert(id, reference);
        reference
    }

    /// `left == right` over values of `ty`, as a `Bool`: expanded inline within the budget, or
    /// a call of the equality glue of `ty` when it does not fit.
    pub(crate) fn equality(
        &mut self,
        ty: &Type,
        left: Compared,
        right: Compared,
    ) -> Result<Value, BackendError> {
        self.start(ty)?;
        self.part(ty, left.into(), right.into())?;
        Ok(self.result())
    }

    /// The body of the equality glue of `ty`: the comparison of the values at the two
    /// addresses, `ty`'s own shape expanded inline and its parts within the budget.
    pub(crate) fn glue_body(
        &mut self,
        ty: &Type,
        left: Value,
        right: Value,
    ) -> Result<Value, BackendError> {
        self.start(ty)?;
        self.expand(
            ty,
            Compared::Memory(left).into(),
            Compared::Memory(right).into(),
        )?;
        Ok(self.result())
    }

    /// Starts the comparison of two values of `ty`.
    fn start(&mut self, ty: &Type) -> Result<(), BackendError> {
        self.budget = INLINE_BUDGET;
        self.charged = false;
        self.pending = None;
        self.flushed = None;
        self.in_registers = self.weight(ty, FLUSH_INTERVAL)?.is_some() && !self.calls(ty)?;
        Ok(())
    }

    /// Whether the inline expansion of `ty` calls a function: the runtime's string equality or
    /// equality glue for boxed contents. Asked only of types whose weight is small.
    fn calls(&self, ty: &Type) -> Result<bool, BackendError> {
        match ty {
            Type::String | Type::List(_) | Type::Fn { .. } => Ok(true),
            Type::Heap(inner) if needs_equality_glue(inner) => Ok(true),
            Type::Reference { inner, .. } | Type::Heap(inner) | Type::Nullable(inner) => {
                self.calls(inner)
            }
            Type::Struct { .. } | Type::Enum { .. } => {
                let layout = self.layout(ty)?;
                let parts: Vec<&Member> = match &layout.shape {
                    Shape::Struct { fields } => fields.iter().collect(),
                    Shape::Enum { variants, .. } => variants
                        .iter()
                        .flat_map(|variant| &variant.payloads)
                        .collect(),
                    _ => Vec::new(),
                };
                for part in parts {
                    if self.calls(&part.ty)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            _ => Ok(false),
        }
    }

    // The conjunction of the results.
    //
    // Cranelift's optimizer places a pure instruction just before its first use, so a chain of
    // `band`s whose only use is the final result would be computed at the end, keeping every
    // result, and the loads behind it, live across the rest of the comparison and its calls.
    // Results are therefore joined into a byte of the frame, whose stores stay in place:
    // before every call, before every branch, and after every few results. A small comparison
    // without calls keeps its few results in registers instead.

    /// Joins `equal` to the conjunction of the results.
    fn join(&mut self, equal: Value) {
        let (all, count) = match self.pending {
            Some((all, count)) => (self.builder.ins().band(all, equal), count + 1),
            None => (equal, 1),
        };
        self.pending = Some((all, count));
        if count >= FLUSH_INTERVAL && !self.in_registers {
            self.flush();
        }
    }

    /// Writes the pending results into the conjunction in the frame.
    fn flush(&mut self) {
        let Some((pending, _)) = self.pending.take() else {
            return;
        };
        self.store(pending);
    }

    /// Stores `pending` joined to the flushed conjunction, if there is one, into the frame.
    fn store(&mut self, pending: Value) {
        let pointer = self.context.pointer;
        let (slot, all) = match self.flushed {
            Some(slot) => {
                let flushed = self.builder.ins().stack_load(pointer, types::I8, slot, 0);
                (slot, self.builder.ins().band(flushed, pending))
            }
            None => {
                let slot = self.builder.create_sized_stack_slot(ir::StackSlotData::new(
                    ir::StackSlotKind::ExplicitSlot,
                    1,
                    0,
                ));
                self.flushed = Some(slot);
                (slot, pending)
            }
        };
        self.builder.ins().stack_store(pointer, all, slot, 0);
    }

    /// Starts a branch: returns the conjunction so far, which each path continues, when it is
    /// in registers; puts it in the frame, where each path joins its results, otherwise.
    fn branch(&mut self) -> Option<Value> {
        if self.in_registers {
            return self.pending.take().map(|(pending, _)| pending);
        }
        self.flush();
        if self.flushed.is_none() {
            let one = self.builder.ins().iconst(types::I8, 1);
            self.store(one);
        }
        None
    }

    /// The block where the paths of a branch meet, with a parameter for the conjunction when
    /// it is in registers.
    fn meeting(&mut self) -> Block {
        let block = self.builder.create_block();
        if self.in_registers {
            self.builder.append_block_param(block, types::I8);
        }
        block
    }

    /// Starts a path of a branch in `block`, continuing the conjunction `before`.
    fn start_path(&mut self, block: Block, before: Option<Value>) {
        self.builder.switch_to_block(block);
        self.pending = before.map(|before| (before, 1));
    }

    /// Ends a path of a branch, taking its conjunction to `meeting`.
    fn end_path(&mut self, meeting: Block) {
        if self.in_registers {
            let all = match self.pending.take() {
                Some((all, _)) => all,
                None => self.builder.ins().iconst(types::I8, 1),
            };
            self.builder.ins().jump(meeting, &[all.into()]);
        } else {
            self.flush();
            self.builder.ins().jump(meeting, &[]);
        }
    }

    /// Ends a path of a branch on which the values are unequal.
    fn end_unequal_path(&mut self, meeting: Block) {
        let zero = self.builder.ins().iconst(types::I8, 0);
        self.pending = None;
        if self.in_registers {
            self.builder.ins().jump(meeting, &[zero.into()]);
        } else {
            let slot = self
                .flushed
                .expect("a branch puts the conjunction in the frame");
            let pointer = self.context.pointer;
            self.builder.ins().stack_store(pointer, zero, slot, 0);
            self.builder.ins().jump(meeting, &[]);
        }
    }

    /// Continues after the paths of a branch meet in `meeting`.
    fn meet(&mut self, meeting: Block) {
        self.builder.switch_to_block(meeting);
        self.pending = if self.in_registers {
            Some((self.builder.block_params(meeting)[0], 1))
        } else {
            None
        };
    }

    /// The conjunction of every result: true when there are none.
    fn result(&mut self) -> Value {
        let pending = self.pending.take().map(|(pending, _)| pending);
        let pointer = self.context.pointer;
        let flushed = self
            .flushed
            .map(|slot| self.builder.ins().stack_load(pointer, types::I8, slot, 0));
        match (flushed, pending) {
            (Some(flushed), Some(pending)) => self.builder.ins().band(flushed, pending),
            (Some(all), None) | (None, Some(all)) => all,
            (None, None) => self.builder.ins().iconst(types::I8, 1),
        }
    }

    /// How many comparisons the inline expansion of `ty` makes, or `None` when it makes more
    /// than `limit`. A box whose contents compare in glue counts one, its call. It looks at no
    /// more than `limit` parts, so asking costs no more than the expansion it allows.
    fn weight(&self, ty: &Type, limit: usize) -> Result<Option<usize>, BackendError> {
        let mut weight = 0;
        if self.add_weight(ty, limit, &mut weight)? {
            Ok(Some(weight))
        } else {
            Ok(None)
        }
    }

    /// Adds the weight of `ty` to `weight`, returning false as soon as it exceeds `limit`.
    fn add_weight(
        &self,
        ty: &Type,
        limit: usize,
        weight: &mut usize,
    ) -> Result<bool, BackendError> {
        let count = |weight: &mut usize, step: usize| {
            *weight += step;
            *weight <= limit
        };
        match ty {
            Type::Unit | Type::Nothing => Ok(true),
            Type::Heap(inner) if needs_equality_glue(inner) => Ok(count(weight, 1)),
            // A list compares in its equality glue: one call.
            Type::List(_) => Ok(count(weight, 1)),
            Type::Reference { inner, .. } | Type::Heap(inner) => {
                self.add_weight(inner, limit, weight)
            }
            Type::Nullable(inner) => Ok(count(weight, 1) && self.add_weight(inner, limit, weight)?),
            Type::Struct { .. } | Type::Enum { .. } => {
                let layout = self.layout(ty)?;
                let parts: Box<dyn Iterator<Item = &Member>> = match &layout.shape {
                    Shape::Struct { fields } => Box::new(fields.iter()),
                    Shape::Enum { variants, .. } => {
                        if !count(weight, 1) {
                            return Ok(false);
                        }
                        Box::new(variants.iter().flat_map(|variant| &variant.payloads))
                    }
                    _ => Box::new(std::iter::empty()),
                };
                for part in parts {
                    if !self.add_weight(&part.ty, limit, weight)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            _ => Ok(count(weight, 1)),
        }
    }

    /// Joins the comparison of a part of type `ty`: an aggregate is expanded inline when it
    /// fits in the budget, which it then uses up, and compared by its equality glue otherwise.
    fn part(&mut self, ty: &Type, left: Operand, right: Operand) -> Result<(), BackendError> {
        if self.charged || !is_aggregate(ty) {
            return self.expand(ty, left, right);
        }
        match self.weight(ty, self.budget)? {
            Some(weight) => {
                self.budget -= weight;
                self.charged = true;
                let expanded = self.expand(ty, left, right);
                self.charged = false;
                expanded
            }
            None => {
                let id = self.context.equality_glue(ty)?;
                let (left, right) = (self.address(left)?, self.address(right)?);
                self.call(id, &[left, right])
            }
        }
    }

    /// The address of a value in memory.
    fn address(&mut self, operand: Operand) -> Result<Value, BackendError> {
        let (base, offset) = operand.memory()?;
        Ok(if offset == 0 {
            base
        } else {
            self.builder.ins().iadd_imm_s(base, i64::from(offset))
        })
    }

    /// Calls `id`, after flushing the pending results so that none is live across the call,
    /// and joins its `Bool` result.
    fn call(&mut self, id: FuncId, arguments: &[Value]) -> Result<(), BackendError> {
        self.flush();
        let function = self.function(id);
        let call = self.builder.ins().call(function, arguments);
        let equal = self.builder.inst_results(call)[0];
        self.join(equal);
        Ok(())
    }

    /// The value of type `ty` at `offset` from `base`.
    fn read(&mut self, base: Value, offset: i32, ty: &Type) -> Operand {
        match representation(ty, self.context.pointer) {
            Some(Representation::Scalar(machine)) => Operand::Scalar(self.builder.ins().load(
                machine,
                MemFlagsData::trusted(),
                base,
                offset,
            )),
            Some(Representation::Memory) => Operand::Memory { base, offset },
            Some(Representation::Empty) | None => Operand::Empty,
        }
    }

    /// The value of `member` of the value in memory `operand`.
    fn member(&mut self, operand: Operand, member: &Member) -> Result<Operand, BackendError> {
        let (base, offset) = operand.memory()?;
        let offset = i32::try_from(member.offset)
            .ok()
            .and_then(|member| offset.checked_add(member))
            .ok_or_else(|| BackendError::new("a value too large to compare"))?;
        Ok(self.read(base, offset, &member.ty))
    }

    /// The tag of type `machine` at the start of the value in memory `operand`.
    fn tag(&mut self, operand: Operand, machine: ir::Type) -> Result<Value, BackendError> {
        let (base, offset) = operand.memory()?;
        Ok(self
            .builder
            .ins()
            .load(machine, MemFlagsData::trusted(), base, offset))
    }

    /// Joins the comparisons of `parts` of `left` and `right`.
    fn parts(
        &mut self,
        parts: &[Member],
        left: Operand,
        right: Operand,
    ) -> Result<(), BackendError> {
        for part in parts {
            let left = self.member(left, part)?;
            let right = self.member(right, part)?;
            self.part(&part.ty, left, right)?;
        }
        Ok(())
    }

    /// Joins `left == right` over values of `ty`, the shape of `ty` expanded inline and its
    /// parts compared by [`Self::part`].
    fn expand(&mut self, ty: &Type, left: Operand, right: Operand) -> Result<(), BackendError> {
        match ty {
            Type::Unit | Type::Nothing => {}
            Type::Int | Type::Bool | Type::Char => {
                let equal = self
                    .builder
                    .ins()
                    .icmp(IntCC::Equal, left.scalar()?, right.scalar()?);
                self.join(equal);
            }
            Type::Float => {
                let equal =
                    self.builder
                        .ins()
                        .fcmp(FloatCC::Equal, left.scalar()?, right.scalar()?);
                self.join(equal);
            }
            // Strings compare their text, in the runtime (D11 of `runtime-strings-and-drops`).
            Type::String => {
                let id = self.context.runtime_function("prism_rt_string_equals")?;
                let (left, right) = (self.address(left)?, self.address(right)?);
                self.call(id, &[left, right])?;
            }
            // Lists compare their elements in their equality glue (D15 of `collections`).
            Type::List(_) => {
                let id = self.context.equality_glue(ty)?;
                let (left, right) = (self.address(left)?, self.address(right)?);
                self.call(id, &[left, right])?;
            }
            // The contents of boxes that may reach themselves compare in equality glue, which
            // ends the expansion (D13 of `heap-boxes`).
            Type::Heap(inner) if needs_equality_glue(inner) => {
                let id = self.context.equality_glue(inner)?;
                self.call(id, &[left.scalar()?, right.scalar()?])?;
            }
            // References and other boxes compare what they point at, never their addresses.
            Type::Reference { inner, .. } | Type::Heap(inner) => {
                let left = self.read(left.scalar()?, 0, inner);
                let right = self.read(right.scalar()?, 0, inner);
                self.part(inner, left, right)?;
            }
            Type::Nullable(inner) if is_pointer(inner) => {
                let (left, right) = (left.scalar()?, right.scalar()?);
                let left_present = self.builder.ins().icmp_imm_s(IntCC::NotEqual, left, 0);
                let right_present = self.builder.ins().icmp_imm_s(IntCC::NotEqual, right, 0);
                let same = self
                    .builder
                    .ins()
                    .icmp(IntCC::Equal, left_present, right_present);
                let both = self.builder.ins().band(left_present, right_present);
                self.if_both_present(both, same, |comparison| {
                    comparison.part(inner, Operand::Scalar(left), Operand::Scalar(right))
                })?;
            }
            Type::Nullable(inner) => {
                let layout = self.layout(ty)?;
                let Shape::Nullable { payload } = layout.shape.clone() else {
                    return Err(BackendError::new(format!("a comparison of {}", ty)));
                };
                // Tags are 0 for `null` and 1 otherwise.
                let left_tag = self.tag(left, types::I8)?;
                let right_tag = self.tag(right, types::I8)?;
                let same = self.builder.ins().icmp(IntCC::Equal, left_tag, right_tag);
                let both = self.builder.ins().band(left_tag, right_tag);
                self.if_both_present(both, same, |comparison| {
                    let left = comparison.member(left, &payload)?;
                    let right = comparison.member(right, &payload)?;
                    comparison.part(inner, left, right)
                })?;
            }
            Type::Struct { .. } => {
                let layout = self.layout(ty)?;
                let Shape::Struct { fields } = &layout.shape else {
                    return Err(BackendError::new(format!("a comparison of {}", ty)));
                };
                self.parts(fields, left, right)?;
            }
            Type::Enum { .. } => self.enum_equality(ty, left, right)?,
            // Function values compare by descriptor and environment, in their equality glue
            // (D9 of `compose-transform`).
            Type::Fn { .. } => {
                let id = self.context.equality_glue(ty)?;
                let (left, right) = (self.address(left)?, self.address(right)?);
                self.call(id, &[left, right])?;
            }
            other => return Err(BackendError::new(format!("a comparison of {}", other))),
        }
        Ok(())
    }

    /// Joins `left == right` over enum values: equal tags, then the current variant's
    /// payloads.
    fn enum_equality(
        &mut self,
        ty: &Type,
        left: Operand,
        right: Operand,
    ) -> Result<(), BackendError> {
        let layout = self.layout(ty)?;
        let (tag, variants) = match &layout.shape {
            Shape::Enum { tag, variants } => (*tag, variants),
            // No value of an enum without variants exists.
            Shape::Uninhabited => return Ok(()),
            _ => return Err(BackendError::new(format!("a comparison of {}", ty))),
        };
        let left_tag = self.tag(left, tag)?;
        let right_tag = self.tag(right, tag)?;
        let same = self.builder.ins().icmp(IntCC::Equal, left_tag, right_tag);
        if variants.iter().all(|variant| variant.payloads.is_empty()) {
            self.join(same);
            return Ok(());
        }
        // The tags are equal: compare the payloads of the current variant. Otherwise the
        // values are unequal.
        let dispatch = self.builder.create_block();
        let unequal = self.builder.create_block();
        let without_payloads = self.builder.create_block();
        let done = self.meeting();
        let before = self.branch();
        self.builder.ins().brif(same, dispatch, &[], unequal, &[]);
        self.start_path(unequal, None);
        self.end_unequal_path(done);
        self.builder.switch_to_block(dispatch);
        let mut switch = Switch::new();
        let mut cases = Vec::new();
        for (index, variant) in variants.iter().enumerate() {
            if variant.payloads.is_empty() {
                continue;
            }
            let case = self.builder.create_block();
            switch.set_entry(index as u128, case);
            cases.push((case, variant.payloads.clone()));
        }
        switch.emit(self.builder, left_tag, without_payloads);
        self.start_path(without_payloads, before);
        self.end_path(done);
        for (case, payloads) in cases {
            self.start_path(case, before);
            self.parts(&payloads, left, right)?;
            self.end_path(done);
        }
        self.meet(done);
        Ok(())
    }

    /// Joins `same` when `both` is zero, and the comparison `compare_payloads` joins
    /// otherwise: two `null`s are equal, and a `null` is unequal to a present value.
    fn if_both_present(
        &mut self,
        both: Value,
        same: Value,
        compare_payloads: impl FnOnce(&mut Self) -> Result<(), BackendError>,
    ) -> Result<(), BackendError> {
        let payloads = self.builder.create_block();
        let not_both = self.builder.create_block();
        let done = self.meeting();
        let before = self.branch();
        self.builder.ins().brif(both, payloads, &[], not_both, &[]);
        self.start_path(not_both, before);
        self.join(same);
        self.end_path(done);
        self.start_path(payloads, before);
        compare_payloads(self)?;
        self.end_path(done);
        self.meet(done);
        Ok(())
    }
}

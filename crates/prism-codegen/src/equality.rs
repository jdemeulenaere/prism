//! Structural equality (D13 of `codegen-aggregates`, D13 of `heap-boxes`).
//!
//! `==` is expanded inline over the layouts of the compared values: fields in declaration
//! order, enum tags then the current variant's payloads, nullables by presence then payload,
//! references by referent, each scalar as its type compares, and strings by the runtime. The
//! expansion stops at boxes, whose contents may reach the box's own type: the contents of two
//! boxes holding a struct, an enum or a nullable of one are compared by the equality glue of the
//! contents' type, `<module>#equals(<type>)`, a function of the unit whose body is the same
//! expansion ([`crate::glue`]). Other contents, which cannot reach themselves, are compared inline.
//! Addresses are never compared.
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

/// A compared value: a scalar, the address of a value in memory, or nothing (`Unit`).
#[derive(Debug, Clone, Copy)]
pub(crate) enum Compared {
    Empty,
    Scalar(Value),
    Memory(Value),
}

impl Compared {
    fn scalar(self) -> Result<Value, BackendError> {
        match self {
            Compared::Scalar(value) => Ok(value),
            other => Err(BackendError::new(format!(
                "expected a scalar value, found {:?}",
                other
            ))),
        }
    }

    fn memory(self) -> Result<Value, BackendError> {
        match self {
            Compared::Memory(address) => Ok(address),
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

/// The comparison of values in one function.
pub(crate) struct Comparison<'r, 'b, 'm, M: Module> {
    pub(crate) builder: &'r mut FunctionBuilder<'b>,
    pub(crate) context: &'r mut UnitContext<'m, M>,
    /// The functions the comparison calls, declared once in the function.
    pub(crate) functions: &'r mut HashMap<FuncId, ir::FuncRef>,
}

impl<M: Module> Comparison<'_, '_, '_, M> {
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

    /// `left == right` over values of `ty`, as a `Bool`.
    pub(crate) fn equality(
        &mut self,
        ty: &Type,
        left: Compared,
        right: Compared,
    ) -> Result<Value, BackendError> {
        let result = self.builder.declare_var(types::I8);
        let unequal = self.builder.create_block();
        let done = self.builder.create_block();
        self.compare(ty, left, right, unequal)?;
        let one = self.builder.ins().iconst(types::I8, 1);
        self.builder.def_var(result, one);
        self.builder.ins().jump(done, &[]);
        self.builder.switch_to_block(unequal);
        let zero = self.builder.ins().iconst(types::I8, 0);
        self.builder.def_var(result, zero);
        self.builder.ins().jump(done, &[]);
        self.builder.switch_to_block(done);
        Ok(self.builder.use_var(result))
    }

    /// Continues in a new block when `condition` holds, and jumps to `otherwise` when not.
    fn continue_if(&mut self, condition: Value, otherwise: Block) {
        let next = self.builder.create_block();
        self.builder
            .ins()
            .brif(condition, next, &[], otherwise, &[]);
        self.builder.switch_to_block(next);
    }

    /// The value of type `ty` at `address`.
    fn read(&mut self, address: Value, ty: &Type) -> Compared {
        match representation(ty, self.context.pointer) {
            Some(Representation::Scalar(machine)) => Compared::Scalar(self.builder.ins().load(
                machine,
                MemFlagsData::trusted(),
                address,
                0,
            )),
            Some(Representation::Memory) => Compared::Memory(address),
            Some(Representation::Empty) | None => Compared::Empty,
        }
    }

    /// The value of `member` of the value in memory at `address`.
    fn member(&mut self, address: Value, member: &Member) -> Compared {
        let member_address = if member.offset == 0 {
            address
        } else {
            self.builder
                .ins()
                .iadd_imm_s(address, i64::from(member.offset))
        };
        self.read(member_address, &member.ty)
    }

    /// Emits the comparison of `left` and `right`, of type `ty`, jumping to `unequal` as soon as
    /// they differ and falling through when they are equal.
    pub(crate) fn compare(
        &mut self,
        ty: &Type,
        left: Compared,
        right: Compared,
        unequal: Block,
    ) -> Result<(), BackendError> {
        match ty {
            Type::Unit | Type::Nothing => {}
            Type::Int | Type::Bool | Type::Char => {
                let equal = self
                    .builder
                    .ins()
                    .icmp(IntCC::Equal, left.scalar()?, right.scalar()?);
                self.continue_if(equal, unequal);
            }
            Type::Float => {
                let equal =
                    self.builder
                        .ins()
                        .fcmp(FloatCC::Equal, left.scalar()?, right.scalar()?);
                self.continue_if(equal, unequal);
            }
            // Strings compare their text, in the runtime (D11 of `runtime-strings-and-drops`).
            Type::String => {
                let id = self.context.runtime_function("prism_rt_string_equals")?;
                let function = self.function(id);
                let call = self
                    .builder
                    .ins()
                    .call(function, &[left.memory()?, right.memory()?]);
                let equal = self.builder.inst_results(call)[0];
                self.continue_if(equal, unequal);
            }
            // The contents of boxes that may reach themselves compare in equality glue, which
            // ends the expansion (D13 of `heap-boxes`).
            Type::Heap(inner) if needs_equality_glue(inner) => {
                let id = self.context.equality_glue(inner)?;
                let function = self.function(id);
                let call = self
                    .builder
                    .ins()
                    .call(function, &[left.scalar()?, right.scalar()?]);
                let equal = self.builder.inst_results(call)[0];
                self.continue_if(equal, unequal);
            }
            // References and other boxes compare what they point at, never their addresses.
            Type::Reference { inner, .. } | Type::Heap(inner) => {
                let left = self.read(left.scalar()?, inner);
                let right = self.read(right.scalar()?, inner);
                self.compare(inner, left, right, unequal)?;
            }
            Type::Nullable(inner) if is_pointer(inner) => {
                let (left, right) = (left.scalar()?, right.scalar()?);
                let left_present = self.builder.ins().icmp_imm_s(IntCC::NotEqual, left, 0);
                let right_present = self.builder.ins().icmp_imm_s(IntCC::NotEqual, right, 0);
                let same = self
                    .builder
                    .ins()
                    .icmp(IntCC::Equal, left_present, right_present);
                self.continue_if(same, unequal);
                self.compare_if_present(left_present, |comparison| {
                    comparison.compare(
                        inner,
                        Compared::Scalar(left),
                        Compared::Scalar(right),
                        unequal,
                    )
                })?;
            }
            Type::Nullable(inner) => {
                let (left, right) = (left.memory()?, right.memory()?);
                let layout = self.layout(ty)?;
                let Shape::Nullable { payload } = layout.shape.clone() else {
                    return Err(BackendError::new(format!("a comparison of {}", ty)));
                };
                let flags = MemFlagsData::trusted();
                let left_tag = self.builder.ins().load(types::I8, flags, left, 0);
                let right_tag = self.builder.ins().load(types::I8, flags, right, 0);
                let same = self.builder.ins().icmp(IntCC::Equal, left_tag, right_tag);
                self.continue_if(same, unequal);
                self.compare_if_present(left_tag, |comparison| {
                    let left = comparison.member(left, &payload);
                    let right = comparison.member(right, &payload);
                    comparison.compare(inner, left, right, unequal)
                })?;
            }
            Type::Struct { .. } => {
                let (left, right) = (left.memory()?, right.memory()?);
                let layout = self.layout(ty)?;
                let Shape::Struct { fields } = &layout.shape else {
                    return Err(BackendError::new(format!("a comparison of {}", ty)));
                };
                for field in fields {
                    let left = self.member(left, field);
                    let right = self.member(right, field);
                    self.compare(&field.ty, left, right, unequal)?;
                }
            }
            Type::Enum { .. } => {
                let (left, right) = (left.memory()?, right.memory()?);
                let layout = self.layout(ty)?;
                let (tag, variants) = match &layout.shape {
                    Shape::Enum { tag, variants } => (*tag, variants),
                    // No value of an enum without variants exists.
                    Shape::Uninhabited => return Ok(()),
                    _ => return Err(BackendError::new(format!("a comparison of {}", ty))),
                };
                let flags = MemFlagsData::trusted();
                let left_tag = self.builder.ins().load(tag, flags, left, 0);
                let right_tag = self.builder.ins().load(tag, flags, right, 0);
                let same = self.builder.ins().icmp(IntCC::Equal, left_tag, right_tag);
                self.continue_if(same, unequal);
                if variants.iter().all(|variant| variant.payloads.is_empty()) {
                    return Ok(());
                }
                // The tags are equal: compare the payloads of the current variant.
                let equal = self.builder.create_block();
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
                switch.emit(self.builder, left_tag, equal);
                for (case, payloads) in cases {
                    self.builder.switch_to_block(case);
                    for payload in &payloads {
                        let left = self.member(left, payload);
                        let right = self.member(right, payload);
                        self.compare(&payload.ty, left, right, unequal)?;
                    }
                    self.builder.ins().jump(equal, &[]);
                }
                self.builder.switch_to_block(equal);
            }
            other => return Err(BackendError::new(format!("a comparison of {}", other))),
        }
        Ok(())
    }

    /// Runs `compare_payloads` only when `present` is not zero, continuing in one block either
    /// way: two `null`s are equal.
    fn compare_if_present(
        &mut self,
        present: Value,
        compare_payloads: impl FnOnce(&mut Self) -> Result<(), BackendError>,
    ) -> Result<(), BackendError> {
        let payloads = self.builder.create_block();
        let after = self.builder.create_block();
        self.builder.ins().brif(present, payloads, &[], after, &[]);
        self.builder.switch_to_block(payloads);
        compare_payloads(self)?;
        self.builder.ins().jump(after, &[]);
        self.builder.switch_to_block(after);
        Ok(())
    }
}

//! Lowering one body of elaborated MIR to Cranelift IR (D6 to D11 and D18 of `codegen-core`).
//!
//! A local that is borrowed lives in a stack slot; every other local is a Cranelift variable,
//! from which Cranelift builds SSA, and a `Range` local is three variables (D7). `Unit` and
//! `Nothing` have no storage. Operators check `Int` overflow and division by zero in cold blocks
//! that call `prism_rt_panic` at the operator's span (D10); `Double` follows IEEE 754, its `%`
//! calling the runtime (D11). Every call of a generated function loads its target from its slot
//! of the function table (D4), and a read of a top-level value calls its accessor the same way,
//! panicking on the null pointer of a value read during its own initialization (D18).

use crate::error::BackendError;
use crate::representation::{
    machine_type, memory_layout, representation, signature, Representation,
};
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
    AggregateKind, BasicBlock, Body, Callee, Constant, LocalId, Operand, Place, Projection, Rvalue,
    StatementKind, TerminatorKind,
};
use prism_runtime::PanicKind;
use prism_typed_tree::{
    BinaryOperator, BuiltinFunction, DeclarationPath, NominalId, Type, UnaryOperator,
};
use std::collections::{HashMap, HashSet};

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
    /// A `Range` local: `start`, `end` and `endInclusive`.
    Range([Variable; 3]),
    /// A borrowed local, in memory.
    Slot(ir::StackSlot),
}

/// A value of some MIR type, lowered.
#[derive(Debug, Clone, Copy)]
enum Lowered {
    Empty,
    Scalar(Value),
    Range([Value; 3]),
}

impl Lowered {
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
    Range([Variable; 3]),
    /// In memory, at the address.
    Memory(Value),
}

/// The lowering of one body into one function.
pub(crate) struct BodyLowering<'a, 'm, M: Module> {
    context: &'a mut UnitContext<'m, M>,
    body: &'a Body,
    builder: FunctionBuilder<'a>,
    blocks: Vec<Block>,
    locals: Vec<Storage>,
    runtime: HashMap<&'static str, ir::FuncRef>,
    /// The global value of each data object the function addresses.
    globals: HashMap<DataId, ir::GlobalValue>,
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
            locals: Vec::new(),
            runtime: HashMap::new(),
            globals: HashMap::new(),
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
        self.declare_locals();
        // The arguments arrive as the entry block's parameters, in order, those without a
        // representation left out.
        let parameters = self.builder.block_params(entry).to_vec();
        let mut parameters = parameters.into_iter();
        for local in self.body.arguments() {
            if machine_type(&self.body.locals[local].ty, self.pointer()).is_none() {
                continue;
            }
            let value = parameters
                .next()
                .ok_or_else(|| BackendError::new("fewer parameters than arguments"))?;
            self.write_place(&Place::local(local), Lowered::Scalar(value))?;
        }
        self.builder.ins().jump(self.blocks[0], &[]);
        for (index, (_, block)) in self.body.blocks.iter().enumerate() {
            self.builder.switch_to_block(self.blocks[index]);
            self.lower_block(block)?;
        }
        Ok(())
    }

    /// Gives each local its storage: a stack slot when it is borrowed, otherwise variables.
    fn declare_locals(&mut self) {
        let borrowed = borrowed_locals(self.body);
        let pointer = self.pointer();
        for (local, declaration) in self.body.locals.iter() {
            let storage = match representation(&declaration.ty, pointer) {
                None | Some(Representation::Empty) => Storage::Empty,
                Some(Representation::Range) => Storage::Range([
                    self.builder.declare_var(types::I64),
                    self.builder.declare_var(types::I64),
                    self.builder.declare_var(types::I8),
                ]),
                Some(Representation::Scalar(_)) if borrowed.contains(&local) => {
                    let (size, align) = memory_layout(&declaration.ty, pointer);
                    Storage::Slot(self.builder.create_sized_stack_slot(StackSlotData::new(
                        StackSlotKind::ExplicitSlot,
                        size,
                        align.trailing_zeros() as u8,
                    )))
                }
                Some(Representation::Scalar(machine)) => {
                    Storage::Variable(self.builder.declare_var(machine))
                }
            };
            self.locals.push(storage);
        }
    }

    fn lower_block(&mut self, block: &BasicBlock) -> Result<(), BackendError> {
        for statement in &block.statements {
            match &statement.kind {
                StatementKind::Assign(place, rvalue) => {
                    let value = self.rvalue(place, rvalue, statement.span)?;
                    self.write_place(place, value)?;
                }
                // Storage markers and reads only matter to the borrow checker.
                StatementKind::StorageLive(_)
                | StatementKind::StorageDead(_)
                | StatementKind::Read(_) => {}
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
            Storage::Range(variables) => Location::Range(variables),
            Storage::Slot(slot) => {
                Location::Memory(self.builder.ins().stack_addr(pointer, slot, 0))
            }
        };
        for projection in &place.projections {
            match (projection, location) {
                (Projection::Deref, _) => {
                    let address = self.read_location(location, &ty)?.scalar()?;
                    location = Location::Memory(address);
                    ty = match ty {
                        Type::Reference { inner, .. } => *inner,
                        other => {
                            return Err(BackendError::new(format!("dereference of {}", other)))
                        }
                    };
                }
                (Projection::Field(index), Location::Range(variables)) if *index < 3 => {
                    location = Location::Variable(variables[*index]);
                    ty = if *index == prism_mir::types::RANGE_END_INCLUSIVE {
                        Type::Bool
                    } else {
                        Type::Int
                    };
                }
                (projection, _) => {
                    return Err(BackendError::new(format!(
                        "unsupported projection {:?}",
                        projection
                    )))
                }
            }
        }
        Ok(location)
    }

    fn read_location(&mut self, location: Location, ty: &Type) -> Result<Lowered, BackendError> {
        Ok(match location {
            Location::Empty => Lowered::Empty,
            Location::Variable(variable) => Lowered::Scalar(self.builder.use_var(variable)),
            Location::Range([start, end, inclusive]) => Lowered::Range([
                self.builder.use_var(start),
                self.builder.use_var(end),
                self.builder.use_var(inclusive),
            ]),
            Location::Memory(address) => match machine_type(ty, self.pointer()) {
                Some(machine) => Lowered::Scalar(self.builder.ins().load(
                    machine,
                    MemFlagsData::trusted(),
                    address,
                    0,
                )),
                // A `Unit` referent has nothing to read.
                None => Lowered::Empty,
            },
        })
    }

    fn read_place(&mut self, place: &Place) -> Result<Lowered, BackendError> {
        let ty = place.ty(self.body, self.types());
        let location = self.location(place)?;
        self.read_location(location, &ty)
    }

    fn write_place(&mut self, place: &Place, value: Lowered) -> Result<(), BackendError> {
        match (self.location(place)?, value) {
            (Location::Empty, _) | (_, Lowered::Empty) => {}
            (Location::Variable(variable), Lowered::Scalar(value)) => {
                self.builder.def_var(variable, value)
            }
            (Location::Range(variables), Lowered::Range(values)) => {
                for (variable, value) in variables.into_iter().zip(values) {
                    self.builder.def_var(variable, value);
                }
            }
            (Location::Memory(address), Lowered::Scalar(value)) => {
                self.builder
                    .ins()
                    .store(MemFlagsData::trusted(), value, address, 0);
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

    /// The address of `place`, which is in memory: a borrowed local or a dereference.
    fn address(&mut self, place: &Place) -> Result<Value, BackendError> {
        match self.location(place)? {
            Location::Memory(address) => Ok(address),
            other => Err(BackendError::new(format!(
                "borrow of {:?}, which is not in memory",
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
                Constant::TopLevelValue(path) => Lowered::Scalar(self.top_level_value(path, span)?),
                other => {
                    return Err(BackendError::new(format!(
                        "unsupported constant {:?}",
                        other
                    )))
                }
            },
        })
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
            Rvalue::Binary(operator, left, right) => {
                let ty = self
                    .operand_type(left)
                    .ok_or_else(|| BackendError::new("operand of unknown type"))?;
                let left = self.operand(left, span)?;
                let right = self.operand(right, span)?;
                self.binary(*operator, &ty, left, right, span)
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
            Rvalue::Aggregate(AggregateKind::Struct { id, .. }, operands)
                if *id == NominalId::builtin_range() && operands.len() == 3 =>
            {
                let mut values = [self.builder.ins().iconst(types::I64, 0); 3];
                for (index, operand) in operands.iter().enumerate() {
                    values[index] = self.operand(operand, span)?.scalar()?;
                }
                Ok(Lowered::Range(values))
            }
            other => Err(BackendError::new(format!(
                "unsupported rvalue {:?} for {:?}",
                other, destination
            ))),
        }
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
        let mut values = Vec::new();
        for argument in arguments {
            match self.operand(argument, span)? {
                Lowered::Empty => {}
                Lowered::Scalar(value) => values.push(value),
                Lowered::Range(_) => return Err(BackendError::new("a Range argument")),
            }
        }
        let result_type = destination.ty(self.body, self.types());
        let result = match callee {
            Callee::Function(_) | Callee::Default { .. } => {
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
                self.builder.inst_results(call).first().copied()
            }
            Callee::Builtin(BuiltinFunction::Clone) => {
                // The clone of a scalar is a copy of the receiver's referent.
                let receiver = *values
                    .first()
                    .ok_or_else(|| BackendError::new("clone without a receiver"))?;
                match machine_type(&result_type, self.pointer()) {
                    Some(machine) => Some(self.builder.ins().load(
                        machine,
                        MemFlagsData::trusted(),
                        receiver,
                        0,
                    )),
                    None => None,
                }
            }
            Callee::Builtin(builtin @ (BuiltinFunction::Print | BuiltinFunction::PrintLine)) => {
                if let (Some(value), Some(ty)) = (values.first(), parameters.first()) {
                    let name = match ty {
                        Type::Int => "prism_rt_print_int",
                        Type::Float => "prism_rt_print_double",
                        Type::Bool => "prism_rt_print_bool",
                        Type::Char => "prism_rt_print_char",
                        other => return Err(BackendError::new(format!("print of {}", other))),
                    };
                    let function = self.runtime_function(name)?;
                    self.builder.ins().call(function, &[*value]);
                }
                if *builtin == BuiltinFunction::PrintLine {
                    let function = self.runtime_function("prism_rt_print_newline")?;
                    self.builder.ins().call(function, &[]);
                }
                None
            }
            Callee::Value(..) => return Err(BackendError::new("a call of a function value")),
        };
        if let Some(result) = result {
            self.write_place(destination, Lowered::Scalar(result))?;
        }
        match target {
            Some(target) => {
                self.builder.ins().jump(self.blocks[target.0 as usize], &[]);
            }
            // The callee returns `Nothing`.
            None => {
                self.builder.ins().trap(UNREACHABLE);
            }
        }
        Ok(())
    }

    fn terminator(&mut self, terminator: &TerminatorKind, span: Span) -> Result<(), BackendError> {
        match terminator {
            TerminatorKind::Goto(target) => {
                self.builder.ins().jump(self.blocks[target.0 as usize], &[]);
            }
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
                let result = self.read_place(&Place::return_place())?;
                match result {
                    Lowered::Scalar(value)
                        if machine_type(self.body.return_type(), self.pointer()).is_some() =>
                    {
                        self.builder.ins().return_(&[value]);
                    }
                    _ => {
                        self.builder.ins().return_(&[]);
                    }
                }
            }
            TerminatorKind::Unreachable => self.panic(PanicKind::Unreachable, "", span)?,
            TerminatorKind::Drop { .. } => return Err(BackendError::new("a drop")),
        }
        Ok(())
    }
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

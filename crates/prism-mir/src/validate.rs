//! The structural invariants of a file's MIR (D16 of `mir-lowering`), checked over every passing
//! fixture and example.

use crate::mir::{
    AggregateKind, BasicBlock, BlockId, Body, BodyId, BodyOwner, Callee, Constant, LocalId,
    LocalKind, MirFile, MirPhase, Operand, Place, Projection, Rvalue, StatementKind,
    TerminatorKind,
};
use crate::types::{needs_drop, projection_type};
use prism_typed_tree::{ArenaId, BinaryOperator, Type, TypeTable, UnaryOperator};
use std::collections::VecDeque;

impl MirFile {
    /// Returns the violations of the MIR's structural invariants, empty for well-formed MIR:
    /// every block is reachable and every jump targets a block; locals are ordered (return place,
    /// arguments, captures, then the others) and every local used is declared; projections are
    /// well typed; every assignment's rvalue has its place's type, except a subtyping
    /// conversion, whose operand's type differs from it; copied operands are `Copy`; aggregates
    /// match their fields; a call's operands have its parameter types and it has no successor
    /// exactly when its destination is `Nothing`; a switch reads a `Bool` or an `Int`; no
    /// operand moves a place through a dereference; only values needing a drop are dropped;
    /// captures appear only in closures and are never marked or dropped; drop flags appear only
    /// in elaborated MIR, are `Bool`, are assigned only constants and read only by a switch; and
    /// every other local is storage-live wherever it is used, never marked live twice, and dead
    /// on every path to the return.
    pub fn validate(&self) -> Vec<String> {
        let mut violations = Vec::new();
        for (id, body) in self.bodies.iter() {
            let mut validator = Validator {
                file: self,
                body,
                types: &self.types,
                name: self.body_name(id),
                violations: Vec::new(),
            };
            validator.body(id);
            violations.extend(validator.violations);
        }
        violations
    }
}

struct Validator<'a> {
    file: &'a MirFile,
    body: &'a Body,
    types: &'a TypeTable,
    name: String,
    violations: Vec<String>,
}

impl Validator<'_> {
    fn report(&mut self, message: String) {
        self.violations.push(format!("{}: {}", self.name, message));
    }

    fn body(&mut self, id: BodyId) {
        self.locals(id);
        if self.body.blocks.is_empty() {
            self.report("the body has no block".to_string());
            return;
        }
        for (block, data) in self.body.blocks.iter() {
            for target in data.terminator.kind.successors() {
                if !self.body.blocks.contains(target) {
                    self.report(format!(
                        "bb{} jumps to bb{}, which does not exist",
                        block.0, target.0
                    ));
                }
            }
        }
        if !self.violations.is_empty() {
            return;
        }
        let reachable = self.reachable();
        for (block, reached) in reachable.iter().enumerate() {
            if !reached {
                self.report(format!("bb{} is not reachable from bb0", block));
            }
        }
        for (block, data) in self.body.blocks.iter() {
            self.block(block, data);
        }
        if self.violations.is_empty() {
            self.storage();
        }
    }

    fn locals(&mut self, id: BodyId) {
        let body = self.body;
        let Some(first) = body.locals.get(LocalId::RETURN) else {
            self.report("the body has no return place".to_string());
            return;
        };
        if first.kind != LocalKind::Return {
            self.report("_0 is not the return place".to_string());
        }
        let is_closure = matches!(body.owner, BodyOwner::Closure { .. });
        if body.capture_count > 0 && !is_closure {
            self.report(format!("{} captures outside a closure", body.capture_count));
        }
        if let BodyOwner::Closure { parent, .. } = body.owner {
            if parent >= id {
                self.report(format!(
                    "the closure's parent body {} does not precede it",
                    parent.0
                ));
            }
        }
        for (local, declaration) in body.locals.iter().skip(1) {
            let index = local.index();
            let expected = if index <= body.argument_count {
                Some(LocalKind::Argument)
            } else if index <= body.argument_count + body.capture_count {
                Some(LocalKind::Capture)
            } else {
                None
            };
            let fits = match expected {
                Some(kind) => declaration.kind == kind,
                None => matches!(
                    declaration.kind,
                    LocalKind::User | LocalKind::Temporary | LocalKind::DropFlag
                ),
            };
            if !fits {
                self.report(format!(
                    "_{} is a {:?} local in the position of {}",
                    index,
                    declaration.kind,
                    match expected {
                        Some(kind) => format!("a {:?} local", kind),
                        None => "a binding or temporary".to_string(),
                    }
                ));
            }
        }
        if body.locals.len() <= body.argument_count + body.capture_count {
            self.report("fewer locals than arguments and captures".to_string());
        }
        for (local, declaration) in body.locals.iter() {
            if declaration.kind != LocalKind::DropFlag {
                continue;
            }
            if self.file.phase != MirPhase::Elaborated {
                self.report(format!(
                    "_{} is a drop flag in MIR that is not elaborated",
                    local.0
                ));
            }
            if declaration.ty != Type::Bool {
                self.report(format!(
                    "the drop flag _{} has type {}",
                    local.0, declaration.ty
                ));
            }
        }
    }

    fn is_drop_flag(&self, local: LocalId) -> bool {
        self.body.locals.get(local).map(|l| l.kind) == Some(LocalKind::DropFlag)
    }

    /// Reports each drop flag among `used`, which may only be read by a switch (D12 of
    /// `borrowck-moves`).
    fn check_no_drop_flag(&mut self, used: &[LocalId], at: &str) {
        for local in used {
            if self.is_drop_flag(*local) {
                self.report(format!(
                    "{} uses the drop flag _{} outside a switch",
                    at, local.0
                ));
            }
        }
    }

    fn reachable(&self) -> Vec<bool> {
        let mut reached = vec![false; self.body.blocks.len()];
        let mut queue = VecDeque::from([BlockId(0)]);
        reached[0] = true;
        while let Some(block) = queue.pop_front() {
            for target in self.body.blocks[block].terminator.kind.successors() {
                if !reached[target.index()] {
                    reached[target.index()] = true;
                    queue.push_back(target);
                }
            }
        }
        reached
    }

    /// The type of `place`, reporting an undeclared local or an ill-typed projection.
    fn place_type(&mut self, place: &Place, at: &str) -> Option<Type> {
        let Some(declaration) = self.body.locals.get(place.local) else {
            self.report(format!(
                "{} uses _{}, which is not declared",
                at, place.local.0
            ));
            return None;
        };
        let mut ty = declaration.ty.clone();
        for projection in &place.projections {
            if let Projection::Index(index) = projection {
                match self.body.locals.get(*index) {
                    Some(local) if local.ty == Type::Int => {}
                    Some(local) => {
                        let message =
                            format!("{} indexes by _{} of type {}", at, index.0, local.ty);
                        self.report(message);
                    }
                    None => {
                        self.report(format!(
                            "{} indexes by _{}, which is not declared",
                            at, index.0
                        ));
                        return None;
                    }
                }
            }
            match projection_type(&ty, *projection, self.types) {
                Some(next) => ty = next,
                None => {
                    self.report(format!("{} projects {:?} out of a {}", at, projection, ty));
                    return None;
                }
            }
        }
        Some(ty)
    }

    fn operand_type(&mut self, operand: &Operand, at: &str) -> Option<Type> {
        match operand {
            Operand::Copy(place, _) => {
                let ty = self.place_type(place, at)?;
                if !self.types.is_copy(&ty) {
                    self.report(format!("{} copies a {}, which is not Copy", at, ty));
                }
                Some(ty)
            }
            Operand::Move(place, _) => {
                if place.projections.contains(&Projection::Deref) {
                    self.report(format!("{} moves out of a dereference", at));
                }
                self.place_type(place, at)
            }
            Operand::Constant(constant) => constant.ty(),
        }
    }

    fn expect(&mut self, found: Option<Type>, expected: &Type, what: &str) {
        if let Some(found) = found {
            if &found != expected {
                self.report(format!(
                    "{} has type {}, expected {}",
                    what, found, expected
                ));
            }
        }
    }

    fn block(&mut self, block: BlockId, data: &BasicBlock) {
        self.drop_flags(block, data);
        for (index, statement) in data.statements.iter().enumerate() {
            let at = format!("bb{}[{}]", block.0, index);
            match &statement.kind {
                StatementKind::Assign(place, rvalue) => {
                    if let Some(ty) = self.place_type(place, &at) {
                        self.rvalue(rvalue, &ty, &at);
                    }
                }
                StatementKind::StorageLive(local) | StatementKind::StorageDead(local) => {
                    if !self.body.locals.contains(*local) {
                        self.report(format!("{} marks _{}, which is not declared", at, local.0));
                    } else if self.body.is_unmarked(*local) {
                        let message = format!(
                            "{} marks _{}, a return place, argument or capture",
                            at, local.0
                        );
                        self.report(message);
                    }
                }
                StatementKind::Read(place) => {
                    self.place_type(place, &at);
                }
            }
        }
        let at = format!("bb{}[terminator]", block.0);
        match &data.terminator.kind {
            TerminatorKind::Goto(_) | TerminatorKind::Return | TerminatorKind::Unreachable => {}
            TerminatorKind::SwitchInt { value, .. } => {
                if let Some(ty) = self.operand_type(value, &at) {
                    if ty != Type::Bool && ty != Type::Int {
                        self.report(format!("{} switches on a {}", at, ty));
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
            } => {
                if let Callee::Value(place, _) = callee {
                    let callee_ty = self.place_type(place, &at);
                    match callee_ty {
                        Some(Type::Fn { params, .. }) if &params != parameters => {
                            self.report(format!(
                                "{} calls a function value of other parameters",
                                at
                            ));
                        }
                        Some(Type::Fn { .. }) | None => {}
                        Some(other) => self.report(format!("{} calls a {}", at, other)),
                    }
                }
                if arguments.len() != parameters.len() {
                    let message = format!(
                        "{} passes {} operands for {} parameters",
                        at,
                        arguments.len(),
                        parameters.len()
                    );
                    self.report(message);
                }
                for (index, (argument, parameter)) in arguments.iter().zip(parameters).enumerate() {
                    let ty = self.operand_type(argument, &at);
                    self.expect(ty, parameter, &format!("{} operand {}", at, index));
                }
                if let Some(ty) = self.place_type(destination, &at) {
                    if let Callee::Value(place, _) = callee {
                        if let Some(Type::Fn { ret, .. }) = place.try_ty(self.body, self.types) {
                            self.expect(Some(ty.clone()), &ret, &format!("{} destination", at));
                        }
                    }
                    if (ty == Type::Nothing) != target.is_none() {
                        let message = format!(
                            "{} has a {} destination and {} successor",
                            at,
                            ty,
                            if target.is_some() { "a" } else { "no" }
                        );
                        self.report(message);
                    }
                }
            }
            TerminatorKind::Drop { place, .. } => {
                if let Some(ty) = self.place_type(place, &at) {
                    if !needs_drop(&ty, self.types) {
                        self.report(format!("{} drops a {}, which needs no drop", at, ty));
                    }
                }
                let local = place.local;
                if self.body.locals.get(local).map(|l| l.kind) == Some(LocalKind::Capture) {
                    self.report(format!("{} drops the capture _{}", at, local.0));
                }
            }
        }
    }

    /// The drop flags of `block`: each is assigned only a `Bool` constant, as a whole local, and
    /// read only as the operand of a switch (D12 of `borrowck-moves`).
    fn drop_flags(&mut self, block: BlockId, data: &BasicBlock) {
        for (index, statement) in data.statements.iter().enumerate() {
            let at = format!("bb{}[{}]", block.0, index);
            let mut used = Vec::new();
            match &statement.kind {
                StatementKind::Assign(place, rvalue) => {
                    if self.is_drop_flag(place.local) {
                        let constant = matches!(
                            rvalue.as_ref(),
                            Rvalue::Use(Operand::Constant(Constant::Bool(_)))
                        );
                        if !place.projections.is_empty() || !constant {
                            self.report(format!(
                                "{} assigns the drop flag _{} other than a constant",
                                at, place.local.0
                            ));
                        }
                        used_by_projections(place, &mut used);
                    } else {
                        used_by_place(place, &mut used);
                    }
                    used_by_rvalue(rvalue, &mut used);
                }
                StatementKind::Read(place) => used_by_place(place, &mut used),
                StatementKind::StorageLive(_) | StatementKind::StorageDead(_) => {}
            }
            self.check_no_drop_flag(&used, &at);
        }
        let at = format!("bb{}[terminator]", block.0);
        let mut used = Vec::new();
        match &data.terminator.kind {
            TerminatorKind::SwitchInt { value, .. } => match value {
                Operand::Copy(place, _) if self.is_drop_flag(place.local) => {
                    used_by_projections(place, &mut used)
                }
                _ => used_by_operand(value, &mut used),
            },
            TerminatorKind::Call {
                callee,
                arguments,
                destination,
                ..
            } => {
                if let Callee::Value(place, _) = callee {
                    used_by_place(place, &mut used);
                }
                for argument in arguments {
                    used_by_operand(argument, &mut used);
                }
                used_by_place(destination, &mut used);
            }
            TerminatorKind::Drop { place, .. } => used_by_place(place, &mut used),
            TerminatorKind::Goto(_) | TerminatorKind::Return | TerminatorKind::Unreachable => {}
        }
        self.check_no_drop_flag(&used, &at);
    }

    fn rvalue(&mut self, rvalue: &Rvalue, ty: &Type, at: &str) {
        let what = format!("{} rvalue", at);
        match rvalue {
            Rvalue::Use(operand) => {
                let found = self.operand_type(operand, at);
                self.expect(found, ty, &what);
            }
            Rvalue::Borrow { mutable, place, .. } => {
                if let Some(referent) = self.place_type(place, at) {
                    let found = Type::Reference {
                        mutable: *mutable,
                        inner: Box::new(referent),
                    };
                    self.expect(Some(found), ty, &what);
                }
            }
            Rvalue::Binary(operator, left, right) => {
                let left = self.operand_type(left, at);
                let right = self.operand_type(right, at);
                if let (Some(left), Some(right)) = (&left, &right) {
                    if left != right {
                        self.report(format!(
                            "{} compares or combines a {} and a {}",
                            at, left, right
                        ));
                    }
                }
                let result = match operator {
                    BinaryOperator::Add
                    | BinaryOperator::Subtract
                    | BinaryOperator::Multiply
                    | BinaryOperator::Divide
                    | BinaryOperator::Remainder => match left {
                        // Concatenation reads its non-`Copy` operands through borrows (D14).
                        Some(Type::Reference {
                            mutable: false,
                            inner,
                        }) => Some(*inner),
                        other => other,
                    },
                    _ => Some(Type::Bool),
                };
                self.expect(result, ty, &what);
            }
            Rvalue::Unary(operator, operand) => {
                let found = self.operand_type(operand, at);
                let result = match operator {
                    UnaryOperator::Not => Some(Type::Bool),
                    UnaryOperator::Negate => found,
                };
                self.expect(result, ty, &what);
            }
            Rvalue::Discriminant(place) => {
                if let Some(subject) = self.place_type(place, at) {
                    if !matches!(subject, Type::Enum(_) | Type::Nullable(_)) {
                        self.report(format!("{} reads the discriminant of a {}", at, subject));
                    }
                }
                self.expect(Some(Type::Int), ty, &what);
            }
            Rvalue::Aggregate(kind, operands) => {
                let (found, fields) = match kind {
                    AggregateKind::Struct(id) => (
                        Type::Struct(id.clone()),
                        self.types
                            .struct_fields(id)
                            .map(|f| f.iter().map(|(_, _, ty)| ty.clone()).collect::<Vec<_>>()),
                    ),
                    AggregateKind::Variant { enum_id, index } => (
                        Type::Enum(enum_id.clone()),
                        self.types
                            .enum_variants(enum_id)
                            .and_then(|variants| variants.get(*index))
                            .map(|(_, payloads)| payloads.clone()),
                    ),
                };
                self.expect(Some(found), ty, &what);
                match fields {
                    Some(fields) if fields.len() == operands.len() => {
                        for (index, (operand, field)) in operands.iter().zip(&fields).enumerate() {
                            let found = self.operand_type(operand, at);
                            self.expect(found, field, &format!("{} field {}", at, index));
                        }
                    }
                    Some(fields) => {
                        let message = format!(
                            "{} builds {} fields of an aggregate with {}",
                            at,
                            operands.len(),
                            fields.len()
                        );
                        self.report(message);
                    }
                    None => {
                        for operand in operands {
                            self.operand_type(operand, at);
                        }
                    }
                }
            }
            Rvalue::WrapNullable(operand) => {
                let found = self.operand_type(operand, at).map(Type::nullable);
                self.expect(found, ty, &what);
            }
            Rvalue::Subtype(operand) => {
                if let Some(found) = self.operand_type(operand, at) {
                    if &found == ty {
                        self.report(format!("{} converts a {} to itself", at, ty));
                    }
                }
            }
            Rvalue::Closure { body, captures } => {
                for capture in captures {
                    self.place_type(capture, at);
                }
                match self.file.bodies.get(*body) {
                    Some(closure) if matches!(closure.owner, BodyOwner::Closure { .. }) => {
                        if closure.capture_count != captures.len() {
                            let message = format!(
                                "{} captures {} places for a closure with {} captures",
                                at,
                                captures.len(),
                                closure.capture_count
                            );
                            self.report(message);
                        }
                        if let Type::Fn { params, ret, .. } = ty {
                            let arguments: Vec<Type> = closure
                                .arguments()
                                .map(|l| closure.locals[l].ty.clone())
                                .collect();
                            if &arguments != params || closure.return_type() != ret.as_ref() {
                                self.report(format!("{} builds a closure of another type", at));
                            }
                        } else {
                            self.report(format!("{} builds a closure into a {}", at, ty));
                        }
                    }
                    _ => self.report(format!("{} builds a closure of body {}", at, body.0)),
                }
            }
        }
    }

    /// The forward storage dataflow: a local must be live (on every path) where it is used,
    /// must not be live (on any path) where it is marked live, and must be dead on every path
    /// to a return. The return place, the arguments and the captures are exempt.
    fn storage(&mut self) {
        let body = self.body;
        let count = body.locals.len();
        let blocks = body.blocks.len();
        // Per block entry: (maybe live, must live); `None` until reached.
        let mut entry: Vec<Option<(Vec<bool>, Vec<bool>)>> = vec![None; blocks];
        entry[0] = Some((vec![false; count], vec![false; count]));
        let mut queue = VecDeque::from([BlockId(0)]);
        while let Some(block) = queue.pop_front() {
            let (mut maybe, mut must) = entry[block.index()]
                .clone()
                .expect("queued blocks are reached");
            for statement in &body.blocks[block].statements {
                match &statement.kind {
                    StatementKind::StorageLive(local) => {
                        maybe[local.index()] = true;
                        must[local.index()] = true;
                    }
                    StatementKind::StorageDead(local) => {
                        maybe[local.index()] = false;
                        must[local.index()] = false;
                    }
                    _ => {}
                }
            }
            for target in body.blocks[block].terminator.kind.successors() {
                let changed = match &mut entry[target.index()] {
                    None => {
                        entry[target.index()] = Some((maybe.clone(), must.clone()));
                        true
                    }
                    Some((target_maybe, target_must)) => {
                        let mut changed = false;
                        for i in 0..count {
                            if maybe[i] && !target_maybe[i] {
                                target_maybe[i] = true;
                                changed = true;
                            }
                            if !must[i] && target_must[i] {
                                target_must[i] = false;
                                changed = true;
                            }
                        }
                        changed
                    }
                };
                if changed {
                    queue.push_back(target);
                }
            }
        }
        for (block, data) in body.blocks.iter() {
            let Some((mut maybe, mut must)) = entry[block.index()].clone() else {
                continue;
            };
            for (index, statement) in data.statements.iter().enumerate() {
                let at = format!("bb{}[{}]", block.0, index);
                let mut used = Vec::new();
                match &statement.kind {
                    StatementKind::StorageLive(local) => {
                        if maybe[local.index()] {
                            self.report(format!(
                                "{} marks _{} live while it may be live",
                                at, local.0
                            ));
                        }
                        maybe[local.index()] = true;
                        must[local.index()] = true;
                    }
                    StatementKind::StorageDead(local) => {
                        maybe[local.index()] = false;
                        must[local.index()] = false;
                    }
                    StatementKind::Assign(place, rvalue) => {
                        used_by_place(place, &mut used);
                        used_by_rvalue(rvalue, &mut used);
                    }
                    StatementKind::Read(place) => used_by_place(place, &mut used),
                }
                self.check_live(&used, &must, &at);
            }
            let at = format!("bb{}[terminator]", block.0);
            let mut used = Vec::new();
            match &data.terminator.kind {
                TerminatorKind::SwitchInt { value, .. } => used_by_operand(value, &mut used),
                TerminatorKind::Call {
                    callee,
                    arguments,
                    destination,
                    ..
                } => {
                    if let Callee::Value(place, _) = callee {
                        used_by_place(place, &mut used);
                    }
                    for argument in arguments {
                        used_by_operand(argument, &mut used);
                    }
                    used_by_place(destination, &mut used);
                }
                TerminatorKind::Drop { place, .. } => used_by_place(place, &mut used),
                TerminatorKind::Return => {
                    for (local, live) in maybe.iter().enumerate() {
                        if *live && !body.is_unmarked(LocalId(local as u32)) {
                            self.report(format!("{} returns while _{} may be live", at, local));
                        }
                    }
                }
                TerminatorKind::Goto(_) | TerminatorKind::Unreachable => {}
            }
            self.check_live(&used, &must, &at);
        }
    }

    fn check_live(&mut self, used: &[LocalId], must: &[bool], at: &str) {
        for local in used {
            if !self.body.is_unmarked(*local) && !must[local.index()] {
                self.report(format!("{} uses _{} while it may be dead", at, local.0));
            }
        }
    }
}

fn used_by_place(place: &Place, used: &mut Vec<LocalId>) {
    used.push(place.local);
    used_by_projections(place, used);
}

/// The index locals of `place`'s projections.
fn used_by_projections(place: &Place, used: &mut Vec<LocalId>) {
    for projection in &place.projections {
        if let Projection::Index(local) = projection {
            used.push(*local);
        }
    }
}

fn used_by_operand(operand: &Operand, used: &mut Vec<LocalId>) {
    if let Some(place) = operand.place() {
        used_by_place(place, used);
    }
}

fn used_by_rvalue(rvalue: &Rvalue, used: &mut Vec<LocalId>) {
    match rvalue {
        Rvalue::Use(operand)
        | Rvalue::Unary(_, operand)
        | Rvalue::WrapNullable(operand)
        | Rvalue::Subtype(operand) => used_by_operand(operand, used),
        Rvalue::Binary(_, left, right) => {
            used_by_operand(left, used);
            used_by_operand(right, used);
        }
        Rvalue::Borrow { place, .. } | Rvalue::Discriminant(place) => used_by_place(place, used),
        Rvalue::Aggregate(_, operands) => {
            for operand in operands {
                used_by_operand(operand, used);
            }
        }
        Rvalue::Closure { captures, .. } => {
            for capture in captures {
                used_by_place(capture, used);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::dump::tests::{elaborated_file, local, small_file, statement, terminator};
    use crate::mir::*;
    use prism_typed_tree::{BinaryOperator, Type};

    fn body(file: &mut MirFile) -> &mut Body {
        &mut file.bodies[BodyId(0)]
    }

    fn violations(file: &MirFile) -> String {
        file.validate().join("\n")
    }

    fn assert_reports(file: &MirFile, expected: &str) {
        let violations = violations(file);
        assert!(
            violations.contains(expected),
            "expected a violation containing {:?}, got:\n{}",
            expected,
            violations
        );
    }

    #[test]
    fn a_well_formed_body_has_no_violation() {
        assert_eq!(violations(&small_file()), "");
    }

    #[test]
    fn unreachable_blocks_and_missing_targets_are_reported() {
        let mut file = small_file();
        body(&mut file).blocks.alloc(BasicBlock {
            statements: vec![],
            terminator: terminator(TerminatorKind::Return),
        });
        assert_reports(&file, "bb2 is not reachable from bb0");
        let mut file = small_file();
        body(&mut file).blocks[BlockId(0)].terminator =
            terminator(TerminatorKind::Goto(BlockId(7)));
        assert_reports(&file, "bb0 jumps to bb7, which does not exist");
    }

    #[test]
    fn undeclared_locals_and_bad_projections_are_reported() {
        let mut file = small_file();
        body(&mut file).blocks[BlockId(0)].statements[2] =
            statement(StatementKind::Read(Place::local(LocalId(9))));
        assert_reports(&file, "uses _9, which is not declared");
        let mut file = small_file();
        body(&mut file).blocks[BlockId(0)].statements[2] = statement(StatementKind::Read(
            Place::local(LocalId(1)).project(Projection::Payload),
        ));
        assert_reports(&file, "projects Payload out of a &User");
        let mut file = small_file();
        body(&mut file).blocks[BlockId(0)].statements[2] = statement(StatementKind::Read(
            Place::local(LocalId(2)).project(Projection::Index(LocalId(2))),
        ));
        assert_reports(&file, "indexes by _2 of type String");
    }

    #[test]
    fn ill_typed_rvalues_are_reported() {
        let mut file = small_file();
        body(&mut file).blocks[BlockId(0)].statements[1] = statement(StatementKind::assign(
            Place::local(LocalId(2)),
            Rvalue::Use(Operand::Constant(Constant::Int(1))),
        ));
        assert_reports(&file, "rvalue has type Int, expected String");
        let mut file = small_file();
        body(&mut file).blocks[BlockId(0)].statements[1] = statement(StatementKind::assign(
            Place::local(LocalId(2)),
            Rvalue::Subtype(Operand::Constant(Constant::String("x".to_string()))),
        ));
        assert_reports(&file, "converts a String to itself");
        let mut file = small_file();
        body(&mut file).blocks[BlockId(0)].statements[3] = statement(StatementKind::assign(
            Place::return_place(),
            Rvalue::Use(Operand::Copy(
                Place::local(LocalId(2)),
                prism_ast::Span::default(),
            )),
        ));
        assert_reports(&file, "copies a String, which is not Copy");
        let mut file = small_file();
        body(&mut file).blocks[BlockId(0)].statements[1] = statement(StatementKind::assign(
            Place::local(LocalId(2)),
            Rvalue::Binary(
                BinaryOperator::Add,
                Operand::Constant(Constant::Int(1)),
                Operand::Constant(Constant::Bool(true)),
            ),
        ));
        assert_reports(&file, "combines a Int and a Bool");
    }

    #[test]
    fn bad_terminators_are_reported() {
        let mut file = small_file();
        body(&mut file).blocks[BlockId(1)].terminator = terminator(TerminatorKind::SwitchInt {
            value: Operand::Constant(Constant::String("x".to_string())),
            targets: vec![],
            otherwise: BlockId(1),
        });
        assert_reports(&file, "switches on a String");
        let mut file = small_file();
        body(&mut file).blocks[BlockId(0)].terminator = terminator(TerminatorKind::Drop {
            place: Place::local(LocalId(1)),
            target: BlockId(1),
        });
        assert_reports(&file, "drops a &User, which needs no drop");
        let mut file = small_file();
        body(&mut file).blocks[BlockId(0)].terminator = terminator(TerminatorKind::Call {
            callee: Callee::Function(prism_typed_tree::DeclarationPath::new("main", "g")),
            arguments: vec![Operand::Constant(Constant::Int(1))],
            parameters: vec![Type::String],
            destination: Place::local(LocalId(2)),
            target: None,
            is_compose: false,
        });
        assert_reports(&file, "operand 0 has type Int, expected String");
        assert_reports(&file, "has a String destination and no successor");
    }

    #[test]
    fn local_order_is_checked() {
        let mut file = small_file();
        body(&mut file).locals[LocalId(1)].kind = LocalKind::User;
        assert_reports(
            &file,
            "_1 is a User local in the position of a Argument local",
        );
        let mut file = small_file();
        body(&mut file).capture_count = 1;
        body(&mut file).locals[LocalId(2)].kind = LocalKind::Capture;
        assert_reports(&file, "1 captures outside a closure");
    }

    #[test]
    fn storage_violations_are_reported() {
        // A use before `storage_live`.
        let mut file = small_file();
        body(&mut file).blocks[BlockId(0)].statements.swap(0, 1);
        assert_reports(&file, "bb0[0] uses _2 while it may be dead");
        // Marked live twice.
        let mut file = small_file();
        body(&mut file).blocks[BlockId(1)]
            .statements
            .insert(0, statement(StatementKind::StorageLive(LocalId(2))));
        assert_reports(&file, "marks _2 live while it may be live");
        // Live at the return.
        let mut file = small_file();
        body(&mut file).blocks[BlockId(1)].statements.clear();
        assert_reports(&file, "returns while _2 may be live");
        // Used without ever being marked live.
        let mut file = small_file();
        let user = file.bodies[BodyId(0)].locals[LocalId(1)].ty.clone();
        let b = body(&mut file);
        let flag = b
            .locals
            .alloc(local(Type::Bool, LocalKind::Temporary, None));
        let _ = user;
        b.blocks[BlockId(0)].statements.insert(
            0,
            statement(StatementKind::assign(
                Place::local(flag),
                Rvalue::Use(Operand::Constant(Constant::Bool(true))),
            )),
        );
        assert_reports(&file, "bb0[0] uses _3 while it may be dead");
        // Arguments are never marked.
        let mut file = small_file();
        body(&mut file).blocks[BlockId(0)]
            .statements
            .push(statement(StatementKind::StorageLive(LocalId(1))));
        assert_reports(&file, "marks _1, a return place, argument or capture");
    }

    #[test]
    fn moves_through_a_dereference_are_reported() {
        let mut file = small_file();
        body(&mut file).blocks[BlockId(0)].statements[3] = statement(StatementKind::assign(
            Place::return_place(),
            Rvalue::Use(Operand::Move(
                Place::local(LocalId(1))
                    .project(Projection::Deref)
                    .project(Projection::Field(0)),
                prism_ast::Span::default(),
            )),
        ));
        assert_reports(&file, "bb0[3] moves out of a dereference");
    }

    #[test]
    fn a_well_formed_elaborated_body_has_no_violation() {
        assert_eq!(violations(&elaborated_file()), "");
    }

    #[test]
    fn drop_flag_violations_are_reported() {
        let flag = LocalId(3);
        // A drop flag in MIR that is not elaborated.
        let mut file = elaborated_file();
        file.phase = MirPhase::Built;
        assert_reports(&file, "_3 is a drop flag in MIR that is not elaborated");
        // Not a `Bool`.
        let mut file = elaborated_file();
        body(&mut file).locals[flag].ty = Type::Int;
        assert_reports(&file, "the drop flag _3 has type Int");
        // Storage-marked.
        let mut file = elaborated_file();
        body(&mut file).blocks[BlockId(0)]
            .statements
            .push(statement(StatementKind::StorageLive(flag)));
        assert_reports(&file, "marks _3, a return place, argument or capture");
        // Assigned other than a constant.
        let mut file = elaborated_file();
        body(&mut file).blocks[BlockId(0)].statements[0] = statement(StatementKind::assign(
            Place::local(flag),
            Rvalue::Unary(
                prism_typed_tree::UnaryOperator::Not,
                Operand::Constant(Constant::Bool(true)),
            ),
        ));
        assert_reports(
            &file,
            "bb0[0] assigns the drop flag _3 other than a constant",
        );
        // Read outside a switch.
        let mut file = elaborated_file();
        body(&mut file).blocks[BlockId(0)]
            .statements
            .push(statement(StatementKind::Read(Place::local(flag))));
        assert_reports(&file, "uses the drop flag _3 outside a switch");
        // Dropped.
        let mut file = elaborated_file();
        body(&mut file).blocks[BlockId(2)].terminator = terminator(TerminatorKind::Drop {
            place: Place::local(flag),
            target: BlockId(1),
        });
        assert_reports(
            &file,
            "bb2[terminator] uses the drop flag _3 outside a switch",
        );
    }
}

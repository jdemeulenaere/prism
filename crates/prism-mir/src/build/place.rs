//! Places, operands, straight-line expressions and statements (D5, D6 and D7 of
//! `mir-lowering`).

use super::{Builder, Preallocated};
use crate::mir::{
    AggregateKind, Constant, LocalDeclaration, LocalId, LocalKind, Operand, Place, Projection,
    Rvalue, StatementKind,
};
use prism_ast::{Literal, Span};
use prism_typed_tree::{
    CoercionKind, ConstructKind, ExpressionId, ExpressionKind, NominalId, StatementId,
    StatementKind as TreeStatementKind, TemplatePart, Type,
};

impl Builder<'_> {
    /// Lowers the expression `id` into `destination`, or for its effects only when its value is
    /// discarded (`None`). A discarded value that needs a drop is held in a temporary of the
    /// innermost scope, which drops it.
    pub(super) fn expression_into(&mut self, destination: Option<&Place>, id: ExpressionId) {
        if self.current.is_none() {
            return;
        }
        let expression = self.expression(id);
        let span = expression.span;
        let ty = expression.ty.clone();
        if destination.is_some()
            && matches!(
                expression.kind,
                ExpressionKind::If { .. }
                    | ExpressionKind::When { .. }
                    | ExpressionKind::Elvis { .. }
                    | ExpressionKind::Block { .. }
            )
        {
            self.preallocate_extended_temporaries(id);
        }
        match &expression.kind {
            ExpressionKind::Error { .. } => {
                panic!("internal error: an error node in a tree without type errors")
            }
            ExpressionKind::Literal(literal) => {
                if let Some(destination) = destination {
                    let constant = Self::constant(literal, &ty);
                    self.assign(
                        destination.clone(),
                        Rvalue::Use(Operand::Constant(constant)),
                        span,
                    );
                }
            }
            ExpressionKind::FunctionReference(_) | ExpressionKind::VariantConstructor { .. } => {
                if let Some(destination) = destination {
                    let operand = self.operand(id);
                    self.assign(destination.clone(), Rvalue::Use(operand), span);
                }
            }
            kind if kind.is_place() => {
                let place = self.place(id);
                match destination {
                    None => self.push_statement(StatementKind::Read(place), span),
                    Some(destination) => {
                        if !self.is_copy(&ty) {
                            panic!(
                                "internal error: a {} place read in place is used as a value",
                                ty
                            );
                        }
                        let operand = Operand::Copy(place, span);
                        self.assign(destination.clone(), Rvalue::Use(operand), span);
                    }
                }
            }
            ExpressionKind::Move { .. } | ExpressionKind::Copy { .. } => {
                let operand = self.operand(id);
                let destination = self.destination_or_temporary(destination, &ty, span);
                self.assign(destination, Rvalue::Use(operand), span);
            }
            ExpressionKind::Borrow {
                mutable,
                origin,
                place,
            } => {
                let (mutable, origin, place) = (*mutable, (*origin).into(), *place);
                let place = self.place(place);
                let destination = self.destination_or_temporary(destination, &ty, span);
                let rvalue = Rvalue::Borrow {
                    mutable,
                    origin,
                    place,
                };
                self.assign(destination, rvalue, span);
            }
            ExpressionKind::Coerce { value, kind } => {
                self.lower_coercion(destination, *value, *kind, &ty, span)
            }
            ExpressionKind::Construct {
                kind,
                fields,
                evaluation_order,
            } => {
                let mut order: Vec<usize> = evaluation_order.clone();
                order.extend((0..fields.len()).filter(|index| !evaluation_order.contains(index)));
                let values: Vec<ExpressionId> =
                    order.iter().map(|&index| fields[index].value).collect();
                let mut operands: Vec<Option<Operand>> = vec![None; fields.len()];
                for (index, operand) in order.into_iter().zip(self.ordered_operands(&values)) {
                    operands[index] = Some(operand);
                }
                // The construction's type gives a generic type's arguments.
                let arguments = match &ty {
                    Type::Struct { arguments, .. } | Type::Enum { arguments, .. } => {
                        arguments.clone()
                    }
                    _ => Vec::new(),
                };
                let kind = match kind {
                    ConstructKind::Struct(id) => AggregateKind::Struct {
                        id: id.clone(),
                        arguments,
                    },
                    ConstructKind::Variant { enum_id, index, .. } => AggregateKind::Variant {
                        enum_id: enum_id.clone(),
                        index: *index,
                        arguments,
                    },
                };
                let operands = operands.into_iter().flatten().collect();
                let destination = self.destination_or_temporary(destination, &ty, span);
                self.assign(destination, Rvalue::Aggregate(kind, operands), span);
            }
            ExpressionKind::Range {
                start,
                end,
                end_inclusive,
            } => {
                let mut bounds = self.ordered_operands(&[*start, *end]).into_iter();
                let (start, end) = (bounds.next().unwrap(), bounds.next().unwrap());
                let end_inclusive = Operand::Constant(Constant::Bool(*end_inclusive));
                let destination = self.destination_or_temporary(destination, &ty, span);
                let rvalue = Rvalue::Aggregate(
                    AggregateKind::plain_struct(NominalId::builtin_range()),
                    vec![start, end, end_inclusive],
                );
                self.assign(destination, rvalue, span);
            }
            ExpressionKind::Binary {
                operator,
                left,
                right,
            } => {
                let (operator, left, right) = (*operator, *left, *right);
                self.lower_binary(destination, operator, left, right, &ty, span);
            }
            ExpressionKind::Template { parts } => self.lower_template(destination, parts, span),
            ExpressionKind::Unary { operator, operand } => {
                let operator = *operator;
                let operand = self.operand(*operand);
                let destination = self.destination_or_temporary(destination, &ty, span);
                self.assign(destination, Rvalue::Unary(operator, operand), span);
            }
            ExpressionKind::Block {
                statements,
                tail,
                close_brace,
            } => self.lower_block(destination, statements, *tail, *close_brace, span),
            ExpressionKind::Return { value } => self.lower_return(*value, span),
            ExpressionKind::Call(call) => self.lower_call(destination, call, &ty, span),
            ExpressionKind::SafeNavigation { chain, .. } => {
                self.lower_chain(destination, *chain, &ty, false, span)
            }
            ExpressionKind::Logical {
                operator,
                left,
                right,
            } => self.lower_logical(destination, *operator, *left, *right, span),
            ExpressionKind::Elvis { nullable, fallback } => {
                self.lower_elvis(destination, *nullable, *fallback, &ty, span)
            }
            ExpressionKind::If {
                condition,
                then_branch,
                else_branch,
            } => self.lower_if(destination, *condition, *then_branch, *else_branch, span),
            ExpressionKind::When {
                subject,
                branches,
                exhaustive,
            } => self.lower_when(destination, *subject, branches, *exhaustive, &ty, span),
            ExpressionKind::Break { target } => self.lower_break(*target, span),
            ExpressionKind::Continue { target } => self.lower_continue(*target, span),
            ExpressionKind::Closure { .. } => self.lower_closure(destination, id),
            ExpressionKind::Local(_)
            | ExpressionKind::TopLevelValue(_)
            | ExpressionKind::Field { .. }
            | ExpressionKind::Deref { .. }
            | ExpressionKind::Index { .. }
            | ExpressionKind::NonNullPayload { .. }
            | ExpressionKind::Temporary { .. } => unreachable!("places are handled above"),
        }
    }

    /// `destination`, or a new temporary of type `ty` for a discarded value.
    pub(super) fn destination_or_temporary(
        &mut self,
        destination: Option<&Place>,
        ty: &Type,
        span: Span,
    ) -> Place {
        match destination {
            Some(destination) => destination.clone(),
            None => Place::local(self.new_temporary(ty.clone(), span)),
        }
    }

    /// The place of the expression `id`: a place node's MIR place (D5), or for any other value a
    /// temporary of the innermost scope holding it.
    pub(super) fn place(&mut self, id: ExpressionId) -> Place {
        let expression = self.expression(id);
        let span = expression.span;
        let ty = expression.ty.clone();
        match &expression.kind {
            ExpressionKind::Local(binding) => match self.bindings.get(binding) {
                Some(place) => place.clone(),
                None => panic!("internal error: binding {} has no MIR place", binding),
            },
            ExpressionKind::TopLevelValue(path) => {
                let reference = Type::Reference {
                    mutable: false,
                    inner: Box::new(ty),
                };
                let pointer = self.new_temporary(reference, span);
                let constant = Constant::TopLevelValue(path.clone());
                self.assign(
                    Place::local(pointer),
                    Rvalue::Use(Operand::Constant(constant)),
                    span,
                );
                Self::deref(Place::local(pointer))
            }
            ExpressionKind::Field { base, index, .. } => {
                let index = *index;
                self.place(*base).project(Projection::Field(index))
            }
            ExpressionKind::Deref { pointer } => Self::deref(self.place(*pointer)),
            ExpressionKind::Index { base, index } => {
                let index = *index;
                let base = self.place(*base);
                let index_span = self.expression(index).span;
                let local = self.new_temporary(Type::Int, index_span);
                self.expression_into(Some(&Place::local(local)), index);
                base.project(Projection::Index(local))
            }
            ExpressionKind::NonNullPayload { nullable } => self.lower_payload(*nullable, span),
            ExpressionKind::Temporary { value } => {
                let value = *value;
                match self.preallocated.get(&id).cloned() {
                    Some(Preallocated::Local(local)) => {
                        self.expression_into(Some(&Place::local(local)), value);
                        Place::local(local)
                    }
                    Some(Preallocated::Slot(slot)) => self.fill_slot(slot, value, &ty, span),
                    None => {
                        let local = self.new_temporary(ty, span);
                        self.expression_into(Some(&Place::local(local)), value);
                        Place::local(local)
                    }
                }
            }
            ExpressionKind::Error { .. } => {
                panic!("internal error: an error node in a tree without type errors")
            }
            _ => {
                let local = self.new_temporary(ty, span);
                self.expression_into(Some(&Place::local(local)), id);
                Place::local(local)
            }
        }
    }

    /// The operand of the expression `id` where its value is consumed or read: a constant for a
    /// literal or a function, the move or copy of a consumed place, the copy of a `Copy` place
    /// read in place, or the move out of a temporary holding any other value (D5).
    pub(super) fn operand(&mut self, id: ExpressionId) -> Operand {
        let expression = self.expression(id);
        let span = expression.span;
        let ty = expression.ty.clone();
        match &expression.kind {
            ExpressionKind::Literal(literal) => Operand::Constant(Self::constant(literal, &ty)),
            ExpressionKind::FunctionReference(path) => {
                Operand::Constant(Constant::Function(path.clone()))
            }
            ExpressionKind::VariantConstructor { enum_id, index, .. } => {
                Operand::Constant(Constant::VariantConstructor {
                    enum_id: enum_id.clone(),
                    index: *index,
                })
            }
            ExpressionKind::Move { place } => Operand::Move(self.place(*place), span),
            ExpressionKind::Copy { place } => Operand::Copy(self.place(*place), span),
            ExpressionKind::Coerce {
                value,
                kind: CoercionKind::Subtype,
            } if self.is_null_literal(*value) => Operand::Constant(Constant::Null(ty)),
            kind if kind.is_place() && !matches!(kind, ExpressionKind::Error { .. }) => {
                let is_optional_mutable_reference = matches!(
                    &ty,
                    Type::Nullable(inner)
                        if matches!(inner.as_ref(), Type::Reference { mutable: true, .. })
                );
                if is_optional_mutable_reference {
                    // A `(&mut T)?` lent as a `(&mut T)?` is reborrowed, not moved (D13 of
                    // `borrowck-elision`).
                    let place = self.place(id);
                    let local = self.new_temporary(ty, span);
                    self.lower_optional_reborrow(&Place::local(local), place, span);
                    return Operand::Move(Place::local(local), span);
                }
                if !self.is_copy(&ty) {
                    panic!(
                        "internal error: a {} place read in place is used as a value",
                        ty
                    );
                }
                Operand::Copy(self.place(id), span)
            }
            _ => {
                let local = self.new_temporary(ty, span);
                self.expression_into(Some(&Place::local(local)), id);
                Operand::Move(Place::local(local), span)
            }
        }
    }

    /// The operands of `values`, evaluated in this order (D5). An operand reading a place that
    /// a later value may write (a call, or a block running statements, follows it) is first
    /// copied or moved into a temporary, so that each operand holds the value its place had
    /// when it was evaluated, as rustc's operands do (`n + inc(&mut n)` adds the old `n`).
    pub(super) fn ordered_operands(&mut self, values: &[ExpressionId]) -> Vec<Operand> {
        let later_may_write = self.later_may_write(values);
        let mut operands = Vec::with_capacity(values.len());
        for (position, &value) in values.iter().enumerate() {
            let operand = self.operand(value);
            if later_may_write[position] {
                let expression = self.expression(value);
                let (ty, span) = (expression.ty.clone(), expression.span);
                operands.push(self.stabilize(operand, ty, span));
            } else {
                operands.push(operand);
            }
        }
        operands
    }

    /// For each of `values`, whether one of the values after it may write a place
    /// ([`Self::may_write`]), found in one backward pass, so that ordering many operands stays
    /// linear in their number.
    pub(super) fn later_may_write(&self, values: &[ExpressionId]) -> Vec<bool> {
        let mut later = vec![false; values.len()];
        let mut any = false;
        for (position, &value) in values.iter().enumerate().rev() {
            later[position] = any;
            any = any || self.may_write(value);
        }
        later
    }

    /// `operand`, or, when it reads a place that code running before its use could write (a
    /// place not rooted at a temporary of the lowering, or reached through a reference), a move
    /// out of a temporary holding its current value.
    pub(super) fn stabilize(&mut self, operand: Operand, ty: Type, span: Span) -> Operand {
        let Some(place) = operand.place() else {
            return operand;
        };
        let rooted_at_temporary = self.locals[place.local].kind == LocalKind::Temporary
            && !place.projections.contains(&Projection::Deref);
        if rooted_at_temporary || self.current.is_none() {
            return operand;
        }
        let local = self.new_temporary(ty, span);
        self.assign(Place::local(local), Rvalue::Use(operand), span);
        Operand::Move(Place::local(local), span)
    }

    /// Whether evaluating `id` may write a place: whether it calls a function or runs a
    /// statement (assignments are statements). Creating a closure runs nothing.
    pub(super) fn may_write(&self, id: ExpressionId) -> bool {
        let any = |ids: &[ExpressionId]| ids.iter().any(|&child| self.may_write(child));
        match &self.expression(id).kind {
            ExpressionKind::Call(_) => true,
            ExpressionKind::Block {
                statements, tail, ..
            } => !statements.is_empty() || tail.is_some_and(|tail| self.may_write(tail)),
            ExpressionKind::Literal(_)
            | ExpressionKind::Local(_)
            | ExpressionKind::TopLevelValue(_)
            | ExpressionKind::FunctionReference(_)
            | ExpressionKind::VariantConstructor { .. }
            | ExpressionKind::Closure { .. }
            | ExpressionKind::Break { .. }
            | ExpressionKind::Continue { .. }
            | ExpressionKind::Error { .. } => false,
            ExpressionKind::Field { base: value, .. }
            | ExpressionKind::Deref { pointer: value }
            | ExpressionKind::NonNullPayload { nullable: value }
            | ExpressionKind::Temporary { value }
            | ExpressionKind::Move { place: value }
            | ExpressionKind::Copy { place: value }
            | ExpressionKind::Borrow { place: value, .. }
            | ExpressionKind::SafeNavigation { chain: value, .. }
            | ExpressionKind::Coerce { value, .. }
            | ExpressionKind::Unary { operand: value, .. } => self.may_write(*value),
            ExpressionKind::Index { base, index } => any(&[*base, *index]),
            ExpressionKind::Construct { fields, .. } => {
                fields.iter().any(|field| self.may_write(field.value))
            }
            ExpressionKind::Range { start, end, .. } => any(&[*start, *end]),
            ExpressionKind::Binary { left, right, .. }
            | ExpressionKind::Logical { left, right, .. } => any(&[*left, *right]),
            ExpressionKind::Elvis { nullable, fallback } => any(&[*nullable, *fallback]),
            ExpressionKind::Template { parts } => parts.iter().any(|part| match part {
                TemplatePart::Value(value) => self.may_write(*value),
                TemplatePart::Text(_) => false,
            }),
            ExpressionKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                any(&[*condition, *then_branch])
                    || else_branch.is_some_and(|branch| self.may_write(branch))
            }
            ExpressionKind::When {
                subject, branches, ..
            } => {
                subject.is_some_and(|subject| self.may_write(subject))
                    || branches.iter().any(|branch| {
                        self.may_write(branch.body)
                            || branch.guard.is_some_and(|guard| self.may_write(guard))
                            || branch
                                .patterns
                                .iter()
                                .any(|pattern| self.pattern_may_write(pattern))
                    })
            }
            ExpressionKind::Return { value } => value.is_some_and(|value| self.may_write(value)),
        }
    }

    /// Whether evaluating `value` may re-point a reference that the place expression `target`
    /// is reached through (D2 of `compound-assignment`): whether `value`, a closure in it
    /// included, assigns or mutably borrows a place that holds such a reference, which is a
    /// prefix of the pointer of one of `target`'s dereferences (`r` for `*r` or `r.n`, `c` or
    /// `c.r` for `c.r.n`). A write through the reference (`*r = 1`) does not re-point it.
    pub(super) fn may_repoint(&self, target: ExpressionId, value: ExpressionId) -> bool {
        let mut pointers = Vec::new();
        let mut current = target;
        loop {
            match &self.expression(current).kind {
                ExpressionKind::Field { base, .. } => current = *base,
                ExpressionKind::Deref { pointer } => {
                    pointers.extend(self.place_path(*pointer));
                    current = *pointer;
                }
                _ => break,
            }
        }
        if pointers.is_empty() {
            return false;
        }
        let mut written = Vec::new();
        self.collect_written_paths(value, &mut written);
        written.iter().any(|(local, steps)| {
            pointers.iter().any(|(pointer_local, pointer_steps)| {
                local == pointer_local && pointer_steps.starts_with(steps)
            })
        })
    }

    /// The place expression `id` as its binding and the projections applied to it, from the
    /// binding outwards (`None` for an index, or a place rooted at a temporary or a top-level
    /// value, which the user cannot assign).
    fn place_path(&self, id: ExpressionId) -> Option<(prism_typed_tree::LocalId, Vec<Projection>)> {
        match &self.expression(id).kind {
            ExpressionKind::Local(local) => Some((*local, Vec::new())),
            ExpressionKind::Field { base, index, .. } => {
                let (local, mut steps) = self.place_path(*base)?;
                steps.push(Projection::Field(*index));
                Some((local, steps))
            }
            ExpressionKind::Deref { pointer } => {
                let (local, mut steps) = self.place_path(*pointer)?;
                steps.push(Projection::Deref);
                Some((local, steps))
            }
            _ => None,
        }
    }

    /// Pushes onto `written` the paths of the places that evaluating `id` assigns, compound
    /// assigns or borrows mutably, closures' bodies included.
    fn collect_written_paths(
        &self,
        id: ExpressionId,
        written: &mut Vec<(prism_typed_tree::LocalId, Vec<Projection>)>,
    ) {
        let kind = &self.expression(id).kind;
        if let ExpressionKind::Borrow {
            mutable: true,
            place,
            ..
        } = kind
        {
            written.extend(self.place_path(*place));
        }
        let mut statements = Vec::new();
        for child in kind.children(&mut statements) {
            self.collect_written_paths(child, written);
        }
        for statement in statements {
            let statement = &self.tree.statements[statement].kind;
            if let TreeStatementKind::Assign { target, .. }
            | TreeStatementKind::CompoundAssign { target, .. } = statement
            {
                written.extend(self.place_path(*target));
            }
            for child in statement.children() {
                self.collect_written_paths(child, written);
            }
        }
    }

    /// Whether testing `pattern` may write a place.
    pub(super) fn pattern_may_write(&self, pattern: &prism_typed_tree::Pattern) -> bool {
        let mut expressions = Vec::new();
        pattern.expressions(&mut expressions);
        expressions.into_iter().any(|value| self.may_write(value))
    }

    pub(super) fn constant(literal: &Literal, ty: &Type) -> Constant {
        match literal {
            Literal::Int(v) => Constant::Int(*v),
            Literal::Float(v) => Constant::Float(*v),
            Literal::String(v) => Constant::String(v.clone()),
            Literal::Char(v) => Constant::Char(*v),
            Literal::Bool(v) => Constant::Bool(*v),
            Literal::Null => Constant::Null(ty.clone()),
        }
    }

    /// Whether `id` is the `null` literal.
    pub(super) fn is_null_literal(&self, id: ExpressionId) -> bool {
        matches!(
            self.expression(id).kind,
            ExpressionKind::Literal(Literal::Null)
        )
    }

    /// Whether `id` is the `null` literal, possibly under its coercion.
    pub(super) fn is_null(&self, id: ExpressionId) -> bool {
        match &self.expression(id).kind {
            ExpressionKind::Coerce { value, .. } => self.is_null(*value),
            ExpressionKind::Literal(Literal::Null) => true,
            _ => false,
        }
    }

    fn lower_coercion(
        &mut self,
        destination: Option<&Place>,
        value: ExpressionId,
        kind: CoercionKind,
        ty: &Type,
        span: Span,
    ) {
        match kind {
            CoercionKind::Diverge => self.expression_into(None, value),
            CoercionKind::WrapNullable => {
                let Some(destination) = destination else {
                    return self.expression_into(None, value);
                };
                let operand = self.operand(value);
                self.assign(destination.clone(), Rvalue::WrapNullable(operand), span);
            }
            CoercionKind::Subtype => {
                let value_kind = &self.expression(value).kind;
                if self.is_null_literal(value) {
                    if let Some(destination) = destination {
                        let constant = Constant::Null(ty.clone());
                        self.assign(
                            destination.clone(),
                            Rvalue::Use(Operand::Constant(constant)),
                            span,
                        );
                    }
                } else if value_kind.is_place() {
                    self.lower_reborrow(destination, value, ty, span);
                } else {
                    let operand = self.operand(value);
                    let destination = self.destination_or_temporary(destination, ty, span);
                    self.assign(destination, Rvalue::Subtype(operand), span);
                }
            }
        }
    }

    /// A block: its statements in a block scope, then its tail in its own temporary scope, which
    /// ends before the block's locals are dropped (D7), at its closing brace `close_brace`.
    pub(super) fn lower_block(
        &mut self,
        destination: Option<&Place>,
        statements: &[StatementId],
        tail: Option<ExpressionId>,
        close_brace: Span,
        span: Span,
    ) {
        self.push_block_scope(close_brace);
        for &statement in statements {
            if self.current.is_none() {
                break;
            }
            self.statement(statement);
        }
        match tail {
            Some(tail) => {
                let tail_span = self.expression(tail).span;
                self.push_scope(tail_span);
                self.expression_into(destination, tail);
                self.pop_scope();
            }
            None => {
                if let Some(destination) = destination {
                    self.assign_unit(destination, span);
                }
            }
        }
        self.pop_scope();
    }

    /// Gives a `Unit` destination its value.
    pub(super) fn assign_unit(&mut self, destination: &Place, span: Span) {
        if self.current.is_some() && self.place_type(destination) == Type::Unit {
            let unit = Rvalue::Use(Operand::Constant(Constant::Unit));
            self.assign(destination.clone(), unit, span);
        }
    }

    /// `return`: the value into the return place, then the exit of every scope but the
    /// arguments', then a jump to the return block.
    fn lower_return(&mut self, value: Option<ExpressionId>, span: Span) {
        match value {
            Some(value) => self.expression_into(Some(&Place::return_place()), value),
            None => self.assign_unit(&Place::return_place(), span),
        }
        if self.current.is_none() {
            return;
        }
        self.exit_scopes_to(1, Some(span));
        let mut block = self.return_block.take();
        self.goto_lazy(&mut block, span);
        self.return_block = block;
    }

    /// Lowers a statement in its own temporary scope.
    pub(super) fn statement(&mut self, id: StatementId) {
        let statement = &self.tree.statements[id];
        let span = statement.span;
        match &statement.kind {
            TreeStatementKind::Let { local, initializer } => {
                self.lower_let(*local, *initializer, span)
            }
            TreeStatementKind::Assign { target, value } => {
                self.push_scope(span);
                self.lower_assign(*target, *value, span);
                self.pop_scope();
            }
            TreeStatementKind::CompoundAssign {
                operator,
                target,
                value,
            } => {
                self.push_scope(span);
                self.lower_compound_assign(*operator, *target, *value, span);
                self.pop_scope();
            }
            TreeStatementKind::Expression { value } => {
                self.push_scope(span);
                self.expression_into(None, *value);
                self.pop_scope();
            }
            TreeStatementKind::While { condition, body } => {
                self.push_scope(span);
                self.lower_while(id, *condition, *body, span);
                self.pop_scope();
            }
            TreeStatementKind::DoWhile { body, condition } => {
                self.push_scope(span);
                self.lower_do_while(id, *body, *condition, span);
                self.pop_scope();
            }
            TreeStatementKind::For {
                variable,
                range,
                body,
            } => {
                self.push_scope(span);
                self.lower_for(id, *variable, *range, *body, span);
                self.pop_scope();
            }
            TreeStatementKind::Destructure { pattern, value } => {
                self.lower_destructure(pattern, *value, span)
            }
        }
    }

    /// A user binding for the typed-tree binding `binding`, owned by the innermost scope and
    /// marked live.
    pub(super) fn declare_binding(
        &mut self,
        binding: prism_typed_tree::LocalId,
        span: Span,
    ) -> LocalId {
        let local = &self.tree.locals[binding];
        let id = self.add_local(LocalDeclaration {
            ty: local.ty.clone(),
            kind: LocalKind::User,
            mutable: local.mutable,
            name: Some(local.name.clone()),
            binding: Some(binding),
            span: local.span,
        });
        self.schedule(id, span);
        self.bindings.insert(binding, Place::local(id));
        id
    }

    /// `val (a, b) = e` (D15 of `payload-patterns`): the bindings and the temporaries of the
    /// unbound non-`Copy` fields belong to the block; the value is evaluated once, as a place,
    /// then bound as an irrefutable struct pattern: copies and borrows first, then the
    /// destructuring of a value taken apart.
    fn lower_destructure(
        &mut self,
        pattern: &prism_typed_tree::Pattern,
        value: ExpressionId,
        span: Span,
    ) {
        let ty = self.expression(value).ty.clone();
        let locals = self.declare_branch_locals(&[pattern], &ty);
        self.mark_branch_locals(&locals, true, span);
        let place = self.place(value);
        self.bind_copies_and_borrows(&place, &ty, pattern, span);
        let move_span = self.first_move_span(pattern).unwrap_or(span);
        let temporaries = locals.temporaries[0].clone();
        self.take_apart(place, &ty, pattern, &mut temporaries.iter(), move_span);
    }

    /// `val x = e`: the binding belongs to the block, the initializer is evaluated into it in
    /// the statement's temporary scope. The temporaries whose written borrow the initializer's
    /// value may be (see [`Self::extended_temporaries`]) belong to the block too, created just
    /// before the binding (D7 of `mir-lowering`, D20 of `borrowck-exclusivity`).
    fn lower_let(
        &mut self,
        binding: prism_typed_tree::LocalId,
        initializer: ExpressionId,
        span: Span,
    ) {
        self.preallocate_extended_temporaries(initializer);
        let local = self.declare_binding(binding, span);
        self.push_scope(span);
        self.expression_into(Some(&Place::local(local)), initializer);
        self.pop_scope();
    }

    /// Creates in the innermost scope, before `value` is evaluated, each temporary whose written
    /// borrow may be `value`'s value and that has no local yet, so that the temporary lives as
    /// long as the value instead of ending with the branch that creates it (D20 of
    /// `borrowck-exclusivity`). Lowering the `Temporary` node assigns that local.
    pub(super) fn preallocate_extended_temporaries(&mut self, value: ExpressionId) {
        for temporary in self.extended_temporaries(value) {
            if self.preallocated.contains_key(&temporary) {
                continue;
            }
            let expression = self.expression(temporary);
            let local = self.new_temporary(expression.ty.clone(), expression.span);
            self.preallocated
                .insert(temporary, Preallocated::Local(local));
        }
    }

    /// The `Temporary` nodes at the root of the places of the written borrows that may be the
    /// value of `value`: `value` itself, or, through coercions, a branch of an `if` with `else`
    /// or of a `when` with a value, the fallback of a `?:` and the tail of a block, at any
    /// depth.
    pub(super) fn extended_temporaries(&self, value: ExpressionId) -> Vec<ExpressionId> {
        let mut temporaries = Vec::new();
        let mut pending = vec![value];
        while let Some(id) = pending.pop() {
            let expression = self.expression(id);
            match &expression.kind {
                ExpressionKind::Coerce { value, .. } => pending.push(*value),
                ExpressionKind::Borrow {
                    origin: prism_typed_tree::BorrowOrigin::Written,
                    place,
                    ..
                } => temporaries.extend(self.temporary_root(*place)),
                ExpressionKind::If {
                    then_branch,
                    else_branch: Some(else_branch),
                    ..
                } => {
                    pending.push(*else_branch);
                    pending.push(*then_branch);
                }
                ExpressionKind::When { branches, .. } if expression.ty != Type::Unit => {
                    pending.extend(branches.iter().rev().map(|branch| branch.body));
                }
                // A left side that cannot be null is the value; the fallback never runs.
                ExpressionKind::Elvis { nullable, .. } if !self.is_nullable(*nullable) => {
                    pending.push(*nullable)
                }
                ExpressionKind::Elvis { fallback, .. } => pending.push(*fallback),
                ExpressionKind::Block {
                    tail: Some(tail), ..
                } => pending.push(*tail),
                _ => {}
            }
        }
        temporaries
    }

    /// The `Temporary` node at the root of the place `id`, reached through fields and indices.
    pub(super) fn temporary_root(&self, mut id: ExpressionId) -> Option<ExpressionId> {
        loop {
            match self.expression(id).kind {
                ExpressionKind::Field { base, .. } | ExpressionKind::Index { base, .. } => {
                    id = base
                }
                ExpressionKind::Temporary { .. } => return Some(id),
                _ => return None,
            }
        }
    }

    /// `target = value`: the value, then a drop of the target's old value when its type needs
    /// one, then the write (D6). A value that moves a place the source names is moved into a
    /// temporary before the drop, so that the drop cannot destroy it (`s = s`). The write is at
    /// the target's span, so a write to a field of a moved value points at the field. A whole
    /// local that needs no drop is written directly; when the value is a straight-line operand
    /// or operator, that write is at the target's span too, so a write while borrowed points at
    /// the variable (D6 of `borrowck-exclusivity`).
    fn lower_assign(&mut self, target: ExpressionId, value: ExpressionId, span: Span) {
        let target_expression = self.expression(target);
        let (target_ty, target_span) = (target_expression.ty.clone(), target_expression.span);
        if !self.needs_drop(&target_ty) && self.is_binding(target) {
            let place = self.place(target);
            self.expression_into(Some(&place), value);
            self.relocate_last_write(&place, target_span);
            return;
        }
        let mut operand = self.operand(value);
        let place = self.place(target);
        if self.needs_drop(&target_ty) {
            let value = self.expression(value);
            let (value_ty, value_span) = (value.ty.clone(), value.span);
            operand = self.stabilize(operand, value_ty, value_span);
            self.emit_drop(place.clone(), span);
        }
        self.assign(place, Rvalue::Use(operand), target_span);
    }

    /// Gives the current block's last statement the span `span` when it writes `place` an
    /// operand or an operator's result, whose operands carry their own spans.
    fn relocate_last_write(&mut self, place: &Place, span: Span) {
        let Some(block) = self.current else {
            return;
        };
        if let Some(statement) = self.blocks[block.0 as usize].statements.last_mut() {
            if let StatementKind::Assign(written, rvalue) = &statement.kind {
                let operand_only = matches!(
                    rvalue.as_ref(),
                    Rvalue::Use(_)
                        | Rvalue::Binary(..)
                        | Rvalue::Template(_)
                        | Rvalue::Unary(..)
                        | Rvalue::WrapNullable(_)
                        | Rvalue::Subtype(_)
                );
                if written == place && operand_only {
                    statement.span = span;
                }
            }
        }
    }

    /// Whether `id` has a nullable type.
    pub(super) fn is_nullable(&self, id: ExpressionId) -> bool {
        matches!(self.expression(id).ty, Type::Nullable(_))
    }

    /// Whether the place `id` is a binding, whose lowering evaluates nothing.
    fn is_binding(&self, id: ExpressionId) -> bool {
        matches!(self.expression(id).kind, ExpressionKind::Local(_))
    }
}

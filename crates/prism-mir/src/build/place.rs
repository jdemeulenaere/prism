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
    StatementKind as TreeStatementKind, Type,
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
                        self.assign(destination.clone(), Rvalue::Use(Operand::Copy(place)), span);
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
                let mut operands: Vec<Option<Operand>> = vec![None; fields.len()];
                for &index in evaluation_order {
                    operands[index] = Some(self.operand(fields[index].value));
                }
                for (index, operand) in operands.iter_mut().enumerate() {
                    if operand.is_none() {
                        *operand = Some(self.operand(fields[index].value));
                    }
                }
                let kind = match kind {
                    ConstructKind::Struct(id) => AggregateKind::Struct(id.clone()),
                    ConstructKind::Variant { enum_id, index, .. } => AggregateKind::Variant {
                        enum_id: enum_id.clone(),
                        index: *index,
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
                let start = self.operand(*start);
                let end = self.operand(*end);
                let end_inclusive = Operand::Constant(Constant::Bool(*end_inclusive));
                let destination = self.destination_or_temporary(destination, &ty, span);
                let rvalue = Rvalue::Aggregate(
                    AggregateKind::Struct(NominalId::builtin_range()),
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
            ExpressionKind::Unary { operator, operand } => {
                let operator = *operator;
                let operand = self.operand(*operand);
                let destination = self.destination_or_temporary(destination, &ty, span);
                self.assign(destination, Rvalue::Unary(operator, operand), span);
            }
            ExpressionKind::Block { statements, tail } => {
                self.lower_block(destination, statements, *tail, span)
            }
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
            ExpressionKind::Move { place } => Operand::Move(self.place(*place)),
            ExpressionKind::Copy { place } => Operand::Copy(self.place(*place)),
            ExpressionKind::Coerce {
                value,
                kind: CoercionKind::Subtype,
            } if self.is_null_literal(*value) => Operand::Constant(Constant::Null(ty)),
            kind if kind.is_place() && !matches!(kind, ExpressionKind::Error { .. }) => {
                if !self.is_copy(&ty) {
                    panic!(
                        "internal error: a {} place read in place is used as a value",
                        ty
                    );
                }
                Operand::Copy(self.place(id))
            }
            _ => {
                let local = self.new_temporary(ty, span);
                self.expression_into(Some(&Place::local(local)), id);
                Operand::Move(Place::local(local))
            }
        }
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
    /// ends before the block's locals are dropped (D7).
    pub(super) fn lower_block(
        &mut self,
        destination: Option<&Place>,
        statements: &[StatementId],
        tail: Option<ExpressionId>,
        span: Span,
    ) {
        self.push_scope(span);
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
        self.exit_scopes_to(1);
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

    /// `val x = e`: the binding belongs to the block, the initializer is evaluated into it in
    /// the statement's temporary scope. When the initializer is a written borrow of a place
    /// rooted at a temporary, that temporary belongs to the block too, created just before the
    /// binding (D7).
    fn lower_let(
        &mut self,
        binding: prism_typed_tree::LocalId,
        initializer: ExpressionId,
        span: Span,
    ) {
        if let Some(temporary) = self.extended_temporary(initializer) {
            let expression = self.expression(temporary);
            let local = self.new_temporary(expression.ty.clone(), expression.span);
            self.preallocated
                .insert(temporary, Preallocated::Local(local));
        }
        let local = self.declare_binding(binding, span);
        self.push_scope(span);
        self.expression_into(Some(&Place::local(local)), initializer);
        self.pop_scope();
    }

    /// The temporary at the root of the place `initializer` borrows, when the initializer is (up
    /// to coercions) a written borrow of a place rooted at a temporary (D7).
    fn extended_temporary(&self, initializer: ExpressionId) -> Option<ExpressionId> {
        let mut id = initializer;
        while let ExpressionKind::Coerce { value, .. } = self.expression(id).kind {
            id = value;
        }
        let ExpressionKind::Borrow {
            origin: prism_typed_tree::BorrowOrigin::Written,
            place,
            ..
        } = self.expression(id).kind
        else {
            return None;
        };
        self.temporary_root(place)
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
    /// one, then the write (D6). A target that needs no drop and whose place evaluates nothing
    /// is written directly.
    fn lower_assign(&mut self, target: ExpressionId, value: ExpressionId, span: Span) {
        let target_ty = self.expression(target).ty.clone();
        if !self.needs_drop(&target_ty) && self.is_plain_place(target) {
            let place = self.place(target);
            self.expression_into(Some(&place), value);
            return;
        }
        let operand = self.operand(value);
        let place = self.place(target);
        if self.needs_drop(&target_ty) {
            self.emit_drop(place.clone(), span);
        }
        self.assign(place, Rvalue::Use(operand), span);
    }

    /// Whether the place `id` is a binding followed by fields and dereferences, whose lowering
    /// evaluates nothing.
    fn is_plain_place(&self, mut id: ExpressionId) -> bool {
        loop {
            match self.expression(id).kind {
                ExpressionKind::Field { base, .. } => id = base,
                ExpressionKind::Deref { pointer } => id = pointer,
                ExpressionKind::Local(_) => return true,
                _ => return false,
            }
        }
    }
}

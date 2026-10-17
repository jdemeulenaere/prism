//! Branches, loops, `?.` chains, reborrows and comparisons (D8, D9, D12 and D14 of
//! `mir-lowering`).

use super::Builder;
use crate::mir::Place;
use prism_ast::Span;
use prism_typed_tree::{
    BinaryOperator, ExpressionId, LocalId as TreeLocalId, LogicalOperator, StatementId, Type,
    WhenBranch,
};

impl Builder<'_> {
    pub(super) fn lower_chain(
        &mut self,
        _destination: Option<&Place>,
        _chain: ExpressionId,
        _ty: &Type,
        _borrow: bool,
        _span: Span,
    ) {
        unimplemented!("lowering of a safe-navigation chain")
    }

    pub(super) fn lower_payload(&mut self, _nullable: ExpressionId, _span: Span) -> Place {
        unimplemented!("lowering of a payload test")
    }

    pub(super) fn lower_logical(
        &mut self,
        _destination: Option<&Place>,
        _operator: LogicalOperator,
        _left: ExpressionId,
        _right: ExpressionId,
        _span: Span,
    ) {
        unimplemented!("lowering of a logical operator")
    }

    pub(super) fn lower_elvis(
        &mut self,
        _destination: Option<&Place>,
        _nullable: ExpressionId,
        _fallback: ExpressionId,
        _ty: &Type,
        _span: Span,
    ) {
        unimplemented!("lowering of an elvis operator")
    }

    pub(super) fn lower_if(
        &mut self,
        _destination: Option<&Place>,
        _condition: ExpressionId,
        _then_branch: ExpressionId,
        _else_branch: Option<ExpressionId>,
        _span: Span,
    ) {
        unimplemented!("lowering of an if")
    }

    pub(super) fn lower_when(
        &mut self,
        _destination: Option<&Place>,
        _subject: Option<ExpressionId>,
        _branches: &[WhenBranch],
        _exhaustive: bool,
        _ty: &Type,
        _span: Span,
    ) {
        unimplemented!("lowering of a when")
    }

    pub(super) fn lower_break(&mut self, _target: Option<StatementId>, _span: Span) {
        unimplemented!("lowering of a break")
    }

    pub(super) fn lower_continue(&mut self, _target: Option<StatementId>, _span: Span) {
        unimplemented!("lowering of a continue")
    }

    pub(super) fn lower_while(
        &mut self,
        _id: StatementId,
        _condition: ExpressionId,
        _body: ExpressionId,
        _span: Span,
    ) {
        unimplemented!("lowering of a while loop")
    }

    pub(super) fn lower_do_while(
        &mut self,
        _id: StatementId,
        _body: ExpressionId,
        _condition: ExpressionId,
        _span: Span,
    ) {
        unimplemented!("lowering of a do-while loop")
    }

    pub(super) fn lower_for(
        &mut self,
        _id: StatementId,
        _variable: TreeLocalId,
        _range: ExpressionId,
        _body: ExpressionId,
        _span: Span,
    ) {
        unimplemented!("lowering of a for loop")
    }

    pub(super) fn lower_reborrow(
        &mut self,
        _destination: Option<&Place>,
        _value: ExpressionId,
        _ty: &Type,
        _span: Span,
    ) {
        unimplemented!("lowering of an optional reborrow")
    }

    pub(super) fn lower_equality(
        &mut self,
        _destination: Option<&Place>,
        _operator: BinaryOperator,
        _left: ExpressionId,
        _right: ExpressionId,
        _span: Span,
    ) {
        unimplemented!("lowering of an equality")
    }
}

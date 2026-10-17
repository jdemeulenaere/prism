//! Calls and default values (D10 and D11 of `mir-lowering`).

use super::{BodySlots, Builder};
use crate::mir::{Body, BodyId, Place};
use prism_ast::Span;
use prism_typed_tree::{Call, ExpressionId, FunctionItem, Type, TypeTable, TypedFile};

impl Builder<'_> {
    pub(super) fn lower_call(
        &mut self,
        _destination: Option<&Place>,
        _call: &Call,
        _ty: &Type,
        _span: Span,
    ) {
        unimplemented!("lowering of a call")
    }

    pub(super) fn fill_slot(
        &mut self,
        _slot: Place,
        _value: ExpressionId,
        _ty: &Type,
        _span: Span,
    ) -> Place {
        unimplemented!("lowering of a default's slot")
    }
}

impl<'a> Builder<'a> {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn lower_default(
        _file: &'a TypedFile,
        _types: &'a TypeTable,
        _bodies: &'a mut BodySlots,
        _function: &FunctionItem,
        _id: BodyId,
        _index: usize,
        _receiver_count: usize,
        _default: ExpressionId,
    ) -> Body {
        unimplemented!("lowering of a default value")
    }
}

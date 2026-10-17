//! Closures (D13 of `mir-lowering`).

use super::Builder;
use crate::mir::Place;
use prism_typed_tree::ExpressionId;

impl Builder<'_> {
    pub(super) fn lower_closure(&mut self, _destination: Option<&Place>, _id: ExpressionId) {
        unimplemented!("lowering of a closure")
    }
}

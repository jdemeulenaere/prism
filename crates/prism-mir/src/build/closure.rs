//! Closures (D13 of `mir-lowering`).
//!
//! Each closure is a body of its own, lowered from the closure's subtree in its parent's
//! arenas: its parameters are its arguments, and each binding it captures is a capture local,
//! which stands for the binding itself and is never marked or dropped. The enclosing body builds
//! the closure value from the places of the captured bindings, so how each is captured is left
//! to the borrow checker.

use super::Builder;
use crate::mir::{BodyOwner, LocalDeclaration, LocalKind, Place, Rvalue};
use prism_typed_tree::{ExpressionId, ExpressionKind, Type};

impl Builder<'_> {
    pub(super) fn lower_closure(&mut self, destination: Option<&Place>, id: ExpressionId) {
        let expression = self.expression(id);
        let ExpressionKind::Closure {
            parameters,
            body,
            is_compose,
            captures,
        } = &expression.kind
        else {
            unreachable!("a closure node");
        };
        let (ty, span) = (expression.ty.clone(), expression.span);
        let Type::Fn { ret, .. } = &ty else {
            panic!("internal error: a closure of type {}", ty);
        };
        let index = self.closure_count;
        self.closure_count += 1;
        let closure_id = self.bodies.reserve();
        let owner = BodyOwner::Closure {
            parent: self.id,
            index,
        };
        let mut closure = Builder::new(
            self.file,
            self.types,
            &mut *self.bodies,
            self.tree,
            closure_id,
            owner,
            span,
        );
        closure.is_compose = *is_compose;
        closure.type_parameters = self.type_parameters.clone();
        closure.set_return_type(ret.as_ref().clone(), span);
        for &parameter in parameters {
            closure.bind_argument(parameter);
        }
        for capture in captures {
            let binding = &self.tree.locals[capture.local];
            let local = closure.add_local(LocalDeclaration {
                ty: binding.ty.clone(),
                kind: LocalKind::Capture,
                mutable: binding.mutable,
                name: Some(binding.name.clone()),
                binding: Some(capture.local),
                span: capture.span,
            });
            closure.capture_count += 1;
            closure.bindings.insert(capture.local, Place::local(local));
        }
        closure.lower_value(*body);
        let lowered = closure.finish();
        self.bodies.fill(closure_id, lowered);
        let captures = captures
            .iter()
            .map(|capture| match self.bindings.get(&capture.local) {
                Some(place) => place.clone(),
                None => panic!(
                    "internal error: captured binding {} has no place",
                    capture.local
                ),
            })
            .collect();
        let destination = self.destination_or_temporary(destination, &ty, span);
        let rvalue = Rvalue::Closure {
            body: closure_id,
            captures,
        };
        self.assign(destination, rvalue, span);
    }
}

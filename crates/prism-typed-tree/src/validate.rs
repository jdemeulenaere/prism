//! The structural invariants of a typed tree (D14 of `typed-tree`), checked over every fixture
//! and example.

use crate::arena::{ExpressionId, LocalId, StatementId};
use crate::tree::{
    Argument, Body, Callee, ExpressionKind, Item, Pattern, StatementKind, TypedFile,
};
use crate::types::Type;

impl TypedFile {
    /// Returns the violations of the tree's structural invariants, empty for a well-formed tree.
    ///
    /// Always checked: every node is reached from at most one parent; the operand of a move,
    /// copy, borrow, field or index, and an assignment's target, is a place; a payload
    /// projection appears only inside a chain; every binding belongs to its body; every named
    /// loop target encloses its jump. With `error_free` (the file has no type error), also: every
    /// node is reached exactly once, there is no error node and no error type, every jump names
    /// its loop, and every child's type equals the type its slot expects.
    pub fn validate(&self, error_free: bool) -> Vec<String> {
        let mut violations = Vec::new();
        for item in &self.items {
            let (body_id, owner) = match item {
                Item::Function(f) => (f.body, f.path.to_string()),
                Item::Value(v) => (v.body, v.path.to_string()),
                Item::Struct(_) | Item::Enum(_) => continue,
            };
            let Some(body) = self.bodies.get(body_id) else {
                violations.push(format!("{}: body {} does not exist", owner, body_id.0));
                continue;
            };
            let mut validator = Validator {
                body,
                owner: &owner,
                error_free,
                expression_parents: vec![0; body.expressions.len()],
                statement_parents: vec![0; body.statements.len()],
                chain_depth: 0,
                loops: Vec::new(),
                violations: Vec::new(),
            };
            validator.body_roots(item);
            violations.extend(validator.violations);
        }
        violations
    }
}

struct Validator<'a> {
    body: &'a Body,
    owner: &'a str,
    error_free: bool,
    expression_parents: Vec<usize>,
    statement_parents: Vec<usize>,
    /// The number of chains enclosing the current node.
    chain_depth: usize,
    /// The loops enclosing the current node, innermost last; a closure starts a new list.
    loops: Vec<StatementId>,
    violations: Vec<String>,
}

impl Validator<'_> {
    fn report(&mut self, message: String) {
        self.violations.push(format!("{}: {}", self.owner, message));
    }

    fn body_roots(&mut self, item: &Item) {
        for (_, local) in self.body.locals.iter() {
            if self.error_free && contains_error(&local.ty) {
                let message = format!("binding '{}' has the error type", local.name);
                self.report(message);
            }
        }
        for parameter in self.body.parameters.clone() {
            self.check_local(parameter);
        }
        match item {
            Item::Function(f) => {
                if self.body.defaults.len() != f.parameters.len() {
                    let message = format!(
                        "{} default slots for {} parameters",
                        self.body.defaults.len(),
                        f.parameters.len()
                    );
                    self.report(message);
                }
                for ((name, ty), default) in f.parameters.iter().zip(&self.body.defaults) {
                    if let Some(default) = default {
                        self.expression(*default);
                        self.expect_type(*default, ty, &format!("default of '{}'", name));
                    }
                }
                if let Some(value) = self.body.value {
                    self.expression(value);
                    self.expect_type(value, &f.return_type, "function body");
                }
            }
            Item::Value(v) => match self.body.value {
                Some(value) => {
                    self.expression(value);
                    self.expect_type(value, &v.ty, "initializer");
                }
                None => self.report("a top-level value without initializer".to_string()),
            },
            Item::Struct(_) | Item::Enum(_) => {}
        }
        if self.error_free {
            for (index, &count) in self.expression_parents.iter().enumerate() {
                if count == 0 {
                    self.violations.push(format!(
                        "{}: expression {} is not reached from the body",
                        self.owner, index
                    ));
                }
            }
            for (index, &count) in self.statement_parents.iter().enumerate() {
                if count == 0 {
                    self.violations.push(format!(
                        "{}: statement {} is not reached from the body",
                        self.owner, index
                    ));
                }
            }
        }
    }

    fn check_local(&mut self, local: LocalId) {
        if !self.body.locals.contains(local) {
            self.report(format!("binding {} does not belong to the body", local));
        }
    }

    fn type_of(&self, id: ExpressionId) -> Option<&Type> {
        self.body.expressions.get(id).map(|e| &e.ty)
    }

    /// Reports a child whose type differs from its slot's, in a tree without type errors.
    fn expect_type(&mut self, id: ExpressionId, expected: &Type, slot: &str) {
        if !self.error_free {
            return;
        }
        if let Some(found) = self.type_of(id) {
            if found != expected {
                let message = format!(
                    "{} (node {}) has type {}, but its slot expects {}",
                    slot, id.0, found, expected
                );
                self.report(message);
            }
        }
    }

    fn expect_place(&mut self, id: ExpressionId, role: &str) {
        if let Some(expression) = self.body.expressions.get(id) {
            if !expression.kind.is_place() {
                let message = format!("{} (node {}) is not a place", role, id.0);
                self.report(message);
            }
        }
    }

    fn statement(&mut self, id: StatementId) {
        let Some(statement) = self.body.statements.get(id) else {
            self.report(format!("statement {} does not exist", id.0));
            return;
        };
        self.statement_parents[id.0 as usize] += 1;
        if self.statement_parents[id.0 as usize] > 1 {
            self.report(format!(
                "statement {} is reached from several parents",
                id.0
            ));
            return;
        }
        match &statement.kind {
            StatementKind::Let { local, initializer } => {
                self.check_local(*local);
                self.expression(*initializer);
                if let Some(local) = self.body.locals.get(*local) {
                    let ty = local.ty.clone();
                    self.expect_type(*initializer, &ty, "initializer");
                }
            }
            StatementKind::Assign { target, value } => {
                self.expression(*target);
                self.expression(*value);
                self.expect_place(*target, "assignment target");
                if let Some(ty) = self.type_of(*target).cloned() {
                    self.expect_type(*value, &ty, "assigned value");
                }
            }
            StatementKind::Expression { value } => self.expression(*value),
            StatementKind::While { condition, body } => {
                self.expression(*condition);
                self.expect_type(*condition, &Type::Bool, "condition");
                self.loops.push(id);
                self.expression(*body);
                self.loops.pop();
            }
            StatementKind::DoWhile { body, condition } => {
                self.loops.push(id);
                self.expression(*body);
                self.loops.pop();
                self.expression(*condition);
                self.expect_type(*condition, &Type::Bool, "condition");
            }
            StatementKind::For {
                variable,
                range,
                body,
            } => {
                self.check_local(*variable);
                self.expression(*range);
                self.loops.push(id);
                self.expression(*body);
                self.loops.pop();
            }
        }
    }

    fn expression(&mut self, id: ExpressionId) {
        let Some(expression) = self.body.expressions.get(id) else {
            self.report(format!("expression {} does not exist", id.0));
            return;
        };
        self.expression_parents[id.0 as usize] += 1;
        if self.expression_parents[id.0 as usize] > 1 {
            self.report(format!(
                "expression {} is reached from several parents",
                id.0
            ));
            return;
        }
        let ty = expression.ty.clone();
        if self.error_free && contains_error(&ty) {
            self.report(format!("expression {} has the error type", id.0));
        }
        match &expression.kind {
            ExpressionKind::Literal(_)
            | ExpressionKind::TopLevelValue(_)
            | ExpressionKind::FunctionReference(_)
            | ExpressionKind::VariantConstructor { .. } => {}
            ExpressionKind::Local(local) => self.check_local(*local),
            ExpressionKind::Field { base, .. } => {
                self.expression(*base);
                self.expect_place(*base, "field base");
            }
            ExpressionKind::Deref { pointer } => {
                self.expression(*pointer);
                if let Some(pointer_ty) = self.type_of(*pointer).cloned() {
                    let referent = match pointer_ty {
                        Type::Reference { inner, .. } | Type::Heap(inner) => Some(*inner),
                        Type::Error => None,
                        other => {
                            let message =
                                format!("dereference of node {} of type {}", pointer.0, other);
                            self.report(message);
                            None
                        }
                    };
                    if let Some(referent) = referent {
                        self.expect_type_of_node(id, &referent, "dereference");
                    }
                }
            }
            ExpressionKind::Index { base, index } => {
                self.expression(*base);
                self.expression(*index);
                self.expect_place(*base, "index base");
                self.expect_type(*index, &Type::Int, "index");
            }
            ExpressionKind::NonNullPayload { nullable } => {
                if self.chain_depth == 0 {
                    self.report(format!("payload {} outside a chain", id.0));
                }
                self.expression(*nullable);
            }
            ExpressionKind::Temporary { value } => {
                self.expression(*value);
                self.expect_type(*value, &ty, "temporary value");
            }
            ExpressionKind::Move { place } | ExpressionKind::Copy { place } => {
                self.expression(*place);
                self.expect_place(*place, "moved or copied operand");
                self.expect_type(*place, &ty, "moved or copied place");
            }
            ExpressionKind::Borrow { mutable, place, .. } => {
                self.expression(*place);
                self.expect_place(*place, "borrowed operand");
                if let Some(place_ty) = self.type_of(*place).cloned() {
                    let reference = Type::Reference {
                        mutable: *mutable,
                        inner: Box::new(place_ty),
                    };
                    self.expect_type_of_node(id, &reference, "borrow");
                }
            }
            ExpressionKind::SafeNavigation { chain, .. } => {
                self.chain_depth += 1;
                self.expression(*chain);
                self.chain_depth -= 1;
                if let Some(chain_ty) = self.type_of(*chain).cloned() {
                    self.expect_type_of_node(id, &chain_ty.nullable(), "chain");
                }
            }
            ExpressionKind::Coerce { value, .. } => {
                self.expression(*value);
                if self.error_free && self.type_of(*value) == Some(&ty) {
                    self.report(format!("coercion {} does not change the type", id.0));
                }
            }
            ExpressionKind::Call(call) => {
                if let Callee::Value(callee) = &call.callee {
                    self.expression(*callee);
                    if let Some(Type::Fn { params, .. }) = self.type_of(*callee).cloned() {
                        if params.len() != call.parameters.len() {
                            let message = format!(
                                "call {} has {} parameters, but its callee takes {}",
                                id.0,
                                call.parameters.len(),
                                params.len()
                            );
                            self.report(message);
                        }
                    }
                }
                if call.arguments.len() != call.parameters.len() {
                    let message = format!(
                        "call {} has {} arguments for {} parameters",
                        id.0,
                        call.arguments.len(),
                        call.parameters.len()
                    );
                    self.report(message);
                }
                for &index in &call.evaluation_order {
                    if !matches!(call.arguments.get(index), Some(Argument::Written(_))) {
                        let message = format!(
                            "call {} evaluates argument {}, which is not written",
                            id.0, index
                        );
                        self.report(message);
                    }
                }
                let written = call
                    .arguments
                    .iter()
                    .filter(|a| matches!(a, Argument::Written(_)))
                    .count();
                if written != call.evaluation_order.len() {
                    let message = format!(
                        "call {} evaluates {} of its {} written arguments",
                        id.0,
                        call.evaluation_order.len(),
                        written
                    );
                    self.report(message);
                }
                for (index, argument) in call.arguments.iter().enumerate() {
                    if let Argument::Written(value) = argument {
                        self.expression(*value);
                        if let Some((_, parameter_ty)) = call.parameters.get(index) {
                            let parameter_ty = parameter_ty.clone();
                            self.expect_type(*value, &parameter_ty, "argument");
                        }
                    }
                }
            }
            ExpressionKind::Construct {
                fields,
                evaluation_order,
                ..
            } => {
                if evaluation_order.len() != fields.len()
                    || evaluation_order.iter().any(|&i| i >= fields.len())
                {
                    self.report(format!("construction {} has a bad evaluation order", id.0));
                }
                for field in fields.clone() {
                    self.expression(field.value);
                    self.expect_type(field.value, &field.ty, "field value");
                }
            }
            ExpressionKind::Range { start, end, .. } => {
                self.expression(*start);
                self.expression(*end);
                self.expect_type(*start, &Type::Int, "range start");
                self.expect_type(*end, &Type::Int, "range end");
            }
            ExpressionKind::Binary { left, right, .. } => {
                self.expression(*left);
                self.expression(*right);
            }
            ExpressionKind::Unary { operand, .. } => self.expression(*operand),
            ExpressionKind::Logical { left, right, .. } => {
                self.expression(*left);
                self.expression(*right);
                self.expect_type(*left, &Type::Bool, "logical operand");
                self.expect_type(*right, &Type::Bool, "logical operand");
            }
            ExpressionKind::Elvis { nullable, fallback } => {
                self.expression(*nullable);
                self.expression(*fallback);
                self.expect_type(*fallback, &ty, "fallback");
                if self.error_free {
                    if let Some(nullable_ty) = self.type_of(*nullable) {
                        if nullable_ty != &ty && nullable_ty != &ty.clone().nullable() {
                            let message =
                                format!("elvis {} over a {} gives a {}", id.0, nullable_ty, ty);
                            self.report(message);
                        }
                    }
                }
            }
            ExpressionKind::Block { statements, tail } => {
                for statement in statements.clone() {
                    self.statement(statement);
                }
                if let Some(tail) = tail {
                    self.expression(*tail);
                    self.expect_type(*tail, &ty, "block tail");
                }
            }
            ExpressionKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.expression(*condition);
                self.expect_type(*condition, &Type::Bool, "condition");
                self.expression(*then_branch);
                if let Some(else_branch) = else_branch {
                    self.expression(*else_branch);
                    self.expect_type(*then_branch, &ty, "then branch");
                    self.expect_type(*else_branch, &ty, "else branch");
                }
            }
            ExpressionKind::When {
                subject, branches, ..
            } => {
                if let Some(subject) = subject {
                    self.expression(*subject);
                }
                for branch in branches.clone() {
                    for pattern in &branch.patterns {
                        match pattern {
                            Pattern::Variant { .. } | Pattern::Else => {}
                            Pattern::Value(value) => self.expression(*value),
                            Pattern::Range { start, end, .. } => {
                                self.expression(*start);
                                self.expression(*end);
                            }
                            Pattern::Condition(condition) => {
                                self.expression(*condition);
                                self.expect_type(*condition, &Type::Bool, "condition");
                            }
                        }
                    }
                    self.expression(branch.body);
                    if ty != Type::Unit {
                        self.expect_type(branch.body, &ty, "when branch");
                    }
                }
            }
            ExpressionKind::Return { value } => {
                if let Some(value) = value {
                    self.expression(*value);
                }
            }
            ExpressionKind::Break { target } | ExpressionKind::Continue { target } => {
                match target {
                    Some(target) if !self.loops.contains(target) => {
                        self.report(format!(
                            "jump {} names {}, which does not enclose it",
                            id.0, target
                        ));
                    }
                    None if self.error_free => {
                        self.report(format!("jump {} names no loop", id.0));
                    }
                    _ => {}
                }
            }
            ExpressionKind::Closure {
                parameters,
                body,
                captures,
                ..
            } => {
                for parameter in parameters.clone() {
                    self.check_local(parameter);
                }
                for capture in captures.clone() {
                    self.check_local(capture.local);
                }
                let loops = std::mem::take(&mut self.loops);
                let chain_depth = std::mem::replace(&mut self.chain_depth, 0);
                self.expression(*body);
                self.loops = loops;
                self.chain_depth = chain_depth;
            }
            ExpressionKind::Error { children } => {
                if self.error_free {
                    self.report(format!("error node {}", id.0));
                }
                for child in children.clone() {
                    self.expression(child);
                }
            }
        }
    }

    /// Reports a node whose own type differs from the one its kind implies.
    fn expect_type_of_node(&mut self, id: ExpressionId, expected: &Type, what: &str) {
        if !self.error_free {
            return;
        }
        if let Some(found) = self.type_of(id) {
            if found != expected {
                let message = format!(
                    "{} {} has type {}, but its operand implies {}",
                    what, id.0, found, expected
                );
                self.report(message);
            }
        }
    }
}

/// Whether `ty` is or contains the error type.
fn contains_error(ty: &Type) -> bool {
    match ty {
        Type::Error => true,
        Type::Reference { inner, .. } | Type::Nullable(inner) | Type::Heap(inner) => {
            contains_error(inner)
        }
        Type::Fn { params, ret, .. } => params.iter().any(contains_error) || contains_error(ret),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use crate::arena::{ExpressionId, LocalId, StatementId};
    use crate::tree::*;
    use crate::types::Type;
    use prism_ast::{Literal, Span};

    fn file() -> TypedFile {
        crate::dump::tests::addition_file()
    }

    fn body(file: &mut TypedFile) -> &mut Body {
        let id = match &file.items[0] {
            Item::Function(f) => f.body,
            _ => unreachable!(),
        };
        &mut file.bodies[id]
    }

    fn add(body: &mut Body, kind: ExpressionKind, ty: Type) -> ExpressionId {
        body.expressions.alloc(Expression {
            kind,
            ty,
            span: Span::dummy(),
        })
    }

    fn violations(file: &TypedFile) -> Vec<String> {
        file.validate(true)
    }

    #[test]
    fn a_well_formed_tree_has_no_violation() {
        assert_eq!(violations(&file()), Vec::<String>::new());
    }

    #[test]
    fn a_node_reached_twice_is_reported() {
        let mut file = file();
        let body = body(&mut file);
        let a = ExpressionId(0);
        let sum = add(
            body,
            ExpressionKind::Binary {
                operator: BinaryOperator::Add,
                left: a,
                right: a,
            },
            Type::Int,
        );
        body.value = Some(sum);
        let found = violations(&file);
        assert!(
            found.iter().any(|v| v.contains("several parents")),
            "{:?}",
            found
        );
        assert!(
            found.iter().any(|v| v.contains("not reached")),
            "{:?}",
            found
        );
    }

    #[test]
    fn a_move_of_a_value_is_reported() {
        let mut file = file();
        let body = body(&mut file);
        let sum = body.value.unwrap();
        let moved = add(body, ExpressionKind::Move { place: sum }, Type::Int);
        body.value = Some(moved);
        let found = violations(&file);
        assert!(
            found.iter().any(|v| v.contains("is not a place")),
            "{:?}",
            found
        );
    }

    #[test]
    fn a_payload_outside_a_chain_is_reported() {
        let mut file = file();
        let body = body(&mut file);
        let null = add(
            body,
            ExpressionKind::Literal(Literal::Null),
            Type::Nothing.nullable(),
        );
        let payload = add(
            body,
            ExpressionKind::NonNullPayload { nullable: null },
            Type::Nothing,
        );
        let statement = body.statements.alloc(Statement {
            kind: StatementKind::Expression { value: payload },
            span: Span::dummy(),
        });
        let tail = body.value.unwrap();
        let block = add(
            body,
            ExpressionKind::Block {
                statements: vec![statement],
                tail: Some(tail),
            },
            Type::Int,
        );
        body.value = Some(block);
        let found = violations(&file);
        assert!(
            found.iter().any(|v| v.contains("outside a chain")),
            "{:?}",
            found
        );
    }

    #[test]
    fn a_foreign_binding_and_a_stray_jump_are_reported() {
        let mut file = file();
        let body = body(&mut file);
        let stray = add(body, ExpressionKind::Local(LocalId(7)), Type::Int);
        let jump = add(
            body,
            ExpressionKind::Break {
                target: Some(StatementId(0)),
            },
            Type::Nothing,
        );
        let statements = [stray, jump].map(|value| {
            body.statements.alloc(Statement {
                kind: StatementKind::Expression { value },
                span: Span::dummy(),
            })
        });
        let tail = body.value.unwrap();
        let block = add(
            body,
            ExpressionKind::Block {
                statements: statements.to_vec(),
                tail: Some(tail),
            },
            Type::Int,
        );
        body.value = Some(block);
        let found = violations(&file);
        assert!(
            found.iter().any(|v| v.contains("does not belong")),
            "{:?}",
            found
        );
        assert!(
            found.iter().any(|v| v.contains("does not enclose")),
            "{:?}",
            found
        );
    }

    #[test]
    fn a_slot_type_mismatch_and_an_error_node_are_reported() {
        let mut file = file();
        let body = body(&mut file);
        let text = add(
            body,
            ExpressionKind::Literal(Literal::String("x".to_string())),
            Type::String,
        );
        let error = add(
            body,
            ExpressionKind::Error {
                children: vec![text],
            },
            Type::Error,
        );
        let sum = body.value.unwrap();
        let statement = body.statements.alloc(Statement {
            kind: StatementKind::Expression { value: error },
            span: Span::dummy(),
        });
        let block = add(
            body,
            ExpressionKind::Block {
                statements: vec![statement],
                tail: Some(sum),
            },
            Type::String,
        );
        body.value = Some(block);
        let found = violations(&file);
        assert!(
            found.iter().any(|v| v.contains("error node")),
            "{:?}",
            found
        );
        assert!(
            found.iter().any(|v| v.contains("slot expects")),
            "{:?}",
            found
        );
        // A tree with type errors may hold error nodes.
        let found = file.validate(false);
        assert!(
            !found.iter().any(|v| v.contains("error node")),
            "{:?}",
            found
        );
    }

    #[test]
    fn a_call_with_missing_arguments_is_reported() {
        let mut file = file();
        let body = body(&mut file);
        let call = add(
            body,
            ExpressionKind::Call(Call {
                callee: Callee::Function(DeclarationPath::new("main", "g")),
                arguments: vec![],
                parameters: vec![(Some("a".to_string()), Type::Int)],
                evaluation_order: vec![],
                has_receiver: false,
                is_compose: false,
            }),
            Type::Int,
        );
        let old = body.value.replace(call).unwrap();
        let statement = body.statements.alloc(Statement {
            kind: StatementKind::Expression { value: old },
            span: Span::dummy(),
        });
        let block = add(
            body,
            ExpressionKind::Block {
                statements: vec![statement],
                tail: Some(call),
            },
            Type::Int,
        );
        body.value = Some(block);
        let found = violations(&file);
        assert!(
            found
                .iter()
                .any(|v| v.contains("0 arguments for 1 parameters")),
            "{:?}",
            found
        );
    }
}

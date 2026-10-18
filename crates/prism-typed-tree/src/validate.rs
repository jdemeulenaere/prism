//! The structural invariants of a typed tree (D14 of `typed-tree`), checked over every fixture
//! and example.

use crate::arena::{ExpressionId, LocalId, StatementId};
use crate::tree::{
    Argument, BinaryOperator, Body, Callee, CoercionKind, ConstructKind, DeclarationPath,
    ExpressionKind, FunctionItem, Item, Pattern, StatementKind, StructItem, TypedFile,
    UnaryOperator,
};
use crate::types::{NominalId, Type, TypeParameter};
use prism_ast::ReceiverMode;
use std::collections::{HashMap, HashSet};

impl TypedFile {
    /// Returns the violations of the tree's structural invariants, empty for a well-formed tree.
    ///
    /// Always checked: every node is reached from at most one parent; the operand of a move,
    /// copy, borrow, field or index, and an assignment's target, is a place; a payload
    /// projection appears only inside a chain; every binding belongs to its body; every named
    /// loop target encloses its jump. With `error_free` (the file has no type error), also: every
    /// node is reached exactly once, there is no error node and no error type, every jump names
    /// its loop, every child's type equals the type its slot expects (also a returned value, a
    /// closure body, a `when` pattern, an operand and a coercion's value), a block whose value
    /// is discarded has no tail of another type, a closure's captures are exactly the bindings
    /// its body uses from outside, and a call, construction or field of the file's own
    /// declarations repeats their signature or fields.
    pub fn validate(&self, error_free: bool) -> Vec<String> {
        let mut violations = Vec::new();
        let mut functions = HashMap::new();
        let mut structs = HashMap::new();
        for item in &self.items {
            match item {
                Item::Function(f) => {
                    functions.insert(&f.path, f);
                }
                Item::Struct(s) => {
                    structs.insert(&s.id, s);
                }
                Item::Enum(_) | Item::Value(_) => {}
            }
        }
        for item in &self.items {
            // A struct's or enum's member types mention its own type parameters only.
            let members: Vec<(&[TypeParameter], &Type, String)> = match item {
                Item::Struct(s) => s
                    .fields
                    .iter()
                    .map(|(name, _, ty)| {
                        (
                            &s.type_parameters[..],
                            ty,
                            format!("{}.{}", s.id.name, name),
                        )
                    })
                    .collect(),
                Item::Enum(e) => e
                    .variants
                    .iter()
                    .flat_map(|(name, payloads)| {
                        payloads.iter().map(move |ty| {
                            (
                                &e.type_parameters[..],
                                ty,
                                format!("{}.{}", e.id.name, name),
                            )
                        })
                    })
                    .collect(),
                _ => Vec::new(),
            };
            for (parameters, ty, member) in members {
                for parameter in foreign_parameters(ty, parameters) {
                    violations.push(format!(
                        "{}: mentions type parameter {}, which its declaration does not declare",
                        member, parameter
                    ));
                }
            }
        }
        for item in &self.items {
            let (body_id, owner, type_parameters) = match item {
                Item::Function(f) => (f.body, f.path.to_string(), &f.type_parameters[..]),
                Item::Value(v) => (v.body, v.path.to_string(), &[][..]),
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
                returns: Vec::new(),
                functions: &functions,
                structs: &structs,
                type_parameters,
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
    /// The return type of the function or closure enclosing the current node, innermost last.
    returns: Vec<Type>,
    /// The file's own functions and structs, whose signatures and fields calls, constructions
    /// and field accesses must repeat.
    functions: &'a HashMap<&'a DeclarationPath, &'a FunctionItem>,
    structs: &'a HashMap<&'a NominalId, &'a StructItem>,
    /// The type parameters of the item whose body this is, the only ones its types may mention.
    type_parameters: &'a [TypeParameter],
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
            let what = format!("binding '{}'", local.name);
            self.check_type_parameters_of(&local.ty, &what);
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
                    self.returns.push(f.return_type.clone());
                    self.expression(value);
                    self.returns.pop();
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

    /// Reports every type parameter `ty` mentions that is not one of the item's own, with the
    /// same index, name and bound (D9 of `generics`).
    fn check_type_parameters_of(&mut self, ty: &Type, what: &str) {
        for parameter in foreign_parameters(ty, self.type_parameters) {
            let message = format!(
                "{} mentions type parameter {}, which {} does not declare",
                what, parameter, self.owner
            );
            self.report(message);
        }
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

    /// Reports `id` when it is not a place. The payload of a nullable is a place only when the
    /// nullable is one.
    fn expect_place(&mut self, id: ExpressionId, role: &str) {
        if let Some(expression) = self.body.expressions.get(id) {
            match &expression.kind {
                ExpressionKind::NonNullPayload { nullable } => {
                    let nullable = *nullable;
                    self.expect_place(nullable, role);
                }
                kind if !kind.is_place() => {
                    let message = format!("{} (node {}) is not a place", role, id.0);
                    self.report(message);
                }
                _ => {}
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
                self.expect_type(
                    *range,
                    &Type::plain_struct(NominalId::builtin_range()),
                    "loop range",
                );
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
        self.check_type_parameters_of(&ty, &format!("expression {}", id.0));
        match &expression.kind {
            ExpressionKind::Literal(_)
            | ExpressionKind::TopLevelValue(_)
            | ExpressionKind::FunctionReference(_)
            | ExpressionKind::VariantConstructor { .. } => {}
            ExpressionKind::Local(local) => self.check_local(*local),
            ExpressionKind::Field { base, index, name } => {
                self.expression(*base);
                self.expect_place(*base, "field base");
                if let Some(Type::Struct {
                    id: struct_id,
                    arguments,
                }) = self.type_of(*base).cloned()
                {
                    if let Some(item) = self.structs.get(&struct_id) {
                        match item.fields.get(*index) {
                            Some((field_name, _, field_ty)) if field_name == name => {
                                // The declared type, with the struct's type arguments.
                                let field_ty = field_ty.substitute(&arguments);
                                self.expect_type_of_node(id, &field_ty, "field");
                            }
                            _ => {
                                let message = format!(
                                    "field {} names field {} '{}', which {} does not declare",
                                    id.0, index, name, struct_id.name
                                );
                                self.report(message);
                            }
                        }
                    }
                }
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
            ExpressionKind::Coerce { value, kind } => {
                self.expression(*value);
                if self.error_free && self.type_of(*value) == Some(&ty) {
                    self.report(format!("coercion {} does not change the type", id.0));
                }
                let value_ty = self.type_of(*value).cloned();
                match kind {
                    CoercionKind::WrapNullable => match &ty {
                        Type::Nullable(inner) => {
                            let inner = inner.as_ref().clone();
                            self.expect_type(*value, &inner, "wrapped value");
                        }
                        _ if self.error_free => {
                            self.report(format!("coercion {} wraps into a non-nullable", id.0));
                        }
                        _ => {}
                    },
                    CoercionKind::Diverge => {
                        if value_ty.is_some_and(|t| t != Type::Nothing) {
                            self.expect_type(*value, &Type::Nothing, "diverging value");
                        }
                    }
                    CoercionKind::Subtype => {}
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
                if let Callee::Function(path) = &call.callee {
                    if let Some(function) = self.functions.get(path) {
                        let declared = declared_parameters(function);
                        if self.error_free
                            && (declared != call.parameters
                                || function.is_compose != call.is_compose
                                || function.receiver.is_some() != call.has_receiver)
                        {
                            let message =
                                format!("call {} of {} does not repeat its signature", id.0, path);
                            self.report(message);
                        }
                        if function.type_parameters.len() != call.type_arguments.len() {
                            let message = format!(
                                "call {} of {} has {} type arguments for {} type parameters",
                                id.0,
                                path,
                                call.type_arguments.len(),
                                function.type_parameters.len()
                            );
                            self.report(message);
                        }
                        let return_type = function.return_type.substitute(&call.type_arguments);
                        self.expect_type_of_node(id, &return_type, "call");
                    }
                }
                for argument in &call.type_arguments {
                    self.check_type_parameters_of(argument, &format!("call {}", id.0));
                }
                for (index, argument) in call.arguments.iter().enumerate() {
                    if let Argument::Written(value) = argument {
                        self.expression(*value);
                        // The slot has the declared type with the call's type arguments.
                        if let Some(parameter_ty) = call.parameter_type(index) {
                            self.expect_type(*value, &parameter_ty, "argument");
                        }
                    }
                }
            }
            ExpressionKind::Construct {
                kind,
                fields,
                evaluation_order,
            } => {
                if let ConstructKind::Struct(struct_id) = kind {
                    if let Some(item) = self.structs.get(struct_id) {
                        let declared = item
                            .fields
                            .iter()
                            .map(|(name, _, ty)| (Some(name.clone()), ty.clone()));
                        let constructed = fields.iter().map(|f| (f.name.clone(), f.ty.clone()));
                        if self.error_free && !declared.eq(constructed) {
                            let message = format!(
                                "construction {} does not repeat the fields of {}",
                                id.0, struct_id.name
                            );
                            self.report(message);
                        }
                    }
                }
                if evaluation_order.len() != fields.len()
                    || evaluation_order.iter().any(|&i| i >= fields.len())
                {
                    self.report(format!("construction {} has a bad evaluation order", id.0));
                }
                // A field's value has its declared type with the construction's type arguments.
                let arguments = match &ty {
                    Type::Struct { arguments, .. } | Type::Enum { arguments, .. } => {
                        arguments.clone()
                    }
                    _ => Vec::new(),
                };
                for field in fields.clone() {
                    self.expression(field.value);
                    self.expect_type(field.value, &field.ty.substitute(&arguments), "field value");
                }
            }
            ExpressionKind::Range { start, end, .. } => {
                self.expression(*start);
                self.expression(*end);
                self.expect_type(*start, &Type::Int, "range start");
                self.expect_type(*end, &Type::Int, "range end");
            }
            ExpressionKind::Binary {
                operator,
                left,
                right,
            } => {
                self.expression(*left);
                self.expression(*right);
                // Both operands are at one type (D20): the result's for arithmetic.
                if let Some(left_ty) = self.type_of(*left).cloned() {
                    self.expect_type(*right, &left_ty, "right operand");
                    let result = match operator {
                        BinaryOperator::Add
                        | BinaryOperator::Subtract
                        | BinaryOperator::Multiply
                        | BinaryOperator::Divide
                        | BinaryOperator::Remainder => left_ty,
                        _ => Type::Bool,
                    };
                    self.expect_type_of_node(id, &result, "binary operator");
                }
            }
            ExpressionKind::Unary { operator, operand } => {
                self.expression(*operand);
                match operator {
                    UnaryOperator::Not => self.expect_type(*operand, &Type::Bool, "operand"),
                    UnaryOperator::Negate => self.expect_type(*operand, &ty, "operand"),
                }
            }
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
            ExpressionKind::Block {
                statements, tail, ..
            } => {
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
                } else {
                    // Without `else`, the branch's value is discarded.
                    self.expect_discarded(*then_branch, "then branch");
                }
            }
            ExpressionKind::When {
                subject, branches, ..
            } => {
                if let Some(subject) = subject {
                    self.expression(*subject);
                }
                let subject_ty = subject.and_then(|s| self.type_of(s).cloned());
                for branch in branches.clone() {
                    for pattern in &branch.patterns {
                        match pattern {
                            Pattern::Variant { enum_id, .. } => {
                                let matches_enum = subject_ty
                                    .as_ref()
                                    .and_then(Type::nominal_id)
                                    .is_some_and(|id| id == enum_id)
                                    && matches!(subject_ty, Some(Type::Enum { .. }));
                                if self.error_free && !matches_enum {
                                    let message = format!(
                                        "when {} matches a variant of {}",
                                        id.0, enum_id.name
                                    );
                                    self.report(message);
                                }
                            }
                            Pattern::Else => {}
                            Pattern::Value(value) => {
                                self.expression(*value);
                                if let Some(subject_ty) = &subject_ty {
                                    self.expect_type(*value, subject_ty, "pattern value");
                                }
                            }
                            Pattern::Range { start, end, .. } => {
                                self.expression(*start);
                                self.expression(*end);
                                if let Some(subject_ty) = &subject_ty {
                                    self.expect_type(*start, subject_ty, "pattern range start");
                                    self.expect_type(*end, subject_ty, "pattern range end");
                                }
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
                    } else {
                        // A `Unit` `when` is a statement, or checked against `Unit`: either way
                        // its branches' values are discarded.
                        self.expect_discarded(branch.body, "when branch");
                    }
                }
            }
            ExpressionKind::Return { value } => {
                let expected = self.returns.last().cloned();
                match (value, expected) {
                    (Some(value), expected) => {
                        self.expression(*value);
                        if let Some(expected) = expected {
                            self.expect_type(*value, &expected, "returned value");
                        }
                    }
                    (None, Some(expected)) if self.error_free && expected != Type::Unit => {
                        self.report(format!("return {} has no value for {}", id.0, expected));
                    }
                    (None, _) => {}
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
                let ret = match &ty {
                    Type::Fn { params, ret, .. } => {
                        let declared: Vec<Type> = parameters
                            .iter()
                            .filter_map(|p| self.body.locals.get(*p).map(|l| l.ty.clone()))
                            .collect();
                        if self.error_free && &declared != params {
                            let message =
                                format!("closure {} binds parameters of other types", id.0);
                            self.report(message);
                        }
                        Some(ret.as_ref().clone())
                    }
                    _ => None,
                };
                let loops = std::mem::take(&mut self.loops);
                let chain_depth = std::mem::replace(&mut self.chain_depth, 0);
                self.returns.extend(ret.clone());
                self.expression(*body);
                if let Some(ret) = ret {
                    self.returns.pop();
                    self.expect_type(*body, &ret, "closure body");
                }
                self.loops = loops;
                self.chain_depth = chain_depth;
                self.check_captures(id, parameters, *body, captures);
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

    /// Reports a block in a position whose value is discarded that still has a value: its last
    /// expression must be a statement (D9).
    fn expect_discarded(&mut self, id: ExpressionId, slot: &str) {
        if !self.error_free {
            return;
        }
        if let Some(expression) = self.body.expressions.get(id) {
            if let ExpressionKind::Block { tail: Some(_), .. } = expression.kind {
                if expression.ty != Type::Unit {
                    let message = format!(
                        "{} (node {}) is discarded, but its block has a {} tail",
                        slot, id.0, expression.ty
                    );
                    self.report(message);
                }
            }
        }
    }

    /// Reports a closure whose captures are not exactly the bindings its body uses and does not
    /// declare (D11), each once.
    fn check_captures(
        &mut self,
        id: ExpressionId,
        parameters: &[LocalId],
        body: ExpressionId,
        captures: &[crate::tree::Capture],
    ) {
        if !self.error_free {
            return;
        }
        let mut declared: HashSet<LocalId> = parameters.iter().copied().collect();
        let mut used = Vec::new();
        self.collect_bindings(body, &mut declared, &mut used);
        let expected: HashSet<LocalId> =
            used.into_iter().filter(|l| !declared.contains(l)).collect();
        let recorded: HashSet<LocalId> = captures.iter().map(|c| c.local).collect();
        if recorded.len() != captures.len() || recorded != expected {
            let mut expected: Vec<_> = expected.into_iter().map(|l| l.to_string()).collect();
            expected.sort();
            let message = format!(
                "closure {} records captures {:?}, but its body uses [{}] from outside",
                id.0,
                captures
                    .iter()
                    .map(|c| c.local.to_string())
                    .collect::<Vec<_>>(),
                expected.join(", ")
            );
            self.report(message);
        }
    }

    /// Collects the bindings that the subtree of `id` declares and those it uses.
    fn collect_bindings(
        &self,
        id: ExpressionId,
        declared: &mut HashSet<LocalId>,
        used: &mut Vec<LocalId>,
    ) {
        let Some(expression) = self.body.expressions.get(id) else {
            return;
        };
        if let ExpressionKind::Local(local) = expression.kind {
            used.push(local);
        }
        if let ExpressionKind::Closure { parameters, .. } = &expression.kind {
            declared.extend(parameters.iter().copied());
        }
        let mut statements = Vec::new();
        for child in children(&expression.kind, &mut statements) {
            self.collect_bindings(child, declared, used);
        }
        while let Some(statement) = statements.pop() {
            let Some(statement) = self.body.statements.get(statement) else {
                continue;
            };
            let expressions = match &statement.kind {
                StatementKind::Let { local, initializer } => {
                    declared.insert(*local);
                    vec![*initializer]
                }
                StatementKind::Assign { target, value } => vec![*target, *value],
                StatementKind::Expression { value } => vec![*value],
                StatementKind::While { condition, body }
                | StatementKind::DoWhile { body, condition } => vec![*condition, *body],
                StatementKind::For {
                    variable,
                    range,
                    body,
                } => {
                    declared.insert(*variable);
                    vec![*range, *body]
                }
            };
            for child in expressions {
                self.collect_bindings(child, declared, used);
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

/// The type parameters `ty` mentions that are not among `declared`, as they are declared.
pub fn foreign_parameters(ty: &Type, declared: &[TypeParameter]) -> Vec<TypeParameter> {
    let mut foreign = Vec::new();
    collect_foreign_parameters(ty, declared, &mut foreign);
    foreign
}

fn collect_foreign_parameters(ty: &Type, declared: &[TypeParameter], out: &mut Vec<TypeParameter>) {
    match ty {
        Type::Parameter(parameter) => {
            if declared.get(parameter.index as usize) != Some(parameter) {
                out.push(parameter.clone());
            }
        }
        Type::Reference { inner, .. } | Type::Nullable(inner) | Type::Heap(inner) => {
            collect_foreign_parameters(inner, declared, out)
        }
        Type::Struct { arguments, .. } | Type::Enum { arguments, .. } => {
            for argument in arguments {
                collect_foreign_parameters(argument, declared, out);
            }
        }
        Type::Fn { params, ret, .. } => {
            for param in params {
                collect_foreign_parameters(param, declared, out);
            }
            collect_foreign_parameters(ret, declared, out);
        }
        _ => {}
    }
}

/// The parameters a call of `function` passes, as [`crate::tree::Call::parameters`] records
/// them: `this` first for an extension, of the type its receiver mode passes.
fn declared_parameters(function: &FunctionItem) -> Vec<(Option<String>, Type)> {
    let receiver = function.receiver.as_ref().map(|(mode, ty)| {
        let ty = match mode {
            ReceiverMode::Shared => Type::Reference {
                mutable: false,
                inner: Box::new(ty.clone()),
            },
            ReceiverMode::Mutable => Type::Reference {
                mutable: true,
                inner: Box::new(ty.clone()),
            },
            ReceiverMode::Consuming => ty.clone(),
        };
        (Some("this".to_string()), ty)
    });
    receiver
        .into_iter()
        .chain(
            function
                .parameters
                .iter()
                .map(|(name, ty)| (Some(name.clone()), ty.clone())),
        )
        .collect()
}

/// The expression children of `kind`, and its statements pushed onto `statements`.
fn children(kind: &ExpressionKind, statements: &mut Vec<StatementId>) -> Vec<ExpressionId> {
    match kind {
        ExpressionKind::Literal(_)
        | ExpressionKind::Local(_)
        | ExpressionKind::TopLevelValue(_)
        | ExpressionKind::FunctionReference(_)
        | ExpressionKind::VariantConstructor { .. }
        | ExpressionKind::Break { .. }
        | ExpressionKind::Continue { .. } => Vec::new(),
        ExpressionKind::Field { base, .. } => vec![*base],
        ExpressionKind::Deref { pointer } => vec![*pointer],
        ExpressionKind::Index { base, index } => vec![*base, *index],
        ExpressionKind::NonNullPayload { nullable } => vec![*nullable],
        ExpressionKind::Temporary { value }
        | ExpressionKind::Coerce { value, .. }
        | ExpressionKind::Move { place: value }
        | ExpressionKind::Copy { place: value }
        | ExpressionKind::Borrow { place: value, .. }
        | ExpressionKind::SafeNavigation { chain: value, .. }
        | ExpressionKind::Unary { operand: value, .. } => vec![*value],
        ExpressionKind::Call(call) => {
            let callee = match call.callee {
                Callee::Value(callee) => Some(callee),
                _ => None,
            };
            callee
                .into_iter()
                .chain(call.arguments.iter().filter_map(|a| match a {
                    Argument::Written(value) => Some(*value),
                    Argument::Default => None,
                }))
                .collect()
        }
        ExpressionKind::Construct { fields, .. } => fields.iter().map(|f| f.value).collect(),
        ExpressionKind::Range { start, end, .. } => vec![*start, *end],
        ExpressionKind::Binary { left, right, .. }
        | ExpressionKind::Logical { left, right, .. } => {
            vec![*left, *right]
        }
        ExpressionKind::Elvis { nullable, fallback } => vec![*nullable, *fallback],
        ExpressionKind::Block {
            statements: s,
            tail,
            ..
        } => {
            statements.extend(s.iter().copied());
            tail.iter().copied().collect()
        }
        ExpressionKind::If {
            condition,
            then_branch,
            else_branch,
        } => [Some(*condition), Some(*then_branch), *else_branch]
            .into_iter()
            .flatten()
            .collect(),
        ExpressionKind::When {
            subject, branches, ..
        } => {
            let mut children: Vec<ExpressionId> = subject.iter().copied().collect();
            for branch in branches {
                for pattern in &branch.patterns {
                    match pattern {
                        Pattern::Value(value) | Pattern::Condition(value) => children.push(*value),
                        Pattern::Range { start, end, .. } => children.extend([*start, *end]),
                        Pattern::Variant { .. } | Pattern::Else => {}
                    }
                }
                children.push(branch.body);
            }
            children
        }
        ExpressionKind::Return { value } => value.iter().copied().collect(),
        ExpressionKind::Closure { body, .. } => vec![*body],
        ExpressionKind::Error { children } => children.clone(),
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
                close_brace: Span::dummy(),
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
                close_brace: Span::dummy(),
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
                close_brace: Span::dummy(),
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
    fn a_closure_missing_a_capture_is_reported() {
        let mut file = file();
        let body = body(&mut file);
        // `{ a + 1 }` reads the parameter `a`, but records no capture.
        let sum = body.value.unwrap();
        let fn_ty = Type::Fn {
            is_compose: false,
            params: vec![],
            ret: Box::new(Type::Int),
        };
        let closure = add(
            body,
            ExpressionKind::Closure {
                parameters: vec![],
                body: sum,
                is_compose: false,
                captures: vec![],
            },
            fn_ty,
        );
        let call = add(
            body,
            ExpressionKind::Call(Call {
                callee: Callee::Value(closure),
                arguments: vec![],
                parameters: vec![],
                type_arguments: vec![],
                evaluation_order: vec![],
                has_receiver: false,
                is_compose: false,
            }),
            Type::Int,
        );
        body.value = Some(call);
        let found = violations(&file);
        assert!(
            found.iter().any(|v| v.contains("records captures")),
            "{:?}",
            found
        );
    }

    #[test]
    fn a_discarded_branch_with_a_value_and_a_mistyped_return_are_reported() {
        let mut file = file();
        let body = body(&mut file);
        // `{ if (true) { a + 1 }; return "x" }`: the `if` has no `else`, so its branch is
        // discarded and must hold its tail as a statement; and `f` returns an `Int`.
        let sum = body.value.unwrap();
        let condition = add(
            body,
            ExpressionKind::Literal(Literal::Bool(true)),
            Type::Bool,
        );
        let branch = add(
            body,
            ExpressionKind::Block {
                statements: vec![],
                tail: Some(sum),
                close_brace: Span::dummy(),
            },
            Type::Int,
        );
        let if_node = add(
            body,
            ExpressionKind::If {
                condition,
                then_branch: branch,
                else_branch: None,
            },
            Type::Unit,
        );
        let text = add(
            body,
            ExpressionKind::Literal(Literal::String("x".to_string())),
            Type::String,
        );
        let ret = add(
            body,
            ExpressionKind::Return { value: Some(text) },
            Type::Nothing,
        );
        let diverge = add(
            body,
            ExpressionKind::Coerce {
                value: ret,
                kind: CoercionKind::Diverge,
            },
            Type::Int,
        );
        let statement = body.statements.alloc(Statement {
            kind: StatementKind::Expression { value: if_node },
            span: Span::dummy(),
        });
        let block = add(
            body,
            ExpressionKind::Block {
                statements: vec![statement],
                tail: Some(diverge),
                close_brace: Span::dummy(),
            },
            Type::Int,
        );
        body.value = Some(block);
        let found = violations(&file);
        assert!(
            found.iter().any(|v| v.contains("is discarded")),
            "{:?}",
            found
        );
        assert!(
            found.iter().any(|v| v.contains("returned value")),
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
                type_arguments: vec![],
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
                close_brace: Span::dummy(),
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

    fn parameter_t() -> Type {
        Type::Parameter(crate::types::TypeParameter {
            index: 0,
            name: "T".to_string(),
            bound: None,
        })
    }

    #[test]
    fn a_mistyped_substituted_slot_is_reported() {
        // `g<String>(1)` for `fun <T> g(x: T)`: the slot is `T` with `T = String`.
        let mut file = file();
        let body = body(&mut file);
        let one = add(body, ExpressionKind::Literal(Literal::Int(1)), Type::Int);
        let call = add(
            body,
            ExpressionKind::Call(Call {
                callee: Callee::Function(DeclarationPath::new("main", "g")),
                arguments: vec![Argument::Written(one)],
                parameters: vec![(Some("x".to_string()), parameter_t())],
                type_arguments: vec![Type::String],
                evaluation_order: vec![0],
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
                close_brace: Span::dummy(),
            },
            Type::Int,
        );
        body.value = Some(block);
        let found = violations(&file);
        assert!(
            found
                .iter()
                .any(|v| v.contains("has type Int, but its slot expects String")),
            "{:?}",
            found
        );
    }

    #[test]
    fn a_foreign_type_parameter_is_reported() {
        let mut file = file();
        let body = body(&mut file);
        body.locals[LocalId(0)].ty = parameter_t();
        let found = violations(&file);
        assert!(
            found.iter().any(|v| v
                .contains("binding 'a' mentions type parameter T, which main::f does not declare")),
            "{:?}",
            found
        );
    }
}

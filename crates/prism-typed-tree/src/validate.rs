//! The structural invariants of a typed tree (D14 of `typed-tree`), checked over every fixture
//! and example.

use crate::arena::{ExpressionId, LocalId, StatementId};
use crate::tree::{
    Argument, BindingMode, Body, BorrowOrigin, BuiltinFunction, Call, Callee, CaptureMode,
    CoercionKind, ConstructKind, DeclarationPath, ExpressionKind, ForIteration, FunctionItem, Item,
    LocalKind, Pattern, StatementKind, StructItem, TemplatePart, TypedFile, UnaryOperator,
};
use crate::types::{NominalId, Type, TypeParameter, TypeTable};
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
                types: &self.types,
                violations: Vec::new(),
                composable: Vec::new(),
                state_targets: std::collections::HashSet::new(),
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
    /// The file's type table, against which patterns are checked.
    types: &'a TypeTable,
    violations: Vec<String>,
    /// Whether each enclosing body or closure is a composable context, innermost last.
    composable: Vec<bool>,
    /// The state places that are assignment targets, which any context may write (D17 of
    /// `state-handles`).
    state_targets: std::collections::HashSet<ExpressionId>,
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
            if local.escaping
                && (!matches!(
                    local.kind,
                    LocalKind::Parameter
                        | LocalKind::ClosureParameter
                        | LocalKind::ImplicitClosureParameter
                ) || !(local.ty.is_function_value() || local.ty == Type::Error))
            {
                let message = format!(
                    "binding '{}' is escaping but is not a parameter of function type",
                    local.name
                );
                self.report(message);
            }
            if has_misshapen_escaping_flags(&local.ty) {
                let message = format!(
                    "binding '{}' has a function type whose escaping flags do not match its parameters",
                    local.name
                );
                self.report(message);
            }
        }
        for (id, expression) in self.body.expressions.iter() {
            if has_misshapen_escaping_flags(&expression.ty) {
                let message = format!(
                    "node {} has a function type whose escaping flags do not match its parameters",
                    id.0
                );
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
                // A default value is never a composable context (D17 of `compose-transform`).
                self.composable.push(false);
                for ((name, ty), default) in f.parameters.iter().zip(&self.body.defaults) {
                    if let Some(default) = default {
                        self.expression(*default);
                        self.expect_type(*default, ty, &format!("default of '{}'", name));
                    }
                }
                self.composable.pop();
                if let Some(value) = self.body.value {
                    self.composable.push(f.is_compose);
                    self.returns.push(f.return_type.clone());
                    self.expression(value);
                    self.returns.pop();
                    self.composable.pop();
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

    /// Records `target` when it is a state place: the target of an assignment.
    fn note_state_target(&mut self, target: ExpressionId) {
        if let Some(ExpressionKind::StatePlace { .. }) =
            self.body.expressions.get(target).map(|e| &e.kind)
        {
            self.state_targets.insert(target);
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

    fn kind(&self, id: ExpressionId) -> Option<&ExpressionKind> {
        self.body.expressions.get(id).map(|e| &e.kind)
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
                self.note_state_target(*target);
                self.expression(*target);
                self.expression(*value);
                self.expect_place(*target, "assignment target");
                if let Some(ty) = self.type_of(*target).cloned() {
                    self.expect_type(*value, &ty, "assigned value");
                }
            }
            StatementKind::CompoundAssign {
                operator,
                target,
                value,
            } => {
                self.note_state_target(*target);
                self.expression(*target);
                self.expression(*value);
                self.expect_place(*target, "compound assignment target");
                if !operator.is_arithmetic() {
                    self.report(format!(
                        "statement {} is a compound assignment with the non-arithmetic operator '{}'",
                        id.0,
                        operator.as_str()
                    ));
                }
                // The operator's result has the target's type (D3 of `compound-assignment`).
                if let Some(ty) = self.type_of(*target).cloned() {
                    self.expect_type(*value, &ty, "compound assignment value");
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
            StatementKind::Destructure { pattern, value } => {
                self.expression(*value);
                match (pattern, self.type_of(*value).cloned()) {
                    (Pattern::Struct { .. }, Some(ty)) => {
                        let ty = match ty {
                            Type::Reference { inner, .. } => *inner,
                            ty => ty,
                        };
                        self.pattern(pattern, &ty, ExpressionId(0));
                    }
                    _ if self.error_free => self.report(format!(
                        "destructuring statement {} has no struct pattern",
                        id.0
                    )),
                    _ => {}
                }
                if pattern.moves() {
                    self.expect_whole_value(*value, "destructured value");
                }
            }
            StatementKind::For {
                variable,
                iterable,
                iteration,
                body,
            } => {
                self.check_local(*variable);
                self.expression(*iterable);
                let variable_ty = self
                    .body
                    .locals
                    .get(*variable)
                    .map(|local| local.ty.clone());
                match iteration {
                    ForIteration::Range => {
                        self.expect_type(
                            *iterable,
                            &Type::plain_struct(NominalId::builtin_range()),
                            "loop range",
                        );
                        if self.error_free && variable_ty.is_some_and(|ty| ty != Type::Int) {
                            self.report(format!("range loop {} binds a non-Int variable", id.0));
                        }
                    }
                    // D10 of `collections`: a borrow of the list by the loop, and a variable
                    // borrowing each element likewise.
                    ForIteration::List { mutable } => {
                        let borrows = matches!(
                            self.kind(*iterable),
                            Some(ExpressionKind::Borrow {
                                mutable: borrow_mutable,
                                origin: BorrowOrigin::Iteration,
                                ..
                            }) if borrow_mutable == mutable
                        );
                        let element = match self.type_of(*iterable) {
                            Some(Type::Reference { inner, .. }) => match inner.as_ref() {
                                Type::List(element) => Some(element.as_ref().clone()),
                                _ => None,
                            },
                            _ => None,
                        };
                        match element {
                            Some(element) if borrows => {
                                let expected = Type::Reference {
                                    mutable: *mutable,
                                    inner: Box::new(element),
                                };
                                if self.error_free && variable_ty != Some(expected.clone()) {
                                    self.report(format!(
                                        "list loop {} binds a variable that is not a {}",
                                        id.0, expected
                                    ));
                                }
                            }
                            _ if self.error_free => self.report(format!(
                                "list loop {} does not iterate a borrow of a list by the loop",
                                id.0
                            )),
                            _ => {}
                        }
                    }
                }
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
                // Only a list's elements are stored (D7 of `collections`).
                match self.type_of(*base).cloned() {
                    Some(Type::List(element)) => {
                        self.expect_type_of_node(id, &element, "element");
                    }
                    Some(Type::Error) | None => {}
                    Some(other) => self.report(format!(
                        "index {} of a {}, which is not a list",
                        id.0, other
                    )),
                }
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
            ExpressionKind::StatePlace { handle } => {
                self.expression(*handle);
                match self.type_of(*handle).cloned() {
                    Some(Type::Struct {
                        id: handle_id,
                        arguments,
                    }) if handle_id.intrinsic().is_some_and(|i| i.is_state_handle()) => {
                        if let [value] = &arguments[..] {
                            self.expect_type_of_node(id, value, "state place");
                        }
                    }
                    Some(Type::Error) | None => {}
                    Some(other) => self.report(format!(
                        "state place {} over a {}, which is not a state handle",
                        id.0, other
                    )),
                }
                // Outside composable contexts a state's value is only assigned (D5 of
                // `state-handles`).
                let composable = self.composable.last().copied().unwrap_or(false);
                if self.error_free && !composable && !self.state_targets.contains(&id) {
                    self.report(format!(
                        "state place {} outside a composable context is not an assignment target",
                        id.0
                    ));
                }
            }
            ExpressionKind::Move { place } | ExpressionKind::Copy { place } => {
                if matches!(&expression.kind, ExpressionKind::Move { .. })
                    && self.error_free
                    && matches!(
                        self.body.expressions.get(*place).map(|e| &e.kind),
                        Some(ExpressionKind::StatePlace { .. })
                    )
                {
                    self.report(format!("move {} out of a state place", id.0));
                }
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
                                || function.receiver.is_some() != call.has_receiver
                                || function.context.is_some() != call.has_context)
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
                if let Callee::Builtin(builtin) = &call.callee {
                    self.builtin_call(id, *builtin, call);
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
                    let result = if operator.is_arithmetic() {
                        left_ty
                    } else {
                        Type::Bool
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
            ExpressionKind::Template { parts } => {
                self.expect_type_of_node(id, &Type::String, "template");
                for part in parts.clone() {
                    let TemplatePart::Value(value) = part else {
                        continue;
                    };
                    self.expression(value);
                    // A part is a value with a text form, read in place: a string is a place
                    // (a computed one a temporary), never moved (D9 of `string-templates`).
                    match self.type_of(value).cloned() {
                        Some(Type::String) => self.expect_place(value, "string template part"),
                        Some(Type::Int | Type::Float | Type::Bool | Type::Char) => {}
                        Some(other) if self.error_free => {
                            let message = format!(
                                "template {} has a part (node {}) of type {}, which has no text form",
                                id.0, value.0, other
                            );
                            self.report(message);
                        }
                        _ => {}
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
                        match (pattern, &subject_ty) {
                            (Pattern::Else, _) => {}
                            (Pattern::Condition(condition), None) => {
                                self.expression(*condition);
                                self.expect_type(*condition, &Type::Bool, "condition");
                            }
                            (pattern, Some(subject_ty)) => {
                                self.pattern(pattern, subject_ty, id);
                            }
                            (_, None) => self.report(format!(
                                "when {} without a subject has a pattern that is not a condition",
                                id.0
                            )),
                        }
                    }
                    self.alternatives_bind_alike(&branch.patterns, id);
                    if branch.patterns.iter().any(Pattern::moves) {
                        if let Some(subject) = subject {
                            self.expect_whole_value(*subject, "when subject");
                        }
                    }
                    if let Some(guard) = branch.guard {
                        self.expression(guard);
                        self.expect_type(guard, &Type::Bool, "guard");
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
                ownership_reason,
                ..
            } => {
                for parameter in parameters.clone() {
                    self.check_local(parameter);
                }
                for capture in captures.clone() {
                    self.check_local(capture.local);
                    let Some(local) = self.body.locals.get(capture.local) else {
                        continue;
                    };
                    let path = capture.path_text(&local.name, &local.ty, self.types);
                    let Some(path_type) = capture.path_type(&local.ty, self.types) else {
                        let message = format!(
                            "closure {} captures '{}', which the type {} has no such path of",
                            id.0, path, local.ty
                        );
                        self.report(message);
                        continue;
                    };
                    if capture.mode == CaptureMode::Copied && !self.types.is_copy(&path_type) {
                        let message = format!(
                            "closure {} copies '{}', whose type {} is not Copy",
                            id.0, path, path_type
                        );
                        self.report(message);
                    }
                    if capture.mode == CaptureMode::Moved && !capture.projections.is_empty() {
                        let message =
                            format!("closure {} moves the path '{}' out alone", id.0, path);
                        self.report(message);
                    }
                    let widens = capture.mode.is_owned() && capture.projections.is_empty();
                    if capture.widened.is_some() && !widens {
                        let message = format!(
                            "closure {} records a widening for '{}', which is not an owned binding",
                            id.0, path
                        );
                        self.report(message);
                    }
                }
                let owns = captures.iter().any(|c| c.mode.is_owned());
                if owns != ownership_reason.is_some() {
                    let message = if owns {
                        format!("closure {} owns captures but records no reason", id.0)
                    } else {
                        format!("closure {} records a reason but owns no capture", id.0)
                    };
                    self.report(message);
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
                let is_compose = matches!(
                    &ty,
                    Type::Fn {
                        is_compose: true,
                        ..
                    }
                );
                self.composable.push(is_compose);
                self.expression(*body);
                self.composable.pop();
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
        used.retain(|(local, _)| !declared.contains(local));
        let expected: HashSet<LocalId> = used.iter().map(|(local, _)| *local).collect();
        let recorded: HashSet<LocalId> = captures.iter().map(|c| c.local).collect();
        // Per binding, its captured paths: none a prefix of another, and one a prefix of each
        // use (D2 of `path-captures`).
        let mut paths: HashMap<LocalId, Vec<&[crate::tree::CaptureProjection]>> = HashMap::new();
        for capture in captures {
            paths
                .entry(capture.local)
                .or_default()
                .push(&capture.projections);
        }
        for (local, list) in &paths {
            for (i, a) in list.iter().enumerate() {
                for b in &list[i + 1..] {
                    if a.starts_with(b) || b.starts_with(a) {
                        let message =
                            format!("closure {} captures overlapping paths of {}", id.0, local);
                        self.report(message);
                    }
                }
            }
        }
        for (local, projections) in &used {
            let covered = paths
                .get(local)
                .is_some_and(|list| list.iter().any(|path| projections.starts_with(path)));
            if !covered && recorded.contains(local) {
                let message = format!(
                    "closure {} uses a path of {} that no capture covers",
                    id.0, local
                );
                self.report(message);
            }
        }
        if recorded != expected {
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

    /// Checks `pattern` against a matched part of type `part` (D14 of `payload-patterns`):
    /// sub-pattern counts follow the definitions, bindings are pattern bindings whose types fit
    /// their modes, and values and ranges have the part's type. `when` names the `when` node in
    /// messages.
    fn pattern(&mut self, pattern: &Pattern, part: &Type, when: ExpressionId) {
        match pattern {
            Pattern::Variant {
                enum_id,
                index,
                arguments,
                fields,
                ..
            } => {
                let matches = matches!(part, Type::Enum { id, arguments: a } if id == enum_id && a == arguments);
                if !matches {
                    if self.error_free {
                        self.report(format!(
                            "when {} matches a variant of {} against a {}",
                            when.0, enum_id.name, part
                        ));
                    }
                    return;
                }
                let payloads = self
                    .types
                    .instantiated_variants(enum_id, arguments)
                    .and_then(|variants| variants.get(*index).map(|(_, p)| p.clone()));
                self.sub_patterns(fields, payloads, when, &enum_id.name);
            }
            Pattern::Struct {
                id,
                arguments,
                fields,
            } => {
                let matches = matches!(part, Type::Struct { id: s, arguments: a } if s == id && a == arguments);
                if !matches {
                    if self.error_free {
                        self.report(format!(
                            "when {} matches struct {} against a {}",
                            when.0, id.name, part
                        ));
                    }
                    return;
                }
                let field_types = self
                    .types
                    .instantiated_fields(id, arguments)
                    .map(|fields| fields.into_iter().map(|(_, _, ty)| ty).collect());
                self.sub_patterns(fields, field_types, when, &id.name);
            }
            Pattern::Deref(inner) => match part {
                Type::Heap(contents)
                | Type::Reference {
                    inner: contents, ..
                } => self.pattern(inner, contents, when),
                _ if self.error_free => self.report(format!(
                    "when {} matches the contents of a box against a {}",
                    when.0, part
                )),
                _ => {}
            },
            Pattern::NonNull(inner) => match part {
                Type::Nullable(payload) => self.pattern(inner, payload, when),
                _ if self.error_free => self.report(format!(
                    "when {} matches a non-null pattern against a {}",
                    when.0, part
                )),
                _ => {}
            },
            Pattern::Wildcard => {}
            Pattern::Binding { local, mode } => {
                self.check_local(*local);
                let Some(binding) = self.body.locals.get(*local) else {
                    return;
                };
                if binding.kind != LocalKind::PatternBinding {
                    self.report(format!(
                        "pattern binding {} is not a pattern binding",
                        local
                    ));
                }
                if !self.error_free {
                    return;
                }
                let expected = mode.binding_type(part);
                if binding.ty != expected {
                    let message = format!(
                        "pattern binding {} ({}) has type {}, but its part is a {}",
                        local,
                        mode.as_str(),
                        binding.ty,
                        part
                    );
                    self.report(message);
                }
                if *mode == BindingMode::Copy && !self.types.is_copy(part) {
                    self.report(format!(
                        "pattern binding {} copies a {}, which is not Copy",
                        local, part
                    ));
                }
            }
            Pattern::Value(value) => {
                self.expression(*value);
                self.expect_type(*value, part, "pattern value");
            }
            Pattern::Range { start, end, .. } => {
                self.expression(*start);
                self.expression(*end);
                self.expect_type(*start, part, "pattern range start");
                self.expect_type(*end, part, "pattern range end");
            }
            Pattern::Condition(condition) => {
                self.expression(*condition);
                self.report(format!("when {} has a condition with a subject", when.0));
            }
            Pattern::Else => self.report(format!("when {} has a nested else pattern", when.0)),
        }
    }

    /// Checks one sub-pattern per part of `parts`, the part types of a variant or struct named
    /// `owner`, when its definition is known.
    fn sub_patterns(
        &mut self,
        fields: &[Pattern],
        parts: Option<Vec<Type>>,
        when: ExpressionId,
        owner: &str,
    ) {
        let Some(parts) = parts else {
            if self.error_free {
                self.report(format!(
                    "when {} matches {}, which has no definition",
                    when.0, owner
                ));
            }
            return;
        };
        if parts.len() != fields.len() {
            self.report(format!(
                "when {} matches {} with {} sub-patterns for {} parts",
                when.0,
                owner,
                fields.len(),
                parts.len()
            ));
            return;
        }
        for (field, part) in fields.iter().zip(&parts) {
            self.pattern(field, part, when);
        }
    }

    /// Reports alternatives of one branch that do not bind the same bindings.
    fn alternatives_bind_alike(&mut self, patterns: &[Pattern], when: ExpressionId) {
        if !self.error_free {
            return;
        }
        let mut sets = patterns
            .iter()
            .filter(|p| !matches!(p, Pattern::Else))
            .map(|pattern| {
                let mut locals: Vec<LocalId> =
                    pattern.bindings().into_iter().map(|(l, _)| l).collect();
                locals.sort();
                locals
            });
        if let Some(first) = sets.next() {
            if sets.any(|other| other != first) {
                self.report(format!(
                    "when {} has alternatives binding different bindings",
                    when.0
                ));
            }
        }
    }

    /// Reports a value that a moving binding takes apart and that is not a whole binding or a
    /// temporary (D6 of `payload-patterns`).
    fn expect_whole_value(&mut self, id: ExpressionId, role: &str) {
        if !self.error_free {
            return;
        }
        let whole = self.body.expressions.get(id).is_some_and(|e| {
            matches!(
                e.kind,
                ExpressionKind::Local(_) | ExpressionKind::Temporary { .. }
            )
        });
        if !whole {
            self.report(format!(
                "{} (node {}) is taken apart but is not a whole binding or a temporary",
                role, id.0
            ));
        }
    }

    /// Collects the bindings that the subtree of `id` declares and those it uses.
    fn collect_bindings(
        &self,
        id: ExpressionId,
        declared: &mut HashSet<LocalId>,
        used: &mut Vec<(LocalId, Vec<crate::tree::CaptureProjection>)>,
    ) {
        let Some(expression) = self.body.expressions.get(id) else {
            return;
        };
        if let ExpressionKind::Local(local) = expression.kind {
            used.push((local, Vec::new()));
        }
        // A chain of fields and dereferences down to a binding is one use of the path it
        // spells out.
        if matches!(
            expression.kind,
            ExpressionKind::Field { .. } | ExpressionKind::Deref { .. }
        ) {
            let mut projections = Vec::new();
            let mut current = id;
            loop {
                match self.body.expressions.get(current).map(|e| &e.kind) {
                    Some(ExpressionKind::Field { base, index, .. }) => {
                        projections.push(crate::tree::CaptureProjection::Field(*index));
                        current = *base;
                    }
                    Some(ExpressionKind::Deref { pointer }) => {
                        projections.push(crate::tree::CaptureProjection::Deref);
                        current = *pointer;
                    }
                    _ => break,
                }
            }
            if let Some(ExpressionKind::Local(local)) =
                self.body.expressions.get(current).map(|e| &e.kind)
            {
                projections.reverse();
                used.push((*local, projections));
                return;
            }
            return self.collect_bindings(current, declared, used);
        }
        if let ExpressionKind::Closure { parameters, .. } = &expression.kind {
            declared.extend(parameters.iter().copied());
        }
        if let ExpressionKind::When { branches, .. } = &expression.kind {
            for branch in branches {
                for pattern in &branch.patterns {
                    declared.extend(pattern.bindings().into_iter().map(|(local, _)| local));
                }
            }
        }
        let mut statements = Vec::new();
        for child in expression.kind.children(&mut statements) {
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
                StatementKind::Assign { target, value }
                | StatementKind::CompoundAssign { target, value, .. } => vec![*target, *value],
                StatementKind::Expression { value } => vec![*value],
                StatementKind::While { condition, body }
                | StatementKind::DoWhile { body, condition } => vec![*condition, *body],
                StatementKind::For {
                    variable,
                    iterable,
                    body,
                    ..
                } => {
                    declared.insert(*variable);
                    vec![*iterable, *body]
                }
                StatementKind::Destructure { pattern, value } => {
                    declared.extend(pattern.bindings().into_iter().map(|(local, _)| local));
                    vec![*value]
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

    /// Checks the shape of a call of the box built-ins (D10 of `heap-boxes`): `Heap` takes one
    /// argument of its declared parameter `T`, with one type argument, and has type `Heap<T>`;
    /// `unbox` takes one box that is moved, not borrowed, and has the type of its contents.
    fn builtin_call(&mut self, id: ExpressionId, builtin: BuiltinFunction, call: &Call) {
        match builtin {
            BuiltinFunction::HeapAllocate => {
                let parameter = BuiltinFunction::heap_contents_parameter().as_type();
                let shape = call.parameters.len() == 1
                    && call.parameters[0].1 == parameter
                    && call.type_arguments.len() == 1
                    && !call.has_receiver;
                if !shape {
                    self.report(format!("call {} of builtin Heap is malformed", id.0));
                    return;
                }
                let boxed = Type::Heap(Box::new(call.type_arguments[0].clone()));
                self.expect_type_of_node(id, &boxed, "call");
            }
            BuiltinFunction::Unbox => {
                let receiver = match call.arguments.as_slice() {
                    [Argument::Written(receiver)] if call.has_receiver => *receiver,
                    _ => {
                        self.report(format!("call {} of builtin unbox is malformed", id.0));
                        return;
                    }
                };
                if matches!(self.kind(receiver), Some(ExpressionKind::Borrow { .. })) {
                    self.report(format!("call {} unboxes a borrow", id.0));
                }
                match self.type_of(receiver).cloned() {
                    Some(Type::Heap(contents)) => {
                        self.expect_type_of_node(id, &contents, "call");
                    }
                    Some(Type::Error) | None => {}
                    Some(other) => {
                        self.report(format!("call {} unboxes a {}", id.0, other));
                    }
                }
            }
            // D4 of `collections`: one parameter `T` per element, the result `List<T>`.
            BuiltinFunction::ListOf => {
                let element = BuiltinFunction::list_element_parameter().as_type();
                let shape = call.type_arguments.len() == 1
                    && !call.has_receiver
                    && call.parameters.iter().all(|(_, ty)| *ty == element);
                if !shape {
                    self.report(format!("call {} of builtin listOf is malformed", id.0));
                    return;
                }
                let list = Type::List(Box::new(call.type_arguments[0].clone()));
                self.expect_type_of_node(id, &list, "call");
            }
            // D6 of `collections`: the member's declared signature, with the element type as the
            // type argument.
            _ => {
                let (Some(parameters), Some(signature)) = (
                    builtin.list_member_parameters(),
                    builtin.list_member_signature(),
                ) else {
                    return;
                };
                if call.type_arguments.len() != 1
                    || !call.has_receiver
                    || call.parameters != parameters
                {
                    self.report(format!(
                        "call {} of builtin {} does not repeat its signature",
                        id.0,
                        builtin.name()
                    ));
                    return;
                }
                let result = signature.result.substitute(&call.type_arguments);
                self.expect_type_of_node(id, &result, "call");
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
        Type::Reference { inner, .. }
        | Type::Nullable(inner)
        | Type::Heap(inner)
        | Type::List(inner) => collect_foreign_parameters(inner, declared, out),
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
    let context = function.context.as_deref().map(|(_, ty)| {
        (
            Some("context".to_string()),
            Type::Reference {
                mutable: false,
                inner: Box::new(ty.clone()),
            },
        )
    });
    context
        .into_iter()
        .chain(receiver)
        .chain(
            function
                .parameters
                .iter()
                .map(|(name, ty)| (Some(name.clone()), ty.clone())),
        )
        .collect()
}

/// Whether `ty` is or contains the error type.
/// Whether a function type in `ty` has a different number of `escaping` flags than parameters.
fn has_misshapen_escaping_flags(ty: &Type) -> bool {
    match ty {
        Type::Reference { inner, .. }
        | Type::Nullable(inner)
        | Type::Heap(inner)
        | Type::List(inner) => has_misshapen_escaping_flags(inner),
        Type::Struct { arguments, .. } | Type::Enum { arguments, .. } => {
            arguments.iter().any(has_misshapen_escaping_flags)
        }
        Type::Fn {
            params,
            escaping,
            ret,
            ..
        } => {
            params.len() != escaping.len()
                || params.iter().any(has_misshapen_escaping_flags)
                || has_misshapen_escaping_flags(ret)
        }
        _ => false,
    }
}

fn contains_error(ty: &Type) -> bool {
    match ty {
        Type::Error => true,
        Type::Reference { inner, .. }
        | Type::Nullable(inner)
        | Type::Heap(inner)
        | Type::List(inner) => contains_error(inner),
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
    fn an_ill_typed_compound_assignment_is_reported() {
        let mut file = file();
        let body = body(&mut file);
        let parameter = body.parameters[0];
        let target = add(body, ExpressionKind::Local(parameter), Type::Int);
        let value = add(
            body,
            ExpressionKind::Literal(Literal::String("a".to_string())),
            Type::String,
        );
        let statement = body.statements.alloc(Statement {
            kind: StatementKind::CompoundAssign {
                operator: BinaryOperator::Equal,
                target,
                value,
            },
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
            found
                .iter()
                .any(|v| v.contains("non-arithmetic operator '=='")),
            "{:?}",
            found
        );
        assert!(
            found
                .iter()
                .any(|v| v.contains("compound assignment value")),
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
        let fn_ty = Type::function(false, vec![], Type::Int);
        let closure = add(
            body,
            ExpressionKind::Closure {
                parameters: vec![],
                body: sum,
                is_compose: false,
                captures: vec![],
                ownership_reason: None,
            },
            fn_ty,
        );
        let call = add(
            body,
            ExpressionKind::Call(Call {
                callee: Callee::Value(closure),
                arguments: vec![],
                parameters: vec![],
                escaping: vec![],
                type_arguments: vec![],
                evaluation_order: vec![],
                has_receiver: false,
                has_context: false,
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
                escaping: vec![false],
                type_arguments: vec![],
                evaluation_order: vec![],
                has_receiver: false,
                has_context: false,
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

    /// `unbox(Heap(a + 1))` as the body of `f`, with the box typed `boxed` and the call of
    /// `unbox` typed `contents`.
    fn box_round_trip(boxed: Type, contents: Type) -> TypedFile {
        let mut file = file();
        let body = body(&mut file);
        let sum = body.value.unwrap();
        let heap = add(
            body,
            ExpressionKind::Call(Call {
                callee: Callee::Builtin(BuiltinFunction::HeapAllocate),
                arguments: vec![Argument::Written(sum)],
                parameters: vec![(
                    Some("value".to_string()),
                    BuiltinFunction::heap_contents_parameter().as_type(),
                )],
                escaping: vec![false],
                type_arguments: vec![Type::Int],
                evaluation_order: vec![0],
                has_receiver: false,
                has_context: false,
                is_compose: false,
            }),
            boxed.clone(),
        );
        let unbox = add(
            body,
            ExpressionKind::Call(Call {
                callee: Callee::Builtin(BuiltinFunction::Unbox),
                arguments: vec![Argument::Written(heap)],
                parameters: vec![(Some("this".to_string()), boxed)],
                escaping: vec![false],
                type_arguments: vec![],
                evaluation_order: vec![0],
                has_receiver: true,
                has_context: false,
                is_compose: false,
            }),
            contents,
        );
        body.value = Some(unbox);
        file
    }

    #[test]
    fn well_formed_box_built_ins_have_no_violation() {
        let file = box_round_trip(Type::Heap(Box::new(Type::Int)), Type::Int);
        assert_eq!(violations(&file), Vec::<String>::new());
    }

    #[test]
    fn mistyped_box_built_ins_are_reported() {
        let file = box_round_trip(Type::Heap(Box::new(Type::Bool)), Type::Bool);
        let found = violations(&file);
        assert!(
            found
                .iter()
                .any(|v| v.contains("has type Heap<Bool>, but its operand implies Heap<Int>")),
            "{:?}",
            found
        );
        let file = box_round_trip(Type::Int, Type::Int);
        let found = violations(&file);
        assert!(
            found.iter().any(|v| v.contains("unboxes a Int")),
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
                escaping: vec![false],
                type_arguments: vec![Type::String],
                evaluation_order: vec![0],
                has_receiver: false,
                has_context: false,
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

    /// `enum Box { Of(Int, String) }` and `fun f(b: Box): Int = when (b) { Box.Of(<pattern>) ->
    /// n }`, built by hand with the sub-patterns `fields`, whose binding `n` has type `n_type`.
    /// The nodes of the original body are left unreached.
    fn when_file(fields: impl FnOnce(LocalId) -> Vec<Pattern>, n_type: Type) -> TypedFile {
        use crate::types::{NominalId, TypeDef};
        let mut file = file();
        let id = NominalId::new("main", "Box");
        file.types.insert_definition(
            id.clone(),
            TypeDef::Enum {
                parameters: Vec::new(),
                variants: vec![("Of".to_string(), vec![Type::Int, Type::String])],
            },
        );
        let enum_type = Type::Enum {
            id: id.clone(),
            arguments: Vec::new(),
        };
        let body = body(&mut file);
        let b = body.parameters[0];
        body.locals[b].ty = enum_type.clone();
        let n = body.locals.alloc(Local {
            name: "n".to_string(),
            ty: n_type,
            mutable: false,
            kind: LocalKind::PatternBinding,
            escaping: false,
            span: Span::dummy(),
        });
        let subject = ExpressionId(0);
        body.expressions[subject].ty = enum_type;
        let read = add(body, ExpressionKind::Local(n), Type::Int);
        let pattern = Pattern::Variant {
            enum_id: id,
            index: 0,
            name: "Of".to_string(),
            arguments: Vec::new(),
            fields: fields(n),
        };
        let when = add(
            body,
            ExpressionKind::When {
                subject: Some(subject),
                branches: vec![WhenBranch {
                    patterns: vec![pattern],
                    guard: None,
                    body: read,
                    span: Span::dummy(),
                }],
                exhaustive: true,
            },
            Type::Int,
        );
        body.value = Some(when);
        file
    }

    #[test]
    fn patterns_are_checked_against_their_definitions() {
        let binding = |mode| move |n| vec![Pattern::Binding { local: n, mode }, Pattern::Wildcard];
        let unreached = |found: &[String]| {
            found
                .iter()
                .filter(|v| !v.contains("not reached"))
                .cloned()
                .collect::<Vec<_>>()
        };
        let well_formed = when_file(binding(BindingMode::Copy), Type::Int);
        assert_eq!(unreached(&violations(&well_formed)), Vec::<String>::new());

        let mistyped = when_file(binding(BindingMode::SharedBorrow), Type::Int);
        let found = unreached(&violations(&mistyped));
        assert!(
            found
                .iter()
                .any(|v| v.contains("(shared borrow) has type Int")),
            "{:?}",
            found
        );

        let short = when_file(
            |n| {
                vec![Pattern::Binding {
                    local: n,
                    mode: BindingMode::Copy,
                }]
            },
            Type::Int,
        );
        let found = unreached(&violations(&short));
        assert!(
            found
                .iter()
                .any(|v| v.contains("1 sub-patterns for 2 parts")),
            "{:?}",
            found
        );

        let copied_string = when_file(
            |n| {
                vec![
                    Pattern::Wildcard,
                    Pattern::Binding {
                        local: n,
                        mode: BindingMode::Copy,
                    },
                ]
            },
            Type::String,
        );
        let found = unreached(&violations(&copied_string));
        assert!(
            found.iter().any(|v| v.contains("which is not Copy")),
            "{:?}",
            found
        );
    }

    #[test]
    fn taking_apart_a_field_is_reported() {
        let mut file = when_file(
            |n| {
                vec![
                    Pattern::Binding {
                        local: n,
                        mode: BindingMode::Copy,
                    },
                    Pattern::Binding {
                        local: n,
                        mode: BindingMode::Move,
                    },
                ]
            },
            Type::Int,
        );
        let body = body(&mut file);
        let when = body.value.unwrap();
        let ty = body.expressions[ExpressionId(0)].ty.clone();
        let field = add(
            body,
            ExpressionKind::Field {
                base: ExpressionId(0),
                index: 0,
                name: "inner".to_string(),
            },
            ty,
        );
        if let ExpressionKind::When { subject, .. } = &mut body.expressions[when].kind {
            *subject = Some(field);
        }
        let found = violations(&file);
        assert!(
            found
                .iter()
                .any(|v| v.contains("is taken apart but is not a whole binding")),
            "{:?}",
            found
        );
    }
}

//! The stable text form of a typed tree, printed by `prism dump-typed-tree` (D13 of
//! `typed-tree`).
//!
//! The structs and enums the file knows from its imports come first, then the items in source
//! order; each body lists its bindings, then its default values and
//! its value as trees, one node per line, two spaces of indentation per level. A line gives the
//! node's role in its parent where the parent has several kinds of children (`target:`), its
//! kind and resolved names, its type after `:` and the line and column where its source starts.

use crate::arena::{ExpressionId, StatementId};
use crate::tree::{
    Argument, Body, BorrowOrigin, Callee, CaptureMode, CoercionKind, ConstructKind, ExpressionKind,
    Item, LocalKind, Pattern, SafeNavigationOrigin, StatementKind, TemplatePart, TypedFile,
};
use crate::types::{NominalId, Type, TypeDef, TypeParameter, BUILTIN_MODULE};
use prism_ast::{Literal, ReceiverMode, Span, TypeMarker, Visibility};
use std::fmt::Write;

impl TypedFile {
    /// Renders the tree as deterministic text.
    pub fn dump(&self) -> String {
        let mut out = String::new();
        writeln!(out, "module {}", self.module).unwrap();
        if let Some(package) = &self.package {
            writeln!(out, "package {}", package).unwrap();
        }
        self.dump_imported_types(&mut out);
        for item in &self.items {
            match item {
                Item::Struct(s) => {
                    writeln!(
                        out,
                        "{}struct {}{}{} {}",
                        visibility_prefix(s.visibility),
                        nominal(&s.id),
                        TypeParameter::render_list(&s.type_parameters),
                        generic_marker_suffix(s.marker, &s.type_parameters, &s.marker_parameters),
                        position(s.span)
                    )
                    .unwrap();
                    dump_fields(&mut out, &s.fields);
                }
                Item::Enum(e) => {
                    writeln!(
                        out,
                        "{}enum {}{}{} {}",
                        visibility_prefix(e.visibility),
                        nominal(&e.id),
                        TypeParameter::render_list(&e.type_parameters),
                        generic_marker_suffix(e.marker, &e.type_parameters, &e.marker_parameters),
                        position(e.span)
                    )
                    .unwrap();
                    dump_variants(&mut out, &e.variants);
                }
                Item::Function(f) => {
                    let mut parameters = Vec::new();
                    if let Some((mode, _)) = &f.receiver {
                        parameters.push(match mode {
                            ReceiverMode::Shared => "this".to_string(),
                            ReceiverMode::Mutable => "mut this".to_string(),
                            ReceiverMode::Consuming => "consuming this".to_string(),
                        });
                    }
                    // The body's parameter locals start with the receiver, if any.
                    let declared = self.bodies[f.body]
                        .parameters
                        .get(usize::from(f.receiver.is_some())..)
                        .unwrap_or_default();
                    for (index, (name, ty)) in f.parameters.iter().enumerate() {
                        let escaping = declared
                            .get(index)
                            .is_some_and(|&local| self.bodies[f.body].locals[local].escaping);
                        let modifier = if escaping { "escaping " } else { "" };
                        parameters.push(format!("{}: {}{}", name, modifier, ty));
                    }
                    let type_parameters = TypeParameter::render_list(&f.type_parameters);
                    writeln!(
                        out,
                        "{}{}fun {}{}({}): {} {}",
                        visibility_prefix(f.visibility),
                        if f.is_compose { "compose " } else { "" },
                        if type_parameters.is_empty() {
                            String::new()
                        } else {
                            format!("{} ", type_parameters)
                        },
                        f.path,
                        parameters.join(", "),
                        f.return_type,
                        position(f.span)
                    )
                    .unwrap();
                    let body = &self.bodies[f.body];
                    let mut printer = Printer {
                        body,
                        out: &mut out,
                    };
                    printer.locals();
                    for ((name, _), default) in f.parameters.iter().zip(&body.defaults) {
                        if let Some(default) = default {
                            printer.expression(*default, 1, &format!("default {}", name));
                        }
                    }
                    if let Some(value) = body.value {
                        printer.expression(value, 1, "");
                    }
                }
                Item::Value(v) => {
                    writeln!(
                        out,
                        "{}val {}: {} {}",
                        visibility_prefix(v.visibility),
                        v.path,
                        v.ty,
                        position(v.span)
                    )
                    .unwrap();
                    let body = &self.bodies[v.body];
                    let mut printer = Printer {
                        body,
                        out: &mut out,
                    };
                    printer.locals();
                    if let Some(value) = body.value {
                        printer.expression(value, 1, "");
                    }
                }
            }
        }
        out
    }
}

impl TypedFile {
    /// Prints every struct and enum of the type table that the file neither declares nor gets
    /// from the compiler, in [`NominalId`] order: those its direct imports export (with their
    /// fields or variants) and those only an imported signature mentions (with their marker).
    fn dump_imported_types(&self, out: &mut String) {
        for id in self.types.ids() {
            if id.module == self.module || id.module == BUILTIN_MODULE {
                continue;
            }
            let marker = marker_suffix(self.types.marker(id));
            let generic_marker = |parameters: &[TypeParameter]| {
                generic_marker_suffix(
                    self.types.marker(id),
                    parameters,
                    self.types.marker_parameters(id),
                )
            };
            match self.types.definition(id) {
                Some(TypeDef::Struct { parameters, fields }) => {
                    let marker = generic_marker(parameters);
                    let parameters = TypeParameter::render_list(parameters);
                    writeln!(out, "import struct {}{}{}", nominal(id), parameters, marker).unwrap();
                    dump_fields(out, fields);
                }
                Some(TypeDef::Enum {
                    parameters,
                    variants,
                }) => {
                    let marker = generic_marker(parameters);
                    let parameters = TypeParameter::render_list(parameters);
                    writeln!(out, "import enum {}{}{}", nominal(id), parameters, marker).unwrap();
                    dump_variants(out, variants);
                }
                None => writeln!(out, "import type {}{}", nominal(id), marker).unwrap(),
            }
        }
    }
}

fn dump_fields(out: &mut String, fields: &[(String, bool, Type)]) {
    for (index, (name, is_var, ty)) in fields.iter().enumerate() {
        let keyword = if *is_var { "var" } else { "val" };
        writeln!(out, "  field {} {} {}: {}", index, keyword, name, ty).unwrap();
    }
}

fn dump_variants(out: &mut String, variants: &[(String, Vec<Type>)]) {
    for (index, (name, payloads)) in variants.iter().enumerate() {
        if payloads.is_empty() {
            writeln!(out, "  variant {} {}", index, name).unwrap();
        } else {
            let payloads: Vec<String> = payloads.iter().map(ToString::to_string).collect();
            writeln!(out, "  variant {} {}({})", index, name, payloads.join(", ")).unwrap();
        }
    }
}

/// Prints the nodes of one body.
struct Printer<'a> {
    body: &'a Body,
    out: &'a mut String,
}

impl Printer<'_> {
    fn locals(&mut self) {
        for (id, local) in self.body.locals.iter() {
            let kind = match local.kind {
                LocalKind::Receiver => "receiver",
                LocalKind::Parameter if local.escaping => "escaping parameter",
                LocalKind::Parameter => "parameter",
                LocalKind::Let if local.mutable => "var",
                LocalKind::Let => "val",
                LocalKind::ForVariable => "for",
                LocalKind::ClosureParameter if local.escaping => "escaping closure parameter",
                LocalKind::ClosureParameter => "closure parameter",
                LocalKind::PatternBinding if local.mutable => "pattern var",
                LocalKind::PatternBinding => "pattern val",
            };
            writeln!(
                self.out,
                "  local {} {}: {} ({})",
                id, local.name, local.ty, kind
            )
            .unwrap();
        }
    }

    fn line(&mut self, depth: usize, label: &str, text: &str) {
        let indent = "  ".repeat(depth);
        if label.is_empty() {
            writeln!(self.out, "{}{}", indent, text).unwrap();
        } else if text.is_empty() {
            writeln!(self.out, "{}{}", indent, label).unwrap();
        } else {
            writeln!(self.out, "{}{}: {}", indent, label, text).unwrap();
        }
    }

    fn local_name(&self, id: crate::arena::LocalId) -> String {
        match self.body.locals.get(id) {
            Some(local) => format!("{} {}", id, local.name),
            None => format!("{} <unknown>", id),
        }
    }

    fn expression(&mut self, id: ExpressionId, depth: usize, label: &str) {
        let Some(expression) = self.body.expressions.get(id) else {
            self.line(depth, label, &format!("<missing node {}>", id.0));
            return;
        };
        let kind = &expression.kind;
        let text = match kind {
            ExpressionKind::Literal(literal) => format!("literal {}", literal_text(literal)),
            ExpressionKind::Local(local) => format!("local {}", self.local_name(*local)),
            ExpressionKind::TopLevelValue(path) => format!("value {}", path),
            ExpressionKind::Field { index, name, .. } => format!("field {} {}", index, name),
            ExpressionKind::Deref { .. } => "deref".to_string(),
            ExpressionKind::Index { .. } => "index".to_string(),
            ExpressionKind::NonNullPayload { .. } => "payload".to_string(),
            ExpressionKind::Temporary { .. } => "temporary".to_string(),
            ExpressionKind::Move { .. } => "move".to_string(),
            ExpressionKind::Copy { .. } => "copy".to_string(),
            ExpressionKind::Borrow {
                mutable, origin, ..
            } => format!(
                "borrow {}({})",
                if *mutable { "mut " } else { "" },
                match origin {
                    BorrowOrigin::Written => "written",
                    BorrowOrigin::Argument => "argument",
                    BorrowOrigin::Receiver => "receiver",
                    BorrowOrigin::Conversion => "conversion",
                }
            ),
            ExpressionKind::SafeNavigation { origin, .. } => match origin {
                SafeNavigationOrigin::Written => "chain".to_string(),
                SafeNavigationOrigin::NullableView => "chain (nullable view)".to_string(),
                SafeNavigationOrigin::BorrowContents => "chain (borrow contents)".to_string(),
                SafeNavigationOrigin::CopyOut => "chain (copy out)".to_string(),
            },
            ExpressionKind::Coerce { kind, .. } => match kind {
                CoercionKind::WrapNullable => "coerce wrap nullable".to_string(),
                CoercionKind::Diverge => "coerce diverge".to_string(),
                CoercionKind::Subtype => "coerce subtype".to_string(),
            },
            ExpressionKind::FunctionReference(path) => format!("function {}", path),
            ExpressionKind::VariantConstructor {
                enum_id,
                index,
                name,
            } => format!(
                "variant constructor {} {} {}",
                nominal(enum_id),
                index,
                name
            ),
            ExpressionKind::Call(call) => {
                let callee = match &call.callee {
                    Callee::Function(path) => {
                        format!("{}{}", path, render_type_arguments(&call.type_arguments))
                    }
                    Callee::Builtin(builtin) => format!(
                        "builtin {}{}",
                        builtin.name(),
                        render_type_arguments(&call.type_arguments)
                    ),
                    Callee::Value(_) => "value".to_string(),
                };
                let mut text = format!(
                    "call {}{}",
                    if call.is_compose { "compose " } else { "" },
                    callee
                );
                if !is_ascending(&call.evaluation_order) {
                    let names: Vec<String> = call
                        .evaluation_order
                        .iter()
                        .map(|&i| parameter_label(call, i))
                        .collect();
                    write!(text, " (order: {})", names.join(", ")).unwrap();
                }
                text
            }
            ExpressionKind::Construct {
                kind,
                fields,
                evaluation_order,
            } => {
                let mut text = match kind {
                    ConstructKind::Struct(id) => format!("construct {}", nominal(id)),
                    ConstructKind::Variant {
                        enum_id,
                        index,
                        name,
                    } => format!("construct {} {} {}", nominal(enum_id), index, name),
                };
                if !is_ascending(evaluation_order) {
                    let names: Vec<String> = evaluation_order
                        .iter()
                        .map(|&i| match fields.get(i).and_then(|f| f.name.clone()) {
                            Some(name) => name,
                            None => i.to_string(),
                        })
                        .collect();
                    write!(text, " (order: {})", names.join(", ")).unwrap();
                }
                text
            }
            ExpressionKind::Range { end_inclusive, .. } => {
                format!("range {}", if *end_inclusive { ".." } else { "..<" })
            }
            ExpressionKind::Binary { operator, .. } => format!("binary {}", operator.as_str()),
            ExpressionKind::Unary { operator, .. } => format!("unary {}", operator.as_str()),
            ExpressionKind::Logical { operator, .. } => format!("logical {}", operator.as_str()),
            ExpressionKind::Elvis { .. } => "elvis".to_string(),
            ExpressionKind::Template { .. } => "template".to_string(),
            ExpressionKind::Block { .. } => "block".to_string(),
            ExpressionKind::If { .. } => "if".to_string(),
            ExpressionKind::When { exhaustive, .. } => {
                if *exhaustive {
                    "when exhaustive".to_string()
                } else {
                    "when".to_string()
                }
            }
            ExpressionKind::Return { .. } => "return".to_string(),
            ExpressionKind::Break { target } => format!("break {}", target_text(*target)),
            ExpressionKind::Continue { target } => format!("continue {}", target_text(*target)),
            ExpressionKind::Closure {
                parameters,
                is_compose,
                captures,
                ownership_reason,
                ..
            } => {
                let parameters: Vec<String> =
                    parameters.iter().map(|p| self.local_name(*p)).collect();
                let mut text = format!(
                    "closure{}({})",
                    if *is_compose { " compose " } else { " " },
                    parameters.join(", ")
                );
                if !captures.is_empty() {
                    let captures: Vec<String> = captures
                        .iter()
                        .map(|c| {
                            let mode = match c.mode {
                                CaptureMode::Borrowed => "borrow",
                                CaptureMode::Moved => "move",
                                CaptureMode::Copied => "copy",
                            };
                            format!("{} {}", mode, self.local_name(c.local))
                        })
                        .collect();
                    write!(text, " captures [{}]", captures.join(", ")).unwrap();
                }
                if let Some(reason) = ownership_reason {
                    write!(
                        text,
                        " (owning: {} {})",
                        reason.short(),
                        position(reason.span)
                    )
                    .unwrap();
                }
                text
            }
            ExpressionKind::Error { .. } => "error".to_string(),
        };
        let text = format!("{}: {} {}", text, expression.ty, position(expression.span));
        self.line(depth, label, &text);
        let depth = depth + 1;
        match kind {
            ExpressionKind::Literal(_)
            | ExpressionKind::Local(_)
            | ExpressionKind::TopLevelValue(_)
            | ExpressionKind::FunctionReference(_)
            | ExpressionKind::VariantConstructor { .. }
            | ExpressionKind::Break { .. }
            | ExpressionKind::Continue { .. } => {}
            ExpressionKind::Field { base, .. } => self.expression(*base, depth, ""),
            ExpressionKind::Deref { pointer } => self.expression(*pointer, depth, ""),
            ExpressionKind::Index { base, index } => {
                self.expression(*base, depth, "base");
                self.expression(*index, depth, "index");
            }
            ExpressionKind::NonNullPayload { nullable } => self.expression(*nullable, depth, ""),
            ExpressionKind::Temporary { value } => self.expression(*value, depth, ""),
            ExpressionKind::Move { place } | ExpressionKind::Copy { place } => {
                self.expression(*place, depth, "")
            }
            ExpressionKind::Borrow { place, .. } => self.expression(*place, depth, ""),
            ExpressionKind::SafeNavigation { chain, .. } => self.expression(*chain, depth, ""),
            ExpressionKind::Coerce { value, .. } => self.expression(*value, depth, ""),
            ExpressionKind::Call(call) => {
                if let Callee::Value(callee) = &call.callee {
                    self.expression(*callee, depth, "callee");
                }
                for (index, argument) in call.arguments.iter().enumerate() {
                    let label = if call.has_receiver && index == 0 {
                        "receiver".to_string()
                    } else {
                        format!("argument {}", parameter_label(call, index))
                    };
                    match argument {
                        Argument::Written(value) => self.expression(*value, depth, &label),
                        Argument::Default => self.line(depth, &label, "default"),
                    }
                }
            }
            ExpressionKind::Construct { fields, .. } => {
                for (index, field) in fields.iter().enumerate() {
                    let label = match &field.name {
                        Some(name) => format!("field {}", name),
                        None => format!("payload {}", index),
                    };
                    self.expression(field.value, depth, &label);
                }
            }
            ExpressionKind::Range { start, end, .. } => {
                self.expression(*start, depth, "start");
                self.expression(*end, depth, "end");
            }
            ExpressionKind::Binary { left, right, .. }
            | ExpressionKind::Logical { left, right, .. } => {
                self.expression(*left, depth, "");
                self.expression(*right, depth, "");
            }
            ExpressionKind::Unary { operand, .. } => self.expression(*operand, depth, ""),
            ExpressionKind::Elvis { nullable, fallback } => {
                self.expression(*nullable, depth, "nullable");
                self.expression(*fallback, depth, "fallback");
            }
            ExpressionKind::Template { parts } => {
                for part in parts {
                    match part {
                        TemplatePart::Text(text) => {
                            self.line(depth, "", &format!("text {:?}", text))
                        }
                        TemplatePart::Value(value) => self.expression(*value, depth, ""),
                    }
                }
            }
            ExpressionKind::Block {
                statements, tail, ..
            } => {
                for statement in statements {
                    self.statement(*statement, depth);
                }
                if let Some(tail) = tail {
                    self.expression(*tail, depth, "tail");
                }
            }
            ExpressionKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.expression(*condition, depth, "condition");
                self.expression(*then_branch, depth, "then");
                if let Some(else_branch) = else_branch {
                    self.expression(*else_branch, depth, "else");
                }
            }
            ExpressionKind::When {
                subject, branches, ..
            } => {
                if let Some(subject) = subject {
                    self.expression(*subject, depth, "subject");
                }
                for branch in branches {
                    self.line(depth, "", &format!("branch {}", position(branch.span)));
                    for pattern in &branch.patterns {
                        self.pattern(pattern, depth + 1);
                    }
                    if let Some(guard) = branch.guard {
                        self.expression(guard, depth + 1, "guard");
                    }
                    self.expression(branch.body, depth + 1, "body");
                }
            }
            ExpressionKind::Return { value } => {
                if let Some(value) = value {
                    self.expression(*value, depth, "");
                }
            }
            ExpressionKind::Closure { body, .. } => self.expression(*body, depth, ""),
            ExpressionKind::Error { children } => {
                for child in children {
                    self.expression(*child, depth, "");
                }
            }
        }
    }

    /// A pattern, its sub-patterns indented under it (D14 of `payload-patterns`).
    fn pattern(&mut self, pattern: &Pattern, depth: usize) {
        match pattern {
            Pattern::Variant {
                enum_id,
                index,
                name,
                arguments,
                fields,
            } => {
                let text = format!(
                    "pattern variant {}{} {} {}",
                    nominal(enum_id),
                    type_arguments_text(arguments),
                    index,
                    name
                );
                self.line(depth, "", &text);
                for field in fields {
                    self.pattern(field, depth + 1);
                }
            }
            Pattern::Struct {
                id,
                arguments,
                fields,
            } => {
                let text = format!(
                    "pattern struct {}{}",
                    nominal(id),
                    type_arguments_text(arguments)
                );
                self.line(depth, "", &text);
                for field in fields {
                    self.pattern(field, depth + 1);
                }
            }
            Pattern::NonNull(inner) => {
                self.line(depth, "", "pattern non-null");
                self.pattern(inner, depth + 1);
            }
            Pattern::Deref(inner) => {
                self.line(depth, "", "pattern deref");
                self.pattern(inner, depth + 1);
            }
            Pattern::Wildcard => self.line(depth, "", "pattern wildcard"),
            Pattern::Binding { local, mode } => {
                let ty = self
                    .body
                    .locals
                    .get(*local)
                    .map_or_else(|| "<unknown>".to_string(), |local| local.ty.to_string());
                let text = format!(
                    "pattern binding {} ({}): {}",
                    self.local_name(*local),
                    mode.as_str(),
                    ty
                );
                self.line(depth, "", &text);
            }
            Pattern::Value(value) => self.expression(*value, depth, "pattern value"),
            Pattern::Range {
                start,
                end,
                end_inclusive,
            } => {
                self.line(
                    depth,
                    "",
                    &format!(
                        "pattern range {}",
                        if *end_inclusive { ".." } else { "..<" }
                    ),
                );
                self.expression(*start, depth + 1, "start");
                self.expression(*end, depth + 1, "end");
            }
            Pattern::Condition(condition) => {
                self.expression(*condition, depth, "pattern condition")
            }
            Pattern::Else => self.line(depth, "", "pattern else"),
        }
    }

    fn statement(&mut self, id: StatementId, depth: usize) {
        let Some(statement) = self.body.statements.get(id) else {
            self.line(depth, "", &format!("<missing statement {}>", id.0));
            return;
        };
        let at = position(statement.span);
        match &statement.kind {
            StatementKind::Let { local, initializer } => {
                let text = format!("let {} {}", self.local_name(*local), at);
                self.line(depth, "", &text);
                self.expression(*initializer, depth + 1, "");
            }
            StatementKind::Assign { target, value } => {
                self.line(depth, "", &format!("assign {}", at));
                self.expression(*target, depth + 1, "target");
                self.expression(*value, depth + 1, "value");
            }
            StatementKind::CompoundAssign {
                operator,
                target,
                value,
            } => {
                let text = format!("compound-assign {} {}", operator.as_str(), at);
                self.line(depth, "", &text);
                self.expression(*target, depth + 1, "target");
                self.expression(*value, depth + 1, "value");
            }
            StatementKind::Expression { value } => {
                self.line(depth, "", &format!("expression {}", at));
                self.expression(*value, depth + 1, "");
            }
            StatementKind::While { condition, body } => {
                self.line(depth, "", &format!("while {} {}", id, at));
                self.expression(*condition, depth + 1, "condition");
                self.expression(*body, depth + 1, "body");
            }
            StatementKind::DoWhile { body, condition } => {
                self.line(depth, "", &format!("do-while {} {}", id, at));
                self.expression(*body, depth + 1, "body");
                self.expression(*condition, depth + 1, "condition");
            }
            StatementKind::For {
                variable,
                range,
                body,
            } => {
                let text = format!("for {} {} {}", id, self.local_name(*variable), at);
                self.line(depth, "", &text);
                self.expression(*range, depth + 1, "range");
                self.expression(*body, depth + 1, "body");
            }
            StatementKind::Destructure { pattern, value } => {
                self.line(depth, "", &format!("destructure {}", at));
                self.expression(*value, depth + 1, "value");
                self.pattern(pattern, depth + 1);
            }
        }
    }
}

/// The name of parameter `index` of `call`, or its index when the callee has no names.
fn parameter_label(call: &crate::tree::Call, index: usize) -> String {
    match call
        .parameters
        .get(index)
        .and_then(|(name, _)| name.clone())
    {
        Some(name) => name,
        None => index.to_string(),
    }
}

fn is_ascending(order: &[usize]) -> bool {
    order.windows(2).all(|pair| pair[0] < pair[1])
}

fn target_text(target: Option<StatementId>) -> String {
    match target {
        Some(target) => target.to_string(),
        None => "<no loop>".to_string(),
    }
}

fn literal_text(literal: &Literal) -> String {
    match literal {
        Literal::Int(v) => v.to_string(),
        Literal::Float(v) => format!("{:?}", v),
        Literal::String(v) => format!("{:?}", v),
        Literal::Char(v) => format!("{:?}", v),
        Literal::Bool(v) => v.to_string(),
        Literal::Null => "null".to_string(),
    }
}

/// `<A, B>` for type arguments, empty for none.
fn type_arguments_text(arguments: &[Type]) -> String {
    if arguments.is_empty() {
        String::new()
    } else {
        let arguments: Vec<String> = arguments.iter().map(ToString::to_string).collect();
        format!("<{}>", arguments.join(", "))
    }
}

fn nominal(id: &NominalId) -> String {
    format!("{}::{}", id.module, id.name)
}

fn position(span: Span) -> String {
    format!("@{}:{}", span.line, span.column)
}

fn visibility_prefix(visibility: Visibility) -> &'static str {
    match visibility {
        Visibility::Private => "",
        Visibility::Package => "package ",
        Visibility::Public => "public ",
    }
}

fn marker_suffix(marker: Option<TypeMarker>) -> String {
    marker.map_or(String::new(), |m| format!(" : {}", m.as_str()))
}

/// The marker of a generic struct or enum, with the type parameters that condition it
/// (` : Copy if T`).
fn generic_marker_suffix(
    marker: Option<TypeMarker>,
    type_parameters: &[TypeParameter],
    marker_parameters: &[u32],
) -> String {
    let mut suffix = marker_suffix(marker);
    if marker.is_some() && !marker_parameters.is_empty() {
        let names: Vec<&str> = marker_parameters
            .iter()
            .filter_map(|index| type_parameters.get(*index as usize))
            .map(|p| p.name.as_str())
            .collect();
        write!(suffix, " if {}", names.join(", ")).unwrap();
    }
    suffix
}

/// A call's type arguments as written after the callee (`<Int>`), or nothing.
fn render_type_arguments(arguments: &[Type]) -> String {
    if arguments.is_empty() {
        return String::new();
    }
    let rendered: Vec<String> = arguments.iter().map(ToString::to_string).collect();
    format!("<{}>", rendered.join(", "))
}

#[cfg(test)]
pub(crate) mod tests {
    use crate::arena::Arena;
    use crate::tree::*;
    use crate::types::Type;
    use prism_ast::{Literal, Span, Visibility};

    fn span(line: u32, column: u32) -> Span {
        Span::new(0, 0, line, column)
    }

    /// `fun f(a: Int): Int = a + 1`, built by hand.
    pub(crate) fn addition_file() -> TypedFile {
        let path = DeclarationPath::new("main", "f");
        let mut body = Body::new(path.clone());
        let a = body.locals.alloc(Local {
            name: "a".to_string(),
            ty: Type::Int,
            mutable: false,
            kind: LocalKind::Parameter,
            escaping: false,
            span: span(1, 7),
        });
        body.parameters.push(a);
        body.defaults.push(None);
        let left = body.expressions.alloc(Expression {
            kind: ExpressionKind::Local(a),
            ty: Type::Int,
            span: span(1, 23),
        });
        let right = body.expressions.alloc(Expression {
            kind: ExpressionKind::Literal(Literal::Int(1)),
            ty: Type::Int,
            span: span(1, 27),
        });
        let sum = body.expressions.alloc(Expression {
            kind: ExpressionKind::Binary {
                operator: BinaryOperator::Add,
                left,
                right,
            },
            ty: Type::Int,
            span: span(1, 23),
        });
        body.value = Some(sum);
        let mut bodies = Arena::new();
        let body = bodies.alloc(body);
        TypedFile {
            module: "main".to_string(),
            package: None,
            items: vec![Item::Function(FunctionItem {
                path,
                visibility: Visibility::Private,
                is_compose: false,
                type_parameters: Vec::new(),
                receiver: None,
                parameters: vec![("a".to_string(), Type::Int)],
                return_type: Type::Int,
                body,
                span: span(1, 1),
            })],
            bodies,
            types: crate::types::TypeTable::with_builtins(),
        }
    }

    #[test]
    fn dumps_a_small_tree() {
        let file = addition_file();
        let expected = "\
module main
fun main::f(a: Int): Int @1:1
  local %0 a: Int (parameter)
  binary +: Int @1:23
    local %0 a: Int @1:23
    literal 1: Int @1:27
";
        assert_eq!(file.dump(), expected);
        assert_eq!(file.dump(), file.dump());
    }

    #[test]
    fn dumps_imported_types_in_id_order() {
        use crate::types::{NominalId, TypeDef};
        use prism_ast::TypeMarker;
        let mut file = addition_file();
        let user = NominalId::new("models/user", "User");
        let color = NominalId::new("models/color", "Color");
        let money = NominalId::new("shop/money", "Money");
        let own = NominalId::new("main", "Own");
        file.types.insert_definition(
            user.clone(),
            TypeDef::Struct {
                parameters: Vec::new(),
                fields: vec![("name".to_string(), false, Type::String)],
            },
        );
        file.types
            .insert_marker(user, Some(TypeMarker::Clone), Vec::new());
        file.types.insert_definition(
            color.clone(),
            TypeDef::Enum {
                parameters: Vec::new(),
                variants: vec![
                    ("Red".to_string(), vec![]),
                    ("Rgb".to_string(), vec![Type::Int]),
                ],
            },
        );
        file.types.insert_marker(color, None, Vec::new());
        file.types
            .insert_marker(money, Some(TypeMarker::Copy), Vec::new());
        file.types.insert_marker(own, None, Vec::new());
        let dump = file.dump();
        let header = "\
module main
import enum models/color::Color
  variant 0 Red
  variant 1 Rgb(Int)
import struct models/user::User : Clone
  field 0 val name: String
import type shop/money::Money : Copy
fun main::f(a: Int): Int @1:1
";
        assert!(dump.starts_with(header), "{}", dump);
    }
}

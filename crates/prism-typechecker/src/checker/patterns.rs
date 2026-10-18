//! Checking the patterns of `when` branches and destructuring declarations against their
//! subject (D1 to D6, D9 and D10 of `payload-patterns`).
//!
//! A pattern is checked against the type of the part it matches, and builds its typed-tree
//! [`Pattern`]. Constructor patterns resolve their name to a variant or a struct, and otherwise
//! fall back to the value their syntax spells. Bindings take the mode the subject's type gives
//! them (D5): by value for an owned subject, by shared borrow (copying `Copy` parts) through a
//! `&`, by mutable borrow through a `&mut`. A branch's bindings are collected per pattern, so
//! that alternatives can be compared and share one binding per name (D10). Constructor, value
//! and range patterns look through boxes, matching their contents in the same mode, while
//! wildcards and bindings take the box itself (D9 of `heap-boxes`).

use super::{binding_name, TypeChecker, THIS};
use crate::error::TypeError;
use crate::scope::{Mutability, Symbol, SymbolKind};
use crate::types::{NominalId, Type};
use prism_ast::expr::{Expr, ExprKind, PatternArgument, PatternKind};
use prism_ast::statement::DestructureEntry;
use prism_ast::Span;
use prism_typed_tree::{
    BindingMode, LocalId, LocalKind, Pattern, StatementId, StatementKind as TreeStatementKind,
};

/// How the subject of a pattern is held, after the type checker peels its references (D5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SubjectMode {
    /// An owned value: parts are copied or moved.
    Owned,
    /// Reached through a shared borrow: `Copy` parts are copied, others borrowed.
    Shared,
    /// Reached through mutable borrows only: every part is borrowed mutably.
    Mutable,
}

impl SubjectMode {
    /// The mode of a binding of a part whose type is `Copy` when `is_copy`.
    fn binding_mode(self, is_copy: bool) -> BindingMode {
        match (self, is_copy) {
            (SubjectMode::Owned, true) | (SubjectMode::Shared, true) => BindingMode::Copy,
            (SubjectMode::Owned, false) => BindingMode::Move,
            (SubjectMode::Shared, false) => BindingMode::SharedBorrow,
            (SubjectMode::Mutable, _) => BindingMode::MutableBorrow,
        }
    }
}

/// One binding of a pattern.
#[derive(Debug, Clone)]
pub(super) struct PatternBinding {
    pub(super) name: String,
    pub(super) local: LocalId,
    pub(super) ty: Type,
    pub(super) mode: BindingMode,
    pub(super) is_var: bool,
    pub(super) span: Span,
}

impl PatternBinding {
    /// The symbol naming the binding: read-only in a guard, as declared in a branch body.
    pub(super) fn symbol(&self, in_guard: bool) -> Symbol {
        let mutability = if self.is_var && !in_guard {
            Mutability::Var
        } else {
            Mutability::Val
        };
        Symbol::variable(&self.name, self.ty.clone(), mutability, self.span).with_local(self.local)
    }
}

/// The bindings one pattern declares, in source order. For an alternative after a branch's
/// first pattern, `first` holds that pattern's bindings, whose locals it reuses (D10).
#[derive(Debug, Default)]
pub(super) struct PatternBindings {
    pub(super) bound: Vec<PatternBinding>,
    pub(super) first: Option<Vec<PatternBinding>>,
}

/// What the name of a constructor pattern resolves to.
enum Constructor {
    /// Variant `index` of the enum `id`.
    Variant { id: NominalId, index: usize },
    /// The struct `id`.
    Struct(NominalId),
    /// Neither: the pattern is the value its syntax spells, if it spells one (D2).
    Other,
}

impl TypeChecker {
    /// The subject mode of a value whose node had `layers` reference layers peeled, outermost
    /// first: any shared layer makes it shared (D5).
    pub(super) fn subject_mode(layers: &[bool]) -> SubjectMode {
        if layers.is_empty() {
            SubjectMode::Owned
        } else if layers.iter().all(|mutable| *mutable) {
            SubjectMode::Mutable
        } else {
            SubjectMode::Shared
        }
    }

    /// Checks `pattern` against a part of type `part`, held by `mode`, recording its bindings in
    /// `bindings`.
    pub(super) fn check_pattern(
        &mut self,
        pattern: &prism_ast::Pattern,
        part: &Type,
        mode: SubjectMode,
        bindings: &mut PatternBindings,
    ) -> Pattern {
        match &pattern.kind {
            PatternKind::Wildcard => Pattern::Wildcard,
            PatternKind::Binding { is_var, name } => {
                self.bind_pattern_name(name, *is_var, part, mode, pattern.span, bindings)
            }
            // Any other pattern matches the contents of a box, keeping the subject's mode (D9 of
            // `heap-boxes`); a wildcard or a binding takes the box itself.
            _ if matches!(part, Type::Heap(_)) => {
                let Type::Heap(contents) = part else {
                    unreachable!("matched above")
                };
                let inner = self.check_pattern(pattern, contents, mode, bindings);
                Pattern::Deref(Box::new(inner))
            }
            PatternKind::InRange {
                start,
                end,
                end_inclusive,
            } => {
                let start = self.check_expr(start, part);
                let end = self.check_expr(end, part);
                Pattern::Range {
                    start,
                    end,
                    end_inclusive: *end_inclusive,
                }
            }
            PatternKind::Value(value) => self.value_pattern(value, part),
            PatternKind::Constructor { callee, arguments } => {
                self.constructor_pattern(pattern, callee, arguments, part, mode, bindings)
            }
        }
    }

    /// A binding `val name` or `var name` of a part of type `part`.
    fn bind_pattern_name(
        &mut self,
        name: &str,
        is_var: bool,
        part: &Type,
        mode: SubjectMode,
        span: Span,
        bindings: &mut PatternBindings,
    ) -> Pattern {
        let binding_mode = mode.binding_mode(self.is_copy(part));
        let ty = binding_mode.binding_type(part);
        if let Some(earlier) = bindings.bound.iter().find(|b| b.name == name) {
            // One pattern binding a name twice (`E4010`); the first binding stands.
            let _ = earlier;
            self.errors.push(TypeError::DuplicateSymbol {
                name: name.to_string(),
                span,
            });
            return Pattern::Wildcard;
        }
        // An alternative reuses the local of the first pattern's binding of the same name when
        // it agrees with it; any difference is reported once the pattern is checked.
        let reused = bindings.first.as_ref().and_then(|first| {
            first
                .iter()
                .find(|b| {
                    b.name == name && b.ty == ty && b.mode == binding_mode && b.is_var == is_var
                })
                .map(|b| b.local)
        });
        let local = reused.unwrap_or_else(|| {
            self.builder.add_local(prism_typed_tree::Local {
                name: name.to_string(),
                ty: ty.clone(),
                mutable: is_var,
                kind: LocalKind::PatternBinding,
                span,
            })
        });
        bindings.bound.push(PatternBinding {
            name: name.to_string(),
            local,
            ty,
            mode: binding_mode,
            is_var,
            span,
        });
        Pattern::Binding {
            local,
            mode: binding_mode,
        }
    }

    /// A value pattern, or a variant written without parentheses: `Red`, `Shape.Circle` (D3),
    /// which matches the variant whatever its payloads. An unresolved bare name is `E4001` with
    /// help suggesting a binding (D1).
    fn value_pattern(&mut self, value: &Expr, part: &Type) -> Pattern {
        let (enum_part, nullable) = match part {
            Type::Enum { .. } => (Some(part.clone()), false),
            Type::Nullable(inner) if matches!(inner.as_ref(), Type::Enum { .. }) => {
                (Some(inner.as_ref().clone()), true)
            }
            _ => (None, false),
        };
        if let Some(Type::Enum { id, arguments }) = &enum_part {
            if let Some(index) = self.variant_named(value, id) {
                if self.reject_unimported_members(enum_part.as_ref().expect("an enum"), value.span)
                {
                    return Pattern::Value(self.error_node(Vec::new(), value.span));
                }
                let payloads = self.variant_payloads(id, arguments, index);
                let pattern = Pattern::Variant {
                    enum_id: id.clone(),
                    index,
                    name: self.variant_name(id, index),
                    arguments: arguments.clone(),
                    fields: payloads.iter().map(|_| Pattern::Wildcard).collect(),
                };
                return if nullable {
                    Pattern::NonNull(Box::new(pattern))
                } else {
                    pattern
                };
            }
        }
        if let ExprKind::Ident(name) = &value.kind {
            if self.is_unbound_pattern_name(name) {
                self.errors.push(TypeError::UnboundPatternName {
                    name: name.clone(),
                    span: value.span,
                });
                return Pattern::Value(self.error_node(Vec::new(), value.span));
            }
        }
        Pattern::Value(self.check_expr(value, part))
    }

    /// Whether the bare name `name` resolves to nothing at all, so that a pattern naming it was
    /// most likely meant to bind it.
    fn is_unbound_pattern_name(&self, name: &str) -> bool {
        self.env.lookup(name).is_none()
            && !self.variant_owners.contains_key(name)
            && !self.ambiguous_variants.contains(name)
            && self.unimported_type_named(name).is_none()
            && super::output_builtin_named(name).is_none()
            && name != THIS
    }

    /// The index of the variant of enum `id` that `value` names: `Enum.Variant` when `Enum` is
    /// that enum, or a bare `Variant` that no local binding shadows.
    fn variant_named(&self, value: &Expr, id: &NominalId) -> Option<usize> {
        let name = match &value.kind {
            ExprKind::Ident(name) => {
                if self.env.resolves_to_local(name) {
                    return None;
                }
                name
            }
            ExprKind::MemberAccess {
                target,
                member,
                is_safe_nav: false,
            } => match self.type_name_symbol(target).map(|sym| sym.kind) {
                Some(SymbolKind::Enum { id: named, .. }) if &named == id => member,
                _ => return None,
            },
            _ => return None,
        };
        self.enum_variants(id)?
            .iter()
            .position(|(variant, _)| variant == name)
    }

    /// The name of variant `index` of enum `id`.
    fn variant_name(&self, id: &NominalId, index: usize) -> String {
        self.types
            .enum_variants(id)
            .and_then(|variants| variants.get(index))
            .map_or_else(String::new, |(name, _)| name.clone())
    }

    /// The payload types of variant `index` of enum `id` instantiated with `arguments`.
    fn variant_payloads(&self, id: &NominalId, arguments: &[Type], index: usize) -> Vec<Type> {
        self.types
            .instantiated_variants(id, arguments)
            .and_then(|variants| variants.into_iter().nth(index))
            .map_or_else(Vec::new, |(_, payloads)| payloads)
    }

    /// What the name of a constructor pattern resolves to, against a part whose enum (if any) is
    /// `part_enum`. A qualified `Enum.Variant` resolves through its type name; a bare name to a
    /// variant of the part's enum, then to a module-level variant or a struct. Reports `E4007`
    /// for a type name that is not visible.
    fn resolve_constructor(&mut self, callee: &Expr, part_enum: Option<&NominalId>) -> Constructor {
        match &callee.kind {
            ExprKind::MemberAccess {
                target,
                member,
                is_safe_nav: false,
            } => {
                let Some(sym) = self.type_name_symbol(target) else {
                    return Constructor::Other;
                };
                let SymbolKind::Enum { id, .. } = &sym.kind else {
                    return Constructor::Other;
                };
                let id = id.clone();
                let index = self
                    .enum_variants(&id)
                    .and_then(|variants| variants.iter().position(|(name, _)| name == member));
                match index {
                    Some(index) => {
                        self.check_visibility(&sym, target.span);
                        Constructor::Variant { id, index }
                    }
                    None => Constructor::Other,
                }
            }
            ExprKind::Ident(name) => {
                // Only a local shadows: a module-level or imported name does not.
                if self.env.resolves_to_local(name) {
                    return Constructor::Other;
                }
                if let Some(id) = part_enum {
                    let index = self
                        .enum_variants(id)
                        .and_then(|variants| variants.iter().position(|(v, _)| v == name));
                    if let Some(index) = index {
                        return Constructor::Variant {
                            id: id.clone(),
                            index,
                        };
                    }
                }
                if let Some(sym) = self.type_name_symbol(callee) {
                    if let SymbolKind::Struct { id, .. } = &sym.kind {
                        let id = id.clone();
                        self.check_visibility(&sym, callee.span);
                        return Constructor::Struct(id);
                    }
                    return Constructor::Other;
                }
                if self.variant_owners.contains_key(name) {
                    let owner = match self.env.lookup(name).map(|sym| &sym.kind) {
                        Some(SymbolKind::Function {
                            fn_type: Type::Fn { ret, .. },
                            ..
                        }) => ret.nominal_id().cloned(),
                        Some(SymbolKind::Variable { ty, .. }) => ty.nominal_id().cloned(),
                        _ => None,
                    };
                    if let Some(id) = owner {
                        let index = self
                            .enum_variants(&id)
                            .and_then(|variants| variants.iter().position(|(v, _)| v == name));
                        if let Some(index) = index {
                            return Constructor::Variant { id, index };
                        }
                    }
                }
                Constructor::Other
            }
            _ => Constructor::Other,
        }
    }

    /// A constructor pattern `callee(arguments)` against a part of type `part`.
    fn constructor_pattern(
        &mut self,
        pattern: &prism_ast::Pattern,
        callee: &Expr,
        arguments: &[PatternArgument],
        part: &Type,
        mode: SubjectMode,
        bindings: &mut PatternBindings,
    ) -> Pattern {
        if part == &Type::Error {
            self.sub_patterns_of_error(arguments, mode, bindings);
            return Pattern::Wildcard;
        }
        // A constructor pattern of `T` matches a non-null `T?`, and the contents of a non-null
        // `Heap<T>?`.
        let (target, nullable) = match part {
            Type::Nullable(inner) => (inner.as_ref().clone(), true),
            other => (other.clone(), false),
        };
        if nullable && matches!(target, Type::Heap(_)) {
            let inner = self.check_pattern(pattern, &target, mode, bindings);
            return Pattern::NonNull(Box::new(inner));
        }
        let part_enum = match &target {
            Type::Enum { id, .. } => Some(id.clone()),
            _ => None,
        };
        let resolved = match self.resolve_constructor(callee, part_enum.as_ref()) {
            Constructor::Other => {
                return match pattern.to_expression() {
                    Some(value) => Pattern::Value(self.check_expr(&value, part)),
                    None => {
                        self.errors.push(TypeError::InvalidPattern {
                            message: format!(
                                "'{}' is not a variant or a struct: only a variant or a struct can be matched by its parts",
                                callee
                            ),
                            span: callee.span,
                            help: Some(
                                "match the value the call returns with a binding and a guard, as in 'val n if n == f(...)'"
                                    .to_string(),
                            ),
                        });
                        self.sub_patterns_of_error(arguments, mode, bindings);
                        Pattern::Wildcard
                    }
                };
            }
            resolved => resolved,
        };
        let matched = match (&resolved, &target) {
            (Constructor::Variant { id, .. }, Type::Enum { id: part_id, .. })
            | (Constructor::Struct(id), Type::Struct { id: part_id, .. }) => id == part_id,
            _ => false,
        };
        if !matched {
            let found = match &resolved {
                Constructor::Variant { id, .. } => Type::Enum {
                    id: id.clone(),
                    arguments: Vec::new(),
                },
                Constructor::Struct(id) => Type::Struct {
                    id: id.clone(),
                    arguments: Vec::new(),
                },
                Constructor::Other => unreachable!("handled above"),
            };
            if part != &Type::Error {
                self.report_mismatch(part, found, pattern.span);
            }
            self.sub_patterns_of_error(arguments, mode, bindings);
            return Pattern::Wildcard;
        }
        if self.reject_unimported_members(&target, callee.span) {
            self.sub_patterns_of_error(arguments, mode, bindings);
            return Pattern::Wildcard;
        }
        let inner = match (resolved, &target) {
            (
                Constructor::Variant { id, index },
                Type::Enum {
                    arguments: args, ..
                },
            ) => {
                let payloads = self.variant_payloads(&id, args, index);
                let name = self.variant_name(&id, index);
                let fields = self.variant_sub_patterns(&name, arguments, &payloads, mode, bindings);
                Pattern::Variant {
                    enum_id: id,
                    index,
                    name,
                    arguments: args.clone(),
                    fields,
                }
            }
            (
                Constructor::Struct(id),
                Type::Struct {
                    arguments: args, ..
                },
            ) => {
                let fields = self
                    .types
                    .instantiated_fields(&id, args)
                    .unwrap_or_default();
                let fields =
                    self.struct_sub_patterns(&id, &target, arguments, &fields, mode, bindings);
                Pattern::Struct {
                    id,
                    arguments: args.clone(),
                    fields,
                }
            }
            _ => unreachable!("matched above"),
        };
        if nullable {
            Pattern::NonNull(Box::new(inner))
        } else {
            inner
        }
    }

    /// The sub-patterns of variant `name`, one positional pattern per payload (D1).
    fn variant_sub_patterns(
        &mut self,
        name: &str,
        arguments: &[PatternArgument],
        payloads: &[Type],
        mode: SubjectMode,
        bindings: &mut PatternBindings,
    ) -> Vec<Pattern> {
        let invalid = if payloads.is_empty() {
            Some((
                format!(
                    "variant '{}' has no payload, so its pattern takes no arguments",
                    name
                ),
                Some(format!("write '{}' without parentheses", name)),
            ))
        } else if let Some(named) = arguments.iter().find(|a| a.name.is_some()) {
            let _ = named;
            Some((
                format!(
                    "the payloads of variant '{}' have no names, so its sub-patterns are positional",
                    name
                ),
                Some("remove the names, giving one sub-pattern per payload in order".to_string()),
            ))
        } else if arguments.len() != payloads.len() {
            Some((
                format!(
                    "variant '{}' has {} payload{}, but the pattern gives {}",
                    name,
                    payloads.len(),
                    if payloads.len() == 1 { "" } else { "s" },
                    arguments.len()
                ),
                Some(format!(
                    "give one sub-pattern per payload, using '_' for the ones to ignore, or write '{}' alone to match any payloads",
                    name
                )),
            ))
        } else {
            None
        };
        if let Some((message, help)) = invalid {
            let span = arguments
                .iter()
                .map(|a| a.span)
                .reduce(Span::merge)
                .unwrap_or_default();
            self.errors.push(TypeError::InvalidPattern {
                message,
                span,
                help,
            });
            self.sub_patterns_of_error(arguments, mode, bindings);
            return payloads.iter().map(|_| Pattern::Wildcard).collect();
        }
        arguments
            .iter()
            .zip(payloads)
            .map(|(argument, payload)| {
                self.check_pattern(&argument.pattern, payload, mode, bindings)
            })
            .collect()
    }

    /// The sub-patterns of the struct `id` of type `struct_ty`, positional for every field or
    /// named, positional ones first; fields the named form leaves out are wildcards (D4).
    fn struct_sub_patterns(
        &mut self,
        id: &NominalId,
        struct_ty: &Type,
        arguments: &[PatternArgument],
        fields: &[(String, bool, Type)],
        mode: SubjectMode,
        bindings: &mut PatternBindings,
    ) -> Vec<Pattern> {
        let mut slots: Vec<Option<&PatternArgument>> = vec![None; fields.len()];
        let mut invalid = None;
        let mut seen_named = false;
        let mut positional = 0;
        for argument in arguments {
            match &argument.name {
                None if seen_named => {
                    invalid.get_or_insert((
                        "a positional sub-pattern cannot follow a named one".to_string(),
                        argument.span,
                    ));
                }
                None => {
                    if positional < fields.len() {
                        slots[positional] = Some(argument);
                    }
                    positional += 1;
                }
                Some(name) => {
                    seen_named = true;
                    match fields.iter().position(|(field, _, _)| field == name) {
                        Some(index) if slots[index].is_some() => {
                            invalid.get_or_insert((
                                format!("field '{}' is matched twice", name),
                                argument.span,
                            ));
                        }
                        Some(index) => slots[index] = Some(argument),
                        None => {
                            self.errors.push(TypeError::UnknownField {
                                target_type: struct_ty.clone(),
                                field_name: name.clone(),
                                span: argument.span,
                                help: None,
                            });
                            self.check_pattern(&argument.pattern, &Type::Error, mode, bindings);
                        }
                    }
                }
            }
        }
        if invalid.is_none() && !seen_named && positional != fields.len() {
            let span = arguments
                .iter()
                .map(|a| a.span)
                .reduce(Span::merge)
                .unwrap_or_default();
            invalid = Some((
                format!(
                    "struct '{}' has {} field{}, but the pattern gives {}; a positional pattern gives every field",
                    id.name,
                    fields.len(),
                    if fields.len() == 1 { "" } else { "s" },
                    positional
                ),
                span,
            ));
        } else if invalid.is_none() && positional > fields.len() {
            invalid = Some((
                format!(
                    "struct '{}' has {} field{}, but the pattern gives {} positional sub-patterns",
                    id.name,
                    fields.len(),
                    if fields.len() == 1 { "" } else { "s" },
                    positional
                ),
                arguments[fields.len()].span,
            ));
        }
        if let Some((message, span)) = invalid {
            self.errors.push(TypeError::InvalidPattern {
                message,
                span,
                help: Some(format!(
                    "give every field in declaration order, or name the fields to match, as in '{}({} = ...)'",
                    id.name,
                    fields.first().map_or("field", |(name, _, _)| name.as_str())
                )),
            });
            self.sub_patterns_of_error(arguments, mode, bindings);
            return fields.iter().map(|_| Pattern::Wildcard).collect();
        }
        slots
            .into_iter()
            .zip(fields)
            .map(|(argument, (_, _, ty))| match argument {
                Some(argument) => self.check_pattern(&argument.pattern, ty, mode, bindings),
                None => Pattern::Wildcard,
            })
            .collect()
    }

    /// Checks the sub-patterns of a pattern that did not fit its part against the error type,
    /// so that their bindings are still declared and nothing more is reported for them.
    fn sub_patterns_of_error(
        &mut self,
        arguments: &[PatternArgument],
        mode: SubjectMode,
        bindings: &mut PatternBindings,
    ) {
        for argument in arguments {
            self.check_pattern(&argument.pattern, &Type::Error, mode, bindings);
        }
    }

    /// Checks a destructuring declaration `val (entries) = initializer` (D13 of
    /// `payload-patterns`): the value is read in place, each entry binds the field it names as a
    /// pattern binding would, in the mode the value's type gives, and the bindings are defined in
    /// the current scope.
    pub(super) fn check_destructure(
        &mut self,
        is_var: bool,
        entries: &[DestructureEntry],
        initializer: &Expr,
        span: Span,
    ) -> StatementId {
        let mut layers = Vec::new();
        let mut value = self.infer_expr(initializer);
        while let Type::Reference { mutable, .. } = self.builder.type_of(value) {
            layers.push(*mutable);
            value = self.deref(value);
        }
        let mode = Self::subject_mode(&layers);
        let ty = self.ty(value);
        let mut bindings = PatternBindings::default();
        let fields = match &ty {
            Type::Struct { id, arguments }
                if !self.reject_unimported_members(&ty, initializer.span) =>
            {
                self.types.instantiated_fields(id, arguments)
            }
            Type::Struct { .. } | Type::Error => None,
            other => {
                let help = matches!(other, Type::Enum { .. }).then(|| {
                    "use a 'when' to match the variants of an enum, binding their payloads with 'val'"
                        .to_string()
                });
                self.errors.push(TypeError::InvalidPattern {
                    message: format!(
                        "only a struct can be destructured, but this value has type '{}'",
                        other
                    ),
                    span: initializer.span,
                    help,
                });
                None
            }
        };
        let pattern = match (fields, &ty) {
            (Some(fields), Type::Struct { id, arguments }) => {
                let mut slots: Vec<Option<&DestructureEntry>> = vec![None; fields.len()];
                for (position, entry) in entries.iter().enumerate() {
                    match fields.iter().position(|(name, _, _)| name == &entry.field) {
                        Some(index) if slots[index].is_some() => {
                            self.errors.push(TypeError::InvalidPattern {
                                message: format!("field '{}' is destructured twice", entry.field),
                                span: entry.field_span,
                                help: Some(format!(
                                    "bind it once; to bind a field under another name, write '{} as name'",
                                    entry.field
                                )),
                            });
                            self.bind_pattern_name(
                                &entry.name,
                                is_var,
                                &Type::Error,
                                mode,
                                entry.name_span,
                                &mut bindings,
                            );
                        }
                        Some(index) => slots[index] = Some(entry),
                        None => {
                            // Written the Kotlin way: positions are not fields (D13).
                            let help = fields.get(position).map(|(field, _, _)| {
                                format!(
                                    "a destructuring declaration binds fields by name: to bind field '{}', write '{} as {}'",
                                    field, field, entry.name
                                )
                            });
                            self.errors.push(TypeError::UnknownField {
                                target_type: ty.clone(),
                                field_name: entry.field.clone(),
                                span: entry.field_span,
                                help,
                            });
                            self.bind_pattern_name(
                                &entry.name,
                                is_var,
                                &Type::Error,
                                mode,
                                entry.name_span,
                                &mut bindings,
                            );
                        }
                    }
                }
                let sub_patterns = slots
                    .into_iter()
                    .zip(&fields)
                    .map(|(entry, (_, _, field_ty))| match entry {
                        Some(entry) => self.bind_pattern_name(
                            &entry.name,
                            is_var,
                            field_ty,
                            mode,
                            entry.name_span,
                            &mut bindings,
                        ),
                        None => Pattern::Wildcard,
                    })
                    .collect();
                Pattern::Struct {
                    id: id.clone(),
                    arguments: arguments.clone(),
                    fields: sub_patterns,
                }
            }
            _ => {
                for entry in entries {
                    self.bind_pattern_name(
                        &entry.name,
                        is_var,
                        &Type::Error,
                        mode,
                        entry.name_span,
                        &mut bindings,
                    );
                }
                Pattern::Wildcard
            }
        };
        if pattern.moves() {
            self.check_take_apart_source(initializer, "destructure");
            value = self.as_place(value);
        }
        for binding in &bindings.bound {
            self.define_local(binding.symbol(false));
        }
        self.builder
            .add_statement(TreeStatementKind::Destructure { pattern, value }, span)
    }

    /// Reports `E4025` when the alternative whose bindings are `bindings` does not bind exactly
    /// the names of the branch's first pattern, `first`, with the same types and modes (D10).
    /// Only the first difference is reported.
    pub(super) fn check_alternative_bindings(
        &mut self,
        first: &[PatternBinding],
        bindings: &[PatternBinding],
        span: Span,
    ) {
        for binding in first {
            match bindings.iter().find(|b| b.name == binding.name) {
                None => {
                    self.errors
                        .push(TypeError::InconsistentAlternativeBindings {
                            message: format!(
                            "'{}' is bound by the first pattern of the branch but not by this one",
                            binding.name
                        ),
                            span,
                        });
                    return;
                }
                Some(other) if other.local != binding.local => {
                    let difference = if other.ty != binding.ty {
                        format!(
                            "'{}' in one pattern and '{}' in another",
                            binding.ty, other.ty
                        )
                    } else if other.mode != binding.mode {
                        format!(
                            "a {} binding in one pattern and a {} binding in another",
                            binding.mode.as_str(),
                            other.mode.as_str()
                        )
                    } else {
                        "a 'val' in one pattern and a 'var' in another".to_string()
                    };
                    self.errors
                        .push(TypeError::InconsistentAlternativeBindings {
                            message: format!("binding '{}' is {}", binding.name, difference),
                            span: other.span,
                        });
                    return;
                }
                Some(_) => {}
            }
        }
        if let Some(extra) = bindings
            .iter()
            .find(|b| !first.iter().any(|f| f.name == b.name))
        {
            self.errors
                .push(TypeError::InconsistentAlternativeBindings {
                    message: format!(
                        "'{}' is not bound by the first pattern of the branch",
                        extra.name
                    ),
                    span: extra.span,
                });
        }
    }

    /// Rejects taking apart the value `expr` (D6 of `payload-patterns`): it must be a whole
    /// owned local or parameter, a consuming `this`, or a temporary. A field, a borrow's
    /// referent, a top-level `val`, a captured or default-borrowed binding, and a branch, block
    /// or `?:` expression are `E4017`.
    pub(super) fn check_take_apart_source(&mut self, expr: &Expr, what: &str) {
        let is_branch = matches!(
            expr.kind,
            ExprKind::If { .. } | ExprKind::When { .. } | ExprKind::Block(_)
        ) || matches!(
            expr.kind,
            ExprKind::Binary {
                op: prism_ast::BinaryOp::Elvis,
                ..
            }
        );
        if is_branch {
            self.errors.push(TypeError::CannotTakeApart {
                source: "the result of a branch, block or '?:'".to_string(),
                help: format!("bind the value to a 'val' first, then {} the binding", what),
                span: expr.span,
            });
            return;
        }
        if let Some(field) = self.implicit_this_field(expr) {
            return self.check_take_apart_source(&field, what);
        }
        if let Some((source, borrowed)) = self.move_source(expr) {
            let help = if let Some(root) = Self::safe_navigation_root(expr) {
                // `&u?.name` would move out of the field: the borrow to match is the nullable's.
                format!(
                    "{} the borrow of what '?.' reads instead, '&{}', with a 'null' branch and a pattern of its fields",
                    what, root
                )
            } else if borrowed {
                format!("{} a borrow of it instead", what)
            } else {
                format!("{} its borrow instead: '&{}'", what, expr)
            };
            self.errors.push(TypeError::CannotTakeApart {
                source,
                help,
                span: expr.span,
            });
        }
    }

    /// The receiver of the first safe navigation of the member chain `expr` (`u` in
    /// `u?.inner.name`), if it has one.
    fn safe_navigation_root(expr: &Expr) -> Option<&Expr> {
        match &expr.kind {
            ExprKind::MemberAccess {
                target,
                is_safe_nav,
                ..
            } => Self::safe_navigation_root(target).or(is_safe_nav.then_some(target.as_ref())),
            _ => None,
        }
    }

    /// Whether the binding `name` refers to is one of the bindings of the guard being checked,
    /// and that binding's mode.
    pub(super) fn guard_binding(&self, name: &str) -> Option<BindingMode> {
        let local = self.env.lookup(name).and_then(Self::local_of)?;
        self.guard_bindings
            .iter()
            .find(|(binding, _)| *binding == local)
            .map(|(_, mode)| *mode)
    }

    /// Reports `E4003` when the place `expr` is reached from a binding of the guard being
    /// checked, which the guard must not mutate (D9). Returns whether it was reported.
    pub(super) fn reject_guard_mutation(&mut self, expr: &Expr, span: Span) -> bool {
        if self.guard_bindings.is_empty() {
            return false;
        }
        let mut root = expr;
        loop {
            match &root.kind {
                ExprKind::MemberAccess { target, .. } | ExprKind::Index { target, .. } => {
                    root = target
                }
                ExprKind::Unary { expr: inner, .. } => root = inner,
                _ => break,
            }
        }
        let Some(name) = binding_name(root) else {
            return false;
        };
        if self.guard_binding(name).is_none() {
            return false;
        }
        self.errors.push(TypeError::ImmutabilityViolation {
            message: format!(
                "cannot mutate binding '{}' in a guard: the guard may be false, and the next branches must find the subject unchanged",
                name
            ),
            span,
            help: Some("mutate it in the branch body instead".to_string()),
        });
        true
    }
}

#[cfg(test)]
mod tests {
    use crate::TypeChecker;

    /// The error codes of checking `src`, and its typed tree's dump. The tree of a file without
    /// errors must be well formed.
    fn check(src: &str) -> (Vec<&'static str>, String) {
        let file = prism_parser::parse_source(src).expect("source must parse");
        let mut checker = TypeChecker::new(None, "main");
        let tree = checker.check_module(&file);
        let codes: Vec<&'static str> = checker.errors.iter().map(|e| e.code()).collect();
        if codes.is_empty() {
            assert_eq!(tree.validate(true), Vec::<String>::new(), "{}", src);
        }
        (codes, tree.dump())
    }

    fn assert_codes(src: &str, expected: &[&str]) -> String {
        let (codes, dump) = check(src);
        assert_eq!(codes, expected, "{}\n{}", src, dump);
        dump
    }

    const SHAPE: &str = "enum Shape { Circle(Double), Rect(Double, Double) }\n";
    const MSG: &str = "enum Msg { Text(String), Count(Int) }\n";

    #[test]
    fn variant_patterns_mirror_construction() {
        let dump = assert_codes(
            &format!(
                "{}fun area(s: Shape): Double = when (s) {{ Shape.Circle(val r) -> r * r * 3.0; Shape.Rect(val w, val h) -> w * h }}\n",
                SHAPE
            ),
            &[],
        );
        assert!(
            dump.contains("pattern binding %1 r (copy): Float"),
            "{}",
            dump
        );
        assert_codes(
            &format!(
                "{}fun f(s: Shape): Double = when (s) {{ Circle(val r) -> r; Rect(_, val h) -> h }}\n",
                SHAPE
            ),
            &[],
        );
        assert_codes(
            &format!(
                "{}fun f(s: Shape): Double = when (s) {{ Shape.Rect(val w) -> w; else -> 0.0 }}\n",
                SHAPE
            ),
            &["E4024"],
        );
        assert_codes(
            "enum Color { Red, Green }\nfun f(c: Color): Int = when (c) { Color.Red(_) -> 1; else -> 2 }\n",
            &["E4024"],
        );
        assert_codes(
            &format!(
                "{}fun f(s: Shape): Double = when (s) {{ Shape.Circle(r = val r) -> r; else -> 0.0 }}\n",
                SHAPE
            ),
            &["E4024"],
        );
    }

    #[test]
    fn struct_patterns_mirror_construction() {
        let point = "struct Point : Copy { val x: Int, val y: Int }\n";
        assert_codes(
            &format!(
                "{}fun f(p: Point): Int = when (p) {{ Point(0, val y) -> y; Point(y = 0) -> 1; Point(val x, _) -> x }}\n",
                point
            ),
            &[],
        );
        for (pattern, code) in [
            ("Point(val x)", "E4024"),
            ("Point(x = 1, x = 2)", "E4024"),
            ("Point(z = 1)", "E4006"),
            ("Point(x = 1, 2)", "E4024"),
        ] {
            assert_codes(
                &format!(
                    "{}fun f(p: Point): Int = when (p) {{ {} -> 1; else -> 2 }}\n",
                    point, pattern
                ),
                &[code],
            );
        }
        assert_codes(
            &format!(
                "{}enum Event {{ Click(Point), Key(Char) }}\nfun f(e: Event): Int = when (e) {{ Event.Click(Point(val x, _)) -> x; Event.Key(_) -> 0 }}\n",
                point
            ),
            &[],
        );
    }

    #[test]
    fn sub_patterns_and_values() {
        assert_codes(
            "enum Shape { Circle(Double), Named(String) }\nfun f(s: Shape, limit: Double): Int = when (s) { Shape.Circle(0.0) -> 0; Shape.Circle(limit) -> 1; Shape.Circle(in 0.0..1.0) -> 2; Shape.Named(\"x\") -> 3; else -> 4 }\n",
            &[],
        );
        assert_codes(
            "fun f(x: Int): Int = when (x) { 0 -> 1; _ -> 2 }\nfun g(x: Int): Int = when (x) { 0 -> 1; val n -> n * 2 }\n",
            &[],
        );
        assert_codes(
            &format!(
                "{}fun isRound(s: &Shape): Bool = when (s) {{ Shape.Circle -> true; Shape.Rect -> false }}\n",
                SHAPE
            ),
            &[],
        );
        assert_codes(
            "fun g(n: Int): Int = n\nfun f(x: Int): Int = when (x) { g(1) + 1 -> 1; g(3) -> 2; else -> 3 }\n",
            &[],
        );
        assert_codes(
            "fun g(n: Int): Int = n\nfun f(x: Int): Int = when (x) { g(val n) -> n; else -> 0 }\n",
            &["E4024"],
        );
    }

    #[test]
    fn bindings_need_val_and_are_scoped_to_their_branch() {
        let boxed = "enum Box { Of(Int) }\n";
        assert_codes(
            &format!(
                "{}fun f(b: Box, n: Int): Int = when (b) {{ Box.Of(n) -> 1; else -> 2 }}\n",
                boxed
            ),
            &[],
        );
        assert_codes(
            &format!(
                "{}fun f(b: Box): Int = when (b) {{ Box.Of(n) -> n }}\n",
                boxed
            ),
            &["E4001", "E4001"],
        );
        assert_codes(
            &format!(
                "{}fun f(b: Box): Int = when (b) {{ Box.Of(var n) -> {{ n += 1; n }} }}\n",
                boxed
            ),
            &[],
        );
        assert_codes(
            "enum P { Two(Int, Int) }\nfun f(p: P): Int = when (p) { P.Two(val a, val a) -> a }\n",
            &["E4010"],
        );
        assert_codes(
            &format!(
                "{}fun f(b: Box, n: Int): Int = when (b) {{ Box.Of(val n) -> n }}\n",
                boxed
            ),
            &[],
        );
    }

    #[test]
    fn binding_modes_follow_the_subject() {
        let dump = assert_codes(
            &format!(
                "{}fun f(m: Msg): String = when (m) {{ Msg.Text(val s) -> s; Msg.Count(_) -> \"n\" }}\n",
                MSG
            ),
            &[],
        );
        assert!(dump.contains("(move): String"), "{}", dump);
        let dump = assert_codes(
            &format!(
                "{}fun len(s: &String): Int = 1\nfun f(m: &Msg): Int = when (m) {{ Msg.Text(val s) -> len(s); Msg.Count(val n) -> n }}\n",
                MSG
            ),
            &[],
        );
        assert!(dump.contains("(shared borrow): &String"), "{}", dump);
        assert!(dump.contains("n (copy): Int"), "{}", dump);
        let dump = assert_codes(
            &format!(
                "{}fun bump(m: &mut Msg) {{ when (m) {{ Msg.Count(val n) -> *n += 1; Msg.Text(val s) -> *s = \"x\" }} }}\n",
                MSG
            ),
            &[],
        );
        assert!(dump.contains("n (mutable borrow): &mut Int"), "{}", dump);
        let dump = assert_codes(
            &format!(
                "{}fun Msg.peek(): Int = when (this) {{ Msg.Count(val n) -> n; Msg.Text(val s) -> 0 }}\nconsuming fun Msg.text(): String = when (this) {{ Msg.Text(val s) -> s; Msg.Count(_) -> \"\" }}\n",
                MSG
            ),
            &[],
        );
        assert!(dump.contains("s (shared borrow): &String"), "{}", dump);
        assert!(dump.contains("s (move): String"), "{}", dump);
        assert_codes(
            &format!(
                "{}fun len(s: &String): Int = 1\nfun f(m: Msg): Int {{ val n = when (&m) {{ Msg.Text(val s) -> len(s); Msg.Count(_) -> 0 }}; return n }}\n",
                MSG
            ),
            &[],
        );
    }

    #[test]
    fn guards() {
        let shape = "enum Shape { Circle(Double), Square(Double) }\n";
        assert_codes(
            &format!(
                "{}fun f(s: Shape): Int = when (s) {{ Shape.Circle(val r) if r > 10.0 -> 1; Shape.Circle(_) -> 2; Shape.Square(_) -> 3 }}\n",
                shape
            ),
            &[],
        );
        assert_codes(
            &format!(
                "{}fun f(s: Shape): Int = when (s) {{ Shape.Circle(val r) if r -> 1; else -> 2 }}\n",
                shape
            ),
            &["E4002"],
        );
        let text = "enum Msg { Text(String) }\n";
        assert_codes(
            &format!(
                "{}fun take(s: String): Bool = true\nfun f(m: Msg): Int = when (m) {{ Msg.Text(val s) if take(s) -> 1; else -> 2 }}\n",
                text
            ),
            &["E4017"],
        );
        assert_codes(
            &format!(
                "{}fun short(s: &String): Bool = true\nfun f(m: Msg): String = when (m) {{ Msg.Text(val s) if short(s) -> s; Msg.Text(_) -> \"long\" }}\n",
                text
            ),
            &[],
        );
        assert_codes(
            "enum Box { Of(Int) }\nfun set(n: &mut Int): Bool = true\nfun f(b: &mut Box): Int = when (b) { Box.Of(val n) if set(n) -> 1; else -> 2 }\n",
            &["E4003"],
        );
    }

    #[test]
    fn alternatives_bind_the_same_names() {
        let shape = "enum Shape { Circle(Double), Square(Double), Empty }\n";
        assert_codes(
            &format!(
                "{}fun size(s: Shape): Double = when (s) {{ Shape.Circle(val d), Shape.Square(val d) -> d; Shape.Empty -> 0.0 }}\n",
                shape
            ),
            &[],
        );
        assert_codes(
            &format!(
                "{}fun f(s: Shape): Double = when (s) {{ Shape.Circle(val d), Shape.Square(val e) -> 1.0; else -> 0.0 }}\n",
                shape
            ),
            &["E4025"],
        );
        assert_codes(
            "enum V { I(Int), D(Double) }\nfun f(v: V): Int = when (v) { V.I(val x), V.D(val x) -> 1 }\n",
            &["E4025"],
        );
    }

    #[test]
    fn nullable_subjects_match_patterns_of_their_payload() {
        assert_codes(
            "enum Shape { Circle(Double), Square(Double) }\nfun f(s: Shape?): Double = when (s) { null -> 0.0; Shape.Circle(val r) -> r; Shape.Square(val d) -> d; else -> 1.0 }\n",
            &[],
        );
        let dump = assert_codes(
            "enum Slot { Of(Int?) }\nfun f(s: Slot): Int = when (s) { Slot.Of(null) -> 0; Slot.Of(1) -> 1; Slot.Of(val n) -> 2 }\n",
            &[],
        );
        assert!(dump.contains("n (copy): Int?"), "{}", dump);
    }

    #[test]
    fn taking_a_value_apart_moves_it_whole() {
        let declarations = "enum Msg { Text(String), Empty }\nstruct Inbox { val last: Msg }\n";
        assert_codes(
            &format!(
                "{}fun f(i: Inbox): String = when (i.last) {{ Msg.Text(val s) -> s; Msg.Empty -> \"\" }}\n",
                declarations
            ),
            &["E4017"],
        );
        assert_codes(
            &format!(
                "{}val WELCOME = Msg.Text(\"hi\")\nfun f(): String = when (WELCOME) {{ Msg.Text(val s) -> s; Msg.Empty -> \"\" }}\n",
                declarations
            ),
            &["E4017"],
        );
        assert_codes(
            &format!(
                "{}fun f(c: Bool, a: Msg, b: Msg): String = when (if (c) a else b) {{ Msg.Text(val s) -> s; Msg.Empty -> \"\" }}\n",
                declarations
            ),
            &["E4017"],
        );
        assert_codes(
            &format!(
                "{}fun make(): Msg = Msg.Empty\nfun f(m: Msg): String = when (m) {{ Msg.Text(val s) -> s; Msg.Empty -> \"\" }}\nfun g(): String = when (make()) {{ Msg.Text(val s) -> s; Msg.Empty -> \"\" }}\n",
                declarations
            ),
            &[],
        );
        assert_codes(
            "enum Pair { Of(String, Int) }\nstruct Inbox { val last: Pair }\nfun f(i: Inbox): Int = when (i.last) { Pair.Of(_, val n) -> n }\n",
            &[],
        );
    }

    #[test]
    fn taking_apart_through_a_safe_navigation_suggests_the_nullables_borrow() {
        let src = "struct User { val name: String, val id: Int }\nstruct Box { val u: User? }\nfun f(b: Box?): Int = when (b?.u) { null -> 0; User(name = val s) -> 1 }\n";
        let file = prism_parser::parse_source(src).expect("source must parse");
        let mut checker = TypeChecker::new(None, "main");
        checker.check_module(&file);
        let codes: Vec<&'static str> = checker.errors.iter().map(|e| e.code()).collect();
        assert_eq!(codes, ["E4017"]);
        let help = format!("{:?}", checker.errors[0]);
        assert!(help.contains("'&b'"), "{}", help);
        assert!(!help.contains("&b?.u"), "{}", help);
    }

    const USER: &str = "struct User { val name: String, val email: String, val age: Int }\n";
    const ACCOUNT: &str = "struct Account { val name: String, val id: Int }\n";

    #[test]
    fn destructuring_declarations_name_fields() {
        let dump = assert_codes(
            &format!(
                "{}fun f(u: User): String {{ val (email, name) = u; return name }}\n",
                USER
            ),
            &[],
        );
        assert!(dump.contains("destructure"), "{}", dump);
        assert_codes(
            &format!(
                "{}fun f(a: User, b: User): Int {{ val (age) = a; val (age as other) = b; return age + other }}\n",
                USER
            ),
            &[],
        );
        for (entries, code) in [
            ("nick", "E4006"),
            ("age, age as a", "E4024"),
            ("name, email as name", "E4010"),
        ] {
            assert_codes(
                &format!("{}fun f(u: User) {{ val ({}) = u }}\n", USER, entries),
                &[code],
            );
        }
        assert_codes(
            "struct Pair(val first: Int, val second: Int)\nfun f(p: Pair): Int { val (a, b) = p; return a }\n",
            &["E4006", "E4006"],
        );
        assert_codes(
            "enum Box { Of(Int) }\nfun f(b: Box) { val (x) = b }\n",
            &["E4024"],
        );
    }

    #[test]
    fn destructuring_follows_the_binding_modes() {
        let users = "struct User { val name: String }\n";
        let dump = assert_codes(
            &format!(
                "{}{}consuming fun Account.toUser(): User {{ val (name) = this; return User(name) }}\n",
                ACCOUNT, users
            ),
            &[],
        );
        assert!(dump.contains("name (move): String"), "{}", dump);
        let dump = assert_codes(
            &format!(
                "{}fun len(s: &String): Int = 1\nfun f(a: &Account): Int {{ val (name, id) = a; return len(name) + id }}\n",
                ACCOUNT
            ),
            &[],
        );
        assert!(dump.contains("name (shared borrow): &String"), "{}", dump);
        assert_codes(
            &format!(
                "{}fun f(a: Account): Int {{ val (id) = a; val (name) = a; return id }}\n",
                ACCOUNT
            ),
            &[],
        );
        assert_codes(
            &format!(
                "{}struct Holder {{ val account: Account }}\nfun f(h: Holder): String {{ val (name) = h.account; return name }}\n",
                ACCOUNT
            ),
            &["E4017"],
        );
    }
}

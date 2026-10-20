//! Operator functions in the type checker (D3, D5, D6, D9 and D11 of `operator-conventions`).
//!
//! An `operator` function is an extension whose name is one of Kotlin's conventions; operator
//! syntax calls it where the built-in operator does not apply. This module holds the table of
//! conventions, the checks of an `operator` declaration against its convention's shape
//! (`E4035`), the rejection of operator functions a built-in operator always shadows (`E4010`),
//! and the names under which the `mut` accessors (`get`, `next`) are kept beside the shared
//! ones. Each check is a fixed amount of work per declaration.

use super::{ExtensionLookup, FoundExtension, MethodCall, Origin, ReceiverKey, TypeChecker};
use crate::error::TypeError;
use crate::scope::SymbolKind;
use crate::types::Type;
use prism_ast::decl::{FunctionBody, FunctionDecl, ReceiverMode};
use prism_ast::expr::{BinaryOp, CallArg, Expr, ExprKind, Literal, UnaryOp};
use prism_ast::{Span, Visibility};
use prism_typed_tree::{
    Argument, BorrowOrigin, BuiltinFunction, Call, Callee, ExpressionId, ExpressionKind,
    ParameterKind, SpelledOperator, UnaryOperator,
};

/// An operator convention: what a function of its name serves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Convention {
    /// `plus`, `minus`, `times`, `div`, `rem`: a binary arithmetic operator.
    Arithmetic,
    /// `unaryMinus`, `not`: a prefix operator.
    Unary,
    /// `compareTo`: the four ordering operators.
    CompareTo,
    /// `equals`: `==` and `!=`, part of its type (D6).
    Equals,
    /// `contains`: `in` and `!in`.
    Contains,
    /// `get`: indexing, shared or `mut` (D9).
    Get,
    /// `set`: a whole-element indexed write (D10).
    Set,
    /// `invoke`: a call of a value (D12).
    Invoke,
    /// `iterator`, `hasNext`, `next`: the cursor protocol of `for` (D11).
    Iterator,
    HasNext,
    Next,
}

/// The convention of a function named `name`, if it names one.
pub(crate) fn convention(name: &str) -> Option<Convention> {
    Some(match name {
        "plus" | "minus" | "times" | "div" | "rem" => Convention::Arithmetic,
        "unaryMinus" | "not" => Convention::Unary,
        "compareTo" => Convention::CompareTo,
        "equals" => Convention::Equals,
        "contains" => Convention::Contains,
        "get" => Convention::Get,
        "set" => Convention::Set,
        "invoke" => Convention::Invoke,
        "iterator" => Convention::Iterator,
        "hasNext" => Convention::HasNext,
        "next" => Convention::Next,
        _ => return None,
    })
}

/// The operator a convention serves, as a diagnostic names it.
fn served(name: &str) -> &'static str {
    match name {
        "plus" => "'+'",
        "minus" => "'-'",
        "times" => "'*'",
        "div" => "'/'",
        "rem" => "'%'",
        "unaryMinus" => "prefix '-'",
        "not" => "prefix '!'",
        "compareTo" => "'<', '<=', '>' and '>='",
        "equals" => "'==' and '!='",
        "contains" => "'in' and '!in'",
        "get" => "indexing 'a[i]'",
        "set" => "indexed assignment 'a[i] = v'",
        "invoke" => "calls 'a(...)'",
        "iterator" | "hasNext" | "next" => "'for' loops",
        _ => "no operator",
    }
}

/// The names of every convention, for the help of an unknown one.
const CONVENTION_NAMES: &str = "plus, minus, times, div, rem, unaryMinus, not, compareTo, equals, contains, get, set, invoke, iterator, hasNext, next, and the hooks drop and clone";

/// A drop or clone hook (D1 of `drop-and-clone-hooks`): an `operator` extension that the
/// language calls when a value of its type is dropped or cloned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Hook {
    /// `operator mut fun T.drop()`.
    Drop,
    /// `operator fun T.clone(): T`.
    Clone,
}

impl Hook {
    /// The hook a function named `name` would be.
    pub(crate) fn named(name: &str) -> Option<Hook> {
        match name {
            "drop" => Some(Hook::Drop),
            "clone" => Some(Hook::Clone),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Hook::Drop => "drop",
            Hook::Clone => "clone",
        }
    }

    /// The hook's declaration for a receiver written `receiver`, as diagnostics show it.
    fn declaration(self, receiver: &str) -> String {
        match self {
            Hook::Drop => format!("operator mut fun {}.drop()", receiver),
            Hook::Clone => format!("operator fun {r}.clone(): {r}", r = receiver),
        }
    }

    /// What a hook of this kind is, for a receiver written `receiver`.
    fn shape(self, receiver: &str) -> String {
        match self {
            Hook::Drop => format!(
                "a drop hook is '{}' declared without a visibility modifier in the file of a struct or enum: a mutable borrow of its value, with no parameter, returning 'Unit'",
                self.declaration(receiver)
            ),
            Hook::Clone => format!(
                "a clone hook is '{}' declared without a visibility modifier in the file of a 'Clone' struct or enum: a shared borrow of its value, with no parameter, returning the clone",
                self.declaration(receiver)
            ),
        }
    }
}

/// The suffix of the extension-table name and declaration path name of a `mut` accessor
/// (`operator mut fun Bag.get`, `operator mut fun Bag.next`), which a type may declare beside a
/// shared one of the same receiver, name and context (D9 and D11).
pub(crate) const MUTABLE_ACCESSOR_SUFFIX: &str = "#mut";

/// Whether an extension named `name`, with receiver `mode` and operator flag `is_operator`, is a
/// `mut` accessor, kept under its own name.
pub(crate) fn is_mutable_accessor(name: &str, mode: ReceiverMode, is_operator: bool) -> bool {
    is_operator && mode == ReceiverMode::Mutable && (name == "get" || name == "next")
}

/// The name an extension is kept and named by: its own, or with
/// [`MUTABLE_ACCESSOR_SUFFIX`] for a `mut` accessor.
pub(crate) fn extension_name(name: &str, mode: ReceiverMode, is_operator: bool) -> String {
    if is_mutable_accessor(name, mode, is_operator) {
        format!("{}{}", name, MUTABLE_ACCESSOR_SUFFIX)
    } else {
        name.to_string()
    }
}

/// The table name of the `mut` accessor named `name` (`get` or `next`).
pub(crate) fn mutable_accessor_name(name: &str) -> String {
    format!("{}{}", name, MUTABLE_ACCESSOR_SUFFIX)
}

impl TypeChecker {
    /// Reports `E4035` for the `operator` function `f` when it is not an extension named by a
    /// convention with its convention's shape (D5): receiver modes, parameters and, when the
    /// result is written, the result (an inferred one is checked once inferred, by
    /// [`Self::check_operator_result`]). `receiver` is the lowered receiver type of an
    /// extension, `None` for a plain function. Gives whether the declaration is valid.
    pub(super) fn check_operator_declaration(
        &mut self,
        f: &FunctionDecl,
        receiver: Option<&Type>,
    ) -> bool {
        let report = |checker: &mut Self, message: String, help: Option<String>| {
            checker.errors.push(TypeError::InvalidOperatorDeclaration {
                message,
                help,
                span: f.span,
            });
            false
        };
        // Drop and clone hooks are conventions of their type, with their own rules (D3 of
        // `drop-and-clone-hooks`).
        if let Some(hook) = Hook::named(&f.name) {
            return self.check_hook_declaration(f, receiver, hook);
        }
        let Some(receiver_ty) = receiver else {
            return report(
                self,
                format!(
                    "an operator function needs a receiver: '{}' must be declared as an extension of the operand operator syntax calls it on",
                    f.name
                ),
                Some(format!(
                    "declare it on the type of the left operand, as in 'operator fun Vector2.plus(other: Vector2): Vector2'; operator conventions are {}",
                    CONVENTION_NAMES
                )),
            );
        };
        let Some(convention) = convention(&f.name) else {
            return report(
                self,
                format!("'{}' is no operator convention", f.name),
                Some(format!(
                    "the operator conventions are {}; remove 'operator' to declare an ordinary extension",
                    CONVENTION_NAMES
                )),
            );
        };
        let mode = f
            .receiver
            .as_ref()
            .map_or(ReceiverMode::Shared, |receiver| receiver.mode);
        if f.is_compose {
            return report(
                self,
                format!(
                    "'{}' cannot be a 'compose fun': operators are not composable",
                    f.name
                ),
                None,
            );
        }
        if f.context.is_some() {
            return report(
                self,
                format!(
                    "'{}' cannot have a context parameter: operator syntax gives no context",
                    f.name
                ),
                None,
            );
        }
        if convention != Convention::Invoke
            && f.params
                .iter()
                .any(|parameter| parameter.default_value.is_some())
        {
            return report(
                self,
                format!(
                    "'{}' cannot have default values: only 'invoke' may have default values",
                    f.name
                ),
                None,
            );
        }
        let parameters: Vec<Type> = {
            let type_parameters = super::type_parameters_of(&f.type_parameters);
            self.with_type_parameters(type_parameters, |c| {
                f.params.iter().map(|p| c.lower_type(&p.ty)).collect()
            })
        };
        let count = parameters.len();
        let shape = |rule: &str| format!("'{}' serves {}, so it {}", f.name, served(&f.name), rule);
        // Receiver modes.
        let mode_ok = match convention {
            Convention::Arithmetic | Convention::Unary => mode != ReceiverMode::Mutable,
            Convention::CompareTo
            | Convention::Equals
            | Convention::Contains
            | Convention::Iterator
            | Convention::HasNext => mode == ReceiverMode::Shared,
            Convention::Get | Convention::Next => mode != ReceiverMode::Consuming,
            Convention::Set => mode == ReceiverMode::Mutable,
            Convention::Invoke => true,
        };
        if !mode_ok {
            let required = match convention {
                Convention::Arithmetic | Convention::Unary => {
                    "has a shared or consuming receiver ('fun' or 'consuming fun')"
                }
                Convention::Get | Convention::Next => {
                    "has a shared or 'mut' receiver ('fun' or 'mut fun')"
                }
                Convention::Set => "has a 'mut' receiver ('mut fun')",
                _ => "has a shared receiver ('fun')",
            };
            return report(self, shape(required), None);
        }
        // Parameters.
        let parameters_ok = match convention {
            Convention::Arithmetic | Convention::CompareTo | Convention::Contains => count == 1,
            Convention::Unary | Convention::Iterator => count == 0,
            Convention::Get => count >= 1,
            Convention::Set => count >= 2,
            Convention::Invoke => true,
            Convention::Equals => {
                count == 1
                    && parameters[0]
                        == Type::Reference {
                            mutable: false,
                            inner: Box::new(receiver_ty.clone()),
                        }
            }
            Convention::HasNext => {
                count == 1 && matches!(parameters[0], Type::Reference { mutable: false, .. })
            }
            Convention::Next => {
                count == 1 && matches!(parameters[0], Type::Reference { mutable: true, .. })
            }
        };
        if !parameters_ok {
            let required = match convention {
                Convention::Arithmetic | Convention::CompareTo | Convention::Contains => {
                    "takes one parameter".to_string()
                }
                Convention::Unary => "takes no parameter".to_string(),
                Convention::Iterator => {
                    "takes no parameter: it returns the initial cursor".to_string()
                }
                Convention::Get => "takes one or more parameters, the indices".to_string(),
                Convention::Set => {
                    "takes the indices and then the value, two or more parameters".to_string()
                }
                Convention::Equals => format!(
                    "takes one parameter of type '&{}', as in 'operator fun {}.equals(other: &{}): Bool'",
                    receiver_ty, receiver_ty, receiver_ty
                ),
                Convention::HasNext => {
                    "takes the cursor as a shared borrow, as in 'hasNext(cursor: &Int)'"
                        .to_string()
                }
                Convention::Next => {
                    "takes the cursor as a mutable borrow, as in 'next(cursor: &mut Int)'"
                        .to_string()
                }
                Convention::Invoke => unreachable!("invoke takes any parameters"),
            };
            return report(self, shape(&required), None);
        }
        if convention == Convention::Equals && !self.check_equality_operator(f, receiver_ty) {
            return false;
        }
        match &f.return_type {
            Some(annotation) => {
                let type_parameters = super::type_parameters_of(&f.type_parameters);
                let result =
                    self.with_type_parameters(type_parameters, |c| c.lower_type(annotation));
                self.check_operator_result(f, &result)
            }
            None if matches!(f.body, Some(FunctionBody::Block(_)) | None) => {
                self.check_operator_result(f, &Type::Unit)
            }
            // An inferred result is checked once inferred.
            None => true,
        }
    }

    /// Reports `E4035` when the result `result` of the `operator` function `f` breaks its
    /// convention: `compareTo` returns `Int`, `equals`, `contains` and `hasNext` return `Bool`,
    /// `set` returns `Unit`, `iterator` a cursor holding no reference, and a `mut` `get` or
    /// `next` a mutable borrow. Gives whether it is valid.
    pub(super) fn check_operator_result(&mut self, f: &FunctionDecl, result: &Type) -> bool {
        if let Some(hook) = Hook::named(&f.name) {
            return self.check_hook_result(f, hook, result);
        }
        let Some(convention) = convention(&f.name) else {
            return true;
        };
        if f.receiver.is_none() || result == &Type::Error {
            return true;
        }
        let mode = f
            .receiver
            .as_ref()
            .map_or(ReceiverMode::Shared, |receiver| receiver.mode);
        let required = match convention {
            Convention::CompareTo if result != &Type::Int => {
                Some("returns 'Int', compared with 0".to_string())
            }
            Convention::Equals | Convention::Contains | Convention::HasNext
                if result != &Type::Bool =>
            {
                Some("returns 'Bool'".to_string())
            }
            Convention::Set if result != &Type::Unit => Some("returns 'Unit'".to_string()),
            Convention::Iterator => result.reference_outside_functions().map(|_| {
                format!(
                    "returns a cursor holding no reference, such as an index, not '{}': the collection is borrowed for the loop and passed to 'hasNext' and 'next'",
                    result
                )
            }),
            Convention::Get | Convention::Next
                if mode == ReceiverMode::Mutable
                    && !matches!(result, Type::Reference { mutable: true, .. }) =>
            {
                Some(format!(
                    "returns, with a 'mut' receiver, a mutable borrow '&mut E' of the element, not '{}'",
                    result
                ))
            }
            _ => None,
        };
        match required {
            Some(required) => {
                self.errors.push(TypeError::InvalidOperatorDeclaration {
                    message: format!(
                        "'{}' serves {}, so it {}",
                        f.name,
                        served(&f.name),
                        required
                    ),
                    help: None,
                    span: f.span,
                });
                false
            }
            None => true,
        }
    }

    /// The rules of an `equals` operator beyond its shape (D6): declared in the file of its
    /// type, a non-generic struct or enum, with that type's visibility. Reports `E4035` and
    /// gives whether they hold.
    fn check_equality_operator(&mut self, f: &FunctionDecl, receiver_ty: &Type) -> bool {
        let own = |id: &prism_typed_tree::NominalId| id.module == self.module_path;
        let (id, generic) = match receiver_ty {
            Type::Struct { id, arguments } | Type::Enum { id, arguments } if own(id) => (
                id.clone(),
                !arguments.is_empty() || !self.types.parameters(id).is_empty(),
            ),
            _ => {
                self.errors.push(TypeError::InvalidOperatorDeclaration {
                    message: format!(
                        "an equality operator of '{}' cannot be declared here: a type's 'equals' is declared in the type's own file",
                        receiver_ty
                    ),
                    help: Some(
                        "'==' calls a type's 'equals' wherever its values are compared, so it belongs to the type's declaration; a built-in type keeps its equality".to_string(),
                    ),
                    span: f.span,
                });
                return false;
            }
        };
        if generic {
            self.errors.push(TypeError::InvalidOperatorDeclaration {
                message: format!(
                    "'{}' is generic, and an equality operator is declared only for a non-generic struct or enum",
                    id.name
                ),
                help: None,
                span: f.span,
            });
            return false;
        }
        let visibility = match self.env.lookup(&id.name).map(|symbol| &symbol.kind) {
            Some(SymbolKind::Struct { visibility, .. } | SymbolKind::Enum { visibility, .. }) => {
                *visibility
            }
            _ => f.visibility,
        };
        if visibility != f.visibility {
            let written = |v: Visibility| match v {
                Visibility::Private => "private (no modifier)".to_string(),
                other => format!("'{}'", other.as_str()),
            };
            self.errors.push(TypeError::InvalidOperatorDeclaration {
                message: format!(
                    "the equality operator of '{}' is {}, but '{}' is {}: a type's 'equals' has the type's visibility",
                    id.name,
                    written(f.visibility),
                    id.name,
                    written(visibility)
                ),
                help: None,
                span: f.span,
            });
            return false;
        }
        true
    }

    /// Reports `E4035` when the drop or clone hook `f` breaks the rules of D3, D4 and D6 of
    /// `drop-and-clone-hooks`: an extension of a struct or enum of this file that is not
    /// intrinsic, without a visibility modifier, `compose`, context or default values, with a
    /// `mut` receiver for `drop` and a shared one for `clone`, no parameter, the type's own type
    /// parameters in order (unbounded, or bounded by `Clone` for a marker parameter of a clone
    /// hook) and the result `Unit` or the type; no drop hook on a `Copy` type, a clone hook beside
    /// the drop hook of a `Clone` type, and clone hooks only on `Clone` types that are not `Copy`.
    /// A fixed amount of work. Gives whether the declaration is valid.
    fn check_hook_declaration(
        &mut self,
        f: &FunctionDecl,
        receiver: Option<&Type>,
        hook: Hook,
    ) -> bool {
        let report = |checker: &mut Self, message: String, help: Option<String>| {
            checker.errors.push(TypeError::InvalidOperatorDeclaration {
                message,
                help,
                span: f.span,
            });
            false
        };
        // The receiver as written in the type's own file (`Pair<A, B>`), or `T`.
        let written = |checker: &Self, ty: Option<&Type>| -> String {
            match ty {
                Some(Type::Struct { id, .. } | Type::Enum { id, .. }) => {
                    let parameters = checker.types.parameters(id);
                    if parameters.is_empty() {
                        id.name.clone()
                    } else {
                        let names: Vec<&str> = parameters.iter().map(|p| p.name.as_str()).collect();
                        format!("{}<{}>", id.name, names.join(", "))
                    }
                }
                _ => "T".to_string(),
            }
        };
        let Some(receiver_ty) = receiver else {
            return report(self, hook.shape("T"), None);
        };
        let (id, arguments) = match receiver_ty {
            Type::Struct { id, arguments } | Type::Enum { id, arguments }
                if id.intrinsic().is_none()
                    && *id != prism_typed_tree::NominalId::builtin_range() =>
            {
                (id.clone(), arguments.clone())
            }
            _ => return report(self, hook.shape("T"), None),
        };
        let receiver_name = written(self, Some(receiver_ty));
        if id.module != self.module_path {
            return report(
                self,
                format!(
                    "a {} hook of '{}' cannot be declared here: a {} hook belongs to the file declaring '{}'",
                    hook.name(),
                    id.name,
                    hook.name(),
                    id.name
                ),
                Some(format!(
                    "a type's hooks run wherever its values are {}, so they are declared beside the type, as '{}'",
                    match hook {
                        Hook::Drop => "dropped",
                        Hook::Clone => "cloned",
                    },
                    hook.declaration(&receiver_name)
                )),
            );
        }
        let mode = f
            .receiver
            .as_ref()
            .map_or(ReceiverMode::Shared, |receiver| receiver.mode);
        let mode_ok = match hook {
            Hook::Drop => mode == ReceiverMode::Mutable,
            Hook::Clone => mode == ReceiverMode::Shared,
        };
        let defaults = f.params.iter().any(|p| p.default_value.is_some());
        if f.visibility != Visibility::Private
            || !mode_ok
            || !f.params.is_empty()
            || f.is_compose
            || f.context.is_some()
            || defaults
        {
            return report(self, hook.shape(&receiver_name), None);
        }
        // The type's own type parameters, in order: a hook serves every instantiation.
        let parameter_count = self.types.parameters(&id).len();
        let marker_parameters = self.types.marker_parameters(&id).to_vec();
        let own_parameters = self.types.parameters(&id);
        let in_order = f.type_parameters.len() == parameter_count
            && arguments.len() == parameter_count
            && arguments.iter().enumerate().all(|(index, argument)| {
                matches!(argument, Type::Parameter(parameter) if parameter.index as usize == index)
            })
            && f
                .type_parameters
                .iter()
                .zip(own_parameters)
                .all(|(written, own)| written.name == own.name);
        let bounds_ok = f
            .type_parameters
            .iter()
            .enumerate()
            .all(|(index, parameter)| match (hook, parameter.bound) {
                (_, None) => true,
                (Hook::Clone, Some(prism_ast::TypeMarker::Clone)) => {
                    marker_parameters.contains(&(index as u32))
                }
                _ => false,
            });
        if !in_order || !bounds_ok {
            let rule = match hook {
                Hook::Drop => "with its type parameters in order and unbounded",
                Hook::Clone => {
                    "with its type parameters in order, each unbounded or, for a parameter the marker depends on, bounded by 'Clone'"
                }
            };
            return report(
                self,
                format!(
                    "a {} hook of '{}' is declared for every '{}', {}",
                    hook.name(),
                    id.name,
                    receiver_name,
                    rule
                ),
                Some(format!("declare it as '{}'", {
                    let parameters: Vec<String> = self
                        .types
                        .parameters(&id)
                        .iter()
                        .map(|p| p.name.clone())
                        .collect();
                    let list = if parameters.is_empty() {
                        String::new()
                    } else {
                        format!("<{}> ", parameters.join(", "))
                    };
                    hook.declaration(&receiver_name)
                        .replacen("fun ", &format!("fun {}", list), 1)
                })),
            );
        }
        // Markers (D4 and D6).
        let marker = self.types.marker(&id);
        match hook {
            Hook::Drop if marker == Some(prism_ast::TypeMarker::Copy) => {
                return report(
                    self,
                    format!(
                        "'{}' is ': Copy', so it cannot have a drop hook: a 'Copy' value is copied bitwise wherever it is read, so a hook would run once per copy",
                        id.name
                    ),
                    Some(format!("remove ': Copy' from '{}'", id.name)),
                );
            }
            Hook::Drop
                if marker == Some(prism_ast::TypeMarker::Clone)
                    && !self.types.has_clone_hook(&id) =>
            {
                return report(
                    self,
                    format!(
                        "'{}' is ': Clone' and has a drop hook, so it needs a clone hook: a field-by-field clone would duplicate what its drop hook releases",
                        id.name
                    ),
                    Some(format!(
                        "declare '{}', building a value that owns its own resources",
                        Hook::Clone.declaration(&receiver_name)
                    )),
                );
            }
            Hook::Clone if marker == Some(prism_ast::TypeMarker::Copy) => {
                return report(
                    self,
                    format!(
                        "'{}' is ': Copy', so it cannot have a clone hook: a 'Copy' value is copied bitwise, and cloning it is a copy",
                        id.name
                    ),
                    None,
                );
            }
            Hook::Clone if marker.is_none() => {
                return report(
                    self,
                    format!(
                        "'{}' is not ': Clone', so it cannot have a clone hook",
                        id.name
                    ),
                    Some(format!(
                        "declare '{}' with the marker, as in 'struct {} : Clone'",
                        id.name, id.name
                    )),
                );
            }
            _ => {}
        }
        match &f.return_type {
            Some(annotation) => {
                let type_parameters = super::type_parameters_of(&f.type_parameters);
                let result =
                    self.with_type_parameters(type_parameters, |c| c.lower_type(annotation));
                self.check_hook_result(f, hook, &result)
            }
            None if matches!(f.body, Some(FunctionBody::Block(_)) | None) => {
                self.check_hook_result(f, hook, &Type::Unit)
            }
            // An inferred result is checked once inferred.
            None => true,
        }
    }

    /// Reports `E4035` when the hook `f` does not return what its kind returns: `Unit` for a
    /// drop hook, its receiver's type for a clone hook. Gives whether it is valid.
    fn check_hook_result(&mut self, f: &FunctionDecl, hook: Hook, result: &Type) -> bool {
        let Some(receiver) = f.receiver.as_ref() else {
            return true;
        };
        if result == &Type::Error {
            return true;
        }
        let type_parameters = super::type_parameters_of(&f.type_parameters);
        let receiver_ty =
            self.with_type_parameters(type_parameters, |c| c.lower_type(&receiver.ty));
        let expected = match hook {
            Hook::Drop => Type::Unit,
            Hook::Clone => receiver_ty.clone(),
        };
        if *result == expected || receiver_ty == Type::Error {
            return true;
        }
        let written = match &receiver_ty {
            Type::Struct { id, .. } | Type::Enum { id, .. } => {
                let parameters = self.types.parameters(id);
                if parameters.is_empty() {
                    id.name.clone()
                } else {
                    let names: Vec<&str> = parameters.iter().map(|p| p.name.as_str()).collect();
                    format!("{}<{}>", id.name, names.join(", "))
                }
            }
            _ => "T".to_string(),
        };
        self.errors.push(TypeError::InvalidOperatorDeclaration {
            message: hook.shape(&written),
            help: None,
            span: f.span,
        });
        false
    }

    /// Reports `E4010` for the operator function `f` on `receiver_ty` with parameter types
    /// `parameters` when a built-in operator always serves its uses (D3): arithmetic, unary and
    /// ordering operators on `Int`, `Double`, `Char`, `Bool` and `String` for operand types the
    /// built-in accepts, and the indexing, iteration and membership conventions on `List` and
    /// `Range` (`get` and `set` on `String`). Gives whether it reported.
    pub(super) fn check_builtin_operator_clash(
        &mut self,
        f: &FunctionDecl,
        receiver_ty: &Type,
        parameters: &[Type],
    ) -> bool {
        // A built-in operator reads its operands through their `&`, `&mut` and `Heap<T>`
        // layers, so it shadows functions on and for such layers too.
        let operand = |ty: &Type| super::peel_indirection(ty).clone();
        let receiver_ty = super::peel_indirection(receiver_ty);
        let range = matches!(receiver_ty, Type::Struct { id, .. } if *id == prism_typed_tree::NominalId::builtin_range());
        let clash = match (f.name.as_str(), receiver_ty) {
            (
                "plus" | "minus" | "times" | "div" | "rem",
                Type::Int | Type::Float | Type::String,
            ) => {
                parameters.len() == 1 && {
                    let op = match f.name.as_str() {
                        "plus" => prism_ast::BinaryOp::Add,
                        "minus" => prism_ast::BinaryOp::Sub,
                        "times" => prism_ast::BinaryOp::Mul,
                        "div" => prism_ast::BinaryOp::Div,
                        _ => prism_ast::BinaryOp::Mod,
                    };
                    super::arithmetic_result(op, receiver_ty, &operand(&parameters[0])).is_some()
                }
            }
            ("unaryMinus", Type::Int | Type::Float) | ("not", Type::Bool) => parameters.is_empty(),
            ("compareTo", Type::Int | Type::Float | Type::Char) => {
                parameters.len() == 1 && &operand(&parameters[0]) == receiver_ty
            }
            ("get" | "set" | "iterator" | "hasNext" | "next" | "contains", Type::List(_)) => true,
            ("get" | "set" | "iterator" | "hasNext" | "next" | "contains", _) if range => true,
            ("get" | "set", Type::String) => true,
            _ => false,
        };
        if clash {
            let constructor = match ReceiverKey::of(receiver_ty) {
                ReceiverKey::List => "List".to_string(),
                _ => receiver_ty.to_string(),
            };
            self.errors.push(TypeError::UncallableExtension {
                message: format!(
                    "operator '{}.{}' can never be called: the built-in operator always takes precedence for {}",
                    constructor,
                    f.name,
                    served(&f.name)
                ),
                span: f.span,
            });
        }
        clash
    }

    /// Reports `E4035` when the file declares both a shared and a `mut` accessor `f` (`get` or
    /// `next`) of one receiver whose parameter types or element types disagree (D9): the
    /// shared one's result is `&E` or `E`, the `mut` one's `&mut E`.
    pub(super) fn check_accessor_pair(
        &mut self,
        f: &FunctionDecl,
        key: &(ReceiverKey, String),
        context: &Option<Type>,
        signature: &Type,
    ) {
        if !f.is_operator || !(f.name == "get" || f.name == "next") {
            return;
        }
        let mode = f
            .receiver
            .as_ref()
            .map_or(ReceiverMode::Shared, |receiver| receiver.mode);
        let other_name = if mode == ReceiverMode::Mutable {
            f.name.clone()
        } else {
            mutable_accessor_name(&f.name)
        };
        let other_key = (key.0.clone(), other_name);
        let Some(other) = self
            .own_extensions
            .get(&other_key)
            .and_then(|extensions| extensions.of_context(context).first())
            .map(|(_, extension)| extension.clone())
        else {
            return;
        };
        if !other.is_operator {
            return;
        }
        let SymbolKind::Function {
            fn_type: other_fn, ..
        } = &other.symbol.kind
        else {
            return;
        };
        let (shared, mutable) = if mode == ReceiverMode::Mutable {
            (other_fn, signature)
        } else {
            (signature, other_fn)
        };
        let (
            Type::Fn {
                params: shared_params,
                ret: shared_ret,
                ..
            },
            Type::Fn {
                params: mutable_params,
                ret: mutable_ret,
                ..
            },
        ) = (shared, mutable)
        else {
            return;
        };
        if **shared_ret == Type::Error || **mutable_ret == Type::Error {
            return;
        }
        let shared_element = match shared_ret.as_ref() {
            Type::Reference {
                mutable: false,
                inner,
            } => inner.as_ref(),
            other => other,
        };
        let mutable_element = match mutable_ret.as_ref() {
            Type::Reference {
                mutable: true,
                inner,
            } => inner.as_ref(),
            other => other,
        };
        if shared_params != mutable_params || shared_element != mutable_element {
            self.errors.push(TypeError::InvalidOperatorDeclaration {
                message: format!(
                    "the shared and the 'mut' '{}' of one type must take the same parameters and reach the same element type: one returns '{}', the other '{}'",
                    f.name, shared_ret, mutable_ret
                ),
                help: Some(format!(
                    "declare the pair as 'operator fun T.{}(...): &E' and 'operator mut fun T.{}(...): &mut E'",
                    f.name, f.name
                )),
                span: f.span,
            });
        }
    }
}

/// An operator use whose function is looked up on its receiver: the receiver already inferred,
/// the other operands as the arguments of the call it spells (D4).
pub(super) struct OperatorUse<'a> {
    /// The operator the call is marked with.
    pub(super) operator: SpelledOperator,
    /// The convention's name (`plus`), or the `mut` accessor's table name (`get#mut`).
    pub(super) name: &'a str,
    /// The receiver as written, and its node: a place read in place or a consumed temporary.
    pub(super) target: &'a Expr,
    pub(super) receiver: ExpressionId,
    pub(super) is_place: bool,
    pub(super) writable: bool,
    pub(super) args: &'a [CallArg],
    pub(super) span: Span,
    pub(super) expected: Option<&'a Type>,
}

/// Why no operator function served an operator use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum OperatorMiss {
    /// No function of the convention's name on the receiver's type.
    Missing,
    /// A function of that name not declared `operator`.
    Unmarked,
    /// A nullable receiver, whose payload's type has the operator function.
    Nullable,
}

impl TypeChecker {
    /// Whether `expr` stands for an operand already inferred.
    pub(super) fn is_preinferred(&self, expr: &Expr) -> bool {
        self.preinferred_arguments
            .contains_key(&(expr as *const Expr as usize))
    }

    /// The node of the operand `expr` stands for, if it was inferred before its operator
    /// function was looked up, with whether it is a place read in place.
    pub(super) fn take_preinferred(&mut self, expr: &Expr) -> Option<(ExpressionId, bool)> {
        if self.preinferred_arguments.is_empty() {
            return None;
        }
        self.preinferred_arguments
            .remove(&(expr as *const Expr as usize))
    }

    /// Infers an operand of an operator as `infer_read` does (a place read in place, anything
    /// else a consumed temporary), without peeling its `&`, `&mut` and `Heap<T>` layers, and
    /// says whether it was read in place.
    pub(super) fn infer_operator_operand(&mut self, expr: &Expr) -> (ExpressionId, bool) {
        let read_in_place = self.is_place(expr)
            || self.implicit_this_field(expr).is_some()
            || matches!(&expr.kind, ExprKind::MemberAccess { target, .. }
                if self.type_name_symbol(target).is_none());
        let id = self.with_origin(Origin::Value, |c| c.infer_read(expr));
        (id, read_in_place)
    }

    /// Calls the operator function `use_.name` of the receiver's type, found as a method call
    /// finds an extension (members, then the file's own extensions, then its imports', through
    /// `&`, `&mut` and `Heap<T>` layers), and marks the call with the operator. A function of
    /// that name not declared `operator`, none at all, or a nullable receiver is a miss, which
    /// the caller reports; an ambiguous import is `E4012`.
    pub(super) fn call_operator(
        &mut self,
        use_: OperatorUse,
    ) -> Result<ExpressionId, OperatorMiss> {
        let receiver_ty = self.ty(use_.receiver);
        if let Type::Nullable(payload) = super::peel_references(&receiver_ty) {
            let mut layers = Vec::new();
            let (lookup, _) = self.find_extension_through_indirection(
                payload.as_ref().clone(),
                use_.name,
                &mut layers,
            );
            return Err(match lookup {
                ExtensionLookup::Found { extension, .. } if extension.is_operator => {
                    OperatorMiss::Nullable
                }
                _ => OperatorMiss::Missing,
            });
        }
        let mut layers = Vec::new();
        let (lookup, reached) =
            self.find_extension_through_indirection(receiver_ty.clone(), use_.name, &mut layers);
        let (extension, own) = match lookup {
            ExtensionLookup::None | ExtensionLookup::MissingContext(_) => {
                return Err(OperatorMiss::Missing)
            }
            ExtensionLookup::Ambiguous(modules) => {
                let call = self.give_up_call(use_.receiver, use_.args, use_.span);
                self.errors.push(TypeError::AmbiguousImport {
                    name: format!("{}.{}", reached, use_.name),
                    modules,
                    span: use_.span,
                });
                return Ok(call);
            }
            ExtensionLookup::Found { extension, .. } if !extension.is_operator => {
                return Err(OperatorMiss::Unmarked)
            }
            ExtensionLookup::Found { extension, own } => (extension, own),
        };
        let member = use_
            .name
            .trim_end_matches(MUTABLE_ACCESSOR_SUFFIX)
            .to_string();
        let callee = Expr::new(ExprKind::Ident(member.clone()), use_.span);
        let call = self.call_found_extension(
            MethodCall {
                callee: &callee,
                target: use_.target,
                member: &member,
                is_safe_nav: false,
                type_arguments: &[],
                args: use_.args,
                expected: use_.expected,
            },
            FoundExtension {
                extension,
                own,
                reached,
                receiver: use_.receiver,
                receiver_ty,
                is_place: use_.is_place,
                writable: use_.writable,
                layers: &layers,
                through_nullable: false,
            },
            use_.span,
        );
        if let ExpressionKind::Call(call) = &mut self.builder.expression_mut(call).kind {
            call.operator = Some(use_.operator);
        }
        Ok(call)
    }

    /// Reports `E4002` for the operator use `use_text` (`a * b`) whose receiver of type
    /// `receiver` has no operator function `function` serving it: with help naming the
    /// declaration to add (`declaration`), saying to mark an existing function `operator`, or
    /// suggesting a safe call `safe_call` on a nullable receiver.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn report_missing_operator(
        &mut self,
        miss: OperatorMiss,
        message: String,
        receiver: &Type,
        function: &str,
        declaration: String,
        safe_call: String,
        span: Span,
    ) {
        let reached = super::peel_indirection(receiver).clone();
        let help = match miss {
            OperatorMiss::Missing => match self.unimported_module_help(&reached) {
                // The function may well exist where the type does (D3): say so first.
                Some(import) => format!(
                    "{} if it declares '{}', or declare '{}'",
                    import, function, declaration
                ),
                None => format!("declare '{}'", declaration),
            },
            OperatorMiss::Unmarked => format!(
                "'{}' must be declared 'operator fun {}.{}' to serve {}",
                function,
                reached,
                function,
                served(function)
            ),
            OperatorMiss::Nullable => format!(
                "the receiver may be null: call the operator function with a safe call, '{}'",
                safe_call
            ),
        };
        self.errors.push(TypeError::MissingOperatorFunction {
            message,
            receiver: reached,
            function: function.to_string(),
            help: Some(help),
            span,
            import_suggestions: Vec::new(),
        });
    }

    /// Registers `node`, inferred from `operand`, as the argument `argument` stands for (a
    /// clone of `operand` in the operator's call), returning its address key.
    fn preinfer(&mut self, argument: &Expr, node: ExpressionId, is_place: bool) -> usize {
        let key = argument as *const Expr as usize;
        self.preinferred_arguments.insert(key, (node, is_place));
        key
    }

    /// Infers `left op right` for an arithmetic or ordering operator (D3, D4 and D7): the
    /// built-in operator when it accepts the operand types, else the operator function of the
    /// left operand's type (`plus`, or `compareTo` compared with `0`). The left operand is the
    /// receiver; the right one is checked as the function's argument, or, when the left one
    /// has a built-in type, was inferred before and is passed on as it is.
    pub(super) fn infer_operator_binary(
        &mut self,
        op: BinaryOp,
        left: &Expr,
        right: &Expr,
        span: Span,
    ) -> ExpressionId {
        let (left_raw, left_in_place) = self.infer_operator_operand(left);
        let left_ty = super::peel_indirection(&self.ty(left_raw)).clone();
        let name = match op {
            BinaryOp::Add => "plus",
            BinaryOp::Sub => "minus",
            BinaryOp::Mul => "times",
            BinaryOp::Div => "div",
            BinaryOp::Mod => "rem",
            _ => "compareTo",
        };
        let spelled = spelled_binary(op);
        let ordering = name == "compareTo";
        let argument = CallArg {
            name: None,
            value: right.clone(),
            span: right.span,
            is_trailing_lambda: false,
            is_spread: false,
        };
        let args = std::slice::from_ref(&argument);
        if has_builtin_operators(&left_ty) {
            let (right_raw, right_in_place) = self.infer_operator_operand(right);
            let right_ty = super::peel_indirection(&self.ty(right_raw)).clone();
            let builtin = left_ty == Type::Error
                || right_ty == Type::Error
                || if ordering {
                    left_ty == right_ty && matches!(left_ty, Type::Int | Type::Float | Type::Char)
                } else {
                    super::arithmetic_result(op, &left_ty, &right_ty).is_some()
                };
            if !builtin {
                // An operator function of a built-in type for other operand types
                // (`Int.times(Dp)`), with the right operand already inferred.
                let key = self.preinfer(&argument.value, right_raw, right_in_place);
                let called = self.call_operator(OperatorUse {
                    operator: spelled,
                    name,
                    target: left,
                    receiver: left_raw,
                    is_place: left_in_place,
                    writable: true,
                    args,
                    span,
                    expected: None,
                });
                self.preinferred_arguments.remove(&key);
                match called {
                    Ok(call) => return self.finish_ordering(op, call, ordering, span),
                    Err(OperatorMiss::Unmarked) => {
                        let (left_id, right_id) = (
                            self.peel_indirection_nodes(left_raw),
                            self.peel_indirection_nodes(right_raw),
                        );
                        self.report_missing_operator(
                            OperatorMiss::Unmarked,
                            operator_message(op, &left_ty, &right_ty),
                            &left_ty,
                            name,
                            String::new(),
                            String::new(),
                            span,
                        );
                        return self.error_node(vec![left_id, right_id], span);
                    }
                    Err(_) => {}
                }
            }
            let left_id = self.peel_indirection_nodes(left_raw);
            let right_id = self.peel_indirection_nodes(right_raw);
            return self.builtin_binary(op, left_id, right_id, span);
        }
        let called = self.call_operator(OperatorUse {
            operator: spelled,
            name,
            target: left,
            receiver: left_raw,
            is_place: left_in_place,
            writable: true,
            args,
            span,
            expected: None,
        });
        match called {
            Ok(call) => self.finish_ordering(op, call, ordering, span),
            Err(miss) => {
                let right_id = self.infer_operand(right);
                let right_ty = self.ty(right_id);
                let left_id = self.peel_indirection_nodes(left_raw);
                if left_ty != Type::Error && right_ty != Type::Error {
                    let parameter = if ordering {
                        format!("&{}", right_ty)
                    } else {
                        right_ty.to_string()
                    };
                    let result = if ordering {
                        ": Int".to_string()
                    } else {
                        String::new()
                    };
                    self.report_missing_operator(
                        miss,
                        operator_message(op, &left_ty, &right_ty),
                        &left_ty,
                        name,
                        format!(
                            "operator fun {}.{}(other: {}){}",
                            left_ty, name, parameter, result
                        ),
                        format!("{}?.{}({})", left, name, right),
                        span,
                    );
                }
                self.error_node(vec![left_id, right_id], span)
            }
        }
    }

    /// The value of an operator use whose call is `call`: the call itself, or for an ordering
    /// operator the built-in comparison of the call's `Int` with `0` (D7).
    fn finish_ordering(
        &mut self,
        op: BinaryOp,
        call: ExpressionId,
        ordering: bool,
        span: Span,
    ) -> ExpressionId {
        if !ordering || self.ty(call) == Type::Error {
            return call;
        }
        let zero = self.add(ExpressionKind::Literal(Literal::Int(0)), Type::Int, span);
        let kind = ExpressionKind::Binary {
            operator: super::binary_operator(op),
            left: call,
            right: zero,
        };
        self.add(kind, Type::Bool, span)
    }

    /// Infers `left in right` or `left !in right` (D8): the built-in range test on a `Range`,
    /// else `right.contains(left)` (a list's intrinsic member or an operator function), with
    /// `left` evaluated first; `!in` negates it.
    pub(super) fn infer_membership(
        &mut self,
        op: BinaryOp,
        left: &Expr,
        right: &Expr,
        span: Span,
    ) -> ExpressionId {
        let spelled = if op == BinaryOp::In {
            SpelledOperator::In
        } else {
            SpelledOperator::NotIn
        };
        let (right_raw, right_in_place) = self.infer_operator_operand(right);
        let right_ty = super::peel_indirection(&self.ty(right_raw)).clone();
        let test = if right_ty == Type::Error {
            let left_id = self.infer_operand(left);
            let right_id = self.peel_indirection_nodes(right_raw);
            return self.error_node(vec![left_id, right_id], span);
        } else if matches!(&right_ty, Type::Struct { id, .. } if *id == prism_typed_tree::NominalId::builtin_range())
        {
            let element = self.check_operand(left, &Type::Int);
            let range = self.peel_indirection_nodes(right_raw);
            let range = self.consume(range);
            let range_ty = self.ty(range);
            let call = Call {
                callee: Callee::Builtin(BuiltinFunction::RangeContains),
                arguments: vec![Argument::Written(range), Argument::Written(element)],
                parameters: vec![
                    (Some(super::THIS.to_string()), range_ty),
                    (Some("element".to_string()), Type::Int),
                ],
                parameter_kinds: vec![ParameterKind::Plain, ParameterKind::Plain],
                type_arguments: Vec::new(),
                // The element is written first.
                evaluation_order: vec![1, 0],
                has_receiver: false,
                has_context: false,
                is_compose: false,
                operator: Some(spelled),
            };
            self.add(ExpressionKind::Call(call), Type::Bool, span)
        } else {
            let argument = CallArg {
                name: None,
                value: left.clone(),
                span: left.span,
                is_trailing_lambda: false,
                is_spread: false,
            };
            let called = self.call_operator(OperatorUse {
                operator: spelled,
                name: "contains",
                target: right,
                receiver: right_raw,
                is_place: right_in_place,
                writable: true,
                args: std::slice::from_ref(&argument),
                span,
                expected: None,
            });
            match called {
                Ok(call) => {
                    // `a in b` evaluates `a` first, as written (D4).
                    if let ExpressionKind::Call(call) = &mut self.builder.expression_mut(call).kind
                    {
                        let receiver = usize::from(call.has_context);
                        call.evaluation_order.retain(|&index| index != receiver);
                        call.evaluation_order.push(receiver);
                    }
                    call
                }
                Err(miss) => {
                    let left_id = self.infer_operand(left);
                    let left_ty = self.ty(left_id);
                    let right_id = self.peel_indirection_nodes(right_raw);
                    if left_ty != Type::Error {
                        self.report_missing_operator(
                            miss,
                            format!(
                                "operator '{}' cannot be applied to '{}' and '{}'",
                                op.as_str(),
                                left_ty,
                                right_ty
                            ),
                            &right_ty,
                            "contains",
                            format!(
                                "operator fun {}.contains(element: &{}): Bool",
                                right_ty, left_ty
                            ),
                            format!("{}?.contains({})", right, left),
                            span,
                        );
                    }
                    return self.error_node(vec![left_id, right_id], span);
                }
            }
        };
        if op == BinaryOp::NotIn && self.ty(test) != Type::Error {
            let kind = ExpressionKind::Unary {
                operator: UnaryOperator::Not,
                operand: test,
            };
            return self.add(kind, Type::Bool, span);
        }
        test
    }

    /// Infers `-operand` or `!operand` (D4): the built-in negation of an `Int` or `Double` or
    /// `!` of a `Bool`, else `unaryMinus()` or `not()` of the operand's type.
    pub(super) fn infer_operator_unary(
        &mut self,
        op: UnaryOp,
        operand: &Expr,
        span: Span,
    ) -> ExpressionId {
        let (raw, in_place) = self.infer_operator_operand(operand);
        let ty = super::peel_indirection(&self.ty(raw)).clone();
        let negate = op == UnaryOp::Neg;
        let builtin = match ty {
            Type::Error | Type::Nothing => true,
            Type::Int | Type::Float => negate,
            Type::Bool => !negate,
            _ => false,
        };
        if builtin || has_builtin_operators(&ty) {
            let id = self.peel_indirection_nodes(raw);
            return self.builtin_unary(op, id, span);
        }
        let (name, spelled) = if negate {
            ("unaryMinus", SpelledOperator::Negate)
        } else {
            ("not", SpelledOperator::Not)
        };
        let called = self.call_operator(OperatorUse {
            operator: spelled,
            name,
            target: operand,
            receiver: raw,
            is_place: in_place,
            writable: true,
            args: &[],
            span,
            expected: None,
        });
        match called {
            Ok(call) => call,
            Err(miss) => {
                let id = self.peel_indirection_nodes(raw);
                self.report_missing_operator(
                    miss,
                    format!("operator '{}' cannot be applied to '{}'", op.as_str(), ty),
                    &ty,
                    name,
                    format!("operator fun {}.{}(): {}", ty, name, ty),
                    format!("{}?.{}()", operand, name),
                    span,
                );
                self.error_node(vec![id], span)
            }
        }
    }
}

/// Whether a built-in operator may accept operands of type `ty`, so its own diagnostics apply
/// when no operator function serves the use: the scalars, `String`, the types that stand for
/// none (errors, `Nothing`, type parameters, unresolved names), and nullables of them.
pub(super) fn has_builtin_operators(ty: &Type) -> bool {
    if let Type::Nullable(inner) = ty {
        return has_builtin_operators(inner);
    }
    matches!(
        ty,
        Type::Int
            | Type::Float
            | Type::Bool
            | Type::Char
            | Type::String
            | Type::Unit
            | Type::Error
            | Type::Nothing
            | Type::Parameter(_)
            | Type::Opaque(_)
    )
}

/// The marked operator of an arithmetic or ordering operator.
fn spelled_binary(op: BinaryOp) -> SpelledOperator {
    match op {
        BinaryOp::Add => SpelledOperator::Add,
        BinaryOp::Sub => SpelledOperator::Subtract,
        BinaryOp::Mul => SpelledOperator::Multiply,
        BinaryOp::Div => SpelledOperator::Divide,
        BinaryOp::Mod => SpelledOperator::Remainder,
        BinaryOp::Lt => SpelledOperator::Less,
        BinaryOp::LtEq => SpelledOperator::LessOrEqual,
        BinaryOp::Gt => SpelledOperator::Greater,
        _ => SpelledOperator::GreaterOrEqual,
    }
}

/// `operator '*' cannot be applied to 'Point' and 'Point'`.
fn operator_message(op: BinaryOp, left: &Type, right: &Type) -> String {
    format!(
        "operator '{}' cannot be applied to '{}' and '{}'",
        op.as_str(),
        left,
        right
    )
}

/// The convention name of an arithmetic operator (`plus` for `+`).
pub(super) fn arithmetic_name(op: BinaryOp) -> &'static str {
    match op {
        BinaryOp::Add => "plus",
        BinaryOp::Sub => "minus",
        BinaryOp::Mul => "times",
        BinaryOp::Div => "div",
        _ => "rem",
    }
}

impl TypeChecker {
    /// The operator function a compound assignment `target op= value` calls on a target of type
    /// `target_ty` when its operator is not built in (D13): the function of the convention on
    /// the target's type, its argument checked from `value` (or the operand `preinferred`
    /// already inferred), and its result of the target's type (`E4002` otherwise). The target
    /// is the receiver, borrowed or moved by the function's mode. A miss is the caller's to
    /// report; `Err(None)` means a diagnostic was reported here.
    pub(super) fn compound_function(
        &mut self,
        operator: BinaryOp,
        target: &Expr,
        target_ty: &Type,
        value: &Expr,
        preinferred: Option<(ExpressionId, bool)>,
        span: Span,
    ) -> Result<(ExpressionId, prism_typed_tree::CompoundFunction), Option<OperatorMiss>> {
        let name = arithmetic_name(operator);
        if let Type::Nullable(payload) = target_ty {
            let mut layers = Vec::new();
            let (lookup, _) = self.find_extension_through_indirection(
                payload.as_ref().clone(),
                name,
                &mut layers,
            );
            return Err(Some(match lookup {
                ExtensionLookup::Found { extension, .. } if extension.is_operator => {
                    OperatorMiss::Nullable
                }
                _ => OperatorMiss::Missing,
            }));
        }
        let mut layers = Vec::new();
        let (lookup, reached) =
            self.find_extension_through_indirection(target_ty.clone(), name, &mut layers);
        let extension = match lookup {
            // The target is the receiver itself, never reached through a reference or box.
            ExtensionLookup::Found { .. } if !layers.is_empty() => {
                return Err(Some(OperatorMiss::Missing))
            }
            ExtensionLookup::Found { extension, .. } if !extension.is_operator => {
                return Err(Some(OperatorMiss::Unmarked))
            }
            ExtensionLookup::Found { extension, own } => {
                if own {
                    let key = super::InferenceKey::Extension {
                        receiver: ReceiverKey::of(&reached),
                        name: name.to_string(),
                        context: extension.context.clone(),
                    };
                    self.ensure_key_inferred(&key, span);
                    match self.find_extension(&reached, name) {
                        ExtensionLookup::Found { extension, .. } => extension,
                        _ => extension,
                    }
                } else {
                    extension
                }
            }
            ExtensionLookup::Ambiguous(modules) => {
                self.errors.push(TypeError::AmbiguousImport {
                    name: format!("{}.{}", reached, name),
                    modules,
                    span,
                });
                return Err(None);
            }
            ExtensionLookup::None | ExtensionLookup::MissingContext(_) => {
                return Err(Some(OperatorMiss::Missing))
            }
        };
        if !self.check_visibility(&extension.symbol, span)
            || !self.receiver_matches(&extension, &reached)
        {
            return Err(None);
        }
        // A consuming function moves the target out and the result is assigned back: only a
        // whole binding can be moved (no partial moves, and none out of a borrow or a list).
        if extension.mode == ReceiverMode::Consuming {
            let before = self.errors.len();
            self.check_move_source(target, target_ty);
            if self.errors.len() > before {
                if let Some(TypeError::CannotMoveOut { source, span, .. }) = self.errors.last() {
                    let (source, span) = (source.clone(), *span);
                    self.errors.pop();
                    self.errors.push(TypeError::ConsumingCompoundTarget {
                        source,
                        function: format!("{}.{}", reached, name),
                        operator: operator.as_str().to_string(),
                        span,
                    });
                }
                return Err(None);
            }
        }
        let SymbolKind::Function {
            fn_type: Type::Fn { params, ret, .. },
            param_names,
            defaults,
            type_parameters,
            ..
        } = &extension.symbol.kind
        else {
            return Err(None);
        };
        let (params, param_names, defaults, ret, type_parameters) = (
            params.clone(),
            param_names.clone(),
            defaults.clone(),
            (**ret).clone(),
            type_parameters.clone(),
        );
        let argument = CallArg {
            name: None,
            value: value.clone(),
            span: value.span,
            is_trailing_lambda: false,
            is_spread: false,
        };
        let key =
            preinferred.map(|(node, is_place)| self.preinfer(&argument.value, node, is_place));
        let args = std::slice::from_ref(&argument);
        let checked = if type_parameters.is_empty() {
            self.check_call_arguments(&params, &param_names, &defaults, args, span)
                .map(|normalized| (normalized, params.clone(), ret.clone(), Vec::new()))
        } else {
            let signature = super::GenericSignature {
                owner: format!("extension '{}.{}'", extension.receiver, name),
                type_parameters,
                params: params.clone(),
                ret: ret.clone(),
                receiver: Some((extension.receiver.clone(), reached.clone())),
            };
            self.check_generic_call_arguments(
                &signature,
                &[],
                Some(target_ty),
                &param_names,
                &defaults,
                args,
                span,
            )
            .map(|(normalized, solution)| {
                let ret = signature.ret.substitute(&solution);
                (normalized, signature.params, ret, solution)
            })
        };
        if let Some(key) = key {
            self.preinferred_arguments.remove(&key);
        }
        let (normalized, params, ret, type_arguments) = match checked {
            Ok(checked) => checked,
            Err(_) => return Err(None),
        };
        let Some(Argument::Written(value_node)) = normalized.arguments.first().cloned() else {
            return Err(None);
        };
        if &ret != target_ty && ret != Type::Error {
            self.errors.push(TypeError::TypeMismatch {
                expected: target_ty.clone(),
                found: ret.clone(),
                span,
                help: Some(format!(
                    "'{}=' assigns the result of '{}.{}' to its target, so the function must return '{}'",
                    operator.as_str(),
                    extension.receiver,
                    name,
                    target_ty
                )),
            });
            return Err(None);
        }
        let this_ty = match extension.mode {
            ReceiverMode::Shared => Type::Reference {
                mutable: false,
                inner: Box::new(extension.receiver.clone()),
            },
            ReceiverMode::Mutable => Type::Reference {
                mutable: true,
                inner: Box::new(extension.receiver.clone()),
            },
            ReceiverMode::Consuming => extension.receiver.clone(),
        };
        let mut parameters = vec![(Some(super::THIS.to_string()), this_ty)];
        parameters.extend(super::parameter_list(&param_names, &params));
        Ok((
            value_node,
            prism_typed_tree::CompoundFunction {
                path: extension.path(name),
                mode: extension.mode,
                parameters,
                type_arguments,
            },
        ))
    }
}

/// Whether a call of a value of type `ty` calls its `invoke` (D12): a type that is not a
/// function type after its `&`, `&mut` and `Heap<T>` layers, nor one that cannot be called at
/// all for another reason (an error, `Nothing`, a scalar or `String`, which have no `invoke`).
pub(super) fn calls_invoke(ty: &Type) -> bool {
    !matches!(
        super::peel_indirection(ty),
        Type::Fn { .. } | Type::Error | Type::Nothing
    ) && !has_builtin_operators(super::peel_indirection(ty))
}

impl TypeChecker {
    /// The call `callee(args)` of the value `node` of a type that is not a function type,
    /// through that type's `invoke` (D12): the value is the receiver, borrowed or moved by the
    /// function's mode (`is_place` and `writable` describe it), and the call's arguments are
    /// `invoke`'s. Without one, the value cannot be called (`E4004`).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn invoke_value(
        &mut self,
        callee: &Expr,
        node: ExpressionId,
        is_place: bool,
        writable: bool,
        args: &[CallArg],
        expected: Option<&Type>,
        span: Span,
    ) -> ExpressionId {
        let called = self.call_operator(OperatorUse {
            operator: SpelledOperator::Invoke,
            name: "invoke",
            target: callee,
            receiver: node,
            is_place,
            writable,
            args,
            span,
            expected,
        });
        match called {
            Ok(call) => call,
            Err(miss) => {
                let ty = self.ty(node);
                let reason = match miss {
                    OperatorMiss::Missing => match self.unimported_module_help(&ty) {
                        Some(import) => format!("; {} if it declares 'invoke'", import),
                        None => String::new(),
                    },
                    OperatorMiss::Unmarked => {
                        ": its 'invoke' is not declared 'operator'".to_string()
                    }
                    OperatorMiss::Nullable => {
                        format!(": it may be null; call '{}?.invoke(...)'", callee)
                    }
                };
                self.errors.push(TypeError::CallSignatureMismatch {
                    message: format!("type '{}' cannot be called as a function{}", ty, reason),
                    span,
                });
                self.error_node(vec![node], span)
            }
        }
    }
}

impl TypeChecker {
    /// `target[indices]` on a type other than `List` and `String` (D9): a call of its `get`
    /// operator with the indices as arguments, the shared one (else the `mut` one when it is the
    /// only one). A `get` returning a borrow makes the element the place behind it,
    /// `Deref(Temporary(call))`, writable when the indexed value is and a `mut` `get` agreeing
    /// with the shared one exists, which a write then switches to
    /// ([`Self::make_accessors_mutable`]); a `get` returning a value makes the element that
    /// value. Without `get`, the target cannot be indexed (`E4002`).
    pub(super) fn infer_accessor_index(
        &mut self,
        target: &Expr,
        indexed: (ExpressionId, bool, bool),
        indices: &[Expr],
        span: Span,
    ) -> (ExpressionId, bool) {
        let (target_id, is_place, target_writable) = indexed;
        let target_ty = self.ty(target_id);
        let args: Vec<CallArg> = indices
            .iter()
            .map(|index| CallArg {
                name: None,
                value: index.clone(),
                span: index.span,
                is_trailing_lambda: false,
                is_spread: false,
            })
            .collect();
        let use_of = |name| OperatorUse {
            operator: SpelledOperator::Index,
            name,
            target,
            receiver: target_id,
            is_place,
            writable: target_writable,
            args: &args,
            span,
            expected: None,
        };
        let mutable_name = mutable_accessor_name("get");
        let (called, only_mutable) = match self.call_operator(use_of("get")) {
            Err(OperatorMiss::Missing) => (self.call_operator(use_of(&mutable_name)), true),
            other => (other, false),
        };
        let call = match called {
            Ok(call) => call,
            Err(miss) => {
                let mut children = vec![self.peel_indirection_nodes(target_id)];
                let mut index_types = Vec::new();
                for index in indices {
                    let node = self.infer_operand(index);
                    index_types.push(self.ty(node).to_string());
                    children.push(node);
                }
                let indexed = super::peel_indirection(&target_ty).clone();
                if indexed != Type::Error {
                    let parameters: Vec<String> = match &index_types[..] {
                        [only] => vec![format!("index: {}", only)],
                        many => many
                            .iter()
                            .enumerate()
                            .map(|(i, ty)| format!("index{}: {}", i + 1, ty))
                            .collect(),
                    };
                    let explain = miss == OperatorMiss::Missing;
                    self.report_missing_operator(
                        miss,
                        format!("'{}' cannot be indexed", indexed),
                        &indexed,
                        "get",
                        format!(
                            "operator fun {}.get({}): &E",
                            indexed,
                            parameters.join(", ")
                        ),
                        format!("{}?.get(...)", target),
                        span,
                    );
                    if let Some(TypeError::MissingOperatorFunction {
                        help: Some(help), ..
                    }) = self.errors.last_mut().filter(|_| explain)
                    {
                        help.push_str(
                            ", returning a borrow of the element 'E' or the element itself",
                        );
                    }
                }
                return (self.error_node(children, span), false);
            }
        };
        let Type::Reference { inner, .. } = self.ty(call) else {
            // A `get` returning a value: the element is that value, a temporary.
            return (call, false);
        };
        let element = inner.as_ref().clone();
        let temporary = self.as_place(call);
        let place = self.deref(temporary);
        if only_mutable {
            // The receiver is a mutable borrow already, checked as one.
            return (place, true);
        }
        let (mutable, needed) = self.mutable_accessor(call, &target_ty, &element);
        let writable = mutable.is_some() && super::contents_writable(&target_ty, target_writable);
        self.builder.accessor_places.insert(
            place,
            crate::builder::AccessorPlace {
                temporary,
                call,
                mutable,
                needed,
            },
        );
        (place, writable)
    }

    /// The `mut` `get` the shared accessor call `call` on a value of type `target_ty` switches
    /// to, if the indexed type has one taking the same parameters and returning `&mut element`;
    /// and the declaration a write needs otherwise.
    fn mutable_accessor(
        &mut self,
        call: ExpressionId,
        target_ty: &Type,
        element: &Type,
    ) -> (Option<crate::builder::MutableAccessor>, String) {
        let ExpressionKind::Call(shared) = self.builder.kind(call).clone() else {
            return (None, String::new());
        };
        let declared: Vec<String> = shared
            .parameters
            .iter()
            .skip(1)
            .map(|(name, ty)| format!("{}: {}", name.clone().unwrap_or_default(), ty))
            .collect();
        let mut layers = Vec::new();
        let (lookup, reached) = self.find_extension_through_indirection(
            target_ty.clone(),
            &mutable_accessor_name("get"),
            &mut layers,
        );
        let declaration = format!(
            "operator mut fun {}.get({}): &mut {}",
            reached,
            declared.join(", "),
            element
        );
        let needed = format!("declare '{}' to change the element", declaration);
        let ExtensionLookup::Found { extension, .. } = lookup else {
            return (None, needed);
        };
        if !extension.is_operator {
            return (None, needed);
        }
        let SymbolKind::Function {
            fn_type: Type::Fn { params, ret, .. },
            param_names,
            type_parameters,
            ..
        } = &extension.symbol.kind
        else {
            return (None, needed);
        };
        let type_arguments = if type_parameters.is_empty() {
            Vec::new()
        } else {
            let mut unknowns = crate::inference::Unknowns::new(type_parameters.len());
            unknowns.match_exact(&extension.receiver, &reached);
            unknowns.partial()
        };
        let result = ret.substitute(&type_arguments);
        let agrees = matches!(&result, Type::Reference { mutable: true, inner } if inner.as_ref() == element)
            && params.len() + 1 == shared.parameters.len()
            && params.iter().zip(shared.parameters.iter().skip(1)).all(
                |(mutable, (_, declared))| {
                    mutable.substitute(&type_arguments)
                        == declared.substitute(&shared.type_arguments)
                },
            );
        if !agrees {
            // A `mut` `get` of another file may disagree with the shared one (D9).
            let help = format!(
                "the 'mut' 'get' of '{}' disagrees with its shared 'get': declare it as '{}'",
                reached, declaration
            );
            return (None, help);
        }
        let mut parameters = vec![(
            Some(super::THIS.to_string()),
            Type::Reference {
                mutable: true,
                inner: Box::new(extension.receiver.clone()),
            },
        )];
        parameters.extend(super::parameter_list(param_names, params));
        (
            Some(crate::builder::MutableAccessor {
                path: extension.path("get"),
                parameters,
                type_arguments,
                result,
            }),
            needed,
        )
    }

    /// Switches every accessor place the place `id` is reached through to its `mut` `get`
    /// (D9), for a write, a mutable borrow or a `mut fun` receiver: the call's callee, its
    /// receiver borrow (now mutable) and the types of the call and its temporary. The
    /// indexed value's own place is made mutable in turn.
    pub(super) fn make_accessors_mutable(&mut self, mut id: ExpressionId) {
        loop {
            match self.builder.kind(id).clone() {
                ExpressionKind::Field { base, .. } | ExpressionKind::Index { base, .. } => {
                    id = base
                }
                ExpressionKind::NonNullPayload { nullable } => id = nullable,
                ExpressionKind::Deref { .. } => {
                    let Some(place) = self.builder.accessor_places.remove(&id) else {
                        return;
                    };
                    let Some(mutable) = place.mutable else {
                        return;
                    };
                    let ExpressionKind::Call(mut call) = self.builder.kind(place.call).clone()
                    else {
                        return;
                    };
                    let Some(Argument::Written(receiver)) = call.arguments.first().cloned() else {
                        return;
                    };
                    call.callee = Callee::Function(mutable.path);
                    call.parameters = mutable.parameters;
                    call.type_arguments = mutable.type_arguments;
                    self.builder.expression_mut(place.call).kind = ExpressionKind::Call(call);
                    self.builder.expression_mut(place.call).ty = mutable.result.clone();
                    self.builder.expression_mut(place.temporary).ty = mutable.result;
                    let ExpressionKind::Borrow {
                        origin,
                        place: borrowed,
                        ..
                    } = self.builder.kind(receiver).clone()
                    else {
                        return;
                    };
                    let span = self.builder.span_of(receiver);
                    self.reject_state_mutation(borrowed, "a 'mut fun' call", span);
                    let referent = self.ty(borrowed);
                    let expression = self.builder.expression_mut(receiver);
                    expression.kind = ExpressionKind::Borrow {
                        mutable: true,
                        origin,
                        place: borrowed,
                    };
                    expression.ty = Type::Reference {
                        mutable: true,
                        inner: Box::new(referent),
                    };
                    id = borrowed;
                }
                _ => return,
            }
        }
    }

    /// The help of a write to the place `id` that is not writable because an accessor place it
    /// is reached through has no `mut` `get`: the declaration to add.
    pub(super) fn accessor_write_help(&self, mut id: ExpressionId) -> Option<String> {
        loop {
            match self.builder.kind(id) {
                ExpressionKind::Field { base, .. } | ExpressionKind::Index { base, .. } => {
                    id = *base
                }
                ExpressionKind::NonNullPayload { nullable } => id = *nullable,
                ExpressionKind::Deref { .. } => {
                    let place = self.builder.accessor_places.get(&id)?;
                    if place.mutable.is_none() {
                        return Some(place.needed.clone());
                    }
                    let ExpressionKind::Call(call) = self.builder.kind(place.call) else {
                        return None;
                    };
                    let Some(Argument::Written(receiver)) = call.arguments.first() else {
                        return None;
                    };
                    let ExpressionKind::Borrow { place, .. } = self.builder.kind(*receiver) else {
                        return None;
                    };
                    id = *place;
                }
                _ => return None,
            }
        }
    }

    /// Adds the help of [`Self::accessor_write_help`] to the last error when it is an
    /// `E4003` without help, about writing the place `id`.
    pub(super) fn explain_accessor_write(&mut self, id: ExpressionId) {
        let Some(fix) = self.accessor_write_help(id) else {
            return;
        };
        if let Some(TypeError::ImmutabilityViolation { help, .. }) = self.errors.last_mut() {
            *help = Some(fix);
        }
    }
}

impl TypeChecker {
    /// Infers the target of an index expression as `infer_index` does: a place read in place,
    /// or a consumed temporary; with whether it is a place and may be written.
    pub(super) fn infer_index_target(&mut self, target: &Expr) -> (ExpressionId, bool, bool) {
        if let Some(inferred) = self
            .preinferred_places
            .remove(&(target as *const Expr as usize))
        {
            return inferred;
        }
        let is_place = self.is_place(target);
        let (id, writable) = if is_place {
            self.infer_place(target)
        } else {
            (self.with_consuming(true, |c| c.infer_expr(target)), true)
        };
        (id, is_place, writable)
    }

    /// `indexed[indices] = value`, or `indexed[indices] op= value` with `compound`, through the
    /// indexed type's `set` operator when it has one (D10): a call statement of `set` with the
    /// indices and the value, or for a compound assignment a block evaluating each index once
    /// into a local, then calling `set` with `get`'s element combined with the value. Returns
    /// `None`, leaving the target inferred for the assignment's own checks, when the type has
    /// no `set`.
    pub(super) fn check_indexed_set(
        &mut self,
        indexed: &Expr,
        indices: &[Expr],
        compound: Option<BinaryOp>,
        value: &Expr,
        span: Span,
    ) -> Option<prism_typed_tree::StatementId> {
        let (receiver, is_place, writable) = self.infer_index_target(indexed);
        let receiver_ty = self.ty(receiver);
        let has_set = !matches!(
            super::peel_indirection(&receiver_ty),
            Type::List(_) | Type::String | Type::Error
        ) && {
            let mut layers = Vec::new();
            !matches!(
                self.find_extension_through_indirection(receiver_ty.clone(), "set", &mut layers)
                    .0,
                ExtensionLookup::None | ExtensionLookup::MissingContext(_)
            )
        };
        if !has_set {
            self.preinferred_places.insert(
                indexed as *const Expr as usize,
                (receiver, is_place, writable),
            );
            return None;
        }
        let as_argument = |value: Expr| CallArg {
            name: None,
            span: value.span,
            value,
            is_trailing_lambda: false,
            is_spread: false,
        };
        let Some(operator) = compound else {
            let mut args: Vec<CallArg> = indices.iter().cloned().map(as_argument).collect();
            args.push(as_argument(value.clone()));
            let call = self.call_set(indexed, (receiver, is_place, writable), &args, span);
            let kind = prism_typed_tree::StatementKind::Expression { value: call };
            return Some(self.builder.add_statement(kind, span));
        };
        // The indexed value is read twice, by `get` and `set`: only a variable or a field path
        // reads the same value twice.
        if !is_variable_path(indexed) {
            self.errors.push(TypeError::ImmutabilityViolation {
                message: format!(
                    "cannot assign through '{}' with '{}=': the value indexed is computed, not a variable or a field",
                    indexed,
                    operator.as_str()
                ),
                span,
                help: Some("hold it in a 'var' first".to_string()),
            });
            let mut children = vec![receiver];
            for index in indices {
                children.push(self.infer_operand(index));
            }
            children.push(self.infer_operand(value));
            let error = self.error_node(children, span);
            let kind = prism_typed_tree::StatementKind::Expression { value: error };
            return Some(self.builder.add_statement(kind, span));
        }
        // Each index is evaluated once, into a local passed to `get` and `set`.
        self.env.enter_scope();
        let mut statements = Vec::new();
        let mut names = Vec::new();
        for index in indices {
            self.synthesized_names += 1;
            let name = format!("index#{}", self.synthesized_names);
            let local = prism_ast::statement::Statement {
                kind: prism_ast::statement::StatementKind::Local {
                    is_var: false,
                    name: name.clone(),
                    ty: None,
                    initializer: index.clone(),
                },
                span: index.span,
            };
            statements.push(self.check_statement(&local));
            names.push(Expr::new(ExprKind::Ident(name), index.span));
        }
        // An index passed by value to both functions must be `Copy`.
        for (index, name) in indices.iter().zip(&names) {
            let ExprKind::Ident(local) = &name.kind else {
                continue;
            };
            let ty = match self.env.lookup(local).map(|symbol| &symbol.kind) {
                Some(SymbolKind::Variable { ty, .. }) => ty.clone(),
                _ => continue,
            };
            if ty != Type::Error && !self.is_copy(&ty) && !self.set_index_is_borrowed(&receiver_ty)
            {
                self.errors.push(TypeError::CannotMoveOut {
                    source: format!(
                        "the index '{}', which '{}=' passes to both 'get' and 'set'",
                        index,
                        operator.as_str()
                    ),
                    ty,
                    borrow: None,
                    clone: None,
                    destructure: None,
                    unbox: None,
                    remove: None,
                    captured: false,
                    span: index.span,
                });
            }
        }
        let element = Expr::new(
            ExprKind::Index {
                target: Box::new(indexed.clone()),
                indices: names.clone(),
            },
            span,
        );
        let combined = Expr::new(
            ExprKind::Binary {
                op: operator,
                left: Box::new(element),
                right: Box::new(value.clone()),
            },
            span,
        );
        let mut args: Vec<CallArg> = names.into_iter().map(as_argument).collect();
        args.push(as_argument(combined));
        let call = self.call_set(indexed, (receiver, is_place, writable), &args, span);
        self.env.exit_scope();
        let kind = ExpressionKind::Block {
            statements,
            tail: Some(call),
            close_brace: span,
        };
        let ty = self.ty(call);
        let block = self.add(kind, ty, span);
        let kind = prism_typed_tree::StatementKind::Expression { value: block };
        Some(self.builder.add_statement(kind, span))
    }

    /// Whether every index parameter of the `set` of `receiver_ty` is a borrow, so an index
    /// local is lent rather than moved.
    fn set_index_is_borrowed(&self, receiver_ty: &Type) -> bool {
        let mut layers = Vec::new();
        let ExtensionLookup::Found { extension, .. } = self
            .find_extension_through_indirection(receiver_ty.clone(), "set", &mut layers)
            .0
        else {
            return false;
        };
        let SymbolKind::Function {
            fn_type: Type::Fn { params, .. },
            ..
        } = &extension.symbol.kind
        else {
            return false;
        };
        params
            .split_last()
            .is_some_and(|(_, indices)| indices.iter().all(|p| matches!(p, Type::Reference { .. })))
    }

    /// A call of the `set` operator of the indexed value `receiver` with `args`, marked as an
    /// indexed assignment; a missing or unmarked `set` is `E4002`.
    fn call_set(
        &mut self,
        indexed: &Expr,
        receiver: (ExpressionId, bool, bool),
        args: &[CallArg],
        span: Span,
    ) -> ExpressionId {
        let (node, is_place, writable) = receiver;
        let called = self.call_operator(OperatorUse {
            operator: SpelledOperator::IndexAssign,
            name: "set",
            target: indexed,
            receiver: node,
            is_place,
            writable,
            args,
            span,
            expected: None,
        });
        match called {
            Ok(call) => call,
            Err(miss) => {
                let ty = super::peel_indirection(&self.ty(node)).clone();
                self.report_missing_operator(
                    miss,
                    format!("'{}' cannot be assigned by index", ty),
                    &ty,
                    "set",
                    format!("operator mut fun {}.set(...)", ty),
                    format!("{}?.set(...)", indexed),
                    span,
                );
                self.give_up_call(node, args, span)
            }
        }
    }
}

/// Whether `expr` is a variable or a field path over one, which reads the same value each time.
fn is_variable_path(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::Ident(_) | ExprKind::This => true,
        ExprKind::MemberAccess {
            target,
            is_safe_nav: false,
            ..
        } => is_variable_path(target),
        _ => false,
    }
}

impl TypeChecker {
    /// The `operator` extension `name` (a table name) of `ty` or of what it reaches through its
    /// `&`, `&mut` and `Heap<T>` layers, with its own inferred type refreshed, and the type it
    /// was found on; or why there is none.
    fn operator_extension(
        &mut self,
        ty: &Type,
        name: &str,
        span: Span,
    ) -> Result<(Box<super::Extension>, Type), OperatorMiss> {
        let mut layers = Vec::new();
        let (lookup, reached) =
            self.find_extension_through_indirection(ty.clone(), name, &mut layers);
        match lookup {
            ExtensionLookup::Found { extension, .. } if !extension.is_operator => {
                Err(OperatorMiss::Unmarked)
            }
            ExtensionLookup::Found { extension, own } => {
                let extension = if own {
                    let key = super::InferenceKey::Extension {
                        receiver: ReceiverKey::of(&reached),
                        name: name.to_string(),
                        context: extension.context.clone(),
                    };
                    self.ensure_key_inferred(&key, span);
                    match self.find_extension(&reached, name) {
                        ExtensionLookup::Found { extension, .. } => extension,
                        _ => extension,
                    }
                } else {
                    extension
                };
                Ok((extension, reached))
            }
            _ => Err(OperatorMiss::Missing),
        }
    }

    /// The protocol call of the extension `extension` found on `reached`: its path, parameters
    /// (`this` borrowed by its mode first), type arguments solved from the receiver, and its
    /// result.
    fn protocol_call(
        &self,
        extension: &super::Extension,
        name: &str,
        reached: &Type,
    ) -> Option<(prism_typed_tree::ProtocolCall, Type)> {
        let SymbolKind::Function {
            fn_type: Type::Fn { params, ret, .. },
            param_names,
            type_parameters,
            ..
        } = &extension.symbol.kind
        else {
            return None;
        };
        let type_arguments = if type_parameters.is_empty() {
            Vec::new()
        } else {
            let mut unknowns = crate::inference::Unknowns::new(type_parameters.len());
            unknowns.match_exact(&extension.receiver, reached);
            unknowns.partial()
        };
        let this_ty = Type::Reference {
            mutable: extension.mode == ReceiverMode::Mutable,
            inner: Box::new(extension.receiver.clone()),
        };
        let mut parameters = vec![(Some(super::THIS.to_string()), this_ty)];
        parameters.extend(super::parameter_list(param_names, params));
        let result = ret.substitute(&type_arguments);
        Some((
            prism_typed_tree::ProtocolCall {
                path: extension.path(name),
                parameters,
                type_arguments,
            },
            result,
        ))
    }

    /// The iteration of a `for` loop over `peeled`, an iterable of a type other than `Range`
    /// and `List` read in place (`iterable_ty` before its layers were peeled), through the
    /// cursor protocol (D11): its `iterator`, `hasNext` and `next` operators (the `mut` `next`
    /// for a `&mut` iterable), whose cursor types must agree. The iterable is borrowed for the
    /// loop. A missing or mismatched part is `E4002`, a `&mut` iterable without a `mut` `next`
    /// `E4003`. Returns `None` when the type has no `iterator`, which the caller reports.
    pub(super) fn cursor_iteration(
        &mut self,
        iterable: &Expr,
        peeled: ExpressionId,
        iterable_ty: &Type,
    ) -> Option<(ExpressionId, prism_typed_tree::ForIteration, Type)> {
        let collection = self.ty(peeled);
        let span = iterable.span;
        let (iterator, reached) = match self.operator_extension(&collection, "iterator", span) {
            Ok(found) => found,
            Err(OperatorMiss::Unmarked) => {
                self.report_missing_operator(
                    OperatorMiss::Unmarked,
                    format!("'{}' cannot be iterated by a 'for' loop", collection),
                    &collection,
                    "iterator",
                    String::new(),
                    String::new(),
                    span,
                );
                return Some((
                    self.wrap_error(peeled),
                    prism_typed_tree::ForIteration::Range,
                    Type::Error,
                ));
            }
            Err(_) => return None,
        };
        let mut mutable = matches!(iterable_ty, Type::Reference { mutable: true, .. })
            && super::contents_writable(iterable_ty, false);
        // A `&mut` value iterates shared when the type has no `mut` `next`, unless the loop
        // asks for mutable iteration with a written `&mut`.
        let written_mutable = matches!(
            iterable.kind,
            ExprKind::Unary {
                op: UnaryOp::BorrowMut,
                ..
            }
        );
        if mutable
            && !written_mutable
            && self
                .operator_extension(&collection, &mutable_accessor_name("next"), span)
                .is_err()
        {
            mutable = false;
        }
        let (iterator_call, cursor) = self.protocol_call(&iterator, "iterator", &reached)?;
        let cursor_ref = |mutable: bool| Type::Reference {
            mutable,
            inner: Box::new(cursor.clone()),
        };
        let has_next = self.operator_extension(&collection, "hasNext", span).ok();
        let next_name = if mutable {
            mutable_accessor_name("next")
        } else {
            "next".to_string()
        };
        let next = self.operator_extension(&collection, &next_name, span).ok();
        let has_next = has_next
            .and_then(|(extension, reached)| self.protocol_call(&extension, "hasNext", &reached));
        let next =
            next.and_then(|(extension, reached)| self.protocol_call(&extension, "next", &reached));
        let next_text = |element: &Option<Type>| {
            format!(
                "operator {}fun {}.next(cursor: &mut {}){}",
                if mutable { "mut " } else { "" },
                collection,
                cursor,
                element
                    .as_ref()
                    .map_or(String::new(), |element| format!(": {}", element))
            )
        };
        let has_next_agrees = has_next
            .as_ref()
            .is_some_and(|(call, _)| call.parameter_type(1) == Some(cursor_ref(false)));
        let next_agrees = next
            .as_ref()
            .is_some_and(|(call, _)| call.parameter_type(1) == Some(cursor_ref(true)));
        if mutable && next.is_none() {
            self.errors.push(TypeError::ImmutabilityViolation {
                message: format!(
                    "cannot iterate '{}' mutably: '{}' has no mutable 'next' operator",
                    iterable, collection
                ),
                span,
                help: Some(format!(
                    "declare '{}' to iterate it with 'for (x in &mut ...)'",
                    next_text(&Some(Type::Reference {
                        mutable: true,
                        inner: Box::new(Type::Opaque("E".to_string())),
                    }))
                )),
            });
            return Some((
                self.wrap_error(peeled),
                prism_typed_tree::ForIteration::Range,
                Type::Error,
            ));
        }
        if !has_next_agrees || !next_agrees {
            let mut needs = Vec::new();
            if !has_next_agrees {
                needs.push(format!(
                    "'operator fun {}.hasNext(cursor: &{}): Bool'",
                    collection, cursor
                ));
            }
            if !next_agrees {
                needs.push(format!("'{}'", next_text(&None)));
            }
            self.errors.push(TypeError::MissingOperatorFunction {
                message: format!(
                    "iterating a '{}' needs {}, for the cursor '{}' its 'iterator' returns",
                    collection,
                    needs.join(" and "),
                    cursor
                ),
                receiver: collection.clone(),
                function: if has_next_agrees { "next" } else { "hasNext" }.to_string(),
                help: Some(
                    "a 'for' loop calls 'iterator()' once, then 'hasNext(&cursor)' before each iteration and 'next(&mut cursor)' for each element".to_string(),
                ),
                span,
                import_suggestions: Vec::new(),
            });
            return Some((
                self.wrap_error(peeled),
                prism_typed_tree::ForIteration::Range,
                Type::Error,
            ));
        }
        let (has_next_call, _) = has_next?;
        let (next_call, element) = next?;
        let borrow = self.borrow(mutable, BorrowOrigin::Iteration, peeled);
        let protocol = prism_typed_tree::CursorProtocol {
            iterator: iterator_call,
            has_next: has_next_call,
            next: next_call,
            cursor,
            element: element.clone(),
            mutable,
        };
        Some((
            borrow,
            prism_typed_tree::ForIteration::Cursor(Box::new(protocol)),
            element,
        ))
    }
}

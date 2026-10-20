//! Operator functions in the type checker (D3, D5, D6, D9 and D11 of `operator-conventions`).
//!
//! An `operator` function is an extension whose name is one of Kotlin's conventions; operator
//! syntax calls it where the built-in operator does not apply. This module holds the table of
//! conventions, the checks of an `operator` declaration against its convention's shape
//! (`E4035`), the rejection of operator functions a built-in operator always shadows (`E4010`),
//! and the names under which the `mut` accessors (`get`, `next`) are kept beside the shared
//! ones. Each check is a fixed amount of work per declaration.

use super::{ReceiverKey, TypeChecker};
use crate::error::TypeError;
use crate::scope::SymbolKind;
use crate::types::Type;
use prism_ast::decl::{FunctionBody, FunctionDecl, ReceiverMode};
use prism_ast::Visibility;

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
const CONVENTION_NAMES: &str = "plus, minus, times, div, rem, unaryMinus, not, compareTo, equals, contains, get, set, invoke, iterator, hasNext, next";

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
        let operand = |ty: &Type| super::peel_references(ty).clone();
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

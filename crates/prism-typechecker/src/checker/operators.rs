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
    Argument, BuiltinFunction, Call, Callee, ExpressionId, ExpressionKind, ParameterKind,
    SpelledOperator, UnaryOperator,
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
            OperatorMiss::Missing => format!("declare '{}'", declaration),
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
/// when no operator function serves the use: the scalars, `String`, and the types that stand
/// for none (errors, `Nothing`, type parameters, unresolved names).
pub(super) fn has_builtin_operators(ty: &Type) -> bool {
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
                if let Some(TypeError::CannotMoveOut {
                    source,
                    borrow,
                    clone,
                    destructure,
                    remove,
                    ..
                }) = self.errors.last_mut()
                {
                    *source = format!(
                        "{}, which the consuming '{}' would take for '{}='",
                        source,
                        name,
                        operator.as_str()
                    );
                    (*borrow, *clone, *destructure, *remove) = (None, None, None, None);
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
                    OperatorMiss::Missing => String::new(),
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

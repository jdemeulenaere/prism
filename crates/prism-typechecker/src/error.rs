//! Semantic analysis and type checker diagnostic error representations.

use crate::types::Type;
use prism_ast::{Span, TypeMarker};
use std::fmt;

/// Semantic and type checking error categories (`E4001` - `E4025`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeError {
    /// E4001: Identifier not found in current lexical scope or module environment.
    UnresolvedSymbol { name: String, span: Span },

    /// E4001: A bare name in a pattern that resolves to nothing. A bare name compares with a
    /// value; a binding needs `val` (D1 of `payload-patterns`).
    UnboundPatternName { name: String, span: Span },

    /// E4001: `this` used outside an extension function, where nothing binds it.
    ThisOutsideExtension { span: Span },

    /// E4001: An output built-in (`print` or `println`) named without being called, which only
    /// a call can do (D15 of `codegen-core`).
    BuiltinFunctionAsValue { name: String, span: Span },

    /// E4002: Expression type is not assignable to the expected type.
    TypeMismatch {
        expected: Type,
        found: Type,
        span: Span,
        /// A fix to suggest, such as borrowing the value.
        help: Option<String>,
    },

    /// E4002: An operator that does not apply to its operands' types: the operator of a
    /// compound assignment (`s -= "a"`), with the target's type on the left (D7 of
    /// `compound-assignment`), an ordering (`false < true`), or any operator but a `null` test
    /// applied to a value of a type parameter (`a == b` for `a: T`, D5 of `generics`). `operator` is written as in the
    /// source (`-=`, `<`).
    OperatorMismatch {
        operator: String,
        left: Type,
        right: Type,
        span: Span,
        /// The operand types the operator accepts.
        help: Option<String>,
    },

    /// E4002: A struct or enum name used where a value is expected (`val u = User`).
    TypeNameAsValue { name: String, span: Span },

    /// E4003: Reassigning to an immutable `val` binding or mutating a `val` struct instance/field.
    ImmutabilityViolation {
        message: String,
        span: Span,
        /// A fix to suggest instead of the generic one (declaring a `var`).
        help: Option<String>,
    },

    /// E4004: Function invocation argument count or parameter type incompatibility.
    CallSignatureMismatch { message: String, span: Span },

    /// E4004: An argument of `print` or `println` of a type it cannot print (D15 of
    /// `codegen-core`).
    UnprintableArgument {
        function: String,
        ty: Type,
        span: Span,
    },

    /// E4004: An argument's type is not assignable to its parameter's type (including struct
    /// constructor fields and enum payloads).
    ArgumentTypeMismatch {
        expected: Type,
        found: Type,
        span: Span,
        /// A fix to suggest, such as borrowing the value.
        help: Option<String>,
    },

    /// E4005: Returned expression type is incompatible with declared function return type.
    ReturnTypeMismatch {
        expected: Type,
        found: Type,
        span: Span,
        /// A fix to suggest, such as borrowing the value.
        help: Option<String>,
    },

    /// E4006: Struct field not found or field access attempted on a non-struct type, or a method
    /// call `x.name()` with neither a field nor a visible extension function `name`.
    UnknownField {
        target_type: Type,
        field_name: String,
        span: Span,
        /// A fix to suggest, such as calling an extension with `?.` or `()`.
        help: Option<String>,
    },

    /// E4007: Attempting to access a `package`-visible symbol from an external package.
    VisibilityViolation {
        symbol: String,
        defining_package: String,
        accessing_package: String,
        span: Span,
    },

    /// E4008: Invoking a composable function (`compose fun`) outside a composable context.
    InvalidComposeContext { callee: String, span: Span },

    /// E4009: Non-exhaustive `when` expression in value position.
    NonExhaustiveWhen {
        message: String,
        /// A value no branch covers, as a pattern (D11 of `payload-patterns`); `None` for a
        /// `when` without a subject.
        witness: Option<String>,
        span: Span,
    },

    /// E4010: Declaring a duplicate symbol within the identical lexical scope.
    DuplicateSymbol { name: String, span: Span },
    /// A top-level declaration named like a built-in that a call always reaches (`Heap`), so it
    /// could never be used (D2 of `heap-boxes`).
    BuiltinNameDeclared { name: String, span: Span },

    /// E4010: An extension function that could never be called, because a member of its
    /// receiver type of the same name always takes precedence (a field, or the built-in
    /// `clone()` of a `Clone` type).
    UncallableExtension { message: String, span: Span },

    /// E4010: A type parameter declared twice in one list, or named like a built-in type the
    /// lowering fixes (D12 of `generics`).
    InvalidTypeParameterName { message: String, span: Span },

    /// E4011: A `public` or `package` function omits its explicit return type annotation.
    MissingReturnType { function_name: String, span: Span },

    /// E4012: A name used by the file is exported by more than one of its imports.
    AmbiguousImport {
        name: String,
        modules: Vec<String>,
        span: Span,
    },

    /// E4013: A type cannot be inferred and must be written: a closure parameter with no expected
    /// function type, a declaration whose inferred type depends on itself, or a type argument
    /// of a generic call (D8 of `generics`). `help` replaces the generic fix when set.
    CannotInferType {
        message: String,
        span: Span,
        help: Option<String>,
    },

    /// E4014: A member of a struct or enum (a field, a variant, its constructor) is used, but the
    /// module defining the type is not a direct import of the file.
    MemberOfUnimportedType {
        type_name: String,
        module: String,
        span: Span,
    },

    /// E4015: A `break` or `continue` has no enclosing loop in its function or closure body.
    BreakOutsideLoop { keyword: &'static str, span: Span },

    /// A `: Copy` or `: Clone` declaration has a field or payload whose type is not `Copy` (or
    /// not `Clone`). `member` describes it: "field 'name'" or "payload 1 of variant 'Text'".
    InvalidMarkerDeclaration {
        marker: TypeMarker,
        type_name: String,
        member: String,
        member_type: Type,
        /// Whether the member's type is `Clone`, so `: Clone` would be accepted instead of `: Copy`.
        member_is_clone: bool,
        span: Span,
    },

    /// `clone()` is called on a value whose type is not `Clone`.
    NotCloneable { ty: Type, span: Span },

    /// E4017: A non-`Copy` value is moved out of a field, a safe navigation, an index, a top-level
    /// `val` or a borrow. `source` describes it ("field 'name' of 'u'", "borrow 'u'"); `borrow`
    /// and `clone` are the fixes to suggest, `borrow` only when the source is not already a borrow
    /// and `clone` only when the type is `Clone`.
    CannotMoveOut {
        source: String,
        ty: Type,
        borrow: Option<String>,
        clone: Option<String>,
        /// For a field of a whole owned local, the destructuring declaration that takes it out
        /// (`val (name) = u`, D7 of `payload-patterns`).
        destructure: Option<String>,
        /// For the contents of a box, the call that moves them out (`h.unbox()`, D6 of
        /// `heap-boxes`).
        unbox: Option<String>,
        span: Span,
    },

    /// E4017: A pattern or destructuring declaration would take apart a value that is not a whole
    /// owned value (D6 of `payload-patterns`). `source` describes it, `help` the fix.
    CannotTakeApart {
        source: String,
        help: String,
        span: Span,
    },

    /// E4017: A binding that takes its part by value is moved in a guard, which may be false
    /// (D9 of `payload-patterns`).
    MoveInGuard { name: String, span: Span },

    /// E4019: `*` writes through a value that is not a reference (`*x = 1` for `x: Int`).
    NotAReference { ty: Type, span: Span },

    /// E4020: A generic type or call is given the wrong number of type arguments: a generic type
    /// written without them or with too many, type arguments for a callee or a type parameter
    /// that takes none. `owner` describes what takes them ("type 'Box'", "function 'id'").
    WrongTypeArgumentCount {
        owner: String,
        expected: usize,
        found: usize,
        span: Span,
    },

    /// E4021: A type argument does not satisfy the bound of its type parameter (`String` for
    /// `T : Copy`). `owner` describes the generic declaration ("function 'dup'").
    UnsatisfiedBound {
        argument: Type,
        parameter: String,
        bound: TypeMarker,
        owner: String,
        span: Span,
    },

    /// E4022: A type argument, written or inferred, contains a reference outside a function type
    /// (D7 of `generics`). `owner` describes the generic declaration or type.
    BorrowInTypeArgument {
        argument: Type,
        owner: String,
        span: Span,
    },

    /// E4023: A struct or enum contains itself by value, so its values would have no finite
    /// size (D20 of `codegen-aggregates`). `path` names the member types leading back to it.
    InfinitelySizedType {
        type_name: String,
        path: String,
        span: Span,
    },

    /// E4024: A pattern that does not fit its subject's definition (D18 of `payload-patterns`):
    /// the wrong number of sub-patterns, a named sub-pattern of a variant, a field named twice,
    /// a positional sub-pattern after a named one, arguments of a payload-free variant, a
    /// constructor pattern naming a function, or a destructuring of something other than a
    /// struct. `help` suggests a fix when one applies.
    InvalidPattern {
        message: String,
        span: Span,
        help: Option<String>,
    },

    /// E4025: The patterns of one branch bind different names, or one name with different types
    /// or binding modes (D10 of `payload-patterns`). `message` names the binding.
    InconsistentAlternativeBindings { message: String, span: Span },

    /// E4026: A string template part of type `ty` has no text form (D8 of `string-templates`),
    /// for `reason`. `help` names the fixes, one per line of help.
    NoTextForm {
        ty: Type,
        reason: NoTextFormReason,
        help: Vec<String>,
        span: Span,
    },

    /// E4027: A generic function reaches a call of itself, or a generic struct or enum reaches
    /// itself through its fields or payloads, with a type argument that nests its own type
    /// parameter in another type, so it would need infinitely many instances (D8 and D9 of
    /// `monomorphization`). `message` names the growing argument, `cycle` the generics on the
    /// cycle (`'ping' -> 'pong' -> 'ping'`), and `is_type` whether they are types.
    PolymorphicRecursion {
        message: String,
        cycle: String,
        is_type: bool,
        span: Span,
    },
}

/// Why a string template part has no text form (D8 of `string-templates`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoTextFormReason {
    /// The part is nullable.
    Nullable,
    /// The part's type has no `toString()`: `Unit`, a function, a type parameter, or a struct or
    /// enum without a visible `toString()` member.
    NoToString,
    /// The part's `toString()` returns another type.
    ToStringReturns(Type),
    /// The part's `toString()` needs arguments.
    ToStringNeedsArguments,
    /// The part's `toString()` takes a `mut` (`true`) or `consuming` (`false`) receiver.
    ToStringReceiver { mutable: bool },
}

impl TypeError {
    /// Returns the diagnostic error code (e.g. "E4001").
    pub fn code(&self) -> &'static str {
        match self {
            TypeError::UnresolvedSymbol { .. } => "E4001",
            TypeError::UnboundPatternName { .. } => "E4001",
            TypeError::ThisOutsideExtension { .. } => "E4001",
            TypeError::BuiltinFunctionAsValue { .. } => "E4001",
            TypeError::TypeMismatch { .. } => "E4002",
            TypeError::OperatorMismatch { .. } => "E4002",
            TypeError::TypeNameAsValue { .. } => "E4002",
            TypeError::ImmutabilityViolation { .. } => "E4003",
            TypeError::CallSignatureMismatch { .. } => "E4004",
            TypeError::UnprintableArgument { .. } => "E4004",
            TypeError::ArgumentTypeMismatch { .. } => "E4004",
            TypeError::ReturnTypeMismatch { .. } => "E4005",
            TypeError::UnknownField { .. } => "E4006",
            TypeError::VisibilityViolation { .. } => "E4007",
            TypeError::InvalidComposeContext { .. } => "E4008",
            TypeError::NonExhaustiveWhen { .. } => "E4009",
            TypeError::DuplicateSymbol { .. } => "E4010",
            TypeError::BuiltinNameDeclared { .. } => "E4010",
            TypeError::UncallableExtension { .. } => "E4010",
            TypeError::InvalidTypeParameterName { .. } => "E4010",
            TypeError::MissingReturnType { .. } => "E4011",
            TypeError::AmbiguousImport { .. } => "E4012",
            TypeError::CannotInferType { .. } => "E4013",
            TypeError::MemberOfUnimportedType { .. } => "E4014",
            TypeError::BreakOutsideLoop { .. } => "E4015",
            TypeError::InvalidMarkerDeclaration { .. } => "E4016",
            TypeError::CannotMoveOut { .. } => "E4017",
            TypeError::CannotTakeApart { .. } => "E4017",
            TypeError::MoveInGuard { .. } => "E4017",
            TypeError::NotCloneable { .. } => "E4018",
            TypeError::NotAReference { .. } => "E4019",
            TypeError::WrongTypeArgumentCount { .. } => "E4020",
            TypeError::UnsatisfiedBound { .. } => "E4021",
            TypeError::BorrowInTypeArgument { .. } => "E4022",
            TypeError::InfinitelySizedType { .. } => "E4023",
            TypeError::InvalidPattern { .. } => "E4024",
            TypeError::InconsistentAlternativeBindings { .. } => "E4025",
            TypeError::NoTextForm { .. } => "E4026",
            TypeError::PolymorphicRecursion { .. } => "E4027",
        }
    }

    /// Primary source span associated with this error.
    pub fn span(&self) -> Span {
        match self {
            TypeError::UnresolvedSymbol { span, .. } => *span,
            TypeError::UnboundPatternName { span, .. } => *span,
            TypeError::CannotTakeApart { span, .. } => *span,
            TypeError::MoveInGuard { span, .. } => *span,
            TypeError::ThisOutsideExtension { span } => *span,
            TypeError::TypeMismatch { span, .. } => *span,
            TypeError::OperatorMismatch { span, .. } => *span,
            TypeError::TypeNameAsValue { span, .. } => *span,
            TypeError::ImmutabilityViolation { span, .. } => *span,
            TypeError::CallSignatureMismatch { span, .. } => *span,
            TypeError::UnprintableArgument { span, .. } => *span,
            TypeError::BuiltinFunctionAsValue { span, .. } => *span,
            TypeError::ArgumentTypeMismatch { span, .. } => *span,
            TypeError::ReturnTypeMismatch { span, .. } => *span,
            TypeError::UnknownField { span, .. } => *span,
            TypeError::VisibilityViolation { span, .. } => *span,
            TypeError::InvalidComposeContext { span, .. } => *span,
            TypeError::NonExhaustiveWhen { span, .. } => *span,
            TypeError::DuplicateSymbol { span, .. } => *span,
            TypeError::BuiltinNameDeclared { span, .. } => *span,
            TypeError::UncallableExtension { span, .. } => *span,
            TypeError::InvalidTypeParameterName { span, .. } => *span,
            TypeError::MissingReturnType { span, .. } => *span,
            TypeError::AmbiguousImport { span, .. } => *span,
            TypeError::CannotInferType { span, .. } => *span,
            TypeError::MemberOfUnimportedType { span, .. } => *span,
            TypeError::BreakOutsideLoop { span, .. } => *span,
            TypeError::InvalidMarkerDeclaration { span, .. } => *span,
            TypeError::NotCloneable { span, .. } => *span,
            TypeError::CannotMoveOut { span, .. } => *span,
            TypeError::NotAReference { span, .. } => *span,
            TypeError::WrongTypeArgumentCount { span, .. } => *span,
            TypeError::UnsatisfiedBound { span, .. } => *span,
            TypeError::BorrowInTypeArgument { span, .. } => *span,
            TypeError::InfinitelySizedType { span, .. } => *span,
            TypeError::InvalidPattern { span, .. } => *span,
            TypeError::InconsistentAlternativeBindings { span, .. } => *span,
            TypeError::NoTextForm { span, .. } => *span,
            TypeError::PolymorphicRecursion { span, .. } => *span,
        }
    }

    /// User-facing descriptive error message.
    pub fn message(&self) -> String {
        match self {
            TypeError::UnresolvedSymbol { name, .. } => {
                format!("unresolved symbol '{}'", name)
            }
            TypeError::UnboundPatternName { name, .. } => {
                format!(
                    "unresolved symbol '{}': a bare name in a pattern is a value compared with the matched part",
                    name
                )
            }
            TypeError::CannotTakeApart { source, .. } => format!(
                "cannot take apart {}: a pattern that moves a part takes the whole value, which must be an owned local or a temporary",
                source
            ),
            TypeError::MoveInGuard { name, .. } => format!(
                "cannot move binding '{}' in a guard: the guard may be false, and the next branches must still find the subject whole",
                name
            ),
            TypeError::ThisOutsideExtension { .. } => {
                "'this' is only available in an extension function ('fun T.name()')".to_string()
            }
            TypeError::TypeMismatch {
                expected, found, ..
            } => {
                let (expected, found) = Type::render_pair(expected, found);
                format!("type mismatch: expected '{}', found '{}'", expected, found)
            }
            TypeError::OperatorMismatch {
                operator,
                left,
                right,
                ..
            } => {
                let (left, right) = Type::render_pair(left, right);
                format!(
                    "operator '{}' cannot be applied to '{}' and '{}'",
                    operator, left, right
                )
            }
            TypeError::ArgumentTypeMismatch {
                expected, found, ..
            } => {
                let (expected, found) = Type::render_pair(expected, found);
                format!(
                    "argument type mismatch: expected '{}', found '{}'",
                    expected, found
                )
            }
            TypeError::TypeNameAsValue { name, .. } => {
                format!(
                    "'{}' is a type, not a value; construct it with '{}(...)' or name an enum variant",
                    name, name
                )
            }
            TypeError::ImmutabilityViolation { message, .. } => message.clone(),
            TypeError::CallSignatureMismatch { message, .. } => message.clone(),
            TypeError::UnprintableArgument { function, ty, .. } => {
                format!("'{}' cannot print a value of type '{}'", function, ty)
            }
            TypeError::BuiltinFunctionAsValue { name, .. } => {
                format!("built-in function '{}' cannot be used as a value", name)
            }
            TypeError::ReturnTypeMismatch {
                expected, found, ..
            } => {
                let (expected, found) = Type::render_pair(expected, found);
                format!(
                    "return type mismatch: expected '{}', found '{}'",
                    expected, found
                )
            }
            TypeError::UnknownField {
                target_type,
                field_name,
                ..
            } => {
                format!("unknown field '{}' on type '{}'", field_name, target_type)
            }
            TypeError::VisibilityViolation {
                symbol,
                defining_package,
                accessing_package,
                ..
            } => {
                format!(
                    "symbol '{}' in package '{}' has package visibility and cannot be accessed from package '{}'",
                    symbol, defining_package, accessing_package
                )
            }
            TypeError::InvalidComposeContext { callee, .. } => {
                format!(
                    "composable function '{}' can only be invoked within a composable context (inside a 'compose fun' or composable closure)",
                    callee
                )
            }
            TypeError::NonExhaustiveWhen { message, .. } => message.clone(),
            TypeError::InvalidPattern { message, .. } => message.clone(),
            TypeError::InconsistentAlternativeBindings { message, .. } => message.clone(),
            TypeError::NoTextForm { ty, reason, .. } => match reason {
                NoTextFormReason::Nullable => {
                    format!("a nullable '{}' has no text form in a string template", ty)
                }
                NoTextFormReason::NoToString => {
                    format!("'{}' has no text form in a string template", ty)
                }
                NoTextFormReason::ToStringReturns(returned) => {
                    format!("'toString()' of '{}' returns '{}', not 'String'", ty, returned)
                }
                NoTextFormReason::ToStringNeedsArguments => {
                    format!("'toString' of '{}' needs arguments", ty)
                }
                NoTextFormReason::ToStringReceiver { mutable } => format!(
                    "a string template does not call the '{}' 'toString()' of '{}' implicitly",
                    if *mutable { "mut" } else { "consuming" },
                    ty
                ),
            },
            TypeError::UncallableExtension { message, .. } => message.clone(),
            TypeError::InvalidTypeParameterName { message, .. } => message.clone(),
            TypeError::BuiltinNameDeclared { name, .. } => {
                format!("a top-level declaration named '{}' would hide the built-in '{}'", name, name)
            }
            TypeError::DuplicateSymbol { name, .. } => {
                format!("duplicate symbol declaration '{}' in the same scope", name)
            }
            TypeError::MissingReturnType { function_name, .. } => {
                format!(
                    "exported function '{}' must declare an explicit return type",
                    function_name
                )
            }
            TypeError::AmbiguousImport { name, modules, .. } => {
                let modules: Vec<String> = modules.iter().map(|m| format!("'/{}'", m)).collect();
                format!(
                    "'{}' is exported by more than one import ({}); remove one of the imports or rename the symbol",
                    name,
                    modules.join(", ")
                )
            }
            TypeError::CannotInferType { message, .. } => message.clone(),
            TypeError::MemberOfUnimportedType {
                type_name, module, ..
            } => format!(
                "'{}' is defined in module '{}', which this file does not import; add 'import /{}' to use the members of '{}'",
                type_name, module, module, type_name
            ),
            TypeError::BreakOutsideLoop { keyword, .. } => format!(
                "'{}' is only allowed inside a loop, and cannot leave a closure",
                keyword
            ),
            TypeError::InvalidMarkerDeclaration {
                marker,
                type_name,
                member,
                member_type,
                ..
            } => format!(
                "'{}' cannot be '{}': its {} has type '{}', which is not '{}'",
                type_name,
                marker.as_str(),
                member,
                member_type,
                marker.as_str()
            ),
            TypeError::NotCloneable { ty, .. } => {
                format!("'{}' is not 'Clone', so it cannot be cloned", ty)
            }
            TypeError::CannotMoveOut {
                source,
                ty,
                unbox: Some(_),
                ..
            } => format!(
                "cannot move out of {}: its type '{}' is not 'Copy', so it cannot be copied out of the box",
                source, ty
            ),
            TypeError::CannotMoveOut {
                source,
                ty,
                borrow: None,
                ..
            } => format!(
                "cannot move out of {}: its type '{}' is not 'Copy', so it cannot be copied out of the borrow",
                source, ty
            ),
            TypeError::CannotMoveOut { source, ty, .. } => format!(
                "cannot move out of {}: its type '{}' is not 'Copy', and a move takes a whole value",
                source, ty
            ),
            TypeError::NotAReference { ty, .. } => format!(
                "cannot write through a value of type '{}': '*' needs a reference or a 'Heap'",
                ty
            ),
            TypeError::WrongTypeArgumentCount {
                owner,
                expected,
                found,
                ..
            } => format!(
                "{} takes {} type argument{}, but {} {} given",
                owner,
                expected,
                if *expected == 1 { "" } else { "s" },
                found,
                if *found == 1 { "was" } else { "were" }
            ),
            TypeError::UnsatisfiedBound {
                argument,
                parameter,
                bound,
                owner,
                ..
            } => format!(
                "type argument '{}' of {} does not satisfy the bound '{} : {}': '{}' is not '{}'",
                argument,
                owner,
                parameter,
                bound.as_str(),
                argument,
                bound.as_str()
            ),
            TypeError::BorrowInTypeArgument {
                argument, owner, ..
            } => format!(
                "type argument '{}' of {} contains a borrow, but type arguments must be borrow-free",
                argument, owner
            ),
            TypeError::InfinitelySizedType { type_name, .. } => format!(
                "'{}' has an infinite size: it contains itself by value",
                type_name
            ),
            TypeError::PolymorphicRecursion { message, .. } => {
                format!("polymorphic recursion: {}", message)
            }
        }
    }
}

impl fmt::Display for TypeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "error[{}]: {}", self.code(), self.message())
    }
}

impl std::error::Error for TypeError {}

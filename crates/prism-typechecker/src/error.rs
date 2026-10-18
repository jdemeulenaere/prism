//! Semantic analysis and type checker diagnostic error representations.

use crate::types::Type;
use prism_ast::{Span, TypeMarker};
use std::fmt;

/// Semantic and type checking error categories (`E4001` - `E4025`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeError {
    /// E4001: Identifier not found in current lexical scope or module environment.
    UnresolvedSymbol { name: String, span: Span },

    /// E4001: A bare name found only on an outer receiver, a field (`field`) or an extension
    /// of `receiver`, which the innermost lambda with a receiver hides (D4 of
    /// `receiver-lambdas`).
    OuterReceiverName {
        name: String,
        receiver: Type,
        field: bool,
        span: Span,
    },

    /// E4001: A bare name in a pattern that resolves to nothing. A bare name compares with a
    /// value; a binding needs `val` (D1 of `payload-patterns`).
    UnboundPatternName { name: String, span: Span },

    /// E4001: `this` used outside an extension function and every lambda with a receiver,
    /// where nothing binds it.
    ThisOutsideExtension { span: Span },

    /// E4001: `it` used where no closure binds it (D6 of `implicit-it`). `missing` says why the
    /// innermost enclosing closure has no implicit `it`.
    UnresolvedImplicitParameter {
        missing: MissingImplicitParameter,
        span: Span,
    },

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

    /// E4004: A closure declaring another number of parameters than its expected function type
    /// has, or none without a header where two or more are expected (D6 of `implicit-it`).
    ClosureParameterCount {
        /// The expected function type's parameter types.
        expected: Vec<Type>,
        /// How many parameters the closure declares.
        declared: usize,
        /// Whether the closure has a `->` header.
        has_header: bool,
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
        /// For an element of a list, the call that takes it out (`xs.removeAt(0)`, D7 of
        /// `collections`).
        remove: Option<String>,
        /// Whether the source is a capture of the closure being checked, which never gives away
        /// what it captures, whether it borrows or owns it (D15 of `escaping-closures`).
        captured: bool,
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

    /// E4026: A string template part, or with `printed` an argument of `print` or `println`,
    /// of type `ty` has no text form (D8 of `string-templates`, D9 of `standard-library`), for
    /// `reason`. `help` names the fixes, one per line of help.
    NoTextForm {
        ty: Type,
        reason: NoTextFormReason,
        help: Vec<String>,
        printed: bool,
        span: Span,
    },

    /// E4029: A declaration of the bundled library that breaks a rule of `external` or
    /// `intrinsic` declarations (D5 and D6 of `standard-library`): an external that is `public`,
    /// generic, an extension, `compose`, has defaults or returns a reference, or an intrinsic
    /// the catalog does not provide or provides with another signature.
    InvalidLibraryDeclaration {
        message: String,
        help: Option<String>,
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

    /// E4028: A closure that owns its captures captures a binding it cannot take by value: a
    /// `var`, or a non-`Copy` capture of the enclosing closure or earlier parameter read in a
    /// default value (D5 of `escaping-closures`). `span` is the capture inside the closure, and
    /// `reason` why the closure owns it.
    CannotCaptureByValue {
        name: String,
        refusal: CaptureRefusal,
        reason: prism_typed_tree::OwnershipReason,
        help: String,
        span: Span,
    },
}

/// Why a closure cannot take a binding by value (D5 of `escaping-closures`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureRefusal {
    /// The binding is a `var`, whose later writes the closure would not see.
    Variable,
    /// The binding is a capture of the enclosing closure, which may be called again.
    EnclosingClosure,
    /// The binding is an earlier parameter, which a default value only borrows.
    DefaultValueInput,
}

impl CaptureRefusal {
    /// The note explaining the refusal.
    pub fn note(self, name: &str) -> String {
        match self {
            CaptureRefusal::Variable => format!(
                "a closure that owns its captures would keep the value '{}' had when it was built, and not see later writes to the 'var'",
                name
            ),
            CaptureRefusal::EnclosingClosure => format!(
                "'{}' is captured by the enclosing closure, which may be called more than once, so it cannot give '{}' away",
                name, name
            ),
            CaptureRefusal::DefaultValueInput => format!(
                "a default value only borrows the earlier parameters, so it cannot give '{}' away",
                name
            ),
        }
    }
}

/// Why the innermost closure around a use of `it` has no implicit parameter (D6 of
/// `implicit-it`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MissingImplicitParameter {
    /// The use is in no closure.
    OutsideClosures,
    /// The closure has a header, declaring these parameters (none for `{ -> }`).
    DeclaredParameters(Vec<String>),
    /// The closure has no expected function type.
    NoExpectedType,
    /// The closure's expected function type has no parameters. It holds that function type when
    /// its return type is known, which the checker fills in only when it reports the error.
    NoExpectedParameters(Option<Type>),
}

impl MissingImplicitParameter {
    /// The help for an unbound `it`.
    pub fn help(&self) -> String {
        const WHERE: &str = "only a closure without a header, passed where a function of one parameter is expected, has an implicit 'it'";
        match self {
            MissingImplicitParameter::OutsideClosures => format!(
                "'it' names the implicit parameter of a closure; {}",
                WHERE
            ),
            MissingImplicitParameter::DeclaredParameters(names) if names.is_empty() => {
                "this closure declares that it takes no parameters ('{ -> ... }'), so it has no implicit 'it'".to_string()
            }
            MissingImplicitParameter::DeclaredParameters(names) => {
                let names: Vec<String> = names.iter().map(|n| format!("'{}'", n)).collect();
                format!(
                    "this closure declares its parameters ({}), so it has no implicit 'it'",
                    names.join(", ")
                )
            }
            MissingImplicitParameter::NoExpectedType => format!(
                "this closure has no expected function type, so it has no implicit 'it'; {}",
                WHERE
            ),
            MissingImplicitParameter::NoExpectedParameters(Some(function_type)) => format!(
                "this closure's type '{}' has no parameters, so it has no implicit 'it'",
                function_type
            ),
            MissingImplicitParameter::NoExpectedParameters(None) => {
                "this closure's expected function type has no parameters, so it has no implicit 'it'".to_string()
            }
        }
    }
}

/// The names `a`, `b`, ... of `count` closure parameters, for a suggested header.
fn suggested_parameters(count: usize) -> String {
    let names: Vec<String> = (0..count)
        .map(|index| match u8::try_from(index) {
            Ok(index) if index < 26 => char::from(b'a' + index).to_string(),
            _ => format!("p{}", index + 1),
        })
        .collect();
    names.join(", ")
}

/// The help for a closure whose parameter count does not fit its expected type (D6 of
/// `implicit-it`).
pub fn closure_parameter_count_help(expected: usize, has_header: bool) -> String {
    match (expected, has_header) {
        (0, _) => "remove the header: the closure's expected function type has no parameters"
            .to_string(),
        (1, _) => "declare the parameter, as in '{ x -> ... }', or remove the header to use the implicit 'it'".to_string(),
        (count, false) => format!(
            "declare them in a header, as in '{{ {} -> ... }}'; only a closure expecting one parameter has an implicit 'it'",
            suggested_parameters(count)
        ),
        (count, true) => format!(
            "declare {} parameters, as in '{{ {} -> ... }}'",
            count,
            suggested_parameters(count)
        ),
    }
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
            TypeError::OuterReceiverName { .. } => "E4001",
            TypeError::UnboundPatternName { .. } => "E4001",
            TypeError::ThisOutsideExtension { .. } => "E4001",
            TypeError::UnresolvedImplicitParameter { .. } => "E4001",
            TypeError::BuiltinFunctionAsValue { .. } => "E4001",
            TypeError::TypeMismatch { .. } => "E4002",
            TypeError::OperatorMismatch { .. } => "E4002",
            TypeError::TypeNameAsValue { .. } => "E4002",
            TypeError::ImmutabilityViolation { .. } => "E4003",
            TypeError::CallSignatureMismatch { .. } => "E4004",
            TypeError::ClosureParameterCount { .. } => "E4004",
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
            TypeError::InvalidLibraryDeclaration { .. } => "E4029",
            TypeError::PolymorphicRecursion { .. } => "E4027",
            TypeError::CannotCaptureByValue { .. } => "E4028",
        }
    }

    /// Primary source span associated with this error.
    pub fn span(&self) -> Span {
        match self {
            TypeError::UnresolvedSymbol { span, .. } => *span,
            TypeError::OuterReceiverName { span, .. } => *span,
            TypeError::UnboundPatternName { span, .. } => *span,
            TypeError::CannotTakeApart { span, .. } => *span,
            TypeError::MoveInGuard { span, .. } => *span,
            TypeError::ThisOutsideExtension { span } => *span,
            TypeError::UnresolvedImplicitParameter { span, .. } => *span,
            TypeError::TypeMismatch { span, .. } => *span,
            TypeError::OperatorMismatch { span, .. } => *span,
            TypeError::TypeNameAsValue { span, .. } => *span,
            TypeError::ImmutabilityViolation { span, .. } => *span,
            TypeError::CallSignatureMismatch { span, .. } => *span,
            TypeError::ClosureParameterCount { span, .. } => *span,
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
            TypeError::InvalidLibraryDeclaration { span, .. } => *span,
            TypeError::PolymorphicRecursion { span, .. } => *span,
            TypeError::CannotCaptureByValue { span, .. } => *span,
        }
    }

    /// User-facing descriptive error message.
    pub fn message(&self) -> String {
        match self {
            TypeError::UnresolvedSymbol { name, .. } => {
                format!("unresolved symbol '{}'", name)
            }
            TypeError::OuterReceiverName { name, .. } => format!(
                "unresolved symbol '{}': only the innermost receiver is implicit",
                name
            ),
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
                "'this' is only available in an extension function ('fun T.name()') or a lambda with a receiver".to_string()
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
            TypeError::UnresolvedImplicitParameter { .. } => "unresolved symbol 'it'".to_string(),
            TypeError::ClosureParameterCount {
                expected, declared, ..
            } => {
                let expected_text = match expected.len() {
                    0 => "no parameters".to_string(),
                    count => {
                        let types: Vec<String> =
                            expected.iter().map(|ty| format!("'{}'", ty)).collect();
                        format!(
                            "{} parameter{} ({})",
                            count,
                            if count == 1 { "" } else { "s" },
                            types.join(", ")
                        )
                    }
                };
                let declared_text = match declared {
                    0 => "declares none".to_string(),
                    count => format!("declares {}", count),
                };
                format!("closure expected {}, but {}", expected_text, declared_text)
            }
            TypeError::BuiltinFunctionAsValue { name, .. } if name == "List" => {
                "the built-in type 'List' cannot be used as a value or called".to_string()
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
            TypeError::NoTextForm {
                ty,
                reason,
                printed,
                ..
            } => {
                let place = if *printed {
                    "as a printed value"
                } else {
                    "in a string template"
                };
                match reason {
                    NoTextFormReason::Nullable => {
                        format!("a nullable '{}' has no text form {}", ty, place)
                    }
                    NoTextFormReason::NoToString => {
                        format!("'{}' has no text form {}", ty, place)
                    }
                    NoTextFormReason::ToStringReturns(returned) => {
                        format!("'toString()' of '{}' returns '{}', not 'String'", ty, returned)
                    }
                    NoTextFormReason::ToStringNeedsArguments => {
                        format!("'toString' of '{}' needs arguments", ty)
                    }
                    NoTextFormReason::ToStringReceiver { mutable } => format!(
                        "{} does not call the '{}' 'toString()' of '{}' implicitly",
                        if *printed {
                            "printing"
                        } else {
                            "a string template"
                        },
                        if *mutable { "mut" } else { "consuming" },
                        ty
                    ),
                }
            }
            TypeError::InvalidLibraryDeclaration { message, .. } => message.clone(),
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
                captured: true,
                ..
            } => format!(
                "cannot move out of {}: its type '{}' is not 'Copy', so it cannot be copied out of the closure",
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
            TypeError::CannotCaptureByValue {
                name,
                refusal: CaptureRefusal::Variable,
                ..
            } => format!("cannot capture the variable '{}' by value", name),
            TypeError::CannotCaptureByValue { name, .. } => {
                format!("cannot move '{}' into the closure", name)
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

impl TypeError {
    /// The help of [`TypeError::OuterReceiverName`]: where the name was found, and how to reach
    /// it. `None` for every other error.
    pub fn outer_receiver_help(&self) -> Option<String> {
        let TypeError::OuterReceiverName {
            name,
            receiver,
            field,
            ..
        } = self
        else {
            return None;
        };
        let receiver_name = receiver.to_string();
        let mut binding: String = receiver_name
            .chars()
            .take_while(|c| c.is_alphanumeric())
            .collect();
        if let Some(first) = binding.get_mut(..1) {
            first.make_ascii_lowercase();
        }
        let (what, used) = if *field {
            ("a field", format!("{}.{}", binding, name))
        } else {
            ("an extension", format!("{}.{}()", binding, name))
        };
        Some(format!(
            "'{}' is {} of the outer receiver '{}', which a lambda with a receiver hides; bind it before the lambda ('val {} = this') and write '{}'",
            name, what, receiver_name, binding, used
        ))
    }
}

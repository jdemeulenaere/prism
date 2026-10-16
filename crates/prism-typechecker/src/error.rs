//! Semantic analysis and type checker diagnostic error representations.

use crate::types::Type;
use prism_ast::Span;
use std::fmt;

/// Semantic and type checking error categories (`E4001` - `E4010`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeError {
    /// E4001: Identifier not found in current lexical scope or module environment.
    UnresolvedSymbol { name: String, span: Span },

    /// E4002: Expression type is not assignable to the expected type.
    TypeMismatch {
        expected: Type,
        found: Type,
        span: Span,
    },

    /// E4003: Reassigning to an immutable `val` binding or mutating a `val` struct instance/field.
    ImmutabilityViolation { message: String, span: Span },

    /// E4004: Function invocation argument count or parameter type incompatibility.
    CallSignatureMismatch { message: String, span: Span },

    /// E4005: Returned expression type is incompatible with declared function return type.
    ReturnTypeMismatch {
        expected: Type,
        found: Type,
        span: Span,
    },

    /// E4006: Struct field not found or field access attempted on a non-struct type.
    UnknownField {
        target_type: Type,
        field_name: String,
        span: Span,
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
    NonExhaustiveWhen { message: String, span: Span },

    /// E4010: Declaring a duplicate symbol within the identical lexical scope.
    DuplicateSymbol { name: String, span: Span },
}

impl TypeError {
    /// Returns the diagnostic error code (e.g. "E4001").
    pub fn code(&self) -> &'static str {
        match self {
            TypeError::UnresolvedSymbol { .. } => "E4001",
            TypeError::TypeMismatch { .. } => "E4002",
            TypeError::ImmutabilityViolation { .. } => "E4003",
            TypeError::CallSignatureMismatch { .. } => "E4004",
            TypeError::ReturnTypeMismatch { .. } => "E4005",
            TypeError::UnknownField { .. } => "E4006",
            TypeError::VisibilityViolation { .. } => "E4007",
            TypeError::InvalidComposeContext { .. } => "E4008",
            TypeError::NonExhaustiveWhen { .. } => "E4009",
            TypeError::DuplicateSymbol { .. } => "E4010",
        }
    }

    /// Primary source span associated with this error.
    pub fn span(&self) -> Span {
        match self {
            TypeError::UnresolvedSymbol { span, .. } => *span,
            TypeError::TypeMismatch { span, .. } => *span,
            TypeError::ImmutabilityViolation { span, .. } => *span,
            TypeError::CallSignatureMismatch { span, .. } => *span,
            TypeError::ReturnTypeMismatch { span, .. } => *span,
            TypeError::UnknownField { span, .. } => *span,
            TypeError::VisibilityViolation { span, .. } => *span,
            TypeError::InvalidComposeContext { span, .. } => *span,
            TypeError::NonExhaustiveWhen { span, .. } => *span,
            TypeError::DuplicateSymbol { span, .. } => *span,
        }
    }

    /// User-facing descriptive error message.
    pub fn message(&self) -> String {
        match self {
            TypeError::UnresolvedSymbol { name, .. } => {
                format!("unresolved symbol '{}'", name)
            }
            TypeError::TypeMismatch {
                expected, found, ..
            } => {
                format!("type mismatch: expected '{}', found '{}'", expected, found)
            }
            TypeError::ImmutabilityViolation { message, .. } => message.clone(),
            TypeError::CallSignatureMismatch { message, .. } => message.clone(),
            TypeError::ReturnTypeMismatch {
                expected, found, ..
            } => {
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
            TypeError::DuplicateSymbol { name, .. } => {
                format!("duplicate symbol declaration '{}' in the same scope", name)
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

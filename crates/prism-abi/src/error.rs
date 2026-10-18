//! ABI extraction and validation errors.

use prism_ast::Span;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AbiError {
    /// Single-file compilation unit invariant: `public` and `package` functions must declare explicit return types.
    MissingExplicitReturnType {
        function_name: String,
        visibility: String,
        span: Span,
    },
    /// Exported top-level variables must declare explicit types.
    MissingExplicitVarType {
        var_name: String,
        visibility: String,
        span: Span,
    },
    /// `E3002`: a workspace file declares something `external` or `intrinsic`, which only the
    /// bundled library may (D12 of `standard-library`). `modifier` is the word written.
    LibraryOnlyDeclaration { modifier: String, span: Span },
}

impl AbiError {
    /// The span the error points at.
    pub fn span(&self) -> Span {
        match self {
            AbiError::MissingExplicitReturnType { span, .. }
            | AbiError::MissingExplicitVarType { span, .. }
            | AbiError::LibraryOnlyDeclaration { span, .. } => *span,
        }
    }
}

impl fmt::Display for AbiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AbiError::MissingExplicitReturnType {
                function_name,
                visibility,
                span,
            } => {
                write!(
                    f,
                    "ABI violation at {}: {} function '{}' must declare an explicit return type for ABI extraction",
                    span, visibility, function_name
                )
            }
            AbiError::MissingExplicitVarType {
                var_name,
                visibility,
                span,
            } => {
                write!(
                    f,
                    "ABI violation at {}: {} variable '{}' must declare an explicit type for ABI extraction",
                    span, visibility, var_name
                )
            }
            AbiError::LibraryOnlyDeclaration { modifier, span } => {
                write!(
                    f,
                    "'{}' declaration outside the bundled library at {}",
                    modifier, span
                )
            }
        }
    }
}

impl std::error::Error for AbiError {}

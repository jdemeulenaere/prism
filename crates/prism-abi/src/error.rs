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
        }
    }
}

impl std::error::Error for AbiError {}

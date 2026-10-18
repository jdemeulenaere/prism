//! The diagnostics of code generation and of the program's entry point (`E6xxx`, D25 of
//! `codegen-core`), and the internal failures of the backend.

use prism_ast::Span;
use std::fmt;

/// What a [`CodegenError`] reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodegenErrorKind {
    /// `E6001`: a construct the Tier 1 backend does not compile yet (D9).
    UnsupportedByBackend,
    /// `E6002`: the entry file declares no `main` (D13).
    MissingMain,
    /// `E6003`: the entry file's `main` breaks a rule of the entry point (D13).
    InvalidMain,
    /// `E6004`: generics of several files instantiate each other with a growing type argument,
    /// which only an import cycle allows (D8 of `monomorphization`).
    PolymorphicRecursionAcrossFiles,
}

impl CodegenErrorKind {
    /// The diagnostic code.
    pub fn code(self) -> &'static str {
        match self {
            CodegenErrorKind::UnsupportedByBackend => "E6001",
            CodegenErrorKind::MissingMain => "E6002",
            CodegenErrorKind::InvalidMain => "E6003",
            CodegenErrorKind::PolymorphicRecursionAcrossFiles => "E6004",
        }
    }
}

/// A code-generation diagnostic in one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodegenError {
    pub kind: CodegenErrorKind,
    /// The diagnostic's title, after `error[<code>]: `.
    pub message: String,
    /// Where it points.
    pub span: Span,
    /// Lines rendered as `= note: ...`.
    pub notes: Vec<String>,
    /// A fix, rendered as `= help: ...`.
    pub help: Option<String>,
}

impl CodegenError {
    /// The diagnostic code (`E6001`).
    pub fn code(&self) -> &'static str {
        self.kind.code()
    }
}

/// A failure inside the backend: Cranelift rejected the IR it was given or could not define or
/// link a function. A program that passed every check never causes one; it is reported as an
/// internal compiler error rather than a panic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendError {
    pub message: String,
}

impl BackendError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for BackendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for BackendError {}

impl From<cranelift_module::ModuleError> for BackendError {
    fn from(error: cranelift_module::ModuleError) -> Self {
        BackendError::new(format!("{:?}", error))
    }
}

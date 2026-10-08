//! Prism module / file representation.

use crate::decl::{Declaration, ImportDecl, PackageDecl};
use crate::span::Span;

/// An entire parsed `.prism` source file.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SourceFile {
    /// Optional package declaration (e.g. `package foo.bar`). If omitted, file is at workspace root.
    pub package: Option<PackageDecl>,
    /// List of imports at the top of the file.
    pub imports: Vec<ImportDecl>,
    /// All top-level declarations in the file.
    pub declarations: Vec<Declaration>,
    /// Source span of the entire file.
    pub span: Span,
}

impl SourceFile {
    pub fn new(
        package: Option<PackageDecl>,
        imports: Vec<ImportDecl>,
        declarations: Vec<Declaration>,
        span: Span,
    ) -> Self {
        Self {
            package,
            imports,
            declarations,
            span,
        }
    }
}

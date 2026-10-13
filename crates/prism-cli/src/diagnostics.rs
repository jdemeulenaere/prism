//! Compiler diagnostics formatting for CLI errors.

use prism_abi::AbiError;
use prism_parser::ParseError;
use prism_resolver::ResolveError;
use std::path::{Path, PathBuf};

/// Formats a snippet with a line pointer when source text is provided.
fn render_code_snippet(source: &str, line: u32, col: u32) -> Option<String> {
    let lines: Vec<&str> = source.lines().collect();
    let line_usize = line as usize;
    if line == 0 || line_usize > lines.len() {
        return None;
    }
    let src_line = lines[line_usize - 1];
    let col_idx = if col > 0 { (col - 1) as usize } else { 0 };
    let pointer = format!("{}^", " ".repeat(col_idx));
    Some(format!(" {:4} | {}\n      | {}", line, src_line, pointer))
}

/// Formats a [`ResolveError`] as a human-readable compiler diagnostic.
pub fn format_resolve_error(err: &ResolveError) -> String {
    let mut out = String::new();
    let code = err.code();

    match err {
        ResolveError::PackagePathMismatch {
            file,
            declared_package,
            expected_suffix,
            actual_dir,
        } => {
            out.push_str(&format!("error[{}]: package path mismatch\n", code));
            out.push_str(&format!("  --> {}:1:1\n", file.display()));
            out.push_str(&format!(
                "   = note: directory path '{}' does not match declared package '{}' (expected suffix: '{}')\n",
                actual_dir.display(),
                declared_package,
                expected_suffix
            ));
        }
        ResolveError::FileNotFound {
            import_path,
            target_path,
        } => {
            out.push_str(&format!("error[{}]: imported file not found\n", code));
            out.push_str(&format!(
                "   = note: cannot resolve import '{}': target file '{}' does not exist\n",
                import_path,
                target_path.display()
            ));
        }
        ResolveError::PackageHeterogeneity {
            file,
            package_dir,
            declared_package,
            expected_package,
        } => {
            out.push_str(&format!(
                "error[{}]: conflicting package in directory\n",
                code
            ));
            out.push_str(&format!("  --> {}:1:1\n", file.display()));
            out.push_str(&format!(
                "   = note: file in directory '{}' declares package '{}' but expected package '{}'\n",
                package_dir.display(),
                declared_package,
                expected_package
            ));
        }
        ResolveError::InconsistentWorkspaceRoot {
            file,
            file_root,
            workspace_root,
        } => {
            out.push_str(&format!(
                "error[{}]: inconsistent workspace root\n",
                code
            ));
            out.push_str(&format!("  --> {}:1:1\n", file.display()));
            out.push_str(&format!(
                "   = note: dependency root '{}' conflicts with workspace root '{}'\n",
                file_root.display(),
                workspace_root.display()
            ));
        }
        ResolveError::WorkspaceBoundaryViolation {
            import_path,
            target_path,
            workspace_root,
        } => {
            out.push_str(&format!(
                "error[{}]: import escapes workspace boundary\n",
                code
            ));
            out.push_str(&format!(
                "   = note: import '{}' targets '{}' which escapes workspace root '{}'\n",
                import_path,
                target_path.display(),
                workspace_root.display()
            ));
        }
        ResolveError::InvalidImportPath {
            import_path,
            reason,
        } => {
            out.push_str(&format!("error[{}]: invalid import path\n", code));
            out.push_str(&format!(
                "   = note: import path '{}' is invalid: {}\n",
                import_path, reason
            ));
        }
        ResolveError::Io { path, message } => {
            out.push_str(&format!("error[{}]: IO error\n", code));
            out.push_str(&format!("  --> {}: {}\n", path.display(), message));
        }
        ResolveError::Parse { path, message } => {
            out.push_str(&format!("error[{}]: syntax error\n", code));
            out.push_str(&format!("  --> {}: {}\n", path.display(), message));
        }
    }

    if let Some(suggestion) = err.suggestion() {
        out.push_str(&format!("   = help: {}\n", suggestion));
    }

    out
}

/// Formats a circular import cycle error.
pub fn format_cycle_error(cycle_nodes: &[PathBuf]) -> String {
    let mut out = String::new();
    out.push_str("error[E1007]: cyclic dependency detected\n");
    let chain = cycle_nodes
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join(" -> ");
    out.push_str(&format!("   = note: circular import cycle: {}\n", chain));
    out.push_str("   = help: refactor circular imports into a shared module or interface\n");
    out
}

/// Formats a parser error.
pub fn format_parse_error(path: &Path, source: Option<&str>, err: &ParseError) -> String {
    let mut out = String::new();
    out.push_str("error[E2001]: syntax error\n");
    out.push_str(&format!(
        "  --> {}:{}:{}\n",
        path.display(),
        err.span.line,
        err.span.column
    ));
    if let Some(src) = source {
        if let Some(snippet) = render_code_snippet(src, err.span.line, err.span.column) {
            out.push_str(&format!("{}\n", snippet));
        }
    }
    out.push_str(&format!("   = note: {}\n", err.message));
    out
}

/// Formats an ABI extraction error.
pub fn format_abi_error(path: &Path, source: Option<&str>, err: &AbiError) -> String {
    let mut out = String::new();
    out.push_str("error[E3001]: ABI specification violation\n");
    let span = match err {
        AbiError::MissingExplicitReturnType { span, .. } => span,
        AbiError::MissingExplicitVarType { span, .. } => span,
    };
    out.push_str(&format!(
        "  --> {}:{}:{}\n",
        path.display(),
        span.line,
        span.column
    ));
    if let Some(src) = source {
        if let Some(snippet) = render_code_snippet(src, span.line, span.column) {
            out.push_str(&format!("{}\n", snippet));
        }
    }
    out.push_str(&format!("   = note: {}\n", err));
    out.push_str("   = help: all public and package declarations must declare explicit types for ABI extraction\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_format_resolve_error_package_mismatch() {
        let err = ResolveError::PackagePathMismatch {
            file: PathBuf::from("src/views.prism"),
            declared_package: "app.ui".to_string(),
            expected_suffix: "app/ui".to_string(),
            actual_dir: PathBuf::from("src"),
        };
        let formatted = format_resolve_error(&err);
        assert!(formatted.contains("error[E1001]: package path mismatch"));
        assert!(formatted.contains("--> src/views.prism:1:1"));
        assert!(formatted.contains("help: Rename directory to match package or update package statement."));
    }

    #[test]
    fn test_format_resolve_error_file_not_found() {
        let err = ResolveError::FileNotFound {
            import_path: "/core/math".to_string(),
            target_path: PathBuf::from("/workspace/core/math.prism"),
        };
        let formatted = format_resolve_error(&err);
        assert!(formatted.contains("error[E1002]: imported file not found"));
        assert!(formatted.contains("cannot resolve import '/core/math'"));
    }

    #[test]
    fn test_format_cycle_error() {
        let cycle = vec![
            PathBuf::from("a.prism"),
            PathBuf::from("b.prism"),
            PathBuf::from("a.prism"),
        ];
        let formatted = format_cycle_error(&cycle);
        assert!(formatted.contains("error[E1007]: cyclic dependency detected"));
        assert!(formatted.contains("a.prism -> b.prism -> a.prism"));
    }

    #[test]
    fn test_format_parse_error() {
        let err = ParseError::new("expected identifier", prism_ast::Span::new(10, 15, 2, 5));
        let src = "fun main() {\n    val = 1\n}";
        let formatted = format_parse_error(Path::new("test.prism"), Some(src), &err);
        assert!(formatted.contains("error[E2001]: syntax error"));
        assert!(formatted.contains("test.prism:2:5"));
        assert!(formatted.contains("val = 1"));
    }
}

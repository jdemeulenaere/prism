//! Integration and acceptance test suite for `prism-resolver`.
//!
//! Conforms strictly to `openspec/specs/package-resolution/spec.md` and
//! `openspec/specs/system-invariants/spec.md`.

use prism_ast::{ImportDecl, Span};
use prism_resolver::{
    candidate_target_path, discover_root, discover_root_from_segments, scan_directory,
    DependencyGraph, ResolveError,
};
use std::path::{Path, PathBuf};

/// Helper RAII test directory for disk-backed fixtures.
struct TestWorkspace {
    root: PathBuf,
}

impl TestWorkspace {
    fn new(test_name: &str) -> Self {
        let unique = format!(
            "prism_resolver_{}_{}_{}",
            test_name,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let root = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&root).expect("failed to create test workspace root");
        Self { root }
    }

    fn write_file(&self, rel_path: &str, content: &str) -> PathBuf {
        let full_path = self.root.join(rel_path);
        if let Some(parent) = full_path.parent() {
            std::fs::create_dir_all(parent).expect("failed to create parent dir");
        }
        std::fs::write(&full_path, content).expect("failed to write test file");
        full_path
    }

    fn path(&self) -> &Path {
        &self.root
    }
}

impl Drop for TestWorkspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

// ---------------------------------------------------------------------------
// Acceptance Fixture 1: Standard Nested Package Root Discovery
// ---------------------------------------------------------------------------

#[test]
fn test_fixture_1_standard_nested_package_pure_path() {
    // Scenario "Matching nested package declaration": /demo/app/ui/views.prism with package app.ui ==> Root is /demo
    let source_path = Path::new("/demo/app/ui/views.prism");
    let pkg_segments = vec!["app", "ui"];

    let root = discover_root_from_segments(source_path, &pkg_segments)
        .expect("pure-path discovery should succeed");
    assert_eq!(root, Path::new("/demo"));
}

#[test]
fn test_fixture_1_standard_nested_package_disk() {
    let ws = TestWorkspace::new("fixture1");
    let file = ws.write_file(
        "app/ui/views.prism",
        "package app.ui\npublic fun render() {}\n",
    );

    let content = std::fs::read_to_string(&file).unwrap();
    let ast = prism_parser::parse_source(&content).unwrap();
    let root = discover_root(&file, ast.package.as_ref()).expect("discovery should succeed");

    assert_eq!(root, ws.path());
}

// ---------------------------------------------------------------------------
// Acceptance Fixture 2: Root File Root Discovery (Empty Package)
// ---------------------------------------------------------------------------

#[test]
fn test_fixture_2_root_file_pure_path() {
    // Scenario "Root-level file with empty package": /demo/main.prism with no package statement ==> Root is /demo
    let source_path = Path::new("/demo/main.prism");
    let empty_pkg: Vec<&str> = vec![];

    let root = discover_root_from_segments(source_path, &empty_pkg)
        .expect("empty package at root should succeed");
    assert_eq!(root, Path::new("/demo"));
}

#[test]
fn test_fixture_2_root_file_disk() {
    let ws = TestWorkspace::new("fixture2");
    let file = ws.write_file("main.prism", "fun main() {}\n");

    let content = std::fs::read_to_string(&file).unwrap();
    let ast = prism_parser::parse_source(&content).unwrap();
    let root =
        discover_root(&file, ast.package.as_ref()).expect("root file discovery should succeed");

    assert_eq!(root, ws.path());
}

// ---------------------------------------------------------------------------
// Acceptance Fixture 3: Leading Slash Root Import
// ---------------------------------------------------------------------------

#[test]
fn test_fixture_3_leading_slash_root_import() {
    // Scenario "Root-anchored import with leading slash": From /demo/app/ui/views.prism: import /core/fmt ==> /demo/core/fmt.prism
    let ws = TestWorkspace::new("fixture3");
    let views = ws.write_file(
        "app/ui/views.prism",
        "package app.ui\nimport /core/fmt\npublic fun render() {}\n",
    );
    let fmt = ws.write_file("core/fmt.prism", "package core\npublic fun println() {}\n");

    let views_content = std::fs::read_to_string(&views).unwrap();
    let views_ast = prism_parser::parse_source(&views_content).unwrap();
    assert_eq!(views_ast.imports.len(), 1);

    let resolved = prism_resolver::resolve_import(&views, &views_ast.imports[0], ws.path())
        .expect("root-anchored import should resolve");

    assert_eq!(resolved, fmt);
}

// ---------------------------------------------------------------------------
// Acceptance Fixture 4: Sibling Import
// ---------------------------------------------------------------------------

#[test]
fn test_fixture_4_sibling_import() {
    // Scenario "Sibling import without leading slash": From /demo/app/ui/views.prism: import widgets ==> /demo/app/ui/widgets.prism
    let ws = TestWorkspace::new("fixture4");
    let views = ws.write_file(
        "app/ui/views.prism",
        "package app.ui\nimport widgets\npublic fun render() {}\n",
    );
    let widgets = ws.write_file(
        "app/ui/widgets.prism",
        "package app.ui\npackage fun button() {}\n",
    );

    let views_content = std::fs::read_to_string(&views).unwrap();
    let views_ast = prism_parser::parse_source(&views_content).unwrap();
    assert_eq!(views_ast.imports.len(), 1);

    let resolved = prism_resolver::resolve_import(&views, &views_ast.imports[0], ws.path())
        .expect("sibling import should resolve");

    assert_eq!(resolved, widgets);
}

// ---------------------------------------------------------------------------
// Acceptance Fixture 5: Directory Mismatch Rejection (E1001)
// ---------------------------------------------------------------------------

#[test]
fn test_fixture_5_directory_mismatch_pure_path() {
    // Scenario "Directory path does not match declared package": /demo/app/ui/views.prism with package app.models ==> Returns E1001
    let source_path = Path::new("/demo/app/ui/views.prism");
    let mismatch_pkg = vec!["app", "models"];

    let err = discover_root_from_segments(source_path, &mismatch_pkg)
        .expect_err("should reject package-directory mismatch");

    assert_eq!(err.code(), "E1001");
    match err {
        ResolveError::PackagePathMismatch {
            declared_package,
            expected_suffix,
            ..
        } => {
            assert_eq!(declared_package, "app.models");
            assert_eq!(expected_suffix, "app/models");
        }
        _ => panic!("expected PackagePathMismatch variant"),
    }
}

#[test]
fn test_fixture_5_directory_mismatch_disk() {
    let ws = TestWorkspace::new("fixture5");
    let file = ws.write_file("app/ui/views.prism", "package app.models\n");

    let content = std::fs::read_to_string(&file).unwrap();
    let ast = prism_parser::parse_source(&content).unwrap();

    let err = discover_root(&file, ast.package.as_ref())
        .expect_err("should reject package-directory mismatch on disk");
    assert_eq!(err.code(), "E1001");
}

// ---------------------------------------------------------------------------
// Acceptance Fixture 6: Import-Directed Isolation
// ---------------------------------------------------------------------------

#[test]
fn test_fixture_6_import_directed_isolation() {
    // Scenario "Unimported sibling files ignored": Compiling /demo/app/main.prism which only imports widgets.prism
    // ignores unimported /demo/app/draft.prism (even if draft.prism contains syntax errors)
    let ws = TestWorkspace::new("fixture6");
    let main = ws.write_file(
        "app/main.prism",
        "package app\nimport widgets\nfun main() {}\n",
    );
    let widgets = ws.write_file("app/widgets.prism", "package app\nfun button() {}\n");

    // Unimported draft file with broken syntax that would fail parsing
    let draft = ws.write_file(
        "app/draft.prism",
        "!!! THIS IS COMPLETELY BROKEN SYNTAX &&& !!!",
    );

    let graph = DependencyGraph::from_entry(&main).expect("graph build should succeed");

    assert_eq!(workspace_len(&graph), 2);
    assert!(graph.get_node(&main).is_some());
    assert!(graph.get_node(&widgets).is_some());
    assert!(graph.get_node(&draft).is_none());
}

// ---------------------------------------------------------------------------
// Acceptance Fixture 7: Mutual Recursion in SCC
// ---------------------------------------------------------------------------

#[test]
fn test_fixture_7_mutual_recursion_scc() {
    // Scenario "Mutually recursive sibling imports": a.prism imports b, b.prism imports a
    // ==> Grouped into single SCC cluster of size 2 without fatal graph cycle error.
    let ws = TestWorkspace::new("fixture7");
    let a = ws.write_file("demo/a.prism", "package demo\nimport b\nfun foo() {}\n");
    let b = ws.write_file("demo/b.prism", "package demo\nimport a\nfun bar() {}\n");

    let graph = DependencyGraph::from_entry(&a).expect("cyclic imports must succeed into SCC");

    assert_eq!(workspace_len(&graph), 2);
    assert!(graph.is_cyclic());

    let sccs = &workspace_sccs(&graph);
    assert_eq!(sccs.len(), 1);
    assert_eq!(sccs[0].files.len(), 2);
    assert!(sccs[0].files.contains(&a));
    assert!(sccs[0].files.contains(&b));
}

// ---------------------------------------------------------------------------
// Acceptance Fixture 8: CLI Directory Scanning & Homogeneity (E1003)
// ---------------------------------------------------------------------------

#[test]
fn test_fixture_8_cli_directory_scanning_and_homogeneity() {
    // Scenario "Homogeneous folder": scan_directory("/demo/app/ui") finds views.prism and widgets.prism,
    // but rejects if any file in that folder declares a different package (E1003).
    let ws = TestWorkspace::new("fixture8");
    let views = ws.write_file("app/ui/views.prism", "package app.ui\nfun v() {}\n");
    let widgets = ws.write_file("app/ui/widgets.prism", "package app.ui\nfun w() {}\n");

    // Hidden file and subdirectory to verify filtering
    ws.write_file("app/ui/.DS_Store", "dummy hidden data");
    ws.write_file("app/ui/sub/nested.prism", "package app.ui.sub\n");

    let ui_dir = ws.path().join("app/ui");
    let scanned = scan_directory(&ui_dir).expect("scan should find homogeneous files");

    assert_eq!(scanned.len(), 2);
    assert_eq!(scanned, vec![views, widgets]);

    // Now introduce a conflicting file with package app.other
    let conflicting = ws.write_file("app/ui/conflict.prism", "package app.other\n");

    let err = scan_directory(&ui_dir).expect_err("should reject heterogeneous package");
    assert_eq!(err.code(), "E1003");
    match err {
        ResolveError::PackageHeterogeneity {
            file,
            declared_package,
            expected_package,
            ..
        } => {
            assert_eq!(file, conflicting);
            assert_eq!(declared_package, "app.other");
            assert_eq!(expected_package, "app.ui");
        }
        _ => panic!("expected PackageHeterogeneity variant"),
    }
}

// ---------------------------------------------------------------------------
// Acceptance Fixture 9: Workspace Jail Enforcement (E1005 & E1006)
// ---------------------------------------------------------------------------

#[test]
fn test_fixture_9_workspace_jail_relative_traversal() {
    // Scenario "Import attempting directory traversal": Parent traversal forbidden (E1006)
    let dummy_file = Path::new("/demo/app/main.prism");
    let dummy_root = Path::new("/demo");

    let invalid_import = ImportDecl {
        is_root: true,
        path: vec!["..".to_string(), "etc".to_string(), "passwd".to_string()],
        span: Span::dummy(),
    };

    let err = prism_resolver::resolve_import(dummy_file, &invalid_import, dummy_root)
        .expect_err("should reject .. in import path");

    assert_eq!(err.code(), "E1006");
}

#[cfg(unix)]
#[test]
fn test_fixture_9_workspace_jail_symlink_escaping() {
    // Scenario "Import target escaping workspace boundary": Import target escaping root (e.g. symlink to /tmp/outside.prism) ==> E1005
    let outside_ws = TestWorkspace::new("fixture9_outside");
    let outside_file = outside_ws.write_file("secret.prism", "package secret\n");

    let inside_ws = TestWorkspace::new("fixture9_inside");
    let app_file = inside_ws.write_file("app.prism", "import secret\n");

    // Create symlink inside inside_ws pointing to outside_file
    let symlink_path = inside_ws.path().join("secret.prism");
    std::os::unix::fs::symlink(&outside_file, &symlink_path)
        .expect("symlink creation should succeed");

    let app_content = std::fs::read_to_string(&app_file).unwrap();
    let app_ast = prism_parser::parse_source(&app_content).unwrap();

    let err = prism_resolver::resolve_import(&app_file, &app_ast.imports[0], inside_ws.path())
        .expect_err("symlink escaping workspace must be rejected");

    assert_eq!(err.code(), "E1005");
}

// ---------------------------------------------------------------------------
// Acceptance Fixture 10: Strict Case-Sensitivity Enforcement (E1001 / E1002)
// ---------------------------------------------------------------------------

#[test]
fn test_fixture_10_strict_case_sensitivity_root_discovery() {
    // Scenario "Strict case-sensitivity enforcement": Directory Foo/Bar with package foo.bar on macOS/Windows ==> Returns E1001
    let ws = TestWorkspace::new("fixture10");
    let file = ws.write_file("Foo/Bar/views.prism", "package foo.bar\n");

    let content = std::fs::read_to_string(&file).unwrap();
    let ast = prism_parser::parse_source(&content).unwrap();

    let err = discover_root(&file, ast.package.as_ref())
        .expect_err("should fail when disk casing Foo/Bar differs from package foo.bar");

    assert_eq!(err.code(), "E1001");
}

#[test]
fn test_fixture_10_strict_case_sensitivity_import() {
    // Target exists as Fmt.prism (capital F) on disk, import is /core/fmt (lowercase f)
    // On macOS APFS, file exists case-insensitively, but resolver must strictly enforce case -> E1002
    let ws = TestWorkspace::new("fixture10_import");
    let views = ws.write_file("views.prism", "import /core/fmt\n");
    ws.write_file("core/Fmt.prism", "package core\npublic fun println() {}\n");

    let views_content = std::fs::read_to_string(&views).unwrap();
    let views_ast = prism_parser::parse_source(&views_content).unwrap();

    let err = prism_resolver::resolve_import(&views, &views_ast.imports[0], ws.path())
        .expect_err("import case mismatch must return FileNotFound E1002");

    assert_eq!(err.code(), "E1002");
}

// ---------------------------------------------------------------------------
// Additional Invariants & Edge Cases
// ---------------------------------------------------------------------------

#[test]
fn test_filesystem_root_file() {
    // /main.prism with empty package -> root is /
    let root = discover_root_from_segments(Path::new("/main.prism"), &[] as &[&str])
        .expect("filesystem root file should resolve to /");
    assert_eq!(root, Path::new("/"));

    // /main.prism with package foo -> E1001
    let err = discover_root_from_segments(Path::new("/main.prism"), &["foo"])
        .expect_err("filesystem root cannot have non-empty package");
    assert_eq!(err.code(), "E1001");
}

#[test]
fn test_windows_drive_root_discovery() {
    // C:\main.prism with empty package -> root is C:\
    let root = discover_root_from_segments(Path::new("C:\\main.prism"), &[] as &[&str])
        .expect("Windows root file should resolve to C:\\");
    assert_eq!(root, Path::new("C:\\"));

    // C:\demo\foo\bar\app.prism with package foo.bar -> root is C:\demo
    let root =
        discover_root_from_segments(Path::new("C:\\demo\\foo\\bar\\app.prism"), &["foo", "bar"])
            .expect("Windows nested package should strip suffix");
    assert_eq!(root, Path::new("C:\\demo"));
}

#[test]
fn test_inconsistent_workspace_root_e1004() {
    // Case A: Imported file dep.prism in core/ declares no package, resolving to <ws>/core,
    // which conflicts with the workspace root <ws> discovered from main.prism -> E1004
    let ws = TestWorkspace::new("fixture_e1004");
    let main = ws.write_file("main.prism", "import /core/dep\n");
    let dep = ws.write_file("core/dep.prism", "fun helper() {}\n");

    let err = DependencyGraph::from_entry(&main)
        .expect_err("transitive file with conflicting root must return E1004");
    assert_eq!(err.code(), "E1004");
    match err {
        ResolveError::InconsistentWorkspaceRoot {
            file,
            file_root,
            workspace_root,
        } => {
            assert_eq!(file, dep);
            assert_ne!(file_root, workspace_root);
        }
        _ => panic!("expected InconsistentWorkspaceRoot variant"),
    }

    // Case B: Multi-entry graph where entry 1 and entry 2 reside in different workspaces -> E1004
    let ws_a = TestWorkspace::new("fixture_e1004_a");
    let entry_a = ws_a.write_file("main.prism", "fun a() {}\n");
    let ws_b = TestWorkspace::new("fixture_e1004_b");
    let entry_b = ws_b.write_file("main.prism", "fun b() {}\n");

    let err2 = DependencyGraph::from_entries(&[&entry_a, &entry_b])
        .expect_err("multi-entry across different workspaces must return E1004");
    assert_eq!(err2.code(), "E1004");
}

#[test]
fn test_dependency_first_topological_sort() {
    // A -> B -> C (linear dependency chain)
    // Topological order must be [C, B, A] (dependency-first)
    let ws = TestWorkspace::new("topological_test");
    let a = ws.write_file("a.prism", "import b\n");
    let b = ws.write_file("b.prism", "import c\n");
    let c = ws.write_file("c.prism", "fun leaf() {}\n");

    let graph = DependencyGraph::from_entry(&a).expect("graph should build");
    assert_eq!(workspace_len(&graph), 3);
    assert!(!graph.is_cyclic());

    let sccs = &workspace_sccs(&graph);
    assert_eq!(sccs.len(), 3);
    assert_eq!(sccs[0].files, vec![c]);
    assert_eq!(sccs[1].files, vec![b]);
    assert_eq!(sccs[2].files, vec![a]);
}

#[test]
fn test_sibling_import_multi_identifier_rejected() {
    let dummy_file = Path::new("/demo/app/main.prism");
    let dummy_root = Path::new("/demo");

    let invalid_sibling = ImportDecl {
        is_root: false,
        path: vec!["foo".to_string(), "bar".to_string()],
        span: Span::dummy(),
    };

    let err = prism_resolver::resolve_import(dummy_file, &invalid_sibling, dummy_root)
        .expect_err("sibling import with multiple identifiers must be rejected");

    assert_eq!(err.code(), "E1006");
}

#[test]
fn test_candidate_target_path_helper() {
    let source_file = Path::new("/demo/app/ui/views.prism");
    let root = Path::new("/demo");

    let root_import = ImportDecl {
        is_root: true,
        path: vec!["core".to_string(), "fmt".to_string()],
        span: Span::dummy(),
    };
    let candidate = candidate_target_path(source_file, &root_import, root).unwrap();
    assert_eq!(candidate, Path::new("/demo/core/fmt.prism"));

    let sibling_import = ImportDecl {
        is_root: false,
        path: vec!["widgets".to_string()],
        span: Span::dummy(),
    };
    let candidate = candidate_target_path(source_file, &sibling_import, root).unwrap();
    assert_eq!(candidate, Path::new("/demo/app/ui/widgets.prism"));
}

// ---------------------------------------------------------------------------
// Adversarial Audit Verifications (ADV-01 through ADV-08)
// ---------------------------------------------------------------------------

#[test]
fn test_adv01_relative_path_case_sensitivity() {
    // ADV-01: When relative path is provided, disk case check must not be skipped.
    let ws = TestWorkspace::new("adv01_rel_case");
    // Create App/Ui on disk (capitalized)
    let file = ws.write_file("App/Ui/views.prism", "package app.ui\n");

    // Change current directory or pass relative path from parent
    let parent = file.parent().unwrap().parent().unwrap();
    let original_cwd = std::env::current_dir().unwrap();
    std::env::set_current_dir(parent).unwrap();

    let rel_path = Path::new("App/Ui/views.prism");
    let err = discover_root_from_segments(rel_path, &["app", "ui"])
        .expect_err("should reject case mismatch even on relative path");
    assert_eq!(err.code(), "E1001");

    // Sibling import from relative single-file
    let sibling_file = Path::new("views.prism");
    let import_decl = ImportDecl {
        is_root: false,
        path: vec!["helper".to_string()],
        span: Span::dummy(),
    };
    ws.write_file("Helper.prism", "fun help() {}\n");
    let sibling_err = prism_resolver::resolve_import(sibling_file, &import_decl, Path::new("."))
        .expect_err("sibling import on relative file must enforce case check");
    assert_eq!(sibling_err.code(), "E1002");

    std::env::set_current_dir(original_cwd).unwrap();
}

#[test]
fn test_adv02_leading_parent_traversal_normalization() {
    // ADV-02: Leading .. must be retained in relative paths
    assert_eq!(
        prism_resolver::normalize_path(Path::new("../a/b")),
        PathBuf::from("../a/b")
    );
    assert_eq!(
        prism_resolver::normalize_path(Path::new("../../secret.prism")),
        PathBuf::from("../../secret.prism")
    );
    assert_eq!(
        prism_resolver::normalize_path(Path::new("a/../../b")),
        PathBuf::from("../b")
    );
    // Root paths still clamp .. at filesystem root
    assert_eq!(
        prism_resolver::normalize_path(Path::new("/..")),
        PathBuf::from("/")
    );
    assert_eq!(
        prism_resolver::normalize_path(Path::new("/a/..")),
        PathBuf::from("/")
    );
}

#[test]
fn test_adv03_windows_verbatim_path_normalization() {
    // ADV-03: Windows verbatim (\\?\) paths must not be corrupted
    assert_eq!(
        prism_resolver::normalize_path(Path::new(r"\\?\C:\foo\bar")),
        PathBuf::from(r"\\?\C:\foo\bar")
    );
}

#[test]
fn test_adv04_self_import_rejected() {
    // ADV-04: File importing itself must be rejected
    let ws = TestWorkspace::new("adv04_self_import");
    let self_file = ws.write_file("self.prism", "import self\n");
    let content = std::fs::read_to_string(&self_file).unwrap();
    let ast = prism_parser::parse_source(&content).unwrap();

    let err = prism_resolver::resolve_import(&self_file, &ast.imports[0], ws.path())
        .expect_err("self-import must be rejected");
    assert_eq!(err.code(), "E1006");
}

#[test]
fn test_adv05_empty_import_path_component_rejected() {
    // ADV-05: Empty path component in import must return E1006
    let dummy_file = Path::new("/demo/main.prism");
    let dummy_root = Path::new("/demo");

    let invalid_import = ImportDecl {
        is_root: true,
        path: vec!["core".to_string(), "".to_string(), "fmt".to_string()],
        span: Span::dummy(),
    };

    let err = prism_resolver::resolve_import(dummy_file, &invalid_import, dummy_root)
        .expect_err("empty path component must be rejected");
    assert_eq!(err.code(), "E1006");
}

#[test]
fn test_adv_diamond_dependency_topological_sort() {
    // A -> B, C; B, C -> D (diamond dependency)
    // Topological order must place D before B and C, and B and C before A.
    let ws = TestWorkspace::new("diamond_test");
    let a = ws.write_file("a.prism", "import b\nimport c\n");
    let b = ws.write_file("b.prism", "import d\n");
    let c = ws.write_file("c.prism", "import d\n");
    let d = ws.write_file("d.prism", "fun base() {}\n");

    let graph = DependencyGraph::from_entry(&a).expect("diamond graph must build");
    assert_eq!(workspace_len(&graph), 4);
    assert!(!graph.is_cyclic());

    let sccs = &workspace_sccs(&graph);
    assert_eq!(sccs.len(), 4);

    let order: Vec<PathBuf> = sccs.iter().flat_map(|c| c.files.clone()).collect();
    let pos_a = order.iter().position(|x| x == &a).unwrap();
    let pos_b = order.iter().position(|x| x == &b).unwrap();
    let pos_c = order.iter().position(|x| x == &c).unwrap();
    let pos_d = order.iter().position(|x| x == &d).unwrap();

    // D must compile before B and C; B and C must compile before A
    assert!(pos_d < pos_b);
    assert!(pos_d < pos_c);
    assert!(pos_b < pos_a);
    assert!(pos_c < pos_a);
}

#[test]
fn test_adv_three_node_mutual_recursion_scc() {
    // A -> B -> C -> A (3-node mutual cycle)
    let ws = TestWorkspace::new("three_node_scc");
    let a = ws.write_file("a.prism", "import b\n");
    let b = ws.write_file("b.prism", "import c\n");
    let c = ws.write_file("c.prism", "import a\n");

    let graph = DependencyGraph::from_entry(&a).expect("3-node cycle must group into SCC");
    assert_eq!(workspace_len(&graph), 3);
    assert!(graph.is_cyclic());

    let sccs = &workspace_sccs(&graph);
    assert_eq!(sccs.len(), 1);
    assert_eq!(sccs[0].files.len(), 3);
    assert!(sccs[0].files.contains(&a));
    assert!(sccs[0].files.contains(&b));
    assert!(sccs[0].files.contains(&c));
}

// ---------------------------------------------------------------------------
// The source store of a watch session (D9 and D16 of `hot-reload`)
// ---------------------------------------------------------------------------

#[test]
fn a_source_store_parses_only_changed_files() {
    let workspace = TestWorkspace::new("store_parses");
    workspace.write_file(
        "lib/math.prism",
        "package lib\npublic fun twice(n: Int): Int = n * 2\n",
    );
    let entry = workspace.write_file(
        "app/main.prism",
        "package app\nimport /lib/math\nfun main() { println(twice(1)) }\n",
    );
    let mut store = prism_resolver::SourceStore::new();
    let first = DependencyGraph::from_entry_with(&entry, &mut store).unwrap();
    assert_eq!(workspace_len(&first), 2);
    assert_eq!(store.parses(), 2);
    let again = DependencyGraph::from_entry_with(&entry, &mut store).unwrap();
    assert_eq!(workspace_len(&again), 2);
    assert_eq!(store.parses(), 2, "an unchanged file is not parsed again");

    let math = workspace.path().join("lib/math.prism");
    std::fs::write(
        &math,
        "package lib\npublic fun twice(n: Int): Int = n * 3\n",
    )
    .unwrap();
    assert!(store.has_changed(&math));
    store.invalidate(std::slice::from_ref(&math));
    DependencyGraph::from_entry_with(&entry, &mut store).unwrap();
    assert_eq!(store.parses(), 3, "only the edited file is parsed again");
    assert!(!store.has_changed(&math));
    // Rewriting the same content is no change.
    std::fs::write(
        &math,
        "package lib\npublic fun twice(n: Int): Int = n * 3\n",
    )
    .unwrap();
    assert!(!store.has_changed(&math));
}

#[test]
fn an_overlay_hides_the_disk_and_can_add_a_file() {
    let workspace = TestWorkspace::new("store_overlay");
    let entry = workspace.write_file("main.prism", "fun main() { println(1) }\n");
    let mut store = prism_resolver::SourceStore::new();
    DependencyGraph::from_entry_with(&entry, &mut store).unwrap();
    let entry_path = prism_resolver::absolute_path(&entry);
    // The overlay imports a file that exists only in the overlay.
    store.set_overlay(
        entry_path.clone(),
        "import helper\nfun main() { println(two()) }\n".to_string(),
    );
    store.set_overlay(
        workspace.path().join("helper.prism"),
        "package\npublic fun two(): Int = 2\n".replace("package\n", ""),
    );
    let graph = DependencyGraph::from_entry_with(&entry, &mut store).unwrap();
    assert_eq!(workspace_len(&graph), 2, "the new import adds a node");
    assert!(graph
        .get_node(&entry_path)
        .unwrap()
        .source
        .contains("two()"));
    assert_eq!(
        std::fs::read_to_string(&entry).unwrap(),
        "fun main() { println(1) }\n",
        "the disk is unchanged"
    );
}

/// The number of workspace files of `graph`, without the library modules every workspace file
/// reaches through the prelude (D4 of `standard-library`).
fn workspace_len(graph: &DependencyGraph) -> usize {
    graph
        .nodes
        .values()
        .filter(|node| !node.is_library())
        .count()
}

/// The clusters of `graph` that hold workspace files, in order.
fn workspace_sccs(graph: &DependencyGraph) -> Vec<prism_resolver::SccCluster> {
    graph
        .sccs()
        .iter()
        .filter(|cluster| {
            cluster
                .files
                .iter()
                .any(|file| !graph.get_node(file).unwrap().is_library())
        })
        .cloned()
        .collect()
}

#[test]
fn a_file_without_imports_has_the_two_prelude_nodes_first() {
    let ws = TestWorkspace::new("prelude_nodes");
    let main = ws.write_file(
        "main.prism",
        "fun main() {}
",
    );
    let graph = DependencyGraph::from_entry(&main).unwrap();
    let node = graph.get_node(&main).unwrap();
    assert!(node.direct_imports.is_empty());
    assert_eq!(
        node.prelude_imports,
        vec![
            PathBuf::from("prism/core.prism"),
            PathBuf::from("prism/collections.prism")
        ]
    );
    assert_eq!(graph.len(), 3);
    let order: Vec<PathBuf> = graph.sccs().iter().flat_map(|c| c.files.clone()).collect();
    assert_eq!(order.last(), Some(&main), "library nodes come first");
    for key in &node.prelude_imports {
        let library = graph.get_node(key).unwrap();
        assert!(library.is_library());
        assert!(
            library.prelude_imports.is_empty(),
            "a library file has no prelude"
        );
    }
}

#[test]
fn a_library_node_is_never_read_from_disk() {
    // A workspace directory `prism` holding `core.prism` beside the entry changes nothing.
    let ws = TestWorkspace::new("library_not_disk");
    ws.write_file("prism/core.prism", "!!! not Prism !!!");
    ws.write_file("prism/system.prism", "!!! not Prism !!!");
    let main = ws.write_file(
        "main.prism",
        "import /prism/system
fun main() {}
",
    );
    let graph = DependencyGraph::from_entry(&main).unwrap();
    let system = graph.get_node(Path::new("prism/system.prism")).unwrap();
    assert!(system.is_library());
    assert!(system.source.contains("exitProcess"));
    assert!(graph
        .get_node(Path::new("prism/core.prism"))
        .unwrap()
        .source
        .contains("intrinsic"));
}

#[test]
fn the_library_graph_holds_every_module() {
    let library = prism_library::Library::bundled();
    let graph = DependencyGraph::from_library(library).unwrap();
    assert_eq!(graph.len(), library.modules().len());
    assert!(graph.nodes.values().all(|node| node.is_library()));
}

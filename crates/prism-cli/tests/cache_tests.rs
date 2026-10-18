//! Scenarios of the build cache and of compilation avoidance (the `build-cache` spec, and
//! "Compilation Avoidance via Interface Hashes" and "Instantiation Records for Generic Body
//! Changes" of `compilation-and-abi`), run against the `prism` binary.
//!
//! Each test builds a workspace and a cache in temporary directories, runs `prism check
//! --explain-cache` with `PRISM_CACHE` set, edits files between runs and compares the
//! explanation lines (D11 of `abi-cache`). Tests are named after the scenarios they protect.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

fn unique_temp_dir(name: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "prism_cache_e2e_{}_{}_{}",
        name,
        nanos,
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

/// A workspace and a cache, both in temporary directories removed on drop.
struct Workspace {
    root: PathBuf,
    cache: PathBuf,
}

impl Workspace {
    fn new(name: &str, files: &[(&str, &str)]) -> Self {
        let base = unique_temp_dir(name);
        let workspace = Self {
            root: base.join("workspace"),
            cache: base.join("cache"),
        };
        for (path, source) in files {
            workspace.write(path, source);
        }
        workspace
    }

    fn write(&self, path: &str, source: &str) {
        let path = self.root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, source).unwrap();
    }

    /// Runs `prism <args...>` in the workspace root with `PRISM_CACHE` set to `cache`.
    fn run_with_cache(&self, cache: &str, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_prism"))
            .current_dir(&self.root)
            .env("PRISM_CACHE", cache)
            .args(args)
            .output()
            .expect("failed to execute prism CLI")
    }

    fn run(&self, args: &[&str]) -> Output {
        self.run_with_cache(self.cache.to_str().unwrap(), args)
    }

    /// The explanation lines of a successful `prism check --explain-cache <entry>`.
    fn explain(&self, entry: &str) -> Vec<String> {
        let output = self.run(&["check", "--explain-cache", entry]);
        assert_success(&output);
        explanation(&output)
    }

    /// Every file and directory under the workspace root.
    fn tree(&self) -> BTreeSet<PathBuf> {
        let mut paths = BTreeSet::new();
        collect(&self.root, &mut paths);
        paths
    }

    /// Every file under the cache's entry directories.
    fn entries(&self) -> BTreeSet<PathBuf> {
        let mut paths = BTreeSet::new();
        if self.cache.is_dir() {
            collect(&self.cache.join("v1"), &mut paths);
        }
        paths.into_iter().filter(|p| p.is_file()).collect()
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        if let Some(base) = self.root.parent() {
            fs::remove_dir_all(base).ok();
        }
    }
}

fn collect(dir: &Path, paths: &mut BTreeSet<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries {
        let path = entry.unwrap().path();
        paths.insert(path.clone());
        if path.is_dir() {
            collect(&path, paths);
        }
    }
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// The lines after `Cache: ...`, without their indentation.
fn explanation(output: &Output) -> Vec<String> {
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .skip_while(|line| !line.starts_with("Cache: "))
        .skip(1)
        .map(|line| line.trim().to_string())
        .collect()
}

/// The report without the explanation.
fn report(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .take_while(|line| !line.starts_with("Cache: "))
        .collect::<Vec<_>>()
        .join("\n")
}

fn lines(expected: &[&str]) -> Vec<String> {
    expected.iter().map(|line| line.to_string()).collect()
}

fn sorted(mut lines: Vec<String>) -> Vec<String> {
    lines.sort();
    lines
}

const TEXT: &str = "package lib\npublic fun shout(s: &String): String = s.clone()\n";
const MAIN: &str = "package app\nimport /lib/text\nfun f(s: String): String = shout(s)\n";

fn text_workspace(name: &str) -> Workspace {
    Workspace::new(name, &[("lib/text.prism", TEXT), ("app/main.prism", MAIN)])
}

// --- Compilation avoidance

/// compilation-and-abi: "Modifying non-generic function body".
#[test]
fn modifying_non_generic_function_body() {
    let workspace = text_workspace("body");
    assert_eq!(
        workspace.explain("app/main.prism"),
        lines(&["checked lib/text.prism", "checked app/main.prism"])
    );
    workspace.write(
        "lib/text.prism",
        "package lib\npublic fun shout(s: &String): String {\n    val t = s.clone()\n    return t\n}\n",
    );
    assert_eq!(
        workspace.explain("app/main.prism"),
        lines(&["checked lib/text.prism", "reused app/main.prism"])
    );
}

/// compilation-and-abi: "Modifying public function signature".
#[test]
fn modifying_public_function_signature() {
    let workspace = Workspace::new(
        "signature",
        &[
            ("lib/text.prism", TEXT),
            (
                "app/main.prism",
                "package app\nimport /lib/text\npublic fun greet(s: String): String = shout(s)\n",
            ),
            (
                "app/cli.prism",
                "package app\nimport main\nfun run(): String = greet(\"a\")\n",
            ),
        ],
    );
    assert_eq!(
        workspace.explain("app/cli.prism"),
        lines(&[
            "checked lib/text.prism",
            "checked app/main.prism",
            "checked app/cli.prism"
        ])
    );
    workspace.write(
        "lib/text.prism",
        "package lib\npublic fun shout(s: &String, n: Int = 1): String = s.clone()\n",
    );
    assert_eq!(
        workspace.explain("app/cli.prism"),
        lines(&[
            "checked lib/text.prism",
            "checked app/main.prism",
            "reused app/cli.prism"
        ])
    );
}

/// compilation-and-abi: "An incompatible signature change is reported in the importer".
#[test]
fn an_incompatible_signature_change_is_reported_in_the_importer() {
    let workspace = text_workspace("incompatible");
    workspace.explain("app/main.prism");
    workspace.write(
        "lib/text.prism",
        "package lib\npublic fun shout(s: Int): String = \"\"\n",
    );
    let output = workspace.run(&["check", "--explain-cache", "app/main.prism"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("error[E4004]"), "{}", stderr);
    assert!(stderr.contains("main.prism"), "{}", stderr);
}

/// compilation-and-abi: "Marking a parameter escaping changes the hashes".
#[test]
fn marking_a_parameter_escaping_changes_the_hashes() {
    let workspace = Workspace::new(
        "escaping",
        &[
            (
                "lib/store.prism",
                "package lib\npublic fun store(f: () -> Unit): Int = 1\n",
            ),
            (
                "app/main.prism",
                "package app\nimport /lib/store\nfun f(): Int = store { }\n",
            ),
        ],
    );
    assert_eq!(
        workspace.explain("app/main.prism"),
        lines(&["checked lib/store.prism", "checked app/main.prism"])
    );
    workspace.write(
        "lib/store.prism",
        "package lib\npublic fun store(f: escaping () -> Unit): Int = 1\n",
    );
    assert_eq!(
        workspace.explain("app/main.prism"),
        lines(&["checked lib/store.prism", "checked app/main.prism"])
    );
}

/// compilation-and-abi: "Modifying private helper function".
#[test]
fn modifying_private_helper_function() {
    let with_helper = |helper: &str| {
        format!(
            "package lib\n{}public fun shout(s: &String): String = s.clone()\n",
            helper
        )
    };
    let workspace = Workspace::new(
        "private",
        &[
            ("lib/text.prism", &with_helper("fun a(): Int = 1\n")),
            ("app/main.prism", MAIN),
        ],
    );
    workspace.explain("app/main.prism");
    // Added, modified, then removed (`a`); returning to an earlier content would be a hit.
    for helper in [
        "fun a(): Int = 1\nfun b(): Int = 2\n",
        "fun a(): Int = 1\nfun b(n: Int): Int = n\n",
        "fun b(n: Int): Int = n\n",
    ] {
        workspace.write("lib/text.prism", &with_helper(helper));
        assert_eq!(
            workspace.explain("app/main.prism"),
            lines(&["checked lib/text.prism", "reused app/main.prism"]),
            "after changing the helper to {:?}",
            helper
        );
    }
}

/// compilation-and-abi: "A failing file is checked again".
#[test]
fn a_failing_file_is_checked_again() {
    let workspace = Workspace::new(
        "failing",
        &[
            ("lib/text.prism", TEXT),
            (
                "app/main.prism",
                "package app\nimport /lib/text\nfun f(s: String): Int = shout(s)\n",
            ),
        ],
    );
    let first = workspace.run(&["check", "app/main.prism"]);
    let entries = workspace.entries();
    let second = workspace.run(&["check", "app/main.prism"]);
    assert!(!first.status.success() && !second.status.success());
    let stderr = String::from_utf8_lossy(&second.stderr);
    assert!(stderr.contains("error[E4005]") || stderr.contains("error[E4002]"));
    assert_eq!(first.stderr, second.stderr);
    assert_eq!(
        workspace.entries(),
        entries,
        "a failing file records nothing"
    );
    assert_eq!(entries.len(), 1, "only lib/text passed: {:?}", entries);
}

/// compilation-and-abi: "Cached and uncached checks agree".
#[test]
fn cached_and_uncached_checks_agree() {
    let workspace = text_workspace("agree");
    let uncached = workspace.run(&["check", "--no-cache", "."]);
    workspace.run(&["check", "."]);
    let warm = workspace.run(&["check", "."]);
    assert_success(&warm);
    assert_eq!(uncached.stdout, warm.stdout);
    assert_eq!(uncached.status.code(), warm.status.code());

    workspace.write("app/main.prism", "package app\nfun f(): Int = \"s\"\n");
    let uncached = workspace.run(&["check", "--no-cache", "."]);
    let cached = workspace.run(&["check", "."]);
    assert!(!cached.status.success());
    assert_eq!(uncached.stdout, cached.stdout);
    assert_eq!(uncached.stderr, cached.stderr);
    assert_eq!(uncached.status.code(), cached.status.code());
}

// --- Instantiation records

const BOX: &str =
    "package lib\npublic fun <T> keep(x: T): T = x\npublic fun <T> drop(x: T): Int = 1\n";

/// compilation-and-abi: "Changing generic function body".
#[test]
fn changing_generic_function_body() {
    let workspace = Workspace::new(
        "generic",
        &[
            ("lib/box.prism", BOX),
            (
                "app/a.prism",
                "package app\nimport /lib/box\nfun f(): Int = keep(1)\n",
            ),
            (
                "app/b.prism",
                "package app\nimport /lib/box\nfun g(): Int = drop(\"s\")\n",
            ),
        ],
    );
    assert_eq!(
        sorted(workspace.explain(".")),
        sorted(lines(&[
            "checked lib/box.prism",
            "checked app/a.prism",
            "instantiate lib/box::keep<Int> for app/a.prism",
            "checked app/b.prism",
            "instantiate lib/box::drop<String> for app/b.prism",
        ]))
    );
    workspace.write(
        "lib/box.prism",
        &BOX.replace("): T = x", "): T {\n    val y = x\n    return y\n}"),
    );
    assert_eq!(
        sorted(workspace.explain(".")),
        sorted(lines(&[
            "checked lib/box.prism",
            "reused app/a.prism",
            "instantiate lib/box::keep<Int> for app/a.prism",
            "reused app/b.prism",
            "reused instance lib/box::drop<String> for app/b.prism",
        ]))
    );
}

/// compilation-and-abi: "Changing a generic signature instantiates again".
#[test]
fn changing_a_generic_signature_instantiates_again() {
    let workspace = Workspace::new(
        "generic_signature",
        &[
            ("lib/box.prism", BOX),
            (
                "app/a.prism",
                "package app\nimport /lib/box\nfun f(): Int = drop(1)\n",
            ),
        ],
    );
    assert_eq!(
        workspace.explain("app/a.prism"),
        lines(&[
            "checked lib/box.prism",
            "checked app/a.prism",
            "instantiate lib/box::drop<Int> for app/a.prism",
        ])
    );
    workspace.write("lib/box.prism", &BOX.replace("drop(x: T)", "drop(x: &T)"));
    workspace.write(
        "app/a.prism",
        "package app\nimport /lib/box\nfun f(): Int = drop(&1)\n",
    );
    assert_eq!(
        workspace.explain("app/a.prism"),
        lines(&[
            "checked lib/box.prism",
            "checked app/a.prism",
            "instantiate lib/box::drop<Int> for app/a.prism",
        ])
    );
}

/// compilation-and-abi: "Polymorphic calls are not recorded".
#[test]
fn polymorphic_calls_are_not_recorded() {
    let workspace = Workspace::new(
        "polymorphic",
        &[
            ("lib/box.prism", BOX),
            (
                "app/a.prism",
                "package app\nimport /lib/box\nfun <T> wrap(x: T): T = keep(x)\n",
            ),
        ],
    );
    assert_eq!(
        workspace.explain("app/a.prism"),
        lines(&["checked lib/box.prism", "checked app/a.prism"])
    );
}

/// build-cache: "Explaining an instance of a generic extension".
#[test]
fn explaining_an_instance_of_a_generic_extension() {
    let library = "package lib\npublic struct Box<T>(val value: T)\n\
                   public fun <T> Box<T>.size(): Int = 1\n";
    let workspace = Workspace::new(
        "extension_instance",
        &[
            ("lib/box.prism", library),
            (
                "app/main.prism",
                "package app\nimport /lib/box\nfun f(): Int = Box(1).size()\n",
            ),
        ],
    );
    assert_eq!(
        workspace.explain("app/main.prism"),
        lines(&[
            "checked lib/box.prism",
            "checked app/main.prism",
            "instantiate lib/box::Box.size<Int> for app/main.prism",
        ])
    );
    workspace.write(
        "lib/box.prism",
        &library.replace("<T> Box<T>", "<U> Box<U>"),
    );
    assert_eq!(
        workspace.explain("app/main.prism"),
        lines(&[
            "checked lib/box.prism",
            "reused app/main.prism",
            "reused instance lib/box::Box.size<Int> for app/main.prism",
        ])
    );
}

// --- The cache itself

/// build-cache: "Cache directory override".
#[test]
fn cache_directory_override() {
    let workspace = text_workspace("override");
    let output = workspace.run(&["check", "--explain-cache", "app/main.prism"]);
    assert_success(&output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(&format!("Cache: {}", workspace.cache.display())),
        "{}",
        stdout
    );
    assert_eq!(workspace.entries().len(), 2);
    assert!(workspace.cache.join("CACHEDIR.TAG").is_file());
}

/// build-cache: "Checking writes nothing in the workspace".
#[test]
fn checking_writes_nothing_in_the_workspace() {
    let workspace = text_workspace("nothing_written");
    let before = workspace.tree();
    workspace.explain("app/main.prism");
    workspace.explain(".");
    assert_eq!(workspace.tree(), before);
}

/// build-cache: "Cache shared across checkouts".
#[test]
fn cache_shared_across_checkouts() {
    let first = text_workspace("checkout_one");
    first.explain("app/main.prism");
    let second = text_workspace("checkout_two");
    let output = second.run_with_cache(
        first.cache.to_str().unwrap(),
        &["check", "--explain-cache", "app/main.prism"],
    );
    assert_success(&output);
    assert_eq!(
        explanation(&output),
        lines(&["reused lib/text.prism", "reused app/main.prism"])
    );
}

/// build-cache: "Unchanged workspace reuses every file", which is also the system-invariants
/// scenario "Identical inputs produce cache hit" for Phase 2.
#[test]
fn unchanged_workspace_reuses_every_file() {
    let workspace = text_workspace("unchanged");
    workspace.explain(".");
    assert_eq!(
        workspace.explain("."),
        lines(&["reused lib/text.prism", "reused app/main.prism"])
    );
}

/// build-cache: "Comment-only edit re-checks the file alone".
#[test]
fn comment_only_edit_re_checks_the_file_alone() {
    let workspace = text_workspace("comment");
    workspace.explain("app/main.prism");
    workspace.write("lib/text.prism", &format!("{}// a note\n", TEXT));
    assert_eq!(
        workspace.explain("app/main.prism"),
        lines(&["checked lib/text.prism", "reused app/main.prism"])
    );
}

/// build-cache: "Corrupt entry is a miss".
#[test]
fn corrupt_entry_is_a_miss() {
    let workspace = text_workspace("corrupt");
    workspace.explain("app/main.prism");
    let entries = workspace.entries();
    assert_eq!(entries.len(), 2);
    for (index, entry) in entries.iter().enumerate() {
        if index == 0 {
            let text = fs::read(entry).unwrap();
            fs::write(entry, &text[..text.len() / 2]).unwrap();
        } else {
            fs::write(entry, "garbage\n").unwrap();
        }
    }
    assert_eq!(
        workspace.explain("app/main.prism"),
        lines(&["checked lib/text.prism", "checked app/main.prism"])
    );
    assert_eq!(
        workspace.explain("app/main.prism"),
        lines(&["reused lib/text.prism", "reused app/main.prism"])
    );
}

/// build-cache: "Unwritable cache directory".
#[test]
fn unwritable_cache_directory() {
    let workspace = text_workspace("unwritable");
    let file = workspace.root.parent().unwrap().join("not-a-directory");
    fs::write(&file, "a regular file").unwrap();
    let without = workspace.run(&["check", "--no-cache", "app/main.prism"]);
    for _ in 0..2 {
        let output = workspace.run_with_cache(
            file.to_str().unwrap(),
            &["check", "--explain-cache", "app/main.prism"],
        );
        assert_success(&output);
        assert!(output.stderr.is_empty());
        assert_eq!(report(&output), report(&without));
        assert_eq!(
            explanation(&output),
            lines(&["checked lib/text.prism", "checked app/main.prism"])
        );
    }
}

/// build-cache: "Concurrent checks share a cache".
#[test]
fn concurrent_checks_share_a_cache() {
    let workspace = Workspace::new(
        "concurrent",
        &[
            ("lib/box.prism", BOX),
            ("lib/text.prism", TEXT),
            (
                "app/main.prism",
                "package app\nimport /lib/text\nimport /lib/box\nfun f(s: String): String = keep(shout(s))\n",
            ),
        ],
    );
    let children: Vec<_> = (0..8)
        .map(|_| {
            Command::new(env!("CARGO_BIN_EXE_prism"))
                .current_dir(&workspace.root)
                .env("PRISM_CACHE", &workspace.cache)
                .args(["check", "."])
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .unwrap()
        })
        .collect();
    let outputs: Vec<Output> = children
        .into_iter()
        .map(|child| child.wait_with_output().unwrap())
        .collect();
    for output in &outputs {
        assert_success(output);
        assert_eq!(output.stdout, outputs[0].stdout);
    }
    assert_eq!(
        workspace.explain("."),
        lines(&[
            "reused lib/box.prism",
            "reused lib/text.prism",
            "reused app/main.prism",
            "reused instance lib/box::keep<String> for app/main.prism",
        ])
    );
}

/// build-cache: "Checking without the cache".
#[test]
fn checking_without_the_cache() {
    let workspace = text_workspace("without");
    let output = workspace.run(&["check", "--no-cache", "--explain-cache", "app/main.prism"]);
    assert_success(&output);
    assert!(!workspace.cache.exists(), "--no-cache writes nothing");

    workspace.explain("app/main.prism");
    let entries = workspace.entries();
    for output in [
        workspace.run(&["check", "--explain-cache", "--no-cache", "app/main.prism"]),
        workspace.run_with_cache("off", &["check", "--explain-cache", "app/main.prism"]),
    ] {
        assert_success(&output);
        assert!(String::from_utf8_lossy(&output.stdout).contains("Cache: off"));
        assert_eq!(
            explanation(&output),
            lines(&["checked lib/text.prism", "checked app/main.prism"])
        );
    }
    assert_eq!(workspace.entries(), entries);
}

/// build-cache: "Explaining a warm check".
#[test]
fn explaining_a_warm_check() {
    let workspace = Workspace::new(
        "explain",
        &[
            ("lib/box.prism", BOX),
            (
                "app/main.prism",
                "package app\nimport /lib/box\nfun f(): Int = keep(1)\n",
            ),
        ],
    );
    assert_eq!(
        workspace.explain("app/main.prism"),
        lines(&[
            "checked lib/box.prism",
            "checked app/main.prism",
            "instantiate lib/box::keep<Int> for app/main.prism",
        ])
    );
    assert_eq!(
        workspace.explain("app/main.prism"),
        lines(&[
            "reused lib/box.prism",
            "reused app/main.prism",
            "reused instance lib/box::keep<Int> for app/main.prism",
        ])
    );
}

// --- Reachable type definitions (D2 of `codegen-aggregates`)

const GEO_CORE: &str = "package geo\n\
     public struct Vec : Copy { val x: Int }\n\
     public fun origin(): Vec = Vec(0)\n";
const GEO_SHAPES: &str = "package geo\nimport /geo/core\n\
     public struct Circle : Copy { val center: Vec, val r: Int }\n\
     public fun unit(): Circle = Circle(origin(), 1)\n";
const GEO_MAIN: &str = "package app\nimport /geo/shapes\nfun main() { println(unit().r) }\n";

fn geo_workspace(name: &str) -> Workspace {
    let workspace = Workspace::new(
        name,
        &[
            ("geo/core.prism", GEO_CORE),
            ("geo/shapes.prism", GEO_SHAPES),
            ("app/main.prism", GEO_MAIN),
        ],
    );
    assert_eq!(
        workspace.explain("app/main.prism"),
        lines(&[
            "checked geo/core.prism",
            "checked geo/shapes.prism",
            "checked app/main.prism"
        ])
    );
    workspace
}

/// The standard output of `prism run app/main.prism`, which must succeed.
fn run_main(workspace: &Workspace) -> String {
    let output = workspace.run(&["run", "app/main.prism"]);
    assert_success(&output);
    String::from_utf8_lossy(&output.stdout).to_string()
}

/// compilation-and-abi: "A field added to a transitively reached type". `app/main` imports only
/// `geo/shapes`, whose ABI records the definition of `geo/core`'s `Vec`, so a field added to
/// `Vec` changes that ABI too and re-checks `app/main`.
#[test]
fn a_field_added_to_a_transitively_reached_type() {
    let workspace = geo_workspace("reachable_field");
    assert_eq!(run_main(&workspace), "1\n");
    workspace.write(
        "geo/core.prism",
        "package geo\n\
         public struct Vec : Copy { val x: Int, val y: Int }\n\
         public fun origin(): Vec = Vec(0, 0)\n",
    );
    assert_eq!(
        workspace.explain("app/main.prism"),
        lines(&[
            "checked geo/core.prism",
            "checked geo/shapes.prism",
            "checked app/main.prism"
        ])
    );
    assert_eq!(run_main(&workspace), "1\n");
}

/// compilation-and-abi: "A body change in the defining file".
#[test]
fn a_body_change_in_the_defining_file() {
    let workspace = geo_workspace("reachable_body");
    workspace.write("geo/core.prism", &GEO_CORE.replace("Vec(0)\n", "Vec(5)\n"));
    assert_eq!(
        workspace.explain("app/main.prism"),
        lines(&[
            "checked geo/core.prism",
            "reused geo/shapes.prism",
            "reused app/main.prism"
        ])
    );
    assert_eq!(run_main(&workspace), "1\n");
}

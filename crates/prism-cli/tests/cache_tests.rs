//! Scenarios of the build cache and of compilation avoidance (the `build-cache` spec, and
//! "Compilation Avoidance via ABI Hashes", "Instances Are Planned per Program", "Instances Are
//! Keyed by Their Record and Type Arguments" and "Code Generation Avoidance" of
//! `compilation-and-abi`), run against the `prism` binary.
//!
//! Each test builds a workspace and a cache in temporary directories, runs `prism check
//! --explain-cache` or `prism run --explain-cache` with `PRISM_CACHE` set, edits files between
//! runs and compares the explanation lines (D11 of `abi-cache`, D10 of `instance-cache`). Tests
//! are named after the scenarios they protect.

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
///
/// The cache starts with the entries of the bundled library, which a check and a run of a
/// one-line program in another directory write: the library's entries are shared by every
/// workspace (the `build-cache` spec, "Library entries are shared between workspaces"), so
/// [`Workspace::entries`] leaves them out and the explanations leave out the library's lines.
struct Workspace {
    root: PathBuf,
    cache: PathBuf,
    /// The entries the library wrote before the test.
    library_entries: BTreeSet<PathBuf>,
}

impl Workspace {
    fn new(name: &str, files: &[(&str, &str)]) -> Self {
        let base = unique_temp_dir(name);
        let mut workspace = Self {
            root: base.join("workspace"),
            cache: base.join("cache"),
            library_entries: BTreeSet::new(),
        };
        let warm_up = base.join("warm-up");
        fs::create_dir_all(&warm_up).unwrap();
        fs::write(warm_up.join("main.prism"), "fun main() {}\n").unwrap();
        for command in ["check", "run"] {
            let output = Command::new(env!("CARGO_BIN_EXE_prism"))
                .current_dir(&warm_up)
                .env("PRISM_CACHE", &workspace.cache)
                .args([command, "main.prism"])
                .output()
                .expect("failed to execute prism CLI");
            assert_success(&output);
        }
        workspace.library_entries = workspace.all_entries();
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

    /// A successful `prism run --explain-cache <entry>`: its standard output and the
    /// explanation lines it printed to stderr.
    fn run_explain(&self, entry: &str) -> (String, Vec<String>) {
        let output = self.run(&["run", "--explain-cache", entry]);
        assert_success(&output);
        (
            String::from_utf8_lossy(&output.stdout).to_string(),
            run_explanation(&output),
        )
    }

    /// Every file and directory under the workspace root.
    fn tree(&self) -> BTreeSet<PathBuf> {
        let mut paths = BTreeSet::new();
        collect(&self.root, &mut paths);
        paths
    }

    /// Every file under the cache's entry directories, but the library's.
    fn entries(&self) -> BTreeSet<PathBuf> {
        self.all_entries()
            .difference(&self.library_entries)
            .cloned()
            .collect()
    }

    /// Every file under the cache's entry directories.
    fn all_entries(&self) -> BTreeSet<PathBuf> {
        let mut paths = BTreeSet::new();
        if self.cache.is_dir() {
            collect(&self.cache.join("v2"), &mut paths);
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

/// The lines after `Cache: ...`, without their indentation and without the library's.
fn explanation(output: &Output) -> Vec<String> {
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .skip_while(|line| !line.starts_with("Cache: "))
        .skip(1)
        .map(|line| line.trim().to_string())
        .filter(|line| !is_library_line(line))
        .collect()
}

/// The explanation lines a `prism run --explain-cache` printed to stderr, after `Cache: ...`,
/// without the library's.
fn run_explanation(output: &Output) -> Vec<String> {
    String::from_utf8_lossy(&output.stderr)
        .lines()
        .skip_while(|line| !line.starts_with("Cache: "))
        .skip(1)
        .take_while(|line| line.starts_with("  "))
        .map(|line| line.trim().to_string())
        .filter(|line| !is_library_line(line))
        .collect()
}

/// Whether an explanation line is about a library module or an instance of a library generic.
fn is_library_line(line: &str) -> bool {
    line.split_once(' ')
        .is_some_and(|(_, rest)| rest.trim_start_matches("instance ").starts_with("prism/"))
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

/// The files of "Editing a getter recompiles nothing downstream" and "A property's type is part
/// of the ABI" (`compilation-and-abi`): a library exporting an extension property with a getter,
/// and an importer reading it.
const GEO: &str = "package lib\npublic struct Box : Copy { val w: Int }\npublic val Box.twice: Int get() = w * 2\n";
const BOX_MAIN: &str =
    "package app\nimport /lib/geo\nimport /prism/core\nfun main() { println(Box(3).twice) }\n";

/// compilation-and-abi: "Editing a getter recompiles nothing downstream".
#[test]
fn editing_a_getter_recompiles_nothing_downstream() {
    let workspace = Workspace::new(
        "getter",
        &[("lib/geo.prism", GEO), ("app/main.prism", BOX_MAIN)],
    );
    let (stdout, _) = workspace.run_explain("app/main.prism");
    assert_eq!(stdout, "6\n");
    workspace.write("lib/geo.prism", &GEO.replace("w * 2", "w + w + 1"));
    let (stdout, explained) = workspace.run_explain("app/main.prism");
    assert_eq!(stdout, "7\n");
    assert!(
        explained.contains(&"checked lib/geo.prism".to_string()),
        "{:?}",
        explained
    );
    assert!(
        explained.contains(&"reused app/main.prism".to_string()),
        "{:?}",
        explained
    );
}

/// compilation-and-abi: "A property's type is part of the ABI".
#[test]
fn a_property_s_type_is_part_of_the_abi() {
    let workspace = Workspace::new(
        "property-type",
        &[("lib/geo.prism", GEO), ("app/main.prism", BOX_MAIN)],
    );
    workspace.explain("app/main.prism");
    workspace.write(
        "lib/geo.prism",
        &GEO.replace("twice: Int get() = w * 2", "twice: Double get() = 2.0"),
    );
    assert_eq!(
        workspace.explain("app/main.prism"),
        lines(&["checked lib/geo.prism", "checked app/main.prism"])
    );
}

/// compilation-and-abi: "Instances of generic members": `isEmpty`, `isNotEmpty` and `indices`
/// are generics of `/prism/core`, which files call without importing it, directly or through
/// another generic, and whose instances are planned for the program and cached like an import's
/// (D4 of `core-library-in-prism`).
#[test]
fn instances_of_generic_members() {
    let workspace = Workspace::new(
        "generic_members",
        &[
            (
                "lib/check.prism",
                "package lib\npublic fun empty(xs: &List<String>): Bool = xs.isEmpty()\npublic fun <T> count(xs: &List<T>): Int {\n    var n = 0\n    for (i in xs.indices) n += 1\n    return n\n}\n",
            ),
            (
                "app/main.prism",
                "package app\nimport /lib/check\nimport /prism/collections\nimport /prism/core\nfun main() {\n    val xs = listOf(\"a\")\n    println(xs.isNotEmpty() && !empty(xs) && count(xs) == 1)\n}\n",
            ),
        ],
    );
    let all_lines = |output: &Output| -> Vec<String> {
        String::from_utf8_lossy(&output.stderr)
            .lines()
            .skip_while(|line| !line.starts_with("Cache: "))
            .skip(1)
            .take_while(|line| line.starts_with("  "))
            .map(|line| line.trim().to_string())
            .collect()
    };
    for verb in ["instantiate", "reused instance"] {
        let output = workspace.run(&["run", "--explain-cache", "app/main.prism"]);
        assert_success(&output);
        assert_eq!(String::from_utf8_lossy(&output.stdout), "true\n");
        let explained = all_lines(&output);
        for instance in [
            "prism/core::List.isEmpty<String>",
            "prism/core::List.isNotEmpty<String>",
            "prism/core::List.indices<String>",
            "lib/check::count<String>",
        ] {
            let line = format!("{} {}", verb, instance);
            assert!(explained.contains(&line), "{}: {:?}", line, explained);
        }
    }
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

/// compilation-and-abi: "Marking a parameter attached changes the hashes".
#[test]
fn marking_a_parameter_attached_changes_the_hashes() {
    let workspace = Workspace::new(
        "attached",
        &[
            (
                "lib/store.prism",
                "package lib\npublic fun store(f: escaping () -> Unit): Int = 1\n",
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
        "package lib\npublic fun store(f: attached () -> Unit): Int = 1\n",
    );
    assert_eq!(
        workspace.explain("app/main.prism"),
        lines(&["checked lib/store.prism", "checked app/main.prism"])
    );
}

/// compilation-and-abi: "Making a parameter vararg changes the hash".
#[test]
fn making_a_parameter_vararg_changes_the_hash() {
    let workspace = Workspace::new(
        "vararg",
        &[
            (
                "lib/math.prism",
                "package lib\npublic fun sum(xs: List<Int>): Int = xs.size\n",
            ),
            (
                "app/main.prism",
                "package app\nimport /prism/collections\nimport /lib/math\nfun f(): Int = sum(listOf(1))\n",
            ),
        ],
    );
    assert!(workspace
        .explain("app/main.prism")
        .contains(&"checked app/main.prism".to_string()));
    workspace.write(
        "lib/math.prism",
        "package lib\npublic fun sum(vararg xs: Int): Int = xs.size\n",
    );
    let output = workspace.run(&["check", "--explain-cache", "app/main.prism"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("error[E4004]"), "{}", stderr);
    // A reused verdict would have passed: the importer was checked again.
    assert!(stderr.contains("main.prism"), "{}", stderr);
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

// --- Instances planned per program (D7 and D10 of `instance-cache`)

const BOX: &str =
    "package lib\npublic fun <T> drop(x: T): Int = 1\npublic fun <T> keep(x: T): T = x\n";

/// compilation-and-abi: "Polymorphic calls are instantiated with their caller".
#[test]
fn polymorphic_calls_are_instantiated_with_their_caller() {
    let workspace = Workspace::new(
        "polymorphic",
        &[
            ("lib/box.prism", BOX),
            (
                "app/a.prism",
                "package app\nimport /lib/box\npublic fun <T> wrap(x: T): T = keep(x)\n",
            ),
            (
                "app/main.prism",
                "package app\nimport /prism/core\nimport a\nfun main() { println(wrap(1)) }\n",
            ),
        ],
    );
    assert_eq!(
        workspace.explain("app/main.prism"),
        lines(&[
            "checked lib/box.prism",
            "checked app/a.prism",
            "checked app/main.prism",
            "instantiate app/a::wrap<Int>",
            "instantiate lib/box::keep<Int>",
        ])
    );
}

/// compilation-and-abi: "Transitive instances through a module the caller does not import".
#[test]
fn transitive_instances_through_a_module_the_caller_does_not_import() {
    let workspace = Workspace::new(
        "transitive",
        &[
            (
                "lib/util.prism",
                "package lib\npublic struct Pair<A, B>(val a: A, val b: B)\n\
                 public fun <T : Copy> twin(x: T): Pair<T, T> = Pair(x, x)\n",
            ),
            (
                "lib/box.prism",
                "package lib\nimport /lib/util\n\
                 public fun <T : Copy> first(x: T): T = twin(x).a\n",
            ),
            (
                "app/main.prism",
                "package app\nimport /prism/core\nimport /lib/box\nfun main() { println(first(7)) }\n",
            ),
        ],
    );
    assert_eq!(
        workspace.explain("app/main.prism"),
        lines(&[
            "checked lib/util.prism",
            "checked lib/box.prism",
            "checked app/main.prism",
            "instantiate lib/box::first<Int>",
            "instantiate lib/util::twin<Int>",
        ])
    );
    assert_eq!(
        workspace.explain("app/main.prism"),
        lines(&[
            "reused lib/util.prism",
            "reused lib/box.prism",
            "reused app/main.prism",
            "instantiate lib/box::first<Int>",
            "instantiate lib/util::twin<Int>",
        ])
    );
}

/// build-cache: "Two callers of one instance".
#[test]
fn two_callers_of_one_instance() {
    let workspace = Workspace::new(
        "two_callers",
        &[
            ("lib/box.prism", BOX),
            (
                "app/a.prism",
                "package app\nimport /lib/box\npublic fun a(): Int = keep(1)\n",
            ),
            (
                "app/b.prism",
                "package app\nimport /lib/box\npublic fun b(): Int = keep(1)\n",
            ),
            (
                "app/main.prism",
                "package app\nimport /prism/core\nimport a\nimport b\nfun main() { println(a() + b()) }\n",
            ),
        ],
    );
    let explanation = workspace.explain("app/main.prism");
    assert_eq!(explanation.len(), 5, "{:?}", explanation);
    assert_eq!(explanation[4], "instantiate lib/box::keep<Int>");
    assert!(explanation[..4]
        .iter()
        .all(|line| line.starts_with("checked ")));
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
            "instantiate lib/box::Box.size<Int>",
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
            "instantiate lib/box::Box.size<Int>",
        ])
    );
}

const PING: &str = "package a\nimport /a/y\n\
     public struct Box<T>(val v: T)\n\
     public fun <T> ping(x: T, n: Int): Int = if (n == 0) 0 else pong(Box(x), n - 1)\n";
const PONG: &str = "package a\nimport /a/x\npublic fun <T> pong(x: T, n: Int): Int = ping(x, n)\n";

/// generics: "Polymorphic recursion found from reused verdicts".
#[test]
fn polymorphic_recursion_found_from_reused_verdicts() {
    let workspace = Workspace::new("recursion", &[("a/x.prism", PING), ("a/y.prism", PONG)]);
    let first = workspace.run(&["check", "a/x.prism"]);
    let entries = workspace.entries();
    let second = workspace.run(&["check", "a/x.prism"]);
    for output in [&first, &second] {
        assert_eq!(output.status.code(), Some(1));
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("error[E6004]"), "{}", stderr);
        assert!(stderr.contains("'a/x::ping'") && stderr.contains("'a/y::pong'"));
    }
    assert_eq!(first.stderr, second.stderr);
    assert!(!entries.is_empty(), "each file alone passes Phase 2");
    assert_eq!(
        workspace.entries(),
        entries,
        "the second check writes no entry"
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
    let first = program_workspace("checkout_one");
    first.explain("app/main.prism");
    first.run_explain("app/main.prism");
    let second = program_workspace("checkout_two");
    let output = second.run_with_cache(
        first.cache.to_str().unwrap(),
        &["check", "--explain-cache", "app/main.prism"],
    );
    assert_success(&output);
    assert_eq!(
        explanation(&output),
        lines(&[
            "reused lib/box.prism",
            "reused app/main.prism",
            "reused instance lib/box::keep<Int>"
        ])
    );
    let output = second.run_with_cache(
        first.cache.to_str().unwrap(),
        &["run", "--explain-cache", "app/main.prism"],
    );
    assert_success(&output);
    assert_eq!(String::from_utf8_lossy(&output.stdout), "1\n");
    assert_eq!(run_explanation(&output), lines(&WARM));
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
            "instantiate lib/box::keep<String>",
        ])
    );
}

/// build-cache: "Checking without the cache".
#[test]
fn checking_without_the_cache() {
    let workspace = text_workspace("without");
    let before = workspace.all_entries();
    let output = workspace.run(&["check", "--no-cache", "--explain-cache", "app/main.prism"]);
    assert_success(&output);
    assert_eq!(workspace.all_entries(), before, "--no-cache writes nothing");

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

// --- Reachable type definitions (D2 of `codegen-aggregates`)

const GEO_CORE: &str = "package geo\n\
     public struct Vec : Copy { val x: Int }\n\
     public fun origin(): Vec = Vec(0)\n";
const GEO_SHAPES: &str = "package geo\nimport /geo/core\n\
     public struct Circle : Copy { val center: Vec, val r: Int }\n\
     public fun unit(): Circle = Circle(origin(), 1)\n";
const GEO_MAIN: &str =
    "package app\nimport /prism/core\nimport /geo/shapes\nfun main() { println(unit().r) }\n";

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

// --- prism run with the cache (D9 of `instance-cache`)

const KEEP: &str = "package lib\npublic fun <T> keep(x: T): T = x\n";
const KEEP_MAIN: &str =
    "package app\nimport /prism/core\nimport /lib/box\nfun main() { println(keep(1)) }\n";
const COLD: [&str; 3] = [
    "checked lib/box.prism",
    "checked app/main.prism",
    "instantiate lib/box::keep<Int>",
];
const WARM: [&str; 3] = [
    "reused lib/box.prism",
    "reused app/main.prism",
    "reused instance lib/box::keep<Int>",
];

/// The program of "Explaining a warm check": `app/main` prints `keep(1)` of `lib/box`.
fn program_workspace(name: &str) -> Workspace {
    Workspace::new(
        name,
        &[("lib/box.prism", KEEP), ("app/main.prism", KEEP_MAIN)],
    )
}

/// build-cache: "Explaining a warm check".
#[test]
fn explaining_a_warm_check() {
    let workspace = program_workspace("warm_check");
    assert_eq!(workspace.explain("app/main.prism"), lines(&COLD));
    let output = workspace.run(&["run", "app/main.prism"]);
    assert_success(&output);
    assert_eq!(workspace.explain("app/main.prism"), lines(&WARM));
    // The library modules come first, reused here: the workspace's cache already holds the
    // entries another workspace wrote for them.
    let output = workspace.run(&["check", "--explain-cache", "app/main.prism"]);
    let all = all_explanation_lines(&output);
    // `/prism/core` alone: the program imports no library module (D3 of
    // `core-library-in-prism`).
    assert_eq!(
        all[..2],
        lines(&["reused prism/core.prism", "reused lib/box.prism"])
    );
}

/// The lines after `Cache: ...` of a `prism check --explain-cache`, the library's included.
fn all_explanation_lines(output: &Output) -> Vec<String> {
    assert_success(output);
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .skip_while(|line| !line.starts_with("Cache: "))
        .skip(1)
        .map(|line| line.trim().to_string())
        .collect()
}

/// standard-library: "Unreached modules are not parsed", and build-cache: "Explaining a warm
/// check", on a cache no workspace used before.
#[test]
fn unreached_modules_are_not_parsed() {
    let base = unique_temp_dir("unreached");
    fs::write(base.join("main.prism"), "fun main() {}\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_prism"))
        .current_dir(&base)
        .env("PRISM_CACHE", base.join("cache"))
        .args(["check", "--explain-cache", "main.prism"])
        .output()
        .unwrap();
    assert_eq!(
        all_explanation_lines(&output),
        lines(&["checked prism/core.prism", "checked main.prism"])
    );
    fs::remove_dir_all(&base).ok();
}

/// standard-library: "An imported module is parsed" (`explicit-library`): an imported library
/// module is parsed beside the member modules, and no other.
#[test]
fn an_imported_module_is_parsed() {
    let base = unique_temp_dir("imported_module");
    fs::write(
        base.join("main.prism"),
        "import /prism/compose/runtime\n\nfun main() {}\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_prism"))
        .current_dir(&base)
        .env("PRISM_CACHE", base.join("cache"))
        .args(["check", "--explain-cache", "main.prism"])
        .output()
        .unwrap();
    assert_eq!(
        all_explanation_lines(&output),
        lines(&[
            "checked prism/compose/runtime.prism",
            "checked prism/core.prism",
            "checked main.prism",
        ])
    );
    fs::remove_dir_all(&base).ok();
}

/// build-cache: "Reordering imports changes only the key" (`explicit-library`, D9 and D14):
/// swapping a file's two imports checks that file again, reuses its importer, and keeps its
/// diagnostics and ABI hash.
#[test]
fn reordering_imports_changes_only_the_key() {
    let base = unique_temp_dir("reordered_imports");
    fs::create_dir_all(base.join("lib")).unwrap();
    fs::write(
        base.join("lib/text.prism"),
        "package lib\n\nimport /prism/collections\nimport /prism/core\n\npublic fun shout(n: Int): Int {\n    println(listOf(n).size)\n    return n\n}\n",
    )
    .unwrap();
    fs::write(
        base.join("main.prism"),
        "import /lib/text\n\nfun main() { val n = shout(1) }\n",
    )
    .unwrap();
    let check = || {
        Command::new(env!("CARGO_BIN_EXE_prism"))
            .current_dir(&base)
            .env("PRISM_CACHE", base.join("cache"))
            .args(["check", "--explain-cache", "main.prism"])
            .output()
            .unwrap()
    };
    let abi_of = |output: &Output| -> String {
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .find(|line| line.contains("lib/text.prism (package"))
            .unwrap_or_else(|| panic!("{:?}", output))
            .to_string()
    };
    let first = check();
    assert_success(&first);
    fs::write(
        base.join("lib/text.prism"),
        "package lib\n\nimport /prism/core\nimport /prism/collections\n\npublic fun shout(n: Int): Int {\n    println(listOf(n).size)\n    return n\n}\n",
    )
    .unwrap();
    let second = check();
    let lines_of = all_explanation_lines(&second);
    assert!(
        lines_of.contains(&"checked lib/text.prism".to_string()),
        "{:?}",
        lines_of
    );
    assert!(
        lines_of.contains(&"reused main.prism".to_string()),
        "{:?}",
        lines_of
    );
    assert_eq!(abi_of(&first), abi_of(&second));
    assert_eq!(String::from_utf8_lossy(&second.stderr), "");
    fs::remove_dir_all(&base).ok();
}

/// standard-library: "The composer module is not in the prelude" (`compose-runtime`): a file
/// without imports never reaches `/prism/composer`, and one importing it does.
#[test]
fn the_composer_module_is_not_in_the_prelude() {
    let base = unique_temp_dir("composer_module");
    let explain = |source: &str| {
        fs::write(base.join("main.prism"), source).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_prism"))
            .current_dir(&base)
            .env("PRISM_CACHE", base.join("cache"))
            .args(["check", "--explain-cache", "main.prism"])
            .output()
            .unwrap();
        assert!(output.status.success(), "{:?}", output);
        all_explanation_lines(&output).join("\n")
    };
    let without = explain("fun main() {}\n");
    assert!(!without.contains("prism/composer.prism"), "{}", without);
    let with = explain("import /prism/composer\n\nfun main() { invalidate(0) }\n");
    assert!(with.contains("prism/composer.prism"), "{}", with);
    fs::remove_dir_all(&base).ok();
}

/// standard-library: "The UI modules are not in the prelude" (`ui-toolkit`,
/// `explicit-library`): a file without imports reaches no module of `prism/compose/`, and one
/// importing material3 reaches the four toolkit modules and not the runtime module.
#[test]
fn the_ui_modules_are_not_in_the_prelude() {
    let base = unique_temp_dir("ui_modules");
    let explain = |source: &str| {
        fs::write(base.join("main.prism"), source).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_prism"))
            .current_dir(&base)
            .env("PRISM_CACHE", base.join("cache"))
            .args(["check", "--explain-cache", "main.prism"])
            .output()
            .unwrap();
        assert!(output.status.success(), "{:?}", output);
        all_explanation_lines(&output)
    };
    let modules = |lines: &[String]| -> Vec<String> {
        lines
            .iter()
            .filter_map(|line| line.split_once(' ').map(|(_, path)| path.to_string()))
            .filter(|path| path.starts_with("prism/compose/"))
            .collect()
    };
    let without = explain("fun main() {}\n");
    assert!(modules(&without).is_empty(), "{:?}", without);
    let with = explain("import /prism/compose/material3\n\nfun main() {}\n");
    let mut reached = modules(&with);
    reached.sort();
    assert_eq!(
        reached,
        [
            "prism/compose/foundation.prism",
            "prism/compose/layout.prism",
            "prism/compose/material3.prism",
            "prism/compose/ui.prism",
        ]
    );
    fs::remove_dir_all(&base).ok();
}

/// build-cache: "Library entries are shared between workspaces".
#[test]
fn library_entries_are_shared_between_workspaces() {
    let base = unique_temp_dir("shared_library");
    let cache = base.join("cache");
    let mut outputs = Vec::new();
    for name in ["first", "second"] {
        let root = base.join(name);
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("main.prism"),
            format!(
                "import /prism/collections\nimport /prism/compose/runtime\nimport /prism/core\n\nfun main() {{ println(listOf(\"{}\").size) }}\n",
                name
            ),
        )
        .unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_prism"))
            .current_dir(&root)
            .env("PRISM_CACHE", &cache)
            .args(["check", "--explain-cache", "main.prism"])
            .output()
            .unwrap();
        outputs.push(all_explanation_lines(&output));
    }
    assert_eq!(
        outputs[0],
        lines(&[
            "checked prism/core.prism",
            "checked prism/collections.prism",
            "checked prism/compose/runtime.prism",
            "checked main.prism",
            // `listOf` is a library generic (D10 of `vararg`), instantiated for the program.
            "instantiate prism/collections::listOf<String>",
        ])
    );
    assert_eq!(
        outputs[1],
        lines(&[
            "reused prism/core.prism",
            "reused prism/collections.prism",
            "reused prism/compose/runtime.prism",
            "checked main.prism",
            // A check plans instances without caching their code.
            "instantiate prism/collections::listOf<String>",
        ])
    );
    fs::remove_dir_all(&base).ok();
}

/// build-cache: "Explaining a warm run".
#[test]
fn explaining_a_warm_run() {
    let workspace = program_workspace("warm_run");
    assert_eq!(
        workspace.run_explain("app/main.prism"),
        ("1\n".to_string(), lines(&COLD))
    );
    assert_eq!(
        workspace.run_explain("app/main.prism"),
        ("1\n".to_string(), lines(&WARM))
    );
}

/// build-cache: "A checked program compiles its units".
#[test]
fn a_checked_program_compiles_its_units() {
    let workspace = program_workspace("compiles_its_units");
    assert_success(&workspace.run(&["check", "app/main.prism"]));
    assert_eq!(
        workspace.run_explain("app/main.prism").1,
        lines(&[
            "compiled lib/box.prism",
            "compiled app/main.prism",
            "instantiate lib/box::keep<Int>",
        ])
    );
}

/// build-cache: "An unchanged program compiles nothing".
#[test]
fn an_unchanged_program_compiles_nothing() {
    let workspace = program_workspace("compiles_nothing");
    workspace.run_explain("app/main.prism");
    let entries = workspace.entries();
    assert_eq!(workspace.run_explain("app/main.prism").1, lines(&WARM));
    assert_eq!(workspace.entries(), entries, "a warm run writes no entry");
}

/// build-cache: "Running without the cache".
#[test]
fn running_without_the_cache() {
    let workspace = program_workspace("run_without");
    workspace.run_explain("app/main.prism");
    let entries = workspace.entries();
    for output in [
        workspace.run(&["run", "--no-cache", "--explain-cache", "app/main.prism"]),
        workspace.run_with_cache("off", &["run", "--explain-cache", "app/main.prism"]),
    ] {
        assert_success(&output);
        assert_eq!(String::from_utf8_lossy(&output.stdout), "1\n");
        assert!(String::from_utf8_lossy(&output.stderr).contains("Cache: off"));
        assert_eq!(run_explanation(&output), lines(&COLD));
    }
    assert_eq!(workspace.entries(), entries, "no entry is written");
}

/// build-cache: "Running writes nothing in the workspace".
#[test]
fn running_writes_nothing_in_the_workspace() {
    let workspace = program_workspace("run_nothing_written");
    let before = workspace.tree();
    workspace.run_explain("app/main.prism");
    assert_eq!(workspace.run_explain("app/main.prism").1, lines(&WARM));
    assert_eq!(workspace.tree(), before);
}

/// build-cache: "Corrupt code entry is a miss".
#[test]
fn corrupt_code_entry_is_a_miss() {
    let workspace = program_workspace("corrupt_code");
    let cold = workspace.run_explain("app/main.prism");
    let entries = workspace.entries();
    assert!(entries.len() >= 5, "{:?}", entries);
    for (index, entry) in entries.iter().enumerate() {
        let bytes = fs::read(entry).unwrap();
        if index % 2 == 0 {
            fs::write(entry, &bytes[..bytes.len() / 2]).unwrap();
        } else {
            fs::write(entry, "garbage\n").unwrap();
        }
    }
    assert_eq!(workspace.run_explain("app/main.prism"), cold);
    assert_eq!(workspace.run_explain("app/main.prism").1, lines(&WARM));
}

/// A flipped byte in the middle of a code entry's body is a miss too: the checksum catches it.
#[test]
fn a_flipped_code_byte_is_a_miss() {
    let workspace = program_workspace("flipped_code");
    workspace.run_explain("app/main.prism");
    for entry in workspace.entries() {
        let mut bytes = fs::read(&entry).unwrap();
        let middle = bytes.len() / 2;
        bytes[middle] ^= 0x01;
        fs::write(&entry, bytes).unwrap();
    }
    assert_eq!(
        workspace.run_explain("app/main.prism"),
        ("1\n".to_string(), lines(&COLD))
    );
}

/// build-cache: "Unwritable cache directory for a run".
#[test]
fn unwritable_cache_directory_for_a_run() {
    let workspace = program_workspace("unwritable_run");
    let file = workspace.root.parent().unwrap().join("not-a-directory");
    fs::write(&file, "a regular file").unwrap();
    let without = workspace.run(&["run", "--no-cache", "app/main.prism"]);
    for _ in 0..2 {
        let output = workspace.run_with_cache(file.to_str().unwrap(), &["run", "app/main.prism"]);
        assert_eq!(output.status.code(), without.status.code());
        assert_eq!(output.stdout, without.stdout);
        assert!(
            output.stderr.is_empty(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

/// build-cache: "Concurrent runs share a cache".
#[test]
fn concurrent_runs_share_a_cache() {
    let workspace = program_workspace("concurrent_runs");
    let children: Vec<_> = (0..8)
        .map(|_| {
            Command::new(env!("CARGO_BIN_EXE_prism"))
                .current_dir(&workspace.root)
                .env("PRISM_CACHE", &workspace.cache)
                .args(["run", "app/main.prism"])
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .unwrap()
        })
        .collect();
    for child in children {
        let output = child.wait_with_output().unwrap();
        assert_success(&output);
        assert_eq!(String::from_utf8_lossy(&output.stdout), "1\n");
    }
    assert_eq!(workspace.run_explain("app/main.prism").1, lines(&WARM));
}

/// build-cache: "A missing record is rebuilt from its file".
#[test]
fn a_missing_record_is_rebuilt_from_its_file() {
    let workspace = program_workspace("missing_record");
    workspace.run_explain("app/main.prism");
    let mut removed = 0;
    for entry in workspace.entries() {
        if fs::read(&entry)
            .unwrap()
            .starts_with(b"prism-cache 2\nkind record\n")
        {
            fs::remove_file(entry).unwrap();
            removed += 1;
        }
    }
    assert_eq!(removed, 1);
    workspace.write(
        "app/main.prism",
        "package app\nimport /prism/core\nimport /lib/box\nfun main() { println(keep(1)); println(keep(\"a\")) }\n",
    );
    assert_eq!(
        workspace.run_explain("app/main.prism"),
        (
            "1\na\n".to_string(),
            lines(&[
                "checked lib/box.prism",
                "checked app/main.prism",
                "reused instance lib/box::keep<Int>",
                "instantiate lib/box::keep<String>",
            ])
        )
    );
}

// --- Compilation and code generation avoidance (`compilation-and-abi`)

/// compilation-and-abi: "Modifying a generic function body re-checks no importer".
#[test]
fn modifying_a_generic_function_body_re_checks_no_importer() {
    let workspace = program_workspace("generic_body_importer");
    assert_eq!(workspace.explain("app/main.prism"), lines(&COLD));
    workspace.write(
        "lib/box.prism",
        "package lib\npublic fun <T> keep(x: T): T { val y = x; return y }\n",
    );
    assert_eq!(
        workspace.explain("app/main.prism"),
        lines(&[
            "checked lib/box.prism",
            "reused app/main.prism",
            "instantiate lib/box::keep<Int>",
        ])
    );
}

/// The workspace of "Changing generic function body": `app/a` calls `keep(1)`, `app/b` calls
/// `drop("s")`, and `app/main` imports both.
fn two_generics_workspace(name: &str, a: &str) -> Workspace {
    Workspace::new(
        name,
        &[
            ("lib/box.prism", BOX),
            (
                "app/a.prism",
                &format!(
                    "package app\nimport /lib/box\npublic fun a(): Int = {}\n",
                    a
                ),
            ),
            (
                "app/b.prism",
                "package app\nimport /lib/box\npublic fun b(): Int = drop(\"s\")\n",
            ),
            (
                "app/main.prism",
                "package app\nimport /prism/core\nimport a\nimport b\nfun main() { println(a()); println(b()) }\n",
            ),
        ],
    )
}

/// compilation-and-abi: "Changing generic function body".
#[test]
fn changing_generic_function_body() {
    let workspace = two_generics_workspace("generic_body", "keep(1)");
    workspace.run_explain("app/main.prism");
    workspace.write(
        "lib/box.prism",
        &BOX.replace("): T = x", "): T { val y = x; return y }"),
    );
    assert_eq!(
        workspace.run_explain("app/main.prism"),
        (
            "1\n1\n".to_string(),
            lines(&[
                "checked lib/box.prism",
                "reused app/a.prism",
                "reused app/b.prism",
                "reused app/main.prism",
                "reused instance lib/box::drop<String>",
                "instantiate lib/box::keep<Int>",
            ])
        )
    );
}

/// compilation-and-abi: "Changing a generic signature instantiates again".
#[test]
fn changing_a_generic_signature_instantiates_again() {
    let workspace = two_generics_workspace("generic_signature_run", "drop(1)");
    workspace.run_explain("app/main.prism");
    workspace.write("lib/box.prism", &BOX.replace("drop(x: T)", "drop(x: &T)"));
    workspace.write(
        "app/a.prism",
        "package app\nimport /lib/box\npublic fun a(): Int = drop(&1)\n",
    );
    workspace.write(
        "app/b.prism",
        "package app\nimport /lib/box\npublic fun b(): Int = drop(&\"s\")\n",
    );
    let (stdout, explanation) = workspace.run_explain("app/main.prism");
    assert_eq!(stdout, "1\n1\n");
    assert!(explanation.contains(&"checked lib/box.prism".to_string()));
    assert!(explanation.contains(&"checked app/a.prism".to_string()));
    assert!(explanation.contains(&"instantiate lib/box::drop<Int>".to_string()));
}

/// compilation-and-abi: "A type argument gains a field".
#[test]
fn a_type_argument_gains_a_field() {
    let workspace = Workspace::new(
        "type_argument_field",
        &[
            ("lib/box.prism", KEEP),
            (
                "app/main.prism",
                "package app\nimport /prism/core\nimport /lib/box\nstruct Thing(val a: Int)\n\
                 fun main() { println(keep(Thing(1)).a); println(keep(2)) }\n",
            ),
        ],
    );
    workspace.run_explain("app/main.prism");
    workspace.write(
        "app/main.prism",
        "package app\nimport /prism/core\nimport /lib/box\nstruct Thing(val b: Int, val a: Int)\n\
         fun main() { println(keep(Thing(0, 1)).a); println(keep(2)) }\n",
    );
    assert_eq!(
        workspace.run_explain("app/main.prism"),
        (
            "1\n2\n".to_string(),
            lines(&[
                "reused lib/box.prism",
                "checked app/main.prism",
                "reused instance lib/box::keep<Int>",
                "instantiate lib/box::keep<app/main::Thing>",
            ])
        )
    );
}

/// The program of "A private helper's signature change regenerates the instances".
fn helper_workspace(name: &str, helper: &str) -> Workspace {
    Workspace::new(
        name,
        &[
            (
                "lib/box.prism",
                &format!(
                    "package lib\npublic fun <T> outer(x: T, n: Int): Int = helper(n)\n{}\n",
                    helper
                ),
            ),
            (
                "app/main.prism",
                "package app\nimport /prism/core\nimport /lib/box\nfun main() { println(outer(\"a\", 1)) }\n",
            ),
        ],
    )
}

/// compilation-and-abi: "A private helper's signature change regenerates the instances".
#[test]
fn a_private_helpers_signature_change_regenerates_the_instances() {
    let workspace = helper_workspace("helper_signature", "fun helper(n: Int): Int = n + 1");
    assert_eq!(workspace.run_explain("app/main.prism").0, "2\n");
    workspace.write(
        "lib/box.prism",
        "package lib\npublic fun <T> outer(x: T, n: Int): Int = helper(n)\nfun helper(n: &Int): Int = n\n",
    );
    assert_eq!(
        workspace.run_explain("app/main.prism"),
        (
            "1\n".to_string(),
            lines(&[
                "checked lib/box.prism",
                "reused app/main.prism",
                "instantiate lib/box::outer<String>",
            ])
        )
    );
}

/// compilation-and-abi: "A private helper's body change keeps the instances".
#[test]
fn a_private_helpers_body_change_keeps_the_instances() {
    let workspace = helper_workspace("helper_body", "fun helper(n: Int): Int = n + 1");
    assert_eq!(workspace.run_explain("app/main.prism").0, "2\n");
    workspace.write(
        "lib/box.prism",
        "package lib\npublic fun <T> outer(x: T, n: Int): Int = helper(n)\nfun helper(n: Int): Int = n + 2\n",
    );
    assert_eq!(
        workspace.run_explain("app/main.prism"),
        (
            "3\n".to_string(),
            lines(&[
                "checked lib/box.prism",
                "reused app/main.prism",
                "reused instance lib/box::outer<String>",
            ])
        )
    );
}

/// compilation-and-abi: "A body edit regenerates one file's code".
#[test]
fn a_body_edit_regenerates_one_files_code() {
    let workspace = Workspace::new(
        "body_edit_code",
        &[
            ("lib/text.prism", TEXT),
            (
                "app/main.prism",
                "package app\nimport /prism/core\nimport /lib/text\nfun main() { println(shout(\"a\")) }\n",
            ),
        ],
    );
    assert_eq!(workspace.run_explain("app/main.prism").0, "a\n");
    workspace.write(
        "lib/text.prism",
        "package lib\npublic fun shout(s: &String): String = s + \"!\"\n",
    );
    assert_eq!(
        workspace.run_explain("app/main.prism"),
        (
            "a!\n".to_string(),
            lines(&["checked lib/text.prism", "reused app/main.prism"])
        )
    );
}

// --- code-generation: "Compiled Units Are Relocatable Artifacts"

/// code-generation: "Cached units run as fresh ones".
#[test]
fn cached_units_run_as_fresh_ones() {
    let workspace = Workspace::new(
        "cached_units",
        &[
            (
                "lib/text.prism",
                "package lib\npublic val greeting: String = \"hello\"\n\
                 public struct Named(val name: String, val n: Int)\n\
                 public fun <T> label(x: T, name: String): Named = Named(name + \"!\", 1)\n\
                 public fun shout(s: &String): String = s + \"?\"\n",
            ),
            (
                "app/main.prism",
                "package app\nimport /prism/core\nimport /prism/collections\nimport /lib/text\n\
                 fun apply(f: (String) -> String, s: String): String = f(s)\n\
                 fun main() {\n\
                     val n = label(1, shout(greeting))\n\
                     println(n.name)\n\
                     val xs = listOf(n.name.clone(), \"x\")\n\
                     println(xs.size)\n\
                     println(apply({ s -> s + greeting }, \"a\"))\n\
                 }\n",
            ),
        ],
    );
    let run = || {
        let output = workspace.run(&[
            "run",
            "--check-allocations",
            "--explain-cache",
            "app/main.prism",
        ]);
        assert_success(&output);
        (
            String::from_utf8_lossy(&output.stdout).to_string(),
            run_explanation(&output),
        )
    };
    let (cold, _) = run();
    assert_eq!(cold, "hello?!\n2\nahello\n");
    let (warm, explanation) = run();
    assert_eq!(warm, cold);
    assert!(
        explanation.iter().all(|line| line.starts_with("reused ")),
        "{:?}",
        explanation
    );
}

/// code-generation: "Dumps never read the cache".
#[test]
fn dumps_never_read_the_cache() {
    let workspace = program_workspace("dumps");
    let before = workspace.run(&["dump-clif", "app/main.prism"]);
    assert_success(&before);
    workspace.run_explain("app/main.prism");
    let after = workspace.run(&["dump-clif", "app/main.prism"]);
    assert_eq!(before.stdout, after.stdout);
    let instances = workspace.run(&["dump-clif", "--instances", "app/main.prism"]);
    assert_success(&instances);
}

// --- Operator functions and equality operators (operator-conventions)

const VECTOR_PLUS: &str =
    "package geometry\npublic struct Vector2 : Copy { val x: Int, val y: Int }\n\
     public fun Vector2.plus(other: Vector2): Vector2 = Vector2(x + other.x, y + other.y)\n";
const VECTOR_MAIN: &str = "package app\nimport /geometry/vector\n\
     fun f(a: Vector2, b: Vector2): Vector2 = a.plus(b)\n";

/// compilation-and-abi: "Marking a function operator changes the hash".
#[test]
fn marking_a_function_operator_changes_the_hash() {
    let workspace = Workspace::new(
        "operator_flag",
        &[
            ("geometry/vector.prism", VECTOR_PLUS),
            ("app/main.prism", VECTOR_MAIN),
        ],
    );
    assert_eq!(
        workspace.explain("app/main.prism"),
        lines(&["checked geometry/vector.prism", "checked app/main.prism"])
    );
    workspace.write(
        "geometry/vector.prism",
        &VECTOR_PLUS.replace(
            "public fun Vector2.plus",
            "public operator fun Vector2.plus",
        ),
    );
    assert_eq!(
        workspace.explain("app/main.prism"),
        lines(&["checked geometry/vector.prism", "checked app/main.prism"])
    );
}

const USER_MODEL: &str = "package models\npublic struct User { val id: Int, val name: String }\n";
const USER_EQUALS: &str = "public operator fun User.equals(other: &User): Bool = id == other.id\n";
const USER_MAIN: &str = "package app\nimport /prism/core\nimport /lib/box\nimport /models/user\n\
     fun main() { println(keep(User(1, \"a\")).id) }\n";

/// compilation-and-abi: "Adding an equality operator changes the type's definition".
#[test]
fn adding_an_equality_operator_changes_the_type_s_definition() {
    let workspace = Workspace::new(
        "equality_flag",
        &[
            ("lib/box.prism", KEEP),
            ("models/user.prism", USER_MODEL),
            ("app/main.prism", USER_MAIN),
        ],
    );
    workspace.run_explain("app/main.prism");
    workspace.write(
        "models/user.prism",
        &format!("{}{}", USER_MODEL, USER_EQUALS),
    );
    assert_eq!(
        workspace.run_explain("app/main.prism"),
        (
            "1\n".to_string(),
            lines(&[
                "reused lib/box.prism",
                "checked models/user.prism",
                "checked app/main.prism",
                "instantiate lib/box::keep<models/user::User>",
            ])
        )
    );
}

/// compilation-and-abi: "Editing an operator body recompiles nothing downstream".
#[test]
fn editing_an_operator_body_recompiles_nothing_downstream() {
    let plus = "public operator fun User.plus(other: &User): Int = id + other.id\n";
    let workspace = Workspace::new(
        "operator_body",
        &[
            ("lib/box.prism", KEEP),
            (
                "models/user.prism",
                &format!("{}{}{}", USER_MODEL, USER_EQUALS, plus),
            ),
            ("app/main.prism", USER_MAIN),
        ],
    );
    workspace.run_explain("app/main.prism");
    workspace.write(
        "models/user.prism",
        &format!(
            "{}{}{}",
            USER_MODEL,
            USER_EQUALS.replace("id == other.id", "name == other.name"),
            plus.replace("id + other.id", "id - other.id")
        ),
    );
    let (output, explained) = workspace.run_explain("app/main.prism");
    assert_eq!(output, "1\n");
    assert!(
        explained.contains(&"checked models/user.prism".to_string())
            && explained.contains(&"reused app/main.prism".to_string())
            && explained.contains(&"reused instance lib/box::keep<models/user::User>".to_string()),
        "{:?}",
        explained
    );
}

// --- Drop and clone hooks (drop-and-clone-hooks)

const SESSION_LIB: &str = "package lib\npublic struct Session(val id: Int)\n";
const SESSION_IMPORTER: &str = "package app\nimport /lib/net\nfun f(s: Session) {}\n";

/// The ABI hash `prism abi <file>` prints.
fn abi_hash(workspace: &Workspace, file: &str) -> String {
    let output = workspace.run(&["abi", file]);
    assert_success(&output);
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|line| line.split("ABI Hash: ").nth(1).map(str::to_string))
        .expect("an ABI hash")
}

/// compilation-and-abi: "Adding a drop hook changes the hashes".
#[test]
fn adding_a_drop_hook_changes_the_hashes() {
    let workspace = Workspace::new(
        "drop_hook_flag",
        &[
            ("lib/net.prism", SESSION_LIB),
            ("app/main.prism", SESSION_IMPORTER),
        ],
    );
    assert_eq!(
        workspace.explain("app/main.prism"),
        lines(&["checked lib/net.prism", "checked app/main.prism"])
    );
    let before = abi_hash(&workspace, "lib/net.prism");
    workspace.write(
        "lib/net.prism",
        &format!("{}operator mut fun Session.drop() {{}}\n", SESSION_LIB),
    );
    assert_ne!(abi_hash(&workspace, "lib/net.prism"), before);
    let abi = workspace.run(&["abi", "lib/net.prism"]);
    assert!(
        String::from_utf8_lossy(&abi.stdout)
            .contains("  public struct Session { val id: Int } with a drop hook\n"),
        "{}",
        String::from_utf8_lossy(&abi.stdout)
    );
    assert_eq!(
        workspace.explain("app/main.prism"),
        lines(&["checked lib/net.prism", "checked app/main.prism"])
    );
}

/// compilation-and-abi: "A reachable type's hook is recorded".
#[test]
fn a_reachable_type_s_hook_is_recorded() {
    let workspace = Workspace::new(
        "reachable_hook",
        &[(
            "lib/a.prism",
            "package lib\nstruct Secret { val k: Int }\noperator mut fun Secret.drop() {}\npublic struct Outer { val s: Secret }\n",
        )],
    );
    let output = workspace.run(&["abi", "lib/a.prism"]);
    assert_success(&output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("  struct lib/a::Secret { val k: Int } with a drop hook\n"),
        "{}",
        stdout
    );
    assert!(
        stdout.contains("  public struct Outer { val s: Secret }\n"),
        "{}",
        stdout
    );
}

/// compilation-and-abi: "Editing a hook's body recompiles nothing downstream".
#[test]
fn editing_a_hook_s_body_recompiles_nothing_downstream() {
    let library = "package lib\nimport /prism/core\npublic struct Session(val id: Int)\n\
                   operator mut fun Session.drop() { println(\"closed\") }\n";
    let workspace = Workspace::new(
        "hook_body",
        &[
            ("lib/net.prism", library),
            (
                "app/main.prism",
                "package app\nimport /lib/net\nfun main() { val s = Session(1) }\n",
            ),
        ],
    );
    let (stdout, _) = workspace.run_explain("app/main.prism");
    assert_eq!(stdout, "closed\n");
    let before = abi_hash(&workspace, "lib/net.prism");
    workspace.write("lib/net.prism", &library.replace("closed", "bye"));
    assert_eq!(abi_hash(&workspace, "lib/net.prism"), before);
    let (stdout, explanation) = workspace.run_explain("app/main.prism");
    assert_eq!(stdout, "bye\n");
    assert_eq!(
        explanation,
        lines(&["checked lib/net.prism", "reused app/main.prism"])
    );
}

/// compilation-and-abi: "Hook instances of a generic type".
#[test]
fn hook_instances_of_a_generic_type() {
    let workspace = Workspace::new(
        "hook_instances",
        &[
            (
                "lib/own.prism",
                "package lib\nimport /prism/core\npublic struct Owned<T>(val v: T)\n\
                 operator mut fun <T> Owned<T>.drop() { println(\"drop owned\") }\n",
            ),
            (
                "app/main.prism",
                "package app\nimport /prism/collections\nimport /lib/own\nfun <T> hold(x: T) {}\n\
                 fun main() { hold(Owned(1)); val xs = listOf(Owned(\"a\")) }\n",
            ),
        ],
    );
    let explanation = workspace.explain("app/main.prism");
    for line in [
        "instantiate lib/own::Owned.drop<Int>",
        "instantiate lib/own::Owned.drop<String>",
    ] {
        assert!(
            explanation.contains(&line.to_string()),
            "{} in {:?}",
            line,
            explanation
        );
    }
    let output = workspace.run(&["run", "--check-allocations", "app/main.prism"]);
    assert_success(&output);
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "drop owned\ndrop owned\n"
    );
}

/// compilation-and-abi: "A hook instance requested inside a generic".
#[test]
fn a_hook_instance_requested_inside_a_generic() {
    let workspace = Workspace::new(
        "hook_instance_in_generic",
        &[(
            "app/main.prism",
            "package app\nimport /prism/core\nstruct Owned<T>(val v: T)\n\
             operator mut fun <T> Owned<T>.drop() { println(\"drop owned\") }\n\
             fun <T> wrap(x: T) { val o = Owned(x) }\nfun main() { wrap(true) }\n",
        )],
    );
    let explanation = workspace.explain("app/main.prism");
    for line in [
        "instantiate app/main::wrap<Bool>",
        "instantiate app/main::Owned.drop<Bool>",
    ] {
        assert!(
            explanation.contains(&line.to_string()),
            "{} in {:?}",
            line,
            explanation
        );
    }
}

/// standard-library: "listOf is written in Prism": each element type a program builds lists of
/// instantiates `listOf` once, and a later run reuses the cached instances (D10 of `vararg`).
#[test]
fn list_of_instances_are_cached() {
    let workspace = Workspace::new(
        "list-of",
        &[(
            "main.prism",
            "import /prism/collections\nimport /prism/core\nfun main() { println(listOf(1, 2).size); println(listOf(\"a\").size); println(listOf(3).size) }\n",
        )],
    );
    let instances = |output: &Output| -> Vec<String> {
        String::from_utf8_lossy(&output.stderr)
            .lines()
            .map(str::trim)
            .filter(|line| line.contains("prism/collections::listOf"))
            .map(str::to_string)
            .collect()
    };
    let first = workspace.run(&["run", "--explain-cache", "main.prism"]);
    assert_success(&first);
    assert_eq!(
        instances(&first),
        lines(&[
            "instantiate prism/collections::listOf<Int>",
            "instantiate prism/collections::listOf<String>"
        ])
    );
    let second = workspace.run(&["run", "--explain-cache", "main.prism"]);
    assert_success(&second);
    assert_eq!(
        instances(&second),
        lines(&[
            "reused instance prism/collections::listOf<Int>",
            "reused instance prism/collections::listOf<String>"
        ])
    );
}

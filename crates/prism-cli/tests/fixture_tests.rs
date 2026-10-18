//! Data-driven spec fixtures for the whole `prism check` pipeline.
//!
//! Each case is a directory `tests/fixtures/<spec>/<scenario-slug>[--<form>]/`, where `<spec>`
//! is a capability under `openspec/specs/` and the slug names the `#### Scenario:` it exercises
//! (kebab-case). A scenario whose WHEN has several forms gets one case per form, named
//! `<scenario-slug>--<form>`. A directory per case lets a case hold several files and package
//! directories; the case directory is the workspace root.
//!
//! ## Header convention
//!
//! Exactly one `.prism` file per case carries the expectation, in `//` comment lines:
//!
//! ```text
//! // expect: ok                 the case must pass `prism check`
//! // expect: E4002              the case must fail with exactly this set of codes
//! // expect: E4002 E4004        several codes: space or comma separated, order and repeats ignored
//! // entry: app/main.prism      optional: what to check, relative to the case directory
//! ```
//!
//! Without `// entry:` the header file itself is checked, so a single-file case is just
//! `main.prism`. An entry that is a directory (`// entry: .`) runs `prism check <dir>`.
//!
//! A case that exposes behaviour the spec does not settle uses `// expect-current: <codes>`
//! instead of `// expect:`, to pin today's behaviour, and must also carry a
//! `// FIXME(decision): <the open question>` line. When the decision is made the line becomes
//! `// expect:`.
//!
//! Every case name must start with the slug of an existing `#### Scenario:` heading of its spec
//! (lower-cased, runs of non-alphanumerics replaced by `-`), which keeps the fixtures traceable to
//! the scenarios they exercise. The scenarios of an active change's delta spec count too, so a
//! change's fixtures can land before it is archived.
//!
//! ## Typed-tree snapshots
//!
//! A case may also hold `typed-tree.snap` beside its `.prism` files: the expected output of
//! `prism dump-typed-tree` on its entry (`prism_cli::dump_typed_tree`), which the runner
//! compares with the actual dump. Every case of the `typed-tree` spec, and every case whose name
//! starts with `typed-tree`, must hold one. `PRISM_BLESS=1 cargo test -p prism-cli --test
//! fixture_tests` writes the actual dumps instead; read each against its scenario's THEN before
//! committing it.
//!
//! ## MIR snapshots
//!
//! Likewise, a case may hold `mir.snap`: the expected output of `prism dump-mir` on its entry
//! (`prism_cli::dump_mir`), which needs an entry without type errors. Every case of the `mir`
//! spec, and every case whose name starts with `mir`, must hold one.
//!
//! A case may also hold `elaborated-mir.snap`: the expected output of `prism dump-mir
//! --elaborated` on its entry (`prism_cli::dump_elaborated_mir`), which needs an entry without
//! type or borrow errors. Every `mir` case of the scenarios of "Drops Are Elaborated After Borrow
//! Checking" (`ELABORATED_SCENARIOS`) must hold one. `PRISM_BLESS=1` writes every kind of
//! snapshot.
//!
//! ## Layout snapshots
//!
//! A case may also hold `layout.snap`: the expected output of `prism dump-layout` on its entry
//! (`prism_cli::dump_layout`), which needs an entry without type or borrow errors. Every
//! `code-generation` case of the layout scenarios (`LAYOUT_SCENARIOS`) must hold one.
//!
//! ## Executable cases
//!
//! A case whose header also has a `// run: <status>` line is a program: the runner spawns
//! `prism run --check-allocations <entry>` with the case directory as working directory (so
//! diagnostics show paths relative to it) and compares the exit status with `<status>` (`0` when
//! `main` returns, `101` after a runtime error, `1` when the program does not compile, `102` when
//! the allocation check fails), and stdout and stderr
//! with `stdout.snap` and `stderr.snap`, an absent file meaning empty output (D21 of
//! `codegen-core`). `PRISM_BLESS=1` writes both, removing the file of an empty output; read each
//! against its scenario's THEN before committing it. The `// expect:` line still states what
//! `prism check` reports, which is `ok` for a program the backend rejects. The `E6xxx` codes in a
//! case's `stderr.snap` count as exercised for the catalog check. Every case of the `execution`
//! spec must be executable. Since every program runs under the allocation check (D13 of
//! `runtime-strings-and-drops`), each one also asserts that no allocation outlives its owner and
//! none is freed twice: a leak or an invalid free changes its status to `102`.
//!
//! The runner uses the same pipeline as the CLI (`prism_cli::check_path`) and takes the codes from
//! the rendered diagnostics (`error[E4002]: ...`). It also checks the fixtures against the
//! catalog in `openspec/specs/diagnostics/spec.md`: every expected code must be a live catalog
//! code, and every live code must be exercised by some case, except those in `UNFIXTURED_CODES`.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Codes that no fixture can produce, with the reason. Each has a Rust test elsewhere.
const UNFIXTURED_CODES: &[(&str, &str)] = &[
    (
        "E1005",
        "needs a symlink leaving the workspace, which cannot be committed portably; see cli_workspace_tests.rs",
    ),
    (
        "E2002",
        "needs input nested more than 256 levels deep; see parser_tests.rs",
    ),
];

struct Case {
    dir: PathBuf,
    name: String,
    header: PathBuf,
    expected: BTreeSet<String>,
    pinned_current: bool,
    /// The exit status `prism run` must give, for an executable case.
    run_status: Option<i32>,
}

fn fixtures_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn prism_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            prism_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "prism") {
            out.push(path);
        }
    }
}

fn parse_codes(list: &str, header: &Path) -> BTreeSet<String> {
    let mut codes = BTreeSet::new();
    for word in list.split(|c: char| c.is_whitespace() || c == ',') {
        if word.is_empty() || word == "ok" {
            continue;
        }
        let looks_like_code = word.len() == 5
            && word.starts_with('E')
            && word[1..].chars().all(|c| c.is_ascii_digit());
        assert!(looks_like_code, "{}: bad code '{}'", header.display(), word);
        codes.insert(word.to_string());
    }
    codes
}

fn load_cases() -> Vec<Case> {
    let root = fixtures_root();
    let mut cases = Vec::new();
    for spec in sorted_dirs(&root) {
        for dir in sorted_dirs(&spec) {
            let name = format!(
                "{}/{}",
                spec.file_name().unwrap().to_string_lossy(),
                dir.file_name().unwrap().to_string_lossy()
            );
            let mut files = Vec::new();
            prism_files(&dir, &mut files);
            let headers: Vec<_> = files
                .iter()
                .filter(|f| {
                    fs::read_to_string(f)
                        .unwrap()
                        .lines()
                        .any(|l| l.starts_with("// expect:") || l.starts_with("// expect-current:"))
                })
                .collect();
            assert_eq!(
                headers.len(),
                1,
                "{}: exactly one file must carry an `// expect:` header, found {:?}",
                name,
                headers
            );
            let header = headers[0].clone();
            let text = fs::read_to_string(&header).unwrap();
            let mut expected = BTreeSet::new();
            let mut pinned_current = false;
            let mut run_status = None;
            for line in text.lines() {
                if let Some(rest) = line.strip_prefix("// run:") {
                    let status = rest
                        .trim()
                        .parse::<i32>()
                        .unwrap_or_else(|_| panic!("{}: bad run status '{}'", name, rest.trim()));
                    run_status = Some(status);
                }
                if let Some(rest) = line.strip_prefix("// expect:") {
                    expected.extend(parse_codes(rest, &header));
                } else if let Some(rest) = line.strip_prefix("// expect-current:") {
                    pinned_current = true;
                    expected.extend(parse_codes(rest, &header));
                }
            }
            if pinned_current {
                assert!(
                    text.lines().any(|l| l.starts_with("// FIXME(decision):")),
                    "{}: `expect-current` needs a `// FIXME(decision):` line",
                    name
                );
            }
            assert!(
                run_status.is_some() || !name.starts_with("execution/"),
                "{}: every case of the execution spec needs a `// run:` line",
                name
            );
            cases.push(Case {
                dir,
                name,
                header,
                expected,
                pinned_current,
                run_status,
            });
        }
    }
    cases
}

fn sorted_dirs(dir: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    dirs
}

fn entry_of(case: &Case) -> PathBuf {
    let text = fs::read_to_string(&case.header).unwrap();
    match text.lines().find_map(|l| l.strip_prefix("// entry:")) {
        Some(rel) => case.dir.join(rel.trim()),
        None => case.header.clone(),
    }
}

/// The set of `E####` codes in rendered diagnostics (`error[E4002]: ...`).
fn codes_in(rendered: &str) -> BTreeSet<String> {
    let mut codes = BTreeSet::new();
    let mut rest = rendered;
    while let Some(i) = rest.find("error[E") {
        let after = &rest[i + "error[".len()..];
        if let Some(end) = after.find(']') {
            codes.insert(after[..end].to_string());
        }
        rest = &rest[i + "error[".len()..];
    }
    codes
}

fn run(case: &Case) -> Result<(), String> {
    let entry = entry_of(case);
    let (actual, rendered) = match prism_cli::check_path(&entry) {
        Ok(_) => (BTreeSet::new(), String::new()),
        Err(err) => {
            let rendered = err.render();
            let codes = codes_in(&rendered);
            if codes.is_empty() {
                // The diagnostics spec requires every reported error to carry a code.
                return Err(format!(
                    "{}: error without a code:\n{}",
                    case.name, rendered
                ));
            }
            (codes, rendered)
        }
    };
    if actual != case.expected {
        return Err(format!(
            "{}: expected {:?}, got {:?}\n{}",
            case.name, case.expected, actual, rendered
        ));
    }
    check_snapshot(case, &entry)?;
    check_run(case, &entry)
}

/// Runs an executable case with `prism run --check-allocations` and compares its exit status and
/// output, so that every program also asserts it leaks nothing and frees nothing twice.
fn check_run(case: &Case, entry: &Path) -> Result<(), String> {
    let Some(expected_status) = case.run_status else {
        return Ok(());
    };
    let relative = entry.strip_prefix(&case.dir).unwrap_or(entry);
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_prism"))
        .arg("run")
        .arg("--check-allocations")
        .arg(relative)
        .current_dir(&case.dir)
        .output()
        .map_err(|error| format!("{}: cannot run prism: {}", case.name, error))?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let status = output.status.code();
    let bless = std::env::var_os("PRISM_BLESS").is_some();
    let mut problems = Vec::new();
    if status != Some(expected_status) {
        problems.push(format!(
            "exit status {:?}, expected {}",
            status, expected_status
        ));
    }
    for (file, actual) in [("stdout.snap", &stdout), ("stderr.snap", &stderr)] {
        let path = case.dir.join(file);
        if bless {
            if actual.is_empty() {
                let _ = fs::remove_file(&path);
            } else {
                fs::write(&path, actual).unwrap();
            }
            continue;
        }
        let expected = fs::read_to_string(&path).unwrap_or_default();
        if expected != *actual {
            problems.push(format!(
                "stale {} (re-bless with PRISM_BLESS=1):\n--- expected\n{}--- actual\n{}",
                file, expected, actual
            ));
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(format!("{}: {}", case.name, problems.join("\n")))
    }
}

/// A text snapshot a case may hold: its file name, and the dump it records.
struct Snapshot {
    file: &'static str,
    /// Whether a case, by its spec and its directory name, must hold one.
    required_for: fn(&str, &str) -> bool,
    /// The dump of an entry, or its rendered errors.
    dump: fn(&Path) -> Result<String, String>,
}

/// The scenarios of the `mir` spec whose cases must hold an `elaborated-mir.snap`: those of
/// "Drops Are Elaborated After Borrow Checking", and those whose THEN describes elaborated MIR.
const ELABORATED_SCENARIOS: &[&str] = &[
    "a-moved-value-is-not-dropped",
    "a-conditionally-moved-value-is-dropped-under-a-flag",
    "assigning-over-a-possibly-moved-variable",
    "reinitialization-after-a-move-drops-nothing",
    "deterministic-scope-exit-cleanup",
    "drop-of-a-declaration-that-a-continue-skipped",
    "a-borrowed-fallback-temporary-lives-until-the-end-of-the-statement",
    "borrowed-temporaries-in-the-branches-of-a-val-initializer",
    "capture-modes-in-elaborated-mir",
    "elaborated-drops-of-a-generic-body",
];

/// The scenarios of the `code-generation` spec whose cases must hold a `layout.snap`: those of
/// the requirements on struct, enum, nullable and generic instance layouts, and "A type without
/// a layout yet".
const LAYOUT_SCENARIOS: &[&str] = &[
    "value-struct-definition",
    "fields-ordered-by-alignment",
    "payload-free-enum",
    "recursive-enum-with-heap-indirection",
    "a-small-payload-follows-the-tag",
    "a-nullable-integer",
    "an-optional-borrow-is-one-pointer",
    "a-generic-struct-at-concrete-types",
    "a-type-without-a-layout-yet",
    "a-struct-holding-a-string",
    "a-nullable-string",
];

const SNAPSHOTS: &[Snapshot] = &[
    Snapshot {
        file: "typed-tree.snap",
        required_for: |spec, dir| spec == "typed-tree" || dir.starts_with("typed-tree"),
        dump: |entry| {
            prism_cli::dump_typed_tree(entry)
                .map(|dump| dump.text)
                .map_err(|err| err.render())
        },
    },
    Snapshot {
        file: "mir.snap",
        required_for: |spec, dir| spec == "mir" || dir.starts_with("mir"),
        dump: |entry| {
            prism_cli::dump_mir(entry)
                .map(|dump| dump.text)
                .map_err(|err| err.render())
        },
    },
    Snapshot {
        file: "elaborated-mir.snap",
        required_for: |spec, dir| {
            spec == "mir"
                && ELABORATED_SCENARIOS
                    .iter()
                    .any(|scenario| dir.split("--").next() == Some(scenario))
        },
        dump: |entry| {
            prism_cli::dump_elaborated_mir(entry)
                .map(|dump| dump.text)
                .map_err(|err| err.render())
        },
    },
    Snapshot {
        file: "layout.snap",
        required_for: |spec, dir| {
            spec == "code-generation"
                && LAYOUT_SCENARIOS
                    .iter()
                    .any(|scenario| dir.split("--").next() == Some(scenario))
        },
        dump: |entry| prism_cli::dump_layout(entry).map_err(|err| err.render()),
    },
];

/// Whether `case` must hold `snapshot`.
fn needs_snapshot(case: &Case, snapshot: &Snapshot) -> bool {
    let (spec, dir) = case.name.split_once('/').unwrap();
    (snapshot.required_for)(spec, dir)
}

/// Compares each dump of `entry` with the case's snapshot, or writes it with `PRISM_BLESS`.
fn check_snapshot(case: &Case, entry: &Path) -> Result<(), String> {
    for snapshot in SNAPSHOTS {
        check_one_snapshot(case, entry, snapshot)?;
    }
    Ok(())
}

fn check_one_snapshot(case: &Case, entry: &Path, snapshot: &Snapshot) -> Result<(), String> {
    let path = case.dir.join(snapshot.file);
    if !path.exists() && !needs_snapshot(case, snapshot) {
        return Ok(());
    }
    let actual = match (snapshot.dump)(entry) {
        Ok(text) => text,
        Err(err) => {
            return Err(format!(
                "{}: nothing to compare with {}:\n{}",
                case.name, snapshot.file, err
            ))
        }
    };
    if std::env::var_os("PRISM_BLESS").is_some() {
        fs::write(&path, &actual).unwrap();
        return Ok(());
    }
    match fs::read_to_string(&path) {
        Ok(expected) if expected == actual => Ok(()),
        Ok(expected) => Err(format!(
            "{}: stale {} (re-bless with PRISM_BLESS=1):\n--- expected\n{}--- actual\n{}",
            case.name, snapshot.file, expected, actual
        )),
        Err(_) => Err(format!(
            "{}: missing {} (write it with PRISM_BLESS=1):\n{}",
            case.name, snapshot.file, actual
        )),
    }
}

#[test]
fn fixtures_produce_their_expected_codes() {
    let cases = load_cases();
    assert!(!cases.is_empty(), "no fixtures found");
    let failures: Vec<String> = cases.iter().filter_map(|c| run(c).err()).collect();
    assert!(
        failures.is_empty(),
        "{} of {} fixtures failed:\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n")
    );
}

/// `(code, retired)` rows of the catalog tables in the diagnostics spec.
fn catalog() -> Vec<(String, bool)> {
    let spec =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../openspec/specs/diagnostics/spec.md");
    let text = fs::read_to_string(spec).unwrap();
    text.lines()
        .filter_map(|l| {
            let rest = l.strip_prefix("| `E")?;
            let code = format!("E{}", &rest[..4]);
            Some((code, l.contains("(retired)")))
        })
        .collect()
}

#[test]
fn fixture_codes_match_the_diagnostics_catalog() {
    let catalog = catalog();
    assert!(catalog.len() > 20, "catalog not parsed");
    let live: BTreeSet<_> = catalog
        .iter()
        .filter(|(_, retired)| !retired)
        .map(|(c, _)| c.clone())
        .collect();

    let cases = load_cases();
    let mut used = BTreeSet::new();
    for case in &cases {
        // The diagnostics `prism run` reports count as exercised (D21 of `codegen-core`).
        if case.run_status.is_some() {
            let stderr = fs::read_to_string(case.dir.join("stderr.snap")).unwrap_or_default();
            for code in codes_in(&stderr) {
                assert!(
                    live.contains(&code),
                    "{}: `{}` in stderr.snap is not a live code in the diagnostics catalog",
                    case.name,
                    code
                );
                used.insert(code);
            }
        }
        for code in &case.expected {
            assert!(
                live.contains(code),
                "{}: `{}` is not a live code in the diagnostics catalog",
                case.name,
                code
            );
            if !case.pinned_current {
                used.insert(code.clone());
            }
        }
    }

    let exempt: BTreeSet<_> = UNFIXTURED_CODES
        .iter()
        .map(|(c, _)| c.to_string())
        .collect();
    let uncovered: Vec<_> = live
        .iter()
        .filter(|c| !used.contains(*c) && !exempt.contains(*c))
        .collect();
    assert!(
        uncovered.is_empty(),
        "catalog codes with no fixture (add one or an UNFIXTURED_CODES entry): {:?}",
        uncovered
    );
    for code in &exempt {
        assert!(
            !used.contains(code),
            "{} now has a fixture; drop it from UNFIXTURED_CODES",
            code
        );
    }
}

fn slug(title: &str) -> String {
    let mut out = String::new();
    for c in title.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

/// The texts of capability `spec`: its main spec, and the delta spec of every active (not yet
/// archived) change that touches it, so a change's fixtures can name its new scenarios before
/// the change is archived into the main specs.
fn spec_texts(spec: &str) -> Vec<String> {
    let openspec = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../openspec");
    let mut paths = vec![openspec.join("specs").join(spec).join("spec.md")];
    if let Ok(changes) = fs::read_dir(openspec.join("changes")) {
        let mut changes: Vec<_> = changes
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.is_dir() && p.file_name().is_some_and(|n| n != "archive"))
            .collect();
        changes.sort();
        paths.extend(
            changes
                .into_iter()
                .map(|c| c.join("specs").join(spec).join("spec.md")),
        );
    }
    paths
        .iter()
        .filter_map(|p| fs::read_to_string(p).ok())
        .collect()
}

#[test]
fn fixture_names_match_spec_scenarios() {
    let mut unknown = Vec::new();
    for case in load_cases() {
        let (spec, dir) = case.name.split_once('/').unwrap();
        let texts = spec_texts(spec);
        assert!(
            !texts.is_empty(),
            "no spec named '{}' for fixture {}",
            spec,
            case.name
        );
        let slugs: BTreeSet<_> = texts
            .iter()
            .flat_map(|text| text.lines())
            .filter_map(|l| l.strip_prefix("#### Scenario:"))
            .map(|t| slug(t.trim()))
            .collect();
        let scenario = dir.split("--").next().unwrap();
        if !slugs.contains(scenario) {
            unknown.push(case.name.clone());
        }
    }
    assert!(
        unknown.is_empty(),
        "fixtures whose name does not start with a scenario slug of their spec: {:#?}",
        unknown
    );
}

/// typed-tree: "Error-free trees". Builds the typed tree of the entry file of every case that
/// reaches Phase 2 and checks its structural invariants (`TypedFile::validate`), with the
/// error-free invariants when the entry has no type error.
#[test]
fn fixture_typed_trees_are_well_formed() {
    let mut violations = Vec::new();
    let mut validated = 0;
    for case in load_cases() {
        let entry = entry_of(&case);
        if !entry.is_file() {
            continue;
        }
        let Ok(dump) = prism_cli::dump_typed_tree(&entry) else {
            continue;
        };
        validated += 1;
        let error_free = dump.type_errors.is_none();
        if case.expected.is_empty() {
            assert!(
                error_free,
                "{}: expected ok, but has type errors",
                case.name
            );
        }
        for violation in dump.tree.validate(error_free) {
            violations.push(format!("{}: {}", case.name, violation));
        }
    }
    assert!(validated > 200, "only {} trees validated", validated);
    assert!(
        violations.is_empty(),
        "{} typed-tree violations:\n{}",
        violations.len(),
        violations.join("\n")
    );
}

/// typed-tree: "Error-free trees", over `examples/`. Every example builds a well-formed typed
/// tree; one that needs an unbuilt subsystem (`examples/demo/ui`) has type errors, so only the
/// invariants of an erroneous tree are checked for it.
#[test]
fn example_typed_trees_are_well_formed() {
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let mut files = Vec::new();
    prism_files(&examples, &mut files);
    assert!(!files.is_empty(), "no examples found");
    let mut violations = Vec::new();
    for file in &files {
        let dump = match prism_cli::dump_typed_tree(file) {
            Ok(dump) => dump,
            Err(err) => panic!("{}: no typed tree:\n{}", file.display(), err.render()),
        };
        let needs_unbuilt_subsystem = file.components().any(|c| c.as_os_str() == "ui");
        let error_free = dump.type_errors.is_none();
        assert!(
            error_free || needs_unbuilt_subsystem,
            "{}: type errors:\n{}",
            file.display(),
            dump.type_errors.map(|e| e.render()).unwrap_or_default()
        );
        for violation in dump.tree.validate(error_free) {
            violations.push(format!("{}: {}", file.display(), violation));
        }
    }
    assert!(
        violations.is_empty(),
        "{} typed-tree violations:\n{}",
        violations.len(),
        violations.join("\n")
    );
}

/// mir: "Error-free MIR". Lowers the entry file of every case whose entry typechecks and checks
/// the MIR's structural invariants (`MirFile::validate`), reporting every violation and every
/// lowering crash at once.
#[test]
fn fixture_mir_is_well_formed() {
    let mut violations = Vec::new();
    let mut validated = 0;
    for case in load_cases() {
        let entry = entry_of(&case);
        if !entry.is_file() {
            continue;
        }
        let Ok(dump) = prism_cli::dump_typed_tree(&entry) else {
            continue;
        };
        if dump.type_errors.is_some() {
            continue;
        }
        validated += 1;
        match std::panic::catch_unwind(|| prism_mir::lower_file(&dump.tree)) {
            Ok(mir) => {
                for violation in mir.validate() {
                    violations.push(format!("{}: {}", case.name, violation));
                }
            }
            Err(panic) => violations.push(format!(
                "{}: lowering panicked: {:?}",
                case.name,
                panic_message(&panic)
            )),
        }
    }
    assert!(validated > 300, "only {} files lowered", validated);
    assert!(
        violations.is_empty(),
        "{} MIR violations:\n{}",
        violations.len(),
        violations.join("\n")
    );
}

/// mir: "Error-free MIR", over `examples/`: every example that typechecks lowers to well-formed
/// MIR (`examples/demo/ui` needs an unbuilt subsystem).
#[test]
fn example_mir_is_well_formed() {
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let mut files = Vec::new();
    prism_files(&examples, &mut files);
    let mut violations = Vec::new();
    for file in &files {
        if file.components().any(|c| c.as_os_str() == "ui") {
            continue;
        }
        let mir = match prism_cli::dump_mir(file) {
            Ok(dump) => dump.mir,
            Err(err) => panic!("{}: no MIR:\n{}", file.display(), err.render()),
        };
        for violation in mir.validate() {
            violations.push(format!("{}: {}", file.display(), violation));
        }
    }
    assert!(
        violations.is_empty(),
        "{} MIR violations:\n{}",
        violations.len(),
        violations.join("\n")
    );
}

/// mir: "Error-free MIR", after borrow checking. Elaborates the entry file of every case that
/// passes `prism check` and checks the elaborated MIR's structural invariants
/// (`MirFile::validate`) and its drops (`prism_borrowck::verify_elaborated`), reporting every
/// violation and every crash at once.
#[test]
fn fixture_elaborated_mir_is_well_formed() {
    let mut violations = Vec::new();
    let mut verified = 0;
    for case in load_cases() {
        if !case.expected.is_empty() {
            continue;
        }
        let entry = entry_of(&case);
        if !entry.is_file() {
            continue;
        }
        verified += 1;
        let elaborate = || {
            prism_cli::dump_elaborated_mir(&entry)
                .map(|dump| dump.mir)
                .map_err(|err| err.render())
        };
        match std::panic::catch_unwind(elaborate) {
            Ok(Ok(mir)) => {
                for violation in mir.validate() {
                    violations.push(format!("{}: {}", case.name, violation));
                }
                for violation in prism_borrowck::verify_elaborated(&mir) {
                    violations.push(format!("{}: {}", case.name, violation));
                }
            }
            Ok(Err(rendered)) => {
                violations.push(format!("{}: no elaborated MIR:\n{}", case.name, rendered))
            }
            Err(panic) => violations.push(format!(
                "{}: elaboration panicked: {:?}",
                case.name,
                panic_message(&panic)
            )),
        }
    }
    assert!(verified > 200, "only {} files elaborated", verified);
    assert!(
        violations.is_empty(),
        "{} elaborated MIR violations:\n{}",
        violations.len(),
        violations.join("\n")
    );
}

/// mir: "Error-free MIR", after borrow checking, over `examples/`: every example except those
/// needing an unbuilt subsystem (`examples/demo/ui`) elaborates to well-formed MIR whose drops
/// verify.
#[test]
fn example_elaborated_mir_is_well_formed() {
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let mut files = Vec::new();
    prism_files(&examples, &mut files);
    let mut violations = Vec::new();
    for file in &files {
        if file.components().any(|c| c.as_os_str() == "ui") {
            continue;
        }
        let mir = match prism_cli::dump_elaborated_mir(file) {
            Ok(dump) => dump.mir,
            Err(err) => panic!("{}: no elaborated MIR:\n{}", file.display(), err.render()),
        };
        for violation in mir.validate() {
            violations.push(format!("{}: {}", file.display(), violation));
        }
        for violation in prism_borrowck::verify_elaborated(&mir) {
            violations.push(format!("{}: {}", file.display(), violation));
        }
    }
    assert!(
        violations.is_empty(),
        "{} elaborated MIR violations:\n{}",
        violations.len(),
        violations.join("\n")
    );
}

/// code-generation: "Symbols Derive from Declaration Paths". Every body of the elaborated MIR of
/// every passing case's entry gets a symbol, and no two bodies of a file share one.
#[test]
fn fixture_symbols_are_distinct() {
    let mut files = 0;
    for case in load_cases() {
        if !case.expected.is_empty() {
            continue;
        }
        let entry = entry_of(&case);
        if !entry.is_file() {
            continue;
        }
        let Ok(dump) = prism_cli::dump_elaborated_mir(&entry) else {
            continue;
        };
        files += 1;
        let mut seen = BTreeSet::new();
        for (_, body) in dump.mir.bodies.iter() {
            let symbol = prism_codegen::symbols::symbol_of(body, &dump.mir);
            assert!(
                seen.insert(symbol.clone()),
                "{}: two bodies named {}",
                case.name,
                symbol
            );
        }
    }
    assert!(files > 200, "only {} files named", files);
}

/// code-generation: "Unsupported Constructs Are Rejected Before Code Generation". The backend
/// compiles every body of every passing case that its support check accepts, without an internal
/// error or a crash: what the check accepts, the lowering handles. Each entry is compiled alone,
/// its imports' slots left unlinked.
#[test]
fn supported_fixtures_compile() {
    let mut compiled = 0;
    let mut failures = Vec::new();
    for case in load_cases() {
        if !case.expected.is_empty() {
            continue;
        }
        let entry = entry_of(&case);
        if !entry.is_file() {
            continue;
        }
        let Ok(dump) = prism_cli::dump_elaborated_mir(&entry) else {
            continue;
        };
        if prism_codegen::check_file(&dump.mir).is_err() {
            continue;
        }
        compiled += 1;
        let mir = dump.mir;
        match std::panic::catch_unwind(move || prism_codegen::Program::compile(&[mir]).map(|_| ()))
        {
            Ok(Ok(())) => {}
            Ok(Err(error)) => failures.push(format!("{}: {}", case.name, error)),
            Err(panic) => failures.push(format!(
                "{}: code generation panicked: {}",
                case.name,
                panic_message(&panic)
            )),
        }
    }
    assert!(compiled > 100, "only {} files compiled", compiled);
    assert!(
        failures.is_empty(),
        "{} files failed to compile:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

fn panic_message(panic: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = panic.downcast_ref::<String>() {
        message.clone()
    } else if let Some(message) = panic.downcast_ref::<&str>() {
        message.to_string()
    } else {
        "<panic>".to_string()
    }
}

/// The passing `loops` cases that use a construct the Tier 1 backend rejects, and the construct.
const LOOPS_BEYOND_THE_BACKEND: &[(&str, &str)] = &[(
    "loops/loop-inside-a-closure",
    "function values and closures",
)];

/// code-generation: "Unsupported Constructs Are Rejected Before Code Generation". Every passing
/// case of the `loops` spec that uses only scalars, strings, nullables and ranges passes the backend's
/// support check; the others are rejected naming the construct they use.
#[test]
fn scalar_loop_fixtures_are_supported() {
    let mut supported = 0;
    for case in load_cases() {
        if !case.name.starts_with("loops/") || !case.expected.is_empty() {
            continue;
        }
        let entry = entry_of(&case);
        let mir = match prism_cli::dump_elaborated_mir(&entry) {
            Ok(dump) => dump.mir,
            Err(err) => panic!("{}: no elaborated MIR:\n{}", case.name, err.render()),
        };
        let beyond = LOOPS_BEYOND_THE_BACKEND
            .iter()
            .find(|(name, _)| *name == case.name);
        match (prism_codegen::check_file(&mir), beyond) {
            (Ok(()), None) => supported += 1,
            (Err(errors), Some((_, construct))) => {
                let expected = format!("the Tier 1 backend does not support {} yet", construct);
                assert!(
                    errors.iter().all(|error| error.message == expected),
                    "{}: {:?}",
                    case.name,
                    errors
                );
            }
            (Ok(()), Some(_)) => panic!("{} is now supported; drop it from the list", case.name),
            (Err(errors), None) => panic!("{} is rejected: {:?}", case.name, errors),
        }
    }
    assert!(supported >= 11, "only {} loops cases supported", supported);
}

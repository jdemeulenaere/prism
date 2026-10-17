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
            for line in text.lines() {
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
            cases.push(Case {
                dir,
                name,
                header,
                expected,
                pinned_current,
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
    if actual == case.expected {
        return Ok(());
    }
    Err(format!(
        "{}: expected {:?}, got {:?}\n{}",
        case.name, case.expected, actual, rendered
    ))
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

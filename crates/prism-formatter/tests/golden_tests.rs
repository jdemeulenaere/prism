//! Golden files for the scenarios of `openspec/specs/formatting`.
//!
//! Each case is `tests/golden/<scenario-slug>[--<form>].prism`, the input, beside
//! `<scenario-slug>[--<form>].expected.prism`, its expected formatting. The slug names a
//! `#### Scenario:` of the `formatting` spec (lower-cased, runs of non-alphanumerics replaced by
//! `-`), in the main spec or in an active change's delta. Every expected file must also format
//! to itself. `PRISM_BLESS=1 cargo test -p prism-formatter --test golden_tests` writes the
//! expected files instead; read each against its scenario's THEN before committing it.

use prism_formatter::format_source;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

fn golden_directory() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

fn slug(heading: &str) -> String {
    let mut slug = String::new();
    for character in heading.to_lowercase().chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character);
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    slug.trim_matches('-').to_string()
}

/// The scenario slugs of the `formatting` spec and of every active change's delta of it.
fn scenario_slugs() -> BTreeSet<String> {
    let openspec = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../openspec");
    let mut specs = vec![openspec.join("specs/formatting/spec.md")];
    if let Ok(changes) = fs::read_dir(openspec.join("changes")) {
        for change in changes {
            specs.push(change.unwrap().path().join("specs/formatting/spec.md"));
        }
    }
    let mut slugs = BTreeSet::new();
    for spec in specs {
        let Ok(text) = fs::read_to_string(&spec) else {
            continue;
        };
        for line in text.lines() {
            if let Some(heading) = line.strip_prefix("#### Scenario:") {
                slugs.insert(slug(heading.trim()));
            }
        }
    }
    slugs
}

#[test]
fn golden_files_format_to_their_expected_output() {
    let bless = std::env::var("PRISM_BLESS").is_ok_and(|value| value == "1");
    let slugs = scenario_slugs();
    assert!(!slugs.is_empty(), "no formatting scenarios found");
    let mut inputs: Vec<PathBuf> = fs::read_dir(golden_directory())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            let name = path.file_name().unwrap().to_string_lossy();
            name.ends_with(".prism") && !name.ends_with(".expected.prism")
        })
        .collect();
    inputs.sort();
    assert!(!inputs.is_empty(), "no golden files");
    let mut failures = Vec::new();
    for input in inputs {
        let name = input.file_stem().unwrap().to_string_lossy().to_string();
        let scenario = name.split("--").next().unwrap();
        if !slugs.contains(scenario) {
            failures.push(format!(
                "{}: '{}' is not a scenario of the formatting spec",
                input.display(),
                scenario
            ));
            continue;
        }
        let expected_path = input.with_file_name(format!("{}.expected.prism", name));
        let source = fs::read_to_string(&input).unwrap();
        let actual = match format_source(&source) {
            Ok(actual) => actual,
            Err(err) => {
                failures.push(format!("{}: {}", input.display(), err));
                continue;
            }
        };
        if bless {
            fs::write(&expected_path, &actual).unwrap();
            continue;
        }
        let expected = fs::read_to_string(&expected_path).unwrap_or_default();
        if actual != expected {
            failures.push(format!(
                "{}: formatted output differs\n--- expected:\n{}\n--- actual:\n{}",
                input.display(),
                expected,
                actual
            ));
        }
        match format_source(&expected) {
            Ok(again) if again == expected => {}
            _ => failures.push(format!(
                "{}: the expected output is not formatted",
                expected_path.display()
            )),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

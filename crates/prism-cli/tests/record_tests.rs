//! Generic records over the fixture corpus (D4 of `instance-cache`): every record of every
//! program that passes `prism check` round-trips through its canonical encoding.

use prism_mir::GenericRecord;
use std::fs;
use std::path::{Path, PathBuf};

fn fixtures_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn sorted_entries(dir: &Path) -> Vec<PathBuf> {
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    entries.sort();
    entries
}

fn prism_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for path in sorted_entries(dir) {
        if path.is_dir() {
            prism_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "prism") {
            out.push(path);
        }
    }
}

/// The entry file of every fixture case expected to pass `prism check`.
pub fn passing_entries() -> Vec<PathBuf> {
    let mut entries = Vec::new();
    for spec in sorted_entries(&fixtures_root()) {
        if !spec.is_dir() {
            continue;
        }
        for case in sorted_entries(&spec).into_iter().filter(|d| d.is_dir()) {
            let mut files = Vec::new();
            prism_files(&case, &mut files);
            for file in files {
                let text = fs::read_to_string(&file).unwrap();
                if !text.lines().any(|l| l.trim() == "// expect: ok") {
                    continue;
                }
                let entry = match text.lines().find_map(|l| l.strip_prefix("// entry:")) {
                    Some(relative) => case.join(relative.trim()),
                    None => file.clone(),
                };
                if entry.is_file() {
                    entries.push(entry);
                }
            }
        }
    }
    entries
}

#[test]
fn every_fixture_record_round_trips() {
    let mut records = 0;
    for entry in passing_entries() {
        let Ok(files) = prism_cli::program_mir(&entry) else {
            continue;
        };
        for file in &files {
            for record in GenericRecord::of_file(file) {
                let bytes = record.encode();
                let decoded = GenericRecord::decode(&bytes).unwrap_or_else(|| {
                    panic!("{}: {} does not decode", entry.display(), record.path)
                });
                assert_eq!(decoded, record, "{}: {}", entry.display(), record.path);
                assert_eq!(
                    decoded.encode(),
                    bytes,
                    "{}: {}",
                    entry.display(),
                    record.path
                );
                records += 1;
            }
        }
    }
    assert!(records > 100, "only {} records in the corpus", records);
}

// --- compilation-and-abi: "Monomorphization Hash of Generic Records" (D2 of `instance-cache`)

/// A fresh workspace directory holding `files`, each `(path, source)`.
fn workspace(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("prism_records_{}_{}", name, nanos));
    for (path, source) in files {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, source).unwrap();
    }
    root
}

/// The Monomorphization Hash of every generic of `lib/box.prism` in a workspace of `box_source`
/// and an `app/main.prism` importing it, by name, and the ABI hash of `lib/box`.
fn hashes(name: &str, box_source: &str) -> (Vec<(String, prism_abi::ContentHash)>, String) {
    let root = workspace(
        name,
        &[
            ("lib/box.prism", box_source),
            (
                "app/main.prism",
                "package app\nimport /prism/core\nimport /lib/box\nfun main() { println(1) }\n",
            ),
        ],
    );
    let entry = root.join("app/main.prism");
    let files = prism_cli::program_mir(&entry).unwrap_or_else(|error| panic!("{}", error.render()));
    let library = files.iter().find(|file| file.module == "lib/box").unwrap();
    let records: Vec<(String, prism_abi::ContentHash)> = GenericRecord::of_file(library)
        .into_iter()
        .map(|record| {
            (
                record.path.to_string(),
                prism_abi::ContentHash::of(&record.encode()),
            )
        })
        .collect();
    let report = prism_cli::check_path(&entry).unwrap();
    let abi = report
        .checked_files
        .iter()
        .find(|file| file.path.ends_with("lib/box.prism"))
        .unwrap()
        .abi_hash
        .clone();
    fs::remove_dir_all(&root).ok();
    (records, abi)
}

fn hash_of(records: &[(String, prism_abi::ContentHash)], name: &str) -> prism_abi::ContentHash {
    records
        .iter()
        .find(|(path, _)| path.ends_with(name))
        .unwrap_or_else(|| panic!("no record {} in {:?}", name, records))
        .1
}

#[test]
fn changing_a_generic_function_body_changes_its_hash() {
    let (before, abi_before) = hashes(
        "body_before",
        "package lib\npublic fun <T> keep(x: T): T = x\n",
    );
    let (after, abi_after) = hashes(
        "body_after",
        "package lib\npublic fun <T> keep(x: T): T { val y = x; return y }\n",
    );
    assert_ne!(hash_of(&before, "::keep"), hash_of(&after, "::keep"));
    assert_eq!(abi_before, abi_after);
}

#[test]
fn a_private_type_held_by_the_generic_gains_a_field() {
    let (before, abi_before) = hashes(
        "tag_before",
        "package lib\nstruct Tag(val n: Int)\nfun makeTag(): Tag = Tag(1)\n\
         public fun <T> tagged(x: T): Int = makeTag().n\n",
    );
    let (after, abi_after) = hashes(
        "tag_after",
        "package lib\nstruct Tag(val n: Int, val m: Int)\nfun makeTag(): Tag = Tag(1, 2)\n\
         public fun <T> tagged(x: T): Int = makeTag().n\n",
    );
    assert_ne!(hash_of(&before, "::tagged"), hash_of(&after, "::tagged"));
    assert_eq!(abi_before, abi_after);
}

const OUTER: &str = "package lib\npublic fun <T> outer(x: T, n: Int): Int = helper(n)\n";

#[test]
fn a_callees_signature_changes_under_an_unchanged_body() {
    let (before, _) = hashes(
        "callee_before",
        &format!("{}fun helper(n: Int): Int = n\n", OUTER),
    );
    let (after, _) = hashes(
        "callee_after",
        &format!("{}fun helper(n: &Int): Int = n\n", OUTER),
    );
    assert_ne!(hash_of(&before, "::outer"), hash_of(&after, "::outer"));
}

#[test]
fn a_non_generic_helpers_body_change_keeps_the_hash() {
    let (before, _) = hashes(
        "helper_before",
        &format!("{}fun helper(n: Int): Int = n\n", OUTER),
    );
    let (after, _) = hashes(
        "helper_after",
        &format!("{}fun helper(n: Int): Int = n + 0\n", OUTER),
    );
    assert_eq!(hash_of(&before, "::outer"), hash_of(&after, "::outer"));
}

#[test]
fn an_edit_after_the_generic_keeps_the_hash() {
    let base = format!("{}fun helper(n: Int): Int = n\n", OUTER);
    let (before, _) = hashes("after_before", &base);
    let (after, _) = hashes("after_after", &format!("{}fun later(): Int = 2\n", base));
    assert_eq!(hash_of(&before, "::outer"), hash_of(&after, "::outer"));
    // A line added before it moves its source positions, at which instances report runtime
    // errors.
    let (moved, _) = hashes(
        "after_moved",
        &format!("\n{}fun helper(n: Int): Int = n\n", OUTER),
    );
    assert_ne!(hash_of(&before, "::outer"), hash_of(&moved, "::outer"));
}

const BOX: &str = "package lib\npublic struct Box<T>(val v: T)\n";

#[test]
fn changing_a_generic_extension_body_changes_its_hash() {
    let (before, abi_before) = hashes(
        "extension_before",
        &format!("{}public fun <T> Box<T>.get(): &T = &v\n", BOX),
    );
    let (after, abi_after) = hashes(
        "extension_after",
        &format!(
            "{}public fun <T> Box<T>.get(): &T {{ val r = &v; return r }}\n",
            BOX
        ),
    );
    assert_ne!(hash_of(&before, ".get"), hash_of(&after, ".get"));
    assert_eq!(abi_before, abi_after);
}

#[test]
fn a_non_generic_extension_of_a_generic_type_has_no_hash() {
    let (before, abi_before) = hashes(
        "sum_before",
        &format!("{}public fun Box<Int>.sum(): Int = v\n", BOX),
    );
    let (_, abi_after) = hashes(
        "sum_after",
        &format!("{}public fun Box<Int>.sum(): Int = v + 1\n", BOX),
    );
    assert!(before.is_empty(), "{:?}", before);
    assert_eq!(abi_before, abi_after);
}

// --- compilation-and-abi: "Instances Are Planned per Program" (D7 of `instance-cache`)

/// The symbols of the instances `files` need, found as `monomorphization`'s worklist found them:
/// by instantiating each needed body and reading the calls of the instance.
fn instantiated_symbols(files: &[prism_mir::MirFile]) -> std::collections::BTreeSet<String> {
    use prism_codegen::symbols::callee_symbol;
    use prism_mir::{Body, TerminatorKind};
    use prism_typed_tree::instantiation::generic_name;
    use std::collections::{BTreeSet, HashMap, VecDeque};
    let mut records: HashMap<String, GenericRecord> = HashMap::new();
    let mut types = prism_typed_tree::TypeTable::new();
    for file in files {
        for record in GenericRecord::of_file(file) {
            records.insert(generic_name(&record.path), record);
        }
        types.extend_from(&file.types);
    }
    let mut seen = BTreeSet::new();
    let mut queue = VecDeque::new();
    let request = |body: &Body, queue: &mut VecDeque<_>, seen: &mut BTreeSet<String>| {
        for (_, block) in body.blocks.iter() {
            let TerminatorKind::Call {
                callee,
                type_arguments,
                ..
            } = &block.terminator.kind
            else {
                continue;
            };
            let (prism_mir::Callee::Function(path)
            | prism_mir::Callee::Default { function: path, .. }) = callee
            else {
                continue;
            };
            if type_arguments.is_empty() || !records.contains_key(&generic_name(path)) {
                continue;
            }
            let symbol = callee_symbol(callee, type_arguments).unwrap();
            if seen.insert(symbol) {
                queue.push_back((generic_name(path), callee.clone(), type_arguments.clone()));
            }
        }
    };
    for file in files {
        for (_, body) in file.bodies.iter() {
            if body.type_parameters.is_empty() {
                request(body, &mut queue, &mut seen);
            }
        }
    }
    while let Some((generic, callee, arguments)) = queue.pop_front() {
        let Some(bodies) = records[&generic].instantiate(&callee, &arguments, &types) else {
            continue;
        };
        for body in &bodies {
            request(body, &mut queue, &mut seen);
        }
    }
    seen
}

#[test]
fn planning_finds_the_instances_instantiation_finds() {
    let mut programs = 0;
    for entry in passing_entries() {
        let Ok(files) = prism_cli::program_mir(&entry) else {
            continue;
        };
        let expected = instantiated_symbols(&files);
        let mirs: Vec<&prism_mir::MirFile> = files.iter().collect();
        let Ok(units) = prism_codegen::collect_instances(&mirs) else {
            continue;
        };
        let planned: std::collections::BTreeSet<String> =
            units.into_iter().map(|unit| unit.symbol).collect();
        assert_eq!(planned, expected, "{}", entry.display());
        if !expected.is_empty() {
            programs += 1;
        }
    }
    assert!(programs > 50, "only {} programs with instances", programs);
}

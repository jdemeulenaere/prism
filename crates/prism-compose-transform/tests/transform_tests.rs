//! The compose transform over real source (`compose-transform`): change words, `remember`'s
//! lowering, keys under edits, and records of composed MIR.

use prism_mir::{Callee, ComposerOperation, GenericRecord, MirFile, Operand, TerminatorKind};
use prism_typed_tree::BuiltinFunction;

/// The composed MIR of `source`, a file `main` that must check and borrow-check.
fn compose(source: &str) -> MirFile {
    let file = prism_parser::parse_source(source).expect("the source parses");
    let mut checker = prism_typechecker::TypeChecker::new(None, "main")
        .with_prelude(prism_typechecker::test_prelude());
    let tree = checker.check_module(&file);
    assert!(
        checker.errors.is_empty(),
        "type errors: {:?}",
        checker.errors
    );
    let elaborated = prism_borrowck::check_file(&prism_mir::lower_file(&tree))
        .unwrap_or_else(|errors| panic!("borrow errors: {:?}", errors));
    let composed = prism_compose_transform::compose_file(&elaborated);
    let violations = composed.validate();
    assert!(
        violations.is_empty(),
        "{}\n{}",
        violations.join("\n"),
        composed.dump()
    );
    composed
}

/// The calls of the body named `name` of `file`, in block order.
fn calls<'f>(file: &'f MirFile, name: &str) -> Vec<&'f TerminatorKind> {
    let (_, body) = file
        .bodies
        .iter()
        .find(|(id, _)| file.body_name(*id) == name)
        .unwrap_or_else(|| panic!("no body {}", name));
    body.blocks
        .iter()
        .map(|(_, block)| &block.terminator.kind)
        .filter(|kind| matches!(kind, TerminatorKind::Call { .. }))
        .collect()
}

/// compose: "Callers Say What They Know of Each Argument". A forwarded parameter's bits are the
/// caller's own, shifted into the callee's position, built with `Bits`; a constant is static.
#[test]
fn a_forwarded_parameter_passes_its_bits() {
    let file = compose(
        "compose fun Inner(a: Int, b: Int) {}\n\
         compose fun Outer(n: Int, s: String) { Inner(7, n) }\n",
    );
    let inner = calls(&file, "main::Outer")
        .into_iter()
        .find(|kind| {
            matches!(kind, TerminatorKind::Call { callee: Callee::Function(path), .. } if path.name == "Inner")
        })
        .expect("the call of Inner");
    let TerminatorKind::Call { arguments, .. } = inner else {
        unreachable!()
    };
    // `7`, `n`, the composer, then one change word built from `n`'s bits.
    assert_eq!(arguments.len(), 4);
    assert!(
        matches!(arguments[3], Operand::Copy(..)),
        "{:?}",
        arguments[3]
    );
    let dump = file.dump();
    // Static for `7` at bits 1-2 (6), `n`'s bits shifted to bits 3-4.
    assert!(dump.contains("= const 6 @"), "{}", dump);
    assert!(dump.contains("<< const 3"), "{}", dump);
    // `Outer` is not skippable: it reads `n`'s bits from its own change word.
    assert!(dump.contains(">>> const 1"), "{}", dump);
}

/// compose: "Remembering With Keys". Each key is compared with its slot, then the value's
/// slot is read, and on a miss the calculation's result stored.
#[test]
fn remember_with_two_keys() {
    let file =
        compose("compose fun K(a: Int, b: Bool) { val v = remember(a, b) { 7 }; println(v) }\n");
    let operations: Vec<String> = calls(&file, "main::K")
        .into_iter()
        .filter_map(|kind| match kind {
            TerminatorKind::Call {
                callee: Callee::Composer(operation),
                ..
            } => Some(operation.name().to_string()),
            _ => None,
        })
        .collect();
    let count = |wanted: &str| operations.iter().filter(|name| *name == wanted).count();
    // Two kept parameters, each kept by one call, then the two keys compared.
    assert_eq!(count("keep"), 2, "{:?}", operations);
    assert_eq!(count("changed"), 2, "{:?}", operations);
    let dump = file.dump();
    assert!(dump.contains("composer changed<Bool>"), "{}", dump);
    assert!(dump.contains("remembered<Int>"), "{}", dump);
    assert!(dump.contains("set_slot<Int>"), "{}", dump);
    assert!(
        !file
            .bodies
            .iter()
            .any(|(_, body)| body.blocks.iter().any(|(_, block)| matches!(
                &block.terminator.kind,
                TerminatorKind::Call {
                    callee: Callee::Builtin(BuiltinFunction::Remember),
                    ..
                }
            ))),
        "remember is lowered"
    );
}

/// The keys of `Counter`'s groups in the composed dump of `source`: every key line of its body
/// and of its content lambda.
fn counter_keys(source: &str) -> Vec<String> {
    let file = compose(source);
    let mut keys = Vec::new();
    for (id, body) in file.bodies.iter() {
        let name = file.body_name(id);
        if !name.starts_with("main::Counter") || name.ends_with("#restart") {
            continue;
        }
        for (_, block) in body.blocks.iter() {
            if let TerminatorKind::Call {
                callee:
                    Callee::Composer(
                        ComposerOperation::StartGroup { key }
                        | ComposerOperation::StartRestartGroup { key },
                    ),
                ..
            } = &block.terminator.kind
            {
                keys.push(format!("{} {:016x} {}", name, key.hash, key.label));
            }
        }
    }
    keys
}

/// compose: "Unrelated edits keep keys".
#[test]
fn unrelated_edits_keep_keys() {
    let before = counter_keys(
        "compose fun Text(n: Int) { }\n\
         compose fun Button(n: Int) { }\n\
         compose fun Column(content: compose () -> Unit) { content() }\n\
         compose fun Counter(on: Bool) { if (on) { Column { Text(1) } } }\n",
    );
    let after = counter_keys(
        "compose fun Text(n: Int) { }\n\
         compose fun Button(n: Int) { }\n\
         compose fun Column(content: compose () -> Unit) { content() }\n\
         // A comment above Counter.\n\
         compose fun Counter(on: Bool) {\n    if (on) {\n        Button(0)\n        Column { Text(2) }\n    }\n}\n",
    );
    assert_eq!(before.len(), 3, "{:?}", before);
    assert_eq!(before, after);
}

/// mir: "Generic Composables Are Composed Once". A generic composable's record holds its
/// composed body and its restart function, and round-trips through the codec.
#[test]
fn a_composed_record_round_trips() {
    let file = compose("compose fun <T : Copy> Show(x: T) { val y = remember(x) { 1 } }\n");
    let records = GenericRecord::of_file(&file);
    assert_eq!(records.len(), 1);
    let record = &records[0];
    assert!(record
        .closures
        .iter()
        .any(|body| matches!(body.owner, prism_mir::BodyOwner::Restart { .. })));
    let bytes = record.encode();
    let decoded = GenericRecord::decode(&bytes).expect("the encoding decodes");
    assert_eq!(&decoded, record);
    let instance = record
        .instantiate(
            &Callee::Function(record.path.clone()),
            &[prism_typed_tree::Type::Int],
            &record.types,
        )
        .expect("an instance");
    assert_eq!(
        instance.len(),
        3,
        "the instance, its calculation closure and its restart function"
    );
}

/// compose: "Which composables are skippable", by the composed dump's headers.
#[test]
fn which_composables_are_skippable() {
    let file = compose(
        "struct Point : Copy { val x: Int }\n\
         compose fun A(n: Int, p: Point) {}\n\
         compose fun B(s: String) {}\n\
         compose fun C(r: &Int) {}\n\
         compose fun D(content: compose () -> Unit) {}\n\
         compose fun <T> E(x: T) {}\n\
         compose fun <T : Copy> F(x: T) {}\n\
         compose fun G(onClick: escaping () -> Unit) {}\n\
         compose fun H(): Int = 1\n\
         compose fun L(s: (&String)?, p: &Point) {}\n\
         compose fun M(r: &mut Int) {}\n",
    );
    let dump = file.dump();
    for name in ["A", "B", "C", "D", "E", "F", "G", "H", "L", "M"] {
        let header = dump
            .lines()
            .find(|line| {
                line.contains(&format!("fun main::{}(", name))
                    || line.contains(&format!("> main::{}(", name))
            })
            .unwrap_or_else(|| panic!("no header for {}:\n{}", name, dump));
        let skippable = header.contains("skippable");
        assert_eq!(
            skippable,
            matches!(name, "A" | "C" | "F" | "G" | "L"),
            "{}: {}",
            name,
            header
        );
    }
}

/// The key the composed body named `name` starts its own group with: the key of the first
/// restart or replaceable group its prologue starts, following the blocks from `bb0`.
fn started_body_key(file: &MirFile, name: &str) -> u64 {
    let (_, body) = file
        .bodies
        .iter()
        .find(|(id, _)| file.body_name(*id) == name)
        .unwrap_or_else(|| panic!("no body {}", name));
    let mut queue = std::collections::VecDeque::from([prism_mir::BlockId(0)]);
    let mut seen = std::collections::HashSet::new();
    while let Some(block) = queue.pop_front() {
        if !seen.insert(block) {
            continue;
        }
        let terminator = &body.blocks[block].terminator.kind;
        if let TerminatorKind::Call {
            callee:
                Callee::Composer(
                    ComposerOperation::StartGroup { key }
                    | ComposerOperation::StartRestartGroup { key },
                ),
            ..
        } = terminator
        {
            return key.hash;
        }
        queue.extend(terminator.successors());
    }
    panic!("{} starts no group", name)
}

/// compose-hot-reload, D5: the summary lists the body key of every composable function and
/// closure, in body order, equal to the key each starts its own group with; a plain closure
/// has none, and the keys of a generic composable go to its generic's summary. A codec round
/// trip keeps them.
#[test]
fn summaries_list_the_body_keys_of_composable_bodies() {
    let file = compose(
        "compose fun Wrap(content: compose () -> Unit) { content() }\n\
         compose fun Show() { val n = remember { 1 }; Wrap { println(n) } }\n\
         compose fun <T : Copy> Each(x: T) { Wrap { println(\"each\") } }\n",
    );
    let records = GenericRecord::of_file(&file);
    let hashes: Vec<[u8; 32]> = records.iter().map(|_| [0; 32]).collect();
    let summary = prism_mir::FileSummary::of(&file, &records, &hashes, None);
    let named: Vec<String> = file
        .bodies
        .iter()
        .filter(|(_, body)| body.body_key.is_some() && body.type_parameters.is_empty())
        .map(|(id, _)| file.body_name(id).to_string())
        .collect();
    assert_eq!(named.len(), 3, "{:?}", named);
    let expected: Vec<u64> = named
        .iter()
        .map(|name| started_body_key(&file, name))
        .collect();
    assert_eq!(summary.composable_bodies, expected);
    assert_eq!(
        summary.composable_bodies[..2],
        [
            prism_compose_transform::keys::body_key("main::Wrap"),
            prism_compose_transform::keys::body_key("main::Show")
        ]
    );
    assert_eq!(summary.generics.len(), 1);
    let generic = &summary.generics[0].composable_bodies;
    assert_eq!(generic.len(), 2, "Each and its content lambda");
    assert_eq!(
        generic[0],
        prism_compose_transform::keys::body_key("main::Each")
    );
    // The composable functions, generic ones included, are named by their declaration paths
    // (D14 of `strong-skipping`); closures are not.
    let key = prism_compose_transform::keys::body_key;
    assert_eq!(
        summary.composable_functions,
        [
            (key("main::Wrap"), "main::Wrap".to_string()),
            (key("main::Show"), "main::Show".to_string()),
            (key("main::Each"), "main::Each".to_string()),
        ]
    );
    let decoded = prism_mir::FileSummary::decode(&summary.encode()).expect("it decodes");
    assert_eq!(decoded, summary);
}

/// The version operand of each call of `main::Show` in the body named `name`, in block order:
/// `const <n>`, the name of the argument or capture copied, or `local` for a version local.
fn show_versions(file: &MirFile, name: &str) -> Vec<String> {
    let (_, body) = file
        .bodies
        .iter()
        .find(|(id, _)| file.body_name(*id) == name)
        .unwrap_or_else(|| panic!("no body {}", name));
    calls(file, name)
        .into_iter()
        .filter_map(|kind| match kind {
            TerminatorKind::Call {
                callee: Callee::Function(path),
                arguments,
                ..
            } if path.name == "Show" => Some(match arguments.last() {
                Some(Operand::Constant(prism_mir::Constant::Int(n))) => format!("const {}", n),
                Some(Operand::Copy(place, _)) => body.locals[place.local]
                    .name
                    .clone()
                    .unwrap_or_else(|| "local".to_string()),
                other => format!("{:?}", other),
            }),
            _ => None,
        })
        .collect()
}

/// Whether the body named `name` calls `state version`.
fn reads_a_version(file: &MirFile, name: &str) -> bool {
    calls(file, name).into_iter().any(|kind| {
        matches!(
            kind,
            TerminatorKind::Call {
                callee: Callee::State(prism_mir::StateOperation::Version),
                ..
            }
        )
    })
}

/// compose: "A Borrowed Argument's Identity Is Its Address and Its Source's Version": each
/// source of a borrow gives its version (D2 of `strong-skipping`).
#[test]
fn identities_of_each_source() {
    let file = compose(
        "val Title: String = \"t\"\n\
         struct Item { val title: String }\n\
         compose fun Show(s: &String) {}\n\
         compose fun Screen(items: &State<List<String>>, item: &Item, p: &String) {\n\
             val local = \"a\" + \"b\"\n\
             Show(local)\n\
             Show(Title)\n\
             Show(p)\n\
             Show(item.title)\n\
             val r = remember { \"x\" + \"y\" }\n\
             Show(r)\n\
             Show(items.value[0])\n\
         }\n",
    );
    // `items` takes `$version0`, unused (D21 of `strong-skipping`), `item` and `p` the next.
    assert_eq!(
        show_versions(&file, "main::Screen"),
        [
            "const 0",
            "const 1",
            "$version2",
            "$version1",
            "const 1",
            "local"
        ]
    );
    assert!(reads_a_version(&file, "main::Screen"));
}

/// The version of a `for` variable over a state's contents is the state's, through the loop.
#[test]
fn identities_through_a_loop_and_its_closures() {
    let file = compose(
        "compose fun Show(s: &String) {}\n\
         compose fun Screen(items: &State<List<String>>) {\n\
             for (s in items.value) { Show(s); key(s.length) { Show(s) } }\n\
         }\n",
    );
    assert_eq!(show_versions(&file, "main::Screen"), ["local"]);
    assert!(reads_a_version(&file, "main::Screen"));
    // The key lambda passes its hidden capture.
    assert_eq!(
        show_versions(&file, "main::Screen#closure0"),
        ["$version(s)"]
    );
}

/// A reference assigned on two branches gets a version local assigned on each; a call's result
/// has no identity.
#[test]
fn identities_of_branches_and_call_results() {
    let file = compose(
        "fun first(xs: &List<String>): &String = &xs[0]\n\
         compose fun Show(s: &String) {}\n\
         compose fun Screen(items: &State<List<String>>, c: Bool) {\n\
             val local = \"a\" + \"b\"\n\
             val r = if (c) &items.value[0] else &local\n\
             Show(r)\n\
             Show(first(&items.value))\n\
         }\n",
    );
    assert_eq!(show_versions(&file, "main::Screen"), ["local", "const 0"]);
    let (_, body) = file
        .bodies
        .iter()
        .find(|(id, _)| file.body_name(*id) == "main::Screen")
        .unwrap();
    // The version local of `r`'s value is assigned a constant on the branch of the local.
    let dump = file.dump();
    assert!(body.blocks.len() > 1, "{}", dump);
    assert!(reads_a_version(&file, "main::Screen"), "{}", dump);
}

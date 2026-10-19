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
    let changed = operations.iter().filter(|name| *name == "changed").count();
    // Two kept parameters compared, then the two keys.
    assert_eq!(changed, 4, "{:?}", operations);
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
         compose fun H(): Int = 1\n",
    );
    let dump = file.dump();
    for name in ["A", "B", "C", "D", "E", "F", "G", "H"] {
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
            matches!(name, "A" | "F" | "G"),
            "{}: {}",
            name,
            header
        );
    }
}

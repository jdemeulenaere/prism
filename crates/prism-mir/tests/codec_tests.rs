//! The record codec and file summaries (D4 and D5 of `instance-cache`).

use prism_mir::{FileSummary, GenericRecord, InstanceRequest, MirFile};
use prism_typed_tree::{DeclarationPath, Type};

/// The MIR of `source`, a file of module `module` that must typecheck.
fn lower_module(module: &str, source: &str) -> MirFile {
    let file = prism_parser::parse_source(source).expect("the source parses");
    let mut checker = prism_typechecker::TypeChecker::new(None, module)
        .with_prelude(prism_typechecker::test_prelude());
    let tree = checker.check_module(&file);
    assert!(
        checker.errors.is_empty(),
        "type errors: {:?}",
        checker.errors
    );
    prism_mir::lower_file(&tree)
}

fn lower(source: &str) -> MirFile {
    lower_module("main", source)
}

/// A generic of every kind of body and construct a record holds: defaults, nested closures,
/// owned and borrowed captures, copied, borrowed and widened paths, templates, lists, boxes, nullables, patterns and calls.
const CORPUS: &str = "struct Pair<A, B>(val a: A, val b: B)\n\
     enum Shape<T> { Dot, Point(T, Int) }\n\
     fun helper(n: Int): Int = n + 1\n\
     fun run(g: () -> Int): Int = g()\n\
     fun <T : Copy> pad(x: T, y: T = x, n: Int = helper(2)): Pair<T, T> = Pair(x, y)\n\
     fun <T : Clone> many(x: &T, n: Int): Int {\n\
         val xs = listOf(x.clone(), x.clone())\n\
         var total = 0\n\
         for (i in xs.indices) { total += i }\n\
         val h = Heap(n)\n\
         val hv = h.unbox()\n\
         val s = \"n = $n, total = ${total + 1}\"\n\
         val m: Int? = if (n > 0) n else null\n\
         val k = m ?: 3\n\
         val shape: Shape<Int> = Shape.Point(n, 2)\n\
         val r = when (shape) { Shape.Dot -> 0; Shape.Point(val a, val b) -> a + b }\n\
         val p = Pair(n, k)\n\
         val q = Pair(n, s.clone())\n\
         val copied = Heap({ p.a + 1 })\n\
         val widened = Heap({ q.b.length + q.a })\n\
         return run { run { total + hv + k + r + s.length + p.b } }\n\
     }\n\
     fun <T> Pair<T, T>.first(): &T = &a\n\
     fun main() { println(pad(1).a); println(many(&\"s\", 2)) }\n";

#[test]
fn a_record_round_trips_to_an_equal_record() {
    let file = lower(CORPUS);
    let records = GenericRecord::of_file(&file);
    assert_eq!(records.len(), 3);
    for record in &records {
        let bytes = record.encode();
        let decoded = GenericRecord::decode(&bytes).expect("the encoding decodes");
        assert_eq!(&decoded, record, "{}", record.path);
        assert_eq!(decoded.encode(), bytes, "re-encoding is identical");
    }
}

#[test]
fn two_encodings_of_one_record_are_identical() {
    let first = GenericRecord::of_file(&lower(CORPUS));
    let second = GenericRecord::of_file(&lower(CORPUS));
    for (a, b) in first.iter().zip(&second) {
        assert_eq!(a.encode(), b.encode(), "{}", a.path);
    }
}

#[test]
fn every_truncation_decodes_to_none() {
    for record in GenericRecord::of_file(&lower(CORPUS)) {
        let bytes = record.encode();
        for length in 0..bytes.len() {
            assert_eq!(
                GenericRecord::decode(&bytes[..length]),
                None,
                "{} cut at {}",
                record.path,
                length
            );
        }
        let mut longer = bytes.clone();
        longer.push(0);
        assert_eq!(GenericRecord::decode(&longer), None);
    }
}

#[test]
fn flipped_bytes_never_panic() {
    let record = &GenericRecord::of_file(&lower(CORPUS))[1];
    let bytes = record.encode();
    for index in 0..bytes.len() {
        for flip in [0x01u8, 0x80, 0xFF] {
            let mut corrupt = bytes.clone();
            corrupt[index] ^= flip;
            // Either a miss or some record: never a panic.
            let _ = GenericRecord::decode(&corrupt);
        }
    }
}

fn hash(bytes: &[u8]) -> [u8; 32] {
    // A stand-in for BLAKE3: these tests compare summaries, not hashes.
    let mut out = [0u8; 32];
    for (index, byte) in bytes.iter().enumerate() {
        out[index % 32] = out[index % 32].wrapping_mul(31).wrapping_add(*byte);
    }
    out
}

fn summarize(file: &MirFile) -> FileSummary {
    let records = GenericRecord::of_file(file);
    let hashes: Vec<[u8; 32]> = records
        .iter()
        .map(|record| hash(&record.encode()))
        .collect();
    FileSummary::of(file, &records, &hashes, None)
}

fn shown(requests: &[InstanceRequest]) -> Vec<String> {
    requests
        .iter()
        .map(|request| {
            let arguments: Vec<String> = request
                .type_arguments
                .iter()
                .map(ToString::to_string)
                .collect();
            let default = request.default.as_ref().map_or(String::new(), |default| {
                format!("#default({})", default.name)
            });
            format!("{}{}<{}>", request.callee, default, arguments.join(", "))
        })
        .collect()
}

#[test]
fn calls_of_own_and_imported_generics_are_roots() {
    let file = lower(
        "fun <T> keep(x: T): T = x\n\
         fun main() { println(keep(1)); println(keep(2)); println(keep(true)) }\n",
    );
    let summary = summarize(&file);
    assert_eq!(
        shown(&summary.roots),
        ["main::keep<Int>", "main::keep<Bool>"]
    );
    assert_eq!(summary.generics.len(), 1);
    assert!(summary.generics[0].calls.is_empty());

    // A call of a generic another module declares is a root too.
    let library = prism_parser::parse_source("package lib\npublic fun <T> keep(x: T): T = x\n")
        .expect("the library parses");
    let abi = prism_abi::FileAbi::extract(&library, "lib/box", &prism_abi::no_imports)
        .expect("the library's ABI extracts");
    let source = prism_parser::parse_source(
        "package app\nimport /lib/box\nfun main() { println(keep(\"s\")) }\n",
    )
    .expect("the caller parses");
    let mut checker = prism_typechecker::TypeChecker::new(Some("app".to_string()), "app/main")
        .with_prelude(prism_typechecker::test_prelude());
    checker.ingest_file_abi(&abi);
    let tree = checker.check_module(&source);
    assert!(checker.errors.is_empty(), "{:?}", checker.errors);
    let caller = summarize(&prism_mir::lower_file(&tree));
    assert_eq!(shown(&caller.roots), ["lib/box::keep<String>"]);
    assert!(caller.generics.is_empty());
}

#[test]
fn a_polymorphic_call_is_a_generic_call_and_not_a_root() {
    let file = lower(
        "fun <T> keep(x: T): T = x\n\
         fun <T> wrap(x: T): T = keep(x)\n\
         fun main() { println(wrap(1)) }\n",
    );
    let summary = summarize(&file);
    assert_eq!(shown(&summary.roots), ["main::wrap<Int>"]);
    let wrap = &summary.generics[1];
    assert_eq!(wrap.path.name, "wrap");
    assert_eq!(shown(&wrap.calls), ["main::keep<T>"]);
    assert!(wrap.calls[0].type_arguments[0].mentions_parameter());
}

#[test]
fn a_default_generators_call_counts_for_its_generic() {
    let file = lower(
        "fun <T> keep(x: T): T = x\n\
         fun <T : Copy> pad(x: T, y: Int = keep(3)): T = x\n\
         fun main() { println(pad(true)) }\n",
    );
    let summary = summarize(&file);
    assert_eq!(
        shown(&summary.roots),
        ["main::pad#default(y)<Bool>", "main::pad<Bool>"]
    );
    let pad = &summary.generics[1];
    assert!(pad.calls.is_empty());
    assert_eq!(pad.defaults.len(), 1);
    assert_eq!(pad.defaults[0].parameter, 1);
    assert_eq!(shown(&pad.defaults[0].calls), ["main::keep<Int>"]);
}

#[test]
fn a_closures_call_counts_for_its_generic() {
    let file = lower(
        "fun <T> keep(x: T): T = x\n\
         fun run(g: () -> Int): Int = g()\n\
         fun <T> later(x: T): Int = run { keep(1) }\n\
         fun main() { println(later(\"a\")); println(run { keep(2) }) }\n",
    );
    let summary = summarize(&file);
    // The closure of `main` is compiled, so its call is a root.
    assert_eq!(
        shown(&summary.roots),
        ["main::later<String>", "main::keep<Int>"]
    );
    assert_eq!(shown(&summary.generics[1].calls), ["main::keep<Int>"]);
}

#[test]
fn a_summary_round_trips() {
    let file = lower(CORPUS);
    let mut summary = summarize(&file);
    summary.entry_point = Some(DeclarationPath::new("main", "main"));
    assert!(!summary.roots.is_empty());
    let bytes = summary.encode();
    assert_eq!(FileSummary::decode(&bytes), Some(summary.clone()));
    for length in 0..bytes.len() {
        assert_eq!(FileSummary::decode(&bytes[..length]), None);
    }
    let types: Vec<&Type> = summary
        .roots
        .iter()
        .flat_map(|root| &root.type_arguments)
        .collect();
    assert!(types.iter().all(|ty| !ty.mentions_parameter()));
}

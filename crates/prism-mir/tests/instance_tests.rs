//! Generic records and instances of generic bodies (D2 and D6 of `monomorphization`).

use prism_mir::{Callee, GenericRecord, MirFile, MirPhase};
use prism_typed_tree::{Arena, NominalId, Type};

/// The MIR of `source`, a file `main` that must typecheck.
fn lower(source: &str) -> MirFile {
    let file = prism_parser::parse_source(source).expect("the source parses");
    let mut checker = prism_typechecker::TypeChecker::new(None, "main").with_library_imports();
    let tree = checker.check_module(&file);
    assert!(
        checker.errors.is_empty(),
        "type errors: {:?}",
        checker.errors
    );
    prism_mir::lower_file(&tree)
}

#[test]
fn a_record_per_generic_with_the_types_its_bodies_use() {
    let file = lower(
        "struct Secret(val n: Int)\n\
         struct Unrelated(val s: String)\n\
         fun <T> hide(x: T, n: Int = 1): Secret { val f = { 2 }; return Secret(n) }\n\
         public fun <T> keep(x: T): T = x\n\
         fun plain(u: Unrelated): Int = 1\n",
    );
    let records = GenericRecord::of_file(&file);
    let names: Vec<String> = records.iter().map(|r| r.path.to_string()).collect();
    assert_eq!(names, ["main::hide", "main::keep"]);
    let hide = &records[0];
    assert_eq!(hide.defaults.len(), 1);
    assert_eq!(hide.closures.len(), 1);
    assert!(hide
        .types
        .definition(&NominalId::new("main", "Secret"))
        .is_some());
    assert!(!hide.types.knows(&NominalId::new("main", "Unrelated")));
    assert!(!records[1].types.knows(&NominalId::new("main", "Secret")));
    let default = Callee::Default {
        function: hide.path.clone(),
        parameter: 1,
        name: "n".to_string(),
    };
    assert!(hide.body_for(&default).is_some());
}

/// The instance of the generic function `name` of `source` for `arguments`, as a file of its
/// own, after checking it is well formed.
fn instance(source: &str, name: &str, arguments: &[Type]) -> String {
    let file = lower(source);
    let record = GenericRecord::of_file(&file)
        .into_iter()
        .find(|record| record.path.name == name)
        .expect("a generic of that name");
    let mut types = record.types.clone();
    types.extend_from(&file.types.restricted_to(arguments));
    let body = prism_mir::instantiate(&record.function, arguments, &types);
    let mut bodies = Arena::new();
    bodies.alloc(body);
    let instance = MirFile {
        module: file.module.clone(),
        phase: MirPhase::Built,
        types,
        bodies,
        externals: Vec::new(),
    };
    let violations = instance.validate();
    assert!(
        violations.is_empty(),
        "{}\n{}",
        violations.join("\n"),
        instance.dump()
    );
    instance.dump()
}

fn nullable_int() -> Type {
    Type::Int.nullable()
}

#[test]
fn an_instance_drops_only_what_needs_a_drop() {
    let source = "fun <T> discard(x: T) {}\n";
    let ints = instance(source, "discard", &[Type::Int]);
    assert!(
        ints.contains("fun main::discard<Int>(_1: Int): Unit"),
        "{}",
        ints
    );
    assert!(!ints.contains("drop"), "{}", ints);
    let strings = instance(source, "discard", &[Type::String]);
    assert!(
        strings.contains("fun main::discard<String>(_1: String)"),
        "{}",
        strings
    );
    assert!(strings.contains("drop _1"), "{}", strings);
}

#[test]
fn wrapping_a_nullable_argument_is_the_value() {
    let source = "fun <T> wrap(a: T): T? = a\n";
    let flat = instance(source, "wrap", &[nullable_int()]);
    assert!(flat.contains("_0 = move _1"), "{}", flat);
    assert!(!flat.contains("= wrap"), "{}", flat);
    let wrapped = instance(source, "wrap", &[Type::Int]);
    assert!(wrapped.contains("_0 = wrap move _1"), "{}", wrapped);
}

#[test]
fn the_payload_of_a_flattened_nullable_is_the_place() {
    let source = "fun <T> first(a: T?, c: T): T = a ?: c\n";
    let flat = instance(source, "first", &[nullable_int()]);
    assert!(!flat.contains("as payload"), "{}", flat);
    assert!(flat.contains("discriminant"), "{}", flat);
    let plain = instance(source, "first", &[Type::Int]);
    assert!(plain.contains("as payload"), "{}", plain);
}

#[test]
fn elvis_on_a_type_parameter_in_both_instances() {
    let source = "fun <T> orElse(a: T, c: T): T = a ?: c\n";
    let plain = instance(source, "orElse", &[Type::Int]);
    assert!(plain.contains("_3 = wrap move _1"), "{}", plain);
    let flat = instance(source, "orElse", &[nullable_int()]);
    assert!(flat.contains("_3 = move _1"), "{}", flat);
    assert!(!flat.contains("as payload"), "{}", flat);
}

#[test]
fn a_clone_under_a_clone_bound() {
    let source = "fun <T : Clone> twin(x: &T): T = x.clone()\n";
    let strings = instance(source, "twin", &[Type::String]);
    assert!(
        strings.contains("fun main::twin<String>(_1: &String): String"),
        "{}",
        strings
    );
    assert!(strings.contains("clone"), "{}", strings);
}

#[test]
fn calls_keep_their_declared_parameter_types() {
    let source = "fun <T> id(x: T): T = x\nfun <T> twice(x: T): T = id(id(x))\n";
    let ints = instance(source, "twice", &[Type::Int]);
    assert!(ints.contains("call main::id<Int>"), "{}", ints);
}

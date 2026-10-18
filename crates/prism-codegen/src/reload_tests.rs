//! Tests of the code that makes reloads possible (`hot-reload`): stack checks as reload points.
//! Tests that run programs and request reloads change the runtime's process-wide stack limit,
//! so they live in `tests/reload_tests.rs`, a process of their own.

use crate::test_support::lower_source;
use crate::{check_file, dump_file};
use prism_mir::MirFile;

fn lowered(source: &str) -> MirFile {
    let file = lower_source(source);
    check_file(&file).unwrap_or_else(|errors| panic!("not supported: {:?}", errors));
    file
}

/// code-generation: "The cold path of a stack check".
#[test]
fn the_cold_path_of_a_stack_check() {
    let text = dump_file(&lowered(
        "fun helper(): Int = 1\nfun main() { println(helper()) }\n",
    ))
    .expect("the file dumps");
    let start = text.find("function main::main\n").expect("main is dumped");
    let main = &text[start..];
    let main = &main[..main.find("\n\nfunction ").unwrap_or(main.len())];
    let named = |symbol: &str| {
        main.lines()
            .find_map(|line| {
                let (name, rest) = line.trim().split_once(" = ")?;
                let rest = rest.strip_prefix("symbol ").unwrap_or(rest);
                rest.starts_with(&format!("%\"{}\"", symbol))
                    .then(|| name.to_string())
            })
            .unwrap_or_else(|| panic!("{} is not imported:\n{}", symbol, main))
    };
    let limit = named("prism_rt_stack_limit");
    let failed = named("prism_rt_stack_check_failed");
    let blocks: Vec<&str> = main.split("\n\n").collect();
    let entry = blocks
        .iter()
        .find(|block| block.starts_with("block0"))
        .expect("an entry block");
    assert!(
        entry.contains(&format!("symbol_value.i64 {}", limit)),
        "{}",
        main
    );
    assert!(entry.contains("load.i64"), "{}", main);
    assert!(entry.contains("get_stack_pointer"), "{}", main);
    assert!(entry.contains("icmp ult"), "{}", main);
    assert!(entry.contains("brif"), "{}", main);
    // The cold block asks the runtime and raises "stack overflow" only when it answers 1.
    let cold = blocks
        .iter()
        .find(|block| block.contains(&format!("call {}(", failed)))
        .unwrap_or_else(|| panic!("no block calls the runtime:\n{}", main));
    assert!(cold.lines().next().unwrap().ends_with("cold:"), "{}", cold);
    assert!(cold.contains("brif"), "{}", cold);
    let panic = named("prism_rt_panic");
    assert!(
        blocks
            .iter()
            .any(|block| block.contains(" cold:") && block.contains(&format!("call {}(", panic))),
        "{}",
        main
    );
}

/// code-generation: "Storage is imported by the unit": the accessor addresses the value's
/// storage and state as imported data symbols, which the linker resolves to the value table.
#[test]
fn storage_is_imported_by_the_unit() {
    use cranelift_module::{FuncOrDataId, Linkage, Module};
    let files = crate::test_support::lower_files(&[(
        "lib/config",
        "package lib\npublic val LIMIT: Int = 10\n",
    )]);
    let mut module = cranelift_jit::JITModule::new(cranelift_jit::JITBuilder::with_isa(
        crate::isa::isa().unwrap(),
        cranelift_module::default_libcall_names(),
    ));
    let unit = crate::CodeUnit::lower(&files[0], &mut module).expect("the unit lowers");
    for symbol in ["lib/config::LIMIT#storage", "lib/config::LIMIT#state"] {
        let Some(FuncOrDataId::Data(id)) = module.declarations().get_name(symbol) else {
            panic!("{} is not declared", symbol);
        };
        assert_eq!(
            module.declarations().get_data_decl(id).linkage,
            Linkage::Import,
            "{}",
            symbol
        );
    }
    assert_eq!(unit.values.len(), 1);
    assert_eq!(unit.values[0].symbol, "lib/config::LIMIT");
    assert_eq!((unit.values[0].size, unit.values[0].align), (8, 8));
    let text = dump_file(&files[0]).expect("the file dumps");
    let accessor = &text[text
        .find("function lib/config::LIMIT#get\n")
        .expect("the accessor")..];
    assert!(
        accessor.contains("%\"lib/config::LIMIT#storage\""),
        "{}",
        accessor
    );
    assert!(
        accessor.contains("%\"lib/config::LIMIT#state\""),
        "{}",
        accessor
    );
}

//! External functions (D5 of `standard-library`): `external fun` declarations of the bundled
//! library, runtime entry points called directly by the C symbol equal to their name.
//!
//! An external's machine signature is the one a Prism function of its signature has
//! ([`crate::representation::signature`]): scalars by value, a value held in memory and a
//! reference by address, a result held in memory through an address passed first, and no result
//! for `Unit` and `Nothing`, a `Nothing` result meaning the function never returns. The backend
//! compares it with the runtime's entry point before any code is generated, and reports a
//! missing symbol or another signature as `E6005` at the declaration.

use crate::error::{CodegenError, CodegenErrorKind};
use crate::representation::{is_pointer, representation, Representation};
use cranelift_codegen::ir::types;
use prism_compose::COMPOSE_FUNCTIONS;
use prism_mir::{ExternalDeclaration, MirFile};
use prism_runtime::{RuntimeFunction, RuntimeType, RUNTIME_FUNCTIONS};
use prism_typed_tree::Type;

/// The runtime's entry point named `symbol`, the one lookup every external and runtime import
/// goes through: the check of its declaration, the lowering of its calls and the unit's
/// imports. It searches the base runtime's table, then the compose runtime's (D2 of
/// `compose-runtime`).
pub fn entry_point(symbol: &str) -> Option<&'static RuntimeFunction> {
    RUNTIME_FUNCTIONS
        .iter()
        .chain(COMPOSE_FUNCTIONS)
        .find(|function| function.name == symbol)
}

/// Every entry point the JIT resolves by symbol, from both tables.
pub fn entry_points() -> impl Iterator<Item = &'static RuntimeFunction> {
    RUNTIME_FUNCTIONS.iter().chain(COMPOSE_FUNCTIONS)
}

/// The machine signature of an external taking `parameters` and returning `result`, as the
/// runtime's table writes signatures: the parameter types, the result type and whether it
/// never returns. `None` for a type without a representation (a type parameter).
pub fn machine_signature(
    parameters: &[Type],
    result: &Type,
) -> Option<(Vec<RuntimeType>, Option<RuntimeType>, bool)> {
    let mut machine = Vec::new();
    let result_type = match representation(result, types::I64)? {
        Representation::Memory => {
            machine.push(RuntimeType::Pointer);
            None
        }
        Representation::Scalar(_) => Some(runtime_type(result)?),
        Representation::Empty => None,
    };
    for parameter in parameters {
        match representation(parameter, types::I64)? {
            Representation::Memory => machine.push(RuntimeType::Pointer),
            Representation::Scalar(_) => machine.push(runtime_type(parameter)?),
            Representation::Empty => {}
        }
    }
    Some((machine, result_type, *result == Type::Nothing))
}

/// The runtime type of a value of `ty` held in one machine value.
fn runtime_type(ty: &Type) -> Option<RuntimeType> {
    Some(match ty {
        Type::Int => RuntimeType::I64,
        Type::Float => RuntimeType::F64,
        Type::Bool => RuntimeType::I8,
        Type::Char => RuntimeType::I32,
        Type::Nullable(inner) if is_pointer(inner) => RuntimeType::Pointer,
        other if is_pointer(other) => RuntimeType::Pointer,
        _ => return None,
    })
}

/// The text of a machine signature in a diagnostic: `(i64, ptr) -> f64`, with `-> !` for a
/// function that never returns.
fn signature_text(parameters: &[RuntimeType], result: Option<RuntimeType>, never: bool) -> String {
    let name = |ty: &RuntimeType| match ty {
        RuntimeType::I8 => "i8",
        RuntimeType::I32 => "i32",
        RuntimeType::I64 => "i64",
        RuntimeType::F64 => "f64",
        RuntimeType::Pointer => "ptr",
    };
    let parameters: Vec<&str> = parameters.iter().map(name).collect();
    let result = match (result, never) {
        (_, true) => " -> !".to_string(),
        (Some(result), false) => format!(" -> {}", name(&result)),
        (None, false) => String::new(),
    };
    format!("({}){}", parameters.join(", "), result)
}

/// `E6005` for each external `file` declares that the runtime lacks or declares with another
/// machine signature.
pub fn check_externals(file: &MirFile) -> Vec<CodegenError> {
    file.externals.iter().filter_map(check_external).collect()
}

fn check_external(external: &ExternalDeclaration) -> Option<CodegenError> {
    let symbol = &external.path.name;
    let mismatch = |message: String, notes: Vec<String>| CodegenError {
        kind: CodegenErrorKind::ExternalMismatch,
        message,
        span: external.span,
        notes,
        help: Some(
            "declare the external with the parameter and result types of the runtime's entry point"
                .to_string(),
        ),
    };
    let Some(entry) = entry_point(symbol) else {
        return Some(mismatch(
            format!("the runtime has no entry point '{}'", symbol),
            vec![format!(
                "an external is named by the C symbol of a function of the runtime's table; '{}' is not one",
                symbol
            )],
        ));
    };
    let declared = machine_signature(&external.parameters, &external.result);
    let runtime = (entry.parameters.to_vec(), entry.result, entry.never_returns);
    if declared.as_ref() == Some(&runtime) {
        return None;
    }
    let declared_text = declared.map_or_else(
        || "a signature without a machine representation".to_string(),
        |(parameters, result, never)| signature_text(&parameters, result, never),
    );
    let runtime_text = signature_text(&runtime.0, runtime.1, runtime.2);
    Some(mismatch(
        format!(
            "the external '{}' does not match the runtime's entry point",
            symbol
        ),
        vec![
            format!("the runtime's '{}' is {}", symbol, runtime_text),
            format!("the declaration passes {}", declared_text),
        ],
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn machine_signatures_follow_the_prism_convention() {
        let string_result = machine_signature(&[Type::Int], &Type::String).unwrap();
        assert_eq!(
            string_result,
            (vec![RuntimeType::Pointer, RuntimeType::I64], None, false)
        );
        let exit = machine_signature(&[Type::Int], &Type::Nothing).unwrap();
        assert_eq!(exit, (vec![RuntimeType::I64], None, true));
        let borrowed = Type::Reference {
            mutable: false,
            inner: Box::new(Type::String),
        };
        let print = machine_signature(&[borrowed], &Type::Unit).unwrap();
        assert_eq!(print, (vec![RuntimeType::Pointer], None, false));
        assert_eq!(
            machine_signature(&[Type::Bool, Type::Char, Type::Float], &Type::Int).unwrap(),
            (
                vec![RuntimeType::I8, RuntimeType::I32, RuntimeType::F64],
                Some(RuntimeType::I64),
                false
            )
        );
    }

    use crate::test_support::lower_files;

    const SYSTEM: &str = "package prism\n\
        public fun exitProcess(status: Int): Nothing = prism_rt_exit(status)\n\
        private external fun prism_rt_exit(status: Int): Nothing\n";

    /// code-generation: "A direct call of an external", and standard-library: "An external
    /// called from library code".
    #[test]
    fn a_direct_call_of_an_external() {
        let file = lower_files(&[("prism/system", SYSTEM)]).remove(0);
        assert!(check_externals(&file).is_empty());
        let clif = crate::dump_file(&file).expect("the unit compiles");
        let body = clif
            .split("\nfunction prism/")
            .find(|function| function.contains("exitProcess"))
            .expect("exitProcess has code");
        assert!(body.contains("prism_rt_exit"), "{}", body);
        assert!(!body.contains("call_indirect"), "a direct call: {}", body);
        assert!(
            body.contains("(i64)"),
            "status is passed as an i64: {}",
            body
        );
        // The external has no code, so no symbol and no slot.
        let symbols = crate::symbols::symbols_of(&file);
        assert!(!symbols
            .iter()
            .any(|symbol| symbol.contains("prism_rt_exit")));
    }

    /// standard-library: "A string result through an address".
    #[test]
    fn a_string_result_through_an_address() {
        let source = "package prism\n\
            public fun text(n: Int): String = prism_rt_int_to_string(n)\n\
            external fun prism_rt_int_to_string(value: Int): String\n";
        let file = lower_files(&[("prism/test", source)]).remove(0);
        assert!(check_externals(&file).is_empty(), "the declaration matches");
        let clif = crate::dump_file(&file).expect("the unit compiles");
        assert!(
            clif.contains("prism_rt_int_to_string") && !clif.contains("call_indirect"),
            "{}",
            clif
        );
        let entry = entry_point("prism_rt_int_to_string").unwrap();
        assert_eq!(entry.parameters, [RuntimeType::Pointer, RuntimeType::I64]);
        assert_eq!(entry.result, None);
    }

    /// code-generation: "An external with the wrong signature".
    #[test]
    fn an_external_with_the_wrong_signature() {
        let source = "package prism\nexternal fun prism_rt_print_int(value: Double): Unit\n";
        let file = lower_files(&[("prism/test", source)]).remove(0);
        let errors = crate::check_file(&file).expect_err("the signature differs");
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code(), "E6005");
        let notes = errors[0].notes.join("\n");
        assert!(
            notes.contains("(i64)") && notes.contains("(f64)"),
            "{}",
            notes
        );
        assert_eq!(errors[0].span.line, 2, "at the declaration");
    }

    /// code-generation: "An unknown external symbol".
    #[test]
    fn an_unknown_external_symbol() {
        let source = "package prism\nexternal fun prism_rt_missing(): Int\n";
        let file = lower_files(&[("prism/test", source)]).remove(0);
        let errors = crate::check_file(&file).expect_err("the runtime lacks it");
        assert_eq!(errors[0].code(), "E6005");
        assert!(
            errors[0]
                .message
                .contains("no entry point 'prism_rt_missing'"),
            "{}",
            errors[0].message
        );
    }

    #[test]
    fn the_exit_entry_point_matches_its_declaration() {
        let entry = entry_point("prism_rt_exit").unwrap();
        assert_eq!(
            machine_signature(&[Type::Int], &Type::Nothing).unwrap(),
            (entry.parameters.to_vec(), entry.result, entry.never_returns)
        );
        assert!(entry_point("prism_rt_missing").is_none());
    }
}

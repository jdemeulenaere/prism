//! `prism dump-clif`: the Cranelift IR of one file's code unit, as text (D23 of
//! `codegen-core`).
//!
//! The file is support-checked and lowered into a unit that is never linked, and each function
//! is printed in body order, then the unit's drop and clone glue in order of its symbols, each
//! headed by `function <symbol>`, with every external name (the
//! function itself, the slots it loads, the runtime functions it calls and its data) shown by
//! symbol. The text is deterministic for one compiler and host.

use crate::error::{BackendError, CodegenError};
use crate::isa::isa;
use crate::supported::check_file;
use crate::unit::CodeUnit;
use cranelift_codegen::ir::Function;
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{default_libcall_names, DataId, FuncId, Module};
use prism_mir::MirFile;
use std::fmt::Write;

/// Why a file has no IR to show.
#[derive(Debug)]
pub enum DumpError {
    /// The file uses constructs the backend does not support (`E6001`).
    Unsupported(Vec<CodegenError>),
    /// The backend failed internally.
    Backend(BackendError),
}

impl From<BackendError> for DumpError {
    fn from(error: BackendError) -> Self {
        DumpError::Backend(error)
    }
}

/// The Cranelift IR of every function the backend generates for `file`.
pub fn dump_file(file: &MirFile) -> Result<String, DumpError> {
    check_file(file).map_err(DumpError::Unsupported)?;
    let mut module = JITModule::new(JITBuilder::with_isa(isa()?, default_libcall_names()));
    let unit = CodeUnit::lower(file, &mut module)?;
    let mut text = String::new();
    // The functions in body order, then the glue in order of its symbols.
    for (index, function) in unit.functions.iter().chain(&unit.glue).enumerate() {
        if index > 0 {
            text.push('\n');
        }
        writeln!(text, "function {}", function.symbol).unwrap();
        text.push_str(&with_symbols(&function.function, &module));
    }
    Ok(text)
}

/// The text of `function` with each external name replaced by its symbol: `%"<symbol>"`.
fn with_symbols(function: &Function, module: &JITModule) -> String {
    let declarations = module.declarations();
    let name_of = |namespace: u32, index: u32| match namespace {
        0 => declarations
            .get_function_decl(FuncId::from_u32(index))
            .linkage_name(FuncId::from_u32(index))
            .into_owned(),
        _ => declarations
            .get_data_decl(DataId::from_u32(index))
            .linkage_name(DataId::from_u32(index))
            .into_owned(),
    };
    let text = function.display().to_string();
    let user_names: Vec<(u32, u32)> = function
        .params
        .user_named_funcs()
        .iter()
        .map(|(_, name)| (name.namespace, name.index))
        .collect();
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut position = 0;
    let mut copied = 0;
    while position < bytes.len() {
        // Names are written `u<namespace>:<index>` (the function itself and the functions it
        // calls) or `userextname<reference>` (the data it addresses).
        let starts_word = position == 0 || !bytes[position - 1].is_ascii_alphanumeric();
        let replacement = if starts_word {
            parse_user_name(&text[position..], &user_names)
        } else {
            None
        };
        match replacement {
            Some(((namespace, index), length)) => {
                out.push_str(&text[copied..position]);
                write!(out, "%\"{}\"", name_of(namespace, index)).unwrap();
                position += length;
                copied = position;
            }
            None => position += 1,
        }
    }
    out.push_str(&text[copied..]);
    out
}

/// The name at the start of `text`, `u<namespace>:<index>` or `userextname<reference>`, and its
/// length.
fn parse_user_name(text: &str, user_names: &[(u32, u32)]) -> Option<((u32, u32), usize)> {
    let digits = |text: &str| text.bytes().take_while(u8::is_ascii_digit).count();
    if let Some(rest) = text.strip_prefix("userextname") {
        let length = digits(rest);
        let reference: usize = rest[..length].parse().ok()?;
        return Some((*user_names.get(reference)?, "userextname".len() + length));
    }
    let rest = text.strip_prefix('u')?;
    let namespace_length = digits(rest);
    let rest_after = rest[namespace_length..].strip_prefix(':')?;
    let index_length = digits(rest_after);
    if namespace_length == 0 || index_length == 0 {
        return None;
    }
    let namespace = rest[..namespace_length].parse().ok()?;
    let index = rest_after[..index_length].parse().ok()?;
    Some(((namespace, index), 1 + namespace_length + 1 + index_length))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::lower_source;

    #[test]
    fn dumping_the_same_file_twice_gives_the_same_text() {
        let source = "val LIMIT: Int = 10\n\
             fun twice(n: Int): Int = n * 2\n\
             fun main() { println(twice(LIMIT) / 3) }\n";
        let first = dump_file(&lower_source(source)).unwrap();
        let second = dump_file(&lower_source(source)).unwrap();
        assert_eq!(first, second);
        for expected in [
            "function main::LIMIT#init\n",
            "function main::LIMIT#get\n",
            "function main::twice\n",
            "function main::main\n",
            "function %\"main::main\"(",
            "%\"main::twice#slot\"",
            "%\"main::LIMIT#get#slot\"",
            "%\"prism_rt_print_int\"",
            "%\"prism_rt_panic\"",
            "call_indirect",
        ] {
            assert!(first.contains(expected), "no {:?} in\n{}", expected, first);
        }
        assert!(!first.contains("userextname"), "{}", first);
    }

    #[test]
    fn concatenation_calls_the_runtime() {
        // The scenario "Concatenation calls the runtime" of the `code-generation` spec.
        let dump = dump_file(&lower_source(
            "fun join(a: &String, b: &String): String = a + b\n",
        ))
        .unwrap();
        // The import `fn<n> = %"prism_rt_string_concatenate" sig<m>`, and a call of it.
        let callee = dump
            .lines()
            .find_map(|line| {
                let (name, _) = line
                    .trim()
                    .split_once(" = %\"prism_rt_string_concatenate\"")?;
                Some(name.to_string())
            })
            .unwrap_or_else(|| panic!("no import of the concatenation in\n{}", dump));
        assert!(dump.contains(&format!("call {}(", callee)), "{}", dump);
        for platform in ["malloc", "free", "memcpy"] {
            assert!(!dump.contains(&format!("%\"{}\"", platform)), "{}", dump);
        }
        // The result's address, then the operands' addresses.
        assert!(
            dump.contains("function %\"main::join\"(i64, i64, i64)"),
            "{}",
            dump
        );
    }

    #[test]
    fn a_file_the_backend_rejects_has_no_dump() {
        match dump_file(&lower_source("fun f(g: () -> Int): Int = 1\n")) {
            Err(DumpError::Unsupported(errors)) => assert_eq!(errors[0].code(), "E6001"),
            other => panic!("expected E6001, got {:?}", other.map(|_| ())),
        }
    }
}

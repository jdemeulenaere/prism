//! The table of runtime functions generated code may call (D17 of `codegen-core`): each one's
//! symbol, address and signature, from which the code generator declares its imports and the JIT
//! resolves them.

use crate::allocation::{
    prism_rt_allocate, prism_rt_free, prism_rt_reallocate, prism_rt_register_top_level_value,
};
use crate::list::{
    prism_rt_list_close_gap, prism_rt_list_empty, prism_rt_list_free, prism_rt_list_grow,
    prism_rt_list_index_out_of_range, prism_rt_list_open_gap, prism_rt_list_reverse,
    prism_rt_list_with_capacity,
};
use crate::output::{
    prism_rt_print_bool, prism_rt_print_char, prism_rt_print_double, prism_rt_print_int,
    prism_rt_print_newline,
};
use crate::panic::{
    prism_rt_double_remainder, prism_rt_panic, prism_rt_panic_without_location,
    prism_rt_unlinked_function,
};
use crate::string::{
    prism_rt_bool_to_string, prism_rt_char_to_string, prism_rt_double_to_string,
    prism_rt_int_to_string, prism_rt_print_string, prism_rt_string_character,
    prism_rt_string_clone, prism_rt_string_concatenate, prism_rt_string_drop,
    prism_rt_string_equals, prism_rt_string_template,
};

/// The type of a runtime function's parameter or result, as the code generator maps it to a
/// machine type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeType {
    /// An 8-bit integer: a `Bool`.
    I8,
    /// A 32-bit integer: a `Char`, a panic kind, a line or column.
    I32,
    /// A 64-bit integer: an `Int`.
    I64,
    /// A 64-bit float: a `Double`.
    F64,
    /// A pointer or a pointer-sized length.
    Pointer,
}

/// One runtime function.
#[derive(Debug, Clone, Copy)]
pub struct RuntimeFunction {
    /// Its symbol, which generated code imports.
    pub name: &'static str,
    /// Its address in this process.
    pub address: RuntimeAddress,
    pub parameters: &'static [RuntimeType],
    /// Its result; `None` for a function returning nothing or never returning.
    pub result: Option<RuntimeType>,
    /// Whether it never returns, so a call of it ends its block.
    pub never_returns: bool,
}

/// The address of a runtime function.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeAddress(*const u8);

// SAFETY: the address of a function is immutable and valid on every thread.
unsafe impl Sync for RuntimeAddress {}
unsafe impl Send for RuntimeAddress {}

impl RuntimeAddress {
    pub fn as_ptr(self) -> *const u8 {
        self.0
    }
}

use RuntimeType::{Pointer, F64, I32, I64, I8};

/// Every `extern "C"` function of the runtime, once each.
pub static RUNTIME_FUNCTIONS: &[RuntimeFunction] = &[
    RuntimeFunction {
        name: "prism_rt_reallocate",
        address: RuntimeAddress(prism_rt_reallocate as *const u8),
        parameters: &[Pointer, Pointer, Pointer, Pointer],
        result: Some(Pointer),
        never_returns: false,
    },
    RuntimeFunction {
        name: "prism_rt_list_with_capacity",
        address: RuntimeAddress(prism_rt_list_with_capacity as *const u8),
        parameters: &[Pointer, Pointer, Pointer, Pointer],
        result: None,
        never_returns: false,
    },
    RuntimeFunction {
        name: "prism_rt_list_grow",
        address: RuntimeAddress(prism_rt_list_grow as *const u8),
        parameters: &[Pointer, Pointer, Pointer],
        result: None,
        never_returns: false,
    },
    RuntimeFunction {
        name: "prism_rt_list_open_gap",
        address: RuntimeAddress(prism_rt_list_open_gap as *const u8),
        parameters: &[Pointer, I64, Pointer, Pointer, Pointer, Pointer, I32, I32],
        result: Some(Pointer),
        never_returns: false,
    },
    RuntimeFunction {
        name: "prism_rt_list_close_gap",
        address: RuntimeAddress(prism_rt_list_close_gap as *const u8),
        parameters: &[Pointer, Pointer, Pointer],
        result: None,
        never_returns: false,
    },
    RuntimeFunction {
        name: "prism_rt_list_reverse",
        address: RuntimeAddress(prism_rt_list_reverse as *const u8),
        parameters: &[Pointer, Pointer],
        result: None,
        never_returns: false,
    },
    RuntimeFunction {
        name: "prism_rt_list_free",
        address: RuntimeAddress(prism_rt_list_free as *const u8),
        parameters: &[Pointer, Pointer, Pointer],
        result: None,
        never_returns: false,
    },
    RuntimeFunction {
        name: "prism_rt_list_index_out_of_range",
        address: RuntimeAddress(prism_rt_list_index_out_of_range as *const u8),
        parameters: &[I64, I64, Pointer, Pointer, I32, I32],
        result: None,
        never_returns: true,
    },
    RuntimeFunction {
        name: "prism_rt_list_empty",
        address: RuntimeAddress(prism_rt_list_empty as *const u8),
        parameters: &[Pointer, Pointer, I32, I32],
        result: None,
        never_returns: true,
    },
    RuntimeFunction {
        name: "prism_rt_print_int",
        address: RuntimeAddress(prism_rt_print_int as *const u8),
        parameters: &[I64],
        result: None,
        never_returns: false,
    },
    RuntimeFunction {
        name: "prism_rt_print_double",
        address: RuntimeAddress(prism_rt_print_double as *const u8),
        parameters: &[F64],
        result: None,
        never_returns: false,
    },
    RuntimeFunction {
        name: "prism_rt_print_bool",
        address: RuntimeAddress(prism_rt_print_bool as *const u8),
        parameters: &[I8],
        result: None,
        never_returns: false,
    },
    RuntimeFunction {
        name: "prism_rt_print_char",
        address: RuntimeAddress(prism_rt_print_char as *const u8),
        parameters: &[I32],
        result: None,
        never_returns: false,
    },
    RuntimeFunction {
        name: "prism_rt_print_newline",
        address: RuntimeAddress(prism_rt_print_newline as *const u8),
        parameters: &[],
        result: None,
        never_returns: false,
    },
    RuntimeFunction {
        name: "prism_rt_double_remainder",
        address: RuntimeAddress(prism_rt_double_remainder as *const u8),
        parameters: &[F64, F64],
        result: Some(F64),
        never_returns: false,
    },
    RuntimeFunction {
        name: "prism_rt_panic",
        address: RuntimeAddress(prism_rt_panic as *const u8),
        parameters: &[I32, Pointer, Pointer, Pointer, Pointer, I32, I32],
        result: None,
        never_returns: true,
    },
    RuntimeFunction {
        name: "prism_rt_panic_without_location",
        address: RuntimeAddress(prism_rt_panic_without_location as *const u8),
        parameters: &[I32],
        result: None,
        never_returns: true,
    },
    RuntimeFunction {
        name: "prism_rt_string_concatenate",
        address: RuntimeAddress(prism_rt_string_concatenate as *const u8),
        parameters: &[Pointer, Pointer, Pointer],
        result: None,
        never_returns: false,
    },
    RuntimeFunction {
        name: "prism_rt_string_template",
        address: RuntimeAddress(prism_rt_string_template as *const u8),
        parameters: &[Pointer, Pointer, Pointer],
        result: None,
        never_returns: false,
    },
    RuntimeFunction {
        name: "prism_rt_string_equals",
        address: RuntimeAddress(prism_rt_string_equals as *const u8),
        parameters: &[Pointer, Pointer],
        result: Some(I8),
        never_returns: false,
    },
    RuntimeFunction {
        name: "prism_rt_string_character",
        address: RuntimeAddress(prism_rt_string_character as *const u8),
        parameters: &[Pointer, I64, Pointer, Pointer, I32, I32],
        result: Some(I32),
        never_returns: false,
    },
    RuntimeFunction {
        name: "prism_rt_string_clone",
        address: RuntimeAddress(prism_rt_string_clone as *const u8),
        parameters: &[Pointer, Pointer],
        result: None,
        never_returns: false,
    },
    RuntimeFunction {
        name: "prism_rt_string_drop",
        address: RuntimeAddress(prism_rt_string_drop as *const u8),
        parameters: &[Pointer],
        result: None,
        never_returns: false,
    },
    RuntimeFunction {
        name: "prism_rt_print_string",
        address: RuntimeAddress(prism_rt_print_string as *const u8),
        parameters: &[Pointer],
        result: None,
        never_returns: false,
    },
    RuntimeFunction {
        name: "prism_rt_int_to_string",
        address: RuntimeAddress(prism_rt_int_to_string as *const u8),
        parameters: &[Pointer, I64],
        result: None,
        never_returns: false,
    },
    RuntimeFunction {
        name: "prism_rt_double_to_string",
        address: RuntimeAddress(prism_rt_double_to_string as *const u8),
        parameters: &[Pointer, F64],
        result: None,
        never_returns: false,
    },
    RuntimeFunction {
        name: "prism_rt_bool_to_string",
        address: RuntimeAddress(prism_rt_bool_to_string as *const u8),
        parameters: &[Pointer, I8],
        result: None,
        never_returns: false,
    },
    RuntimeFunction {
        name: "prism_rt_char_to_string",
        address: RuntimeAddress(prism_rt_char_to_string as *const u8),
        parameters: &[Pointer, I32],
        result: None,
        never_returns: false,
    },
    RuntimeFunction {
        name: "prism_rt_allocate",
        address: RuntimeAddress(prism_rt_allocate as *const u8),
        parameters: &[Pointer, Pointer],
        result: Some(Pointer),
        never_returns: false,
    },
    RuntimeFunction {
        name: "prism_rt_free",
        address: RuntimeAddress(prism_rt_free as *const u8),
        parameters: &[Pointer, Pointer, Pointer],
        result: None,
        never_returns: false,
    },
    RuntimeFunction {
        name: "prism_rt_register_top_level_value",
        address: RuntimeAddress(prism_rt_register_top_level_value as *const u8),
        parameters: &[Pointer, Pointer],
        result: None,
        never_returns: false,
    },
    RuntimeFunction {
        name: "prism_rt_unlinked_function",
        address: RuntimeAddress(prism_rt_unlinked_function as *const u8),
        parameters: &[],
        result: None,
        never_returns: true,
    },
];

/// The runtime function named `name`.
pub fn runtime_function(name: &str) -> Option<&'static RuntimeFunction> {
    RUNTIME_FUNCTIONS
        .iter()
        .find(|function| function.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// The `extern "C"` functions this crate's sources define, read from the sources themselves
    /// so a function added without a table entry fails this test.
    fn exported_functions() -> HashSet<String> {
        let sources = [
            include_str!("output.rs"),
            include_str!("panic.rs"),
            include_str!("double.rs"),
            include_str!("allocation.rs"),
            include_str!("string.rs"),
            include_str!("list.rs"),
        ];
        let mut names = HashSet::new();
        for source in sources {
            for line in source.lines() {
                let line = line.trim_start();
                let Some(rest) = line
                    .strip_prefix("pub extern \"C\" fn ")
                    .or_else(|| line.strip_prefix("pub unsafe extern \"C\" fn "))
                else {
                    continue;
                };
                let name: String = rest
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                names.insert(name);
            }
        }
        names
    }

    #[test]
    fn every_exported_function_is_listed_once() {
        let listed: Vec<&str> = RUNTIME_FUNCTIONS.iter().map(|f| f.name).collect();
        let unique: HashSet<&str> = listed.iter().copied().collect();
        assert_eq!(listed.len(), unique.len(), "a function is listed twice");
        let exported = exported_functions();
        let listed: HashSet<String> = unique.iter().map(|s| s.to_string()).collect();
        assert_eq!(listed, exported);
        for function in RUNTIME_FUNCTIONS {
            assert!(!function.address.as_ptr().is_null());
            assert!(!(function.never_returns && function.result.is_some()));
        }
        assert!(runtime_function("prism_rt_panic").is_some_and(|f| f.never_returns));
    }
}

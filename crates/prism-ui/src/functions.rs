//! The table of UI entry points generated code may call (D2 of `ui-toolkit`): each one's
//! symbol, address and machine signature, in the form of `prism-runtime`'s table, which the
//! backend's entry-point lookup searches after `RUNTIME_FUNCTIONS` and `COMPOSE_FUNCTIONS`.

use crate::entry::*;
use prism_runtime::{RuntimeAddress, RuntimeFunction, RuntimeType};
use RuntimeType::{Pointer, I64, I8};

const fn address(function: *const ()) -> RuntimeAddress {
    RuntimeAddress::new(function.cast())
}

/// Every `extern "C"` function of the UI toolkit, once each. Node and handler handles, colours,
/// codes and sizes are `I64`, `Bool`s `I8`, and strings, modifiers and function values passed
/// by address `Pointer`.
pub static UI_FUNCTIONS: &[RuntimeFunction] = &[
    RuntimeFunction {
        name: "prism_ui_set_text",
        address: address(prism_ui_set_text as *const ()),
        parameters: &[I64, Pointer],
        result: None,
        never_returns: false,
    },
    RuntimeFunction {
        name: "prism_ui_set_text_style",
        address: address(prism_ui_set_text_style as *const ()),
        parameters: &[I64, I64, I64],
        result: None,
        never_returns: false,
    },
    RuntimeFunction {
        name: "prism_ui_set_modifier",
        address: address(prism_ui_set_modifier as *const ()),
        parameters: &[I64, Pointer],
        result: None,
        never_returns: false,
    },
    RuntimeFunction {
        name: "prism_ui_set_arrangement",
        address: address(prism_ui_set_arrangement as *const ()),
        parameters: &[I64, I64, I64],
        result: None,
        never_returns: false,
    },
    RuntimeFunction {
        name: "prism_ui_set_alignment",
        address: address(prism_ui_set_alignment as *const ()),
        parameters: &[I64, I64],
        result: None,
        never_returns: false,
    },
    RuntimeFunction {
        name: "prism_ui_set_content_color",
        address: address(prism_ui_set_content_color as *const ()),
        parameters: &[I64, I64],
        result: None,
        never_returns: false,
    },
    RuntimeFunction {
        name: "prism_ui_set_enabled",
        address: address(prism_ui_set_enabled as *const ()),
        parameters: &[I64, I8],
        result: None,
        never_returns: false,
    },
    RuntimeFunction {
        name: "prism_ui_set_on_value_change",
        address: address(prism_ui_set_on_value_change as *const ()),
        parameters: &[I64, I64],
        result: None,
        never_returns: false,
    },
    RuntimeFunction {
        name: "prism_ui_register_click",
        address: address(prism_ui_register_click as *const ()),
        parameters: &[Pointer],
        result: Some(I64),
        never_returns: false,
    },
    RuntimeFunction {
        name: "prism_ui_register_toggle",
        address: address(prism_ui_register_toggle as *const ()),
        parameters: &[Pointer],
        result: Some(I64),
        never_returns: false,
    },
    RuntimeFunction {
        name: "prism_ui_register_text",
        address: address(prism_ui_register_text as *const ()),
        parameters: &[Pointer],
        result: Some(I64),
        never_returns: false,
    },
    RuntimeFunction {
        name: "prism_ui_handlers_equal",
        address: address(prism_ui_handlers_equal as *const ()),
        parameters: &[I64, I64],
        result: Some(I8),
        never_returns: false,
    },
    RuntimeFunction {
        name: "prism_ui_modifier_overflow",
        address: address(prism_ui_modifier_overflow as *const ()),
        parameters: &[],
        result: None,
        never_returns: true,
    },
    RuntimeFunction {
        name: "prism_ui_register_measure_policy",
        address: address(prism_ui_register_measure_policy as *const ()),
        parameters: &[Pointer],
        result: Some(I64),
        never_returns: false,
    },
    RuntimeFunction {
        name: "prism_ui_register_layout_modifier",
        address: address(prism_ui_register_layout_modifier as *const ()),
        parameters: &[Pointer],
        result: Some(I64),
        never_returns: false,
    },
    RuntimeFunction {
        name: "prism_ui_set_measure_policy",
        address: address(prism_ui_set_measure_policy as *const ()),
        parameters: &[I64, I64],
        result: None,
        never_returns: false,
    },
    RuntimeFunction {
        name: "prism_ui_measure",
        address: address(prism_ui_measure as *const ()),
        parameters: &[Pointer, I64, I64, Pointer],
        result: None,
        never_returns: false,
    },
    RuntimeFunction {
        name: "prism_ui_measured",
        address: address(prism_ui_measured as *const ()),
        parameters: &[Pointer, I64],
        result: None,
        never_returns: false,
    },
    RuntimeFunction {
        name: "prism_ui_begin_placement",
        address: address(prism_ui_begin_placement as *const ()),
        parameters: &[I64, I64, I64, I64],
        result: None,
        never_returns: false,
    },
    RuntimeFunction {
        name: "prism_ui_end_placement",
        address: address(prism_ui_end_placement as *const ()),
        parameters: &[I64, I64],
        result: None,
        never_returns: false,
    },
    RuntimeFunction {
        name: "prism_ui_layout_rest",
        address: address(prism_ui_layout_rest as *const ()),
        parameters: &[I64, I64, I64, I64, I64, I64],
        result: None,
        never_returns: false,
    },
    RuntimeFunction {
        name: "prism_ui_place",
        address: address(prism_ui_place as *const ()),
        parameters: &[I64, I64, I64, I64, I64, I64],
        result: None,
        never_returns: false,
    },
    RuntimeFunction {
        name: "prism_ui_line_properties",
        address: address(prism_ui_line_properties as *const ()),
        parameters: &[Pointer, I64],
        result: None,
        never_returns: false,
    },
    RuntimeFunction {
        name: "prism_ui_alignment",
        address: address(prism_ui_alignment as *const ()),
        parameters: &[Pointer, I64],
        result: None,
        never_returns: false,
    },
    RuntimeFunction {
        name: "prism_ui_parent_data",
        address: address(prism_ui_parent_data as *const ()),
        parameters: &[Pointer, I64],
        result: None,
        never_returns: false,
    },
    RuntimeFunction {
        name: "prism_ui_run_app",
        address: address(prism_ui_run_app as *const ()),
        parameters: &[Pointer, I64, I64, Pointer, Pointer],
        result: None,
        never_returns: false,
    },
];

/// The UI entry point named `name`.
pub fn ui_function(name: &str) -> Option<&'static RuntimeFunction> {
    UI_FUNCTIONS.iter().find(|function| function.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// The `extern "C"` functions this crate's sources define, read from the sources themselves
    /// so a function added without a table entry fails this test.
    fn exported_functions() -> HashSet<String> {
        let mut names = HashSet::new();
        for line in include_str!("entry.rs").lines() {
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
        names
    }

    #[test]
    fn every_entry_point_is_listed_once() {
        let listed: Vec<&str> = UI_FUNCTIONS.iter().map(|f| f.name).collect();
        let unique: HashSet<&str> = listed.iter().copied().collect();
        assert_eq!(listed.len(), unique.len(), "an entry point is listed twice");
        let listed: HashSet<String> = unique.iter().map(|s| s.to_string()).collect();
        assert_eq!(listed, exported_functions());
        for function in UI_FUNCTIONS {
            assert!(!function.address.as_ptr().is_null());
            assert!(function.name.starts_with("prism_ui_"));
            assert!(!(function.never_returns && function.result.is_some()));
        }
        assert!(ui_function("prism_ui_run_app").is_some());
        assert!(ui_function("prism_compose_start").is_none());
    }
}

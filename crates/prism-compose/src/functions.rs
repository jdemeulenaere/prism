//! The table of compose entry points generated code may call (D2 of `compose-runtime`): each
//! one's symbol, address and machine signature, in the form of `prism-runtime`'s table, from
//! which the backend checks externals (`E6005`), declares its imports and the JIT resolves
//! them.

use prism_runtime::RuntimeFunction;

/// Every `extern "C"` function of the compose runtime, once each.
pub static COMPOSE_FUNCTIONS: &[RuntimeFunction] = &[];

/// The compose entry point named `name`.
pub fn compose_function(name: &str) -> Option<&'static RuntimeFunction> {
    COMPOSE_FUNCTIONS
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

    /// compose: "Every entry point is listed once".
    #[test]
    fn every_entry_point_is_listed_once() {
        let listed: Vec<&str> = COMPOSE_FUNCTIONS.iter().map(|f| f.name).collect();
        let unique: HashSet<&str> = listed.iter().copied().collect();
        assert_eq!(listed.len(), unique.len(), "an entry point is listed twice");
        let listed: HashSet<String> = unique.iter().map(|s| s.to_string()).collect();
        assert_eq!(listed, exported_functions());
        for function in COMPOSE_FUNCTIONS {
            assert!(!function.address.as_ptr().is_null());
            assert!(function.name.starts_with("prism_compose_"));
            assert!(!(function.never_returns && function.result.is_some()));
        }
    }

    /// compose: "Every entry point is listed once": the crate depends on `prism-runtime` only.
    #[test]
    fn the_crate_depends_on_the_runtime_only() {
        let manifest = include_str!("../Cargo.toml");
        let mut section = "";
        let mut dependencies = Vec::new();
        for line in manifest.lines() {
            let line = line.trim();
            if line.starts_with('[') {
                section = line;
                continue;
            }
            if section.contains("dependencies") && !line.is_empty() && !line.starts_with('#') {
                dependencies.push(line.split('=').next().unwrap().trim().to_string());
            }
        }
        assert_eq!(dependencies, ["prism-runtime"], "{}", manifest);
    }
}

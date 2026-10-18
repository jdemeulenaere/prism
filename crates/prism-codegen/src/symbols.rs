//! Symbols of generated functions and data, derived from declaration paths alone (D5 of
//! `codegen-core`).
//!
//! | Body | Symbol |
//! |---|---|
//! | function `name` of module `m` | `m::name` |
//! | extension `name` on receiver `R` of module `m` | `m::R.name`, `R` as the ABI writes it |
//! | generator of the default of parameter `p` | `<function symbol>#default(p)` |
//! | initializer and accessor of top-level `val` `name` | `m::name#init`, `m::name#get` |
//! | slot of any of these in the function table | `<symbol>#slot` |
//! | storage of top-level `val` `name` | `m::name#storage`, `m::name#state` |
//!
//! A closure, which is never compiled yet, is named `<parent symbol>#closure<index>`.

use prism_mir::{BodyOwner, Callee, MirFile};
use prism_typed_tree::DeclarationPath;

/// The symbol of the function or extension `path`.
pub fn function_symbol(path: &DeclarationPath) -> String {
    match &path.receiver {
        Some(receiver) => format!(
            "{}::{}.{}",
            path.module,
            receiver.receiver_constructor(),
            path.name
        ),
        None => format!("{}::{}", path.module, path.name),
    }
}

/// The symbol of the generator of the default value of the parameter `parameter` of `function`.
pub fn default_symbol(function: &DeclarationPath, parameter: &str) -> String {
    format!("{}#default({})", function_symbol(function), parameter)
}

/// The symbol of the initializer of the top-level `val` `path`, its MIR body.
pub fn initializer_symbol(path: &DeclarationPath) -> String {
    format!("{}#init", function_symbol(path))
}

/// The symbol of the accessor of the top-level `val` `path`, which initializes it on its first
/// call and returns its address (D18).
pub fn accessor_symbol(path: &DeclarationPath) -> String {
    format!("{}#get", function_symbol(path))
}

/// The symbol of the storage of the top-level `val` `path`.
pub fn storage_symbol(path: &DeclarationPath) -> String {
    format!("{}#storage", function_symbol(path))
}

/// The symbol of the initialization state byte of the top-level `val` `path`.
pub fn state_symbol(path: &DeclarationPath) -> String {
    format!("{}#state", function_symbol(path))
}

/// The symbol of the slot of the function table that holds the address of `symbol` (D4).
pub fn slot_symbol(symbol: &str) -> String {
    format!("{}#slot", symbol)
}

/// The function symbol a slot symbol stands for, if `name` is one.
pub fn slot_owner(name: &str) -> Option<&str> {
    name.strip_suffix("#slot")
}

/// The symbol of the body owned by `owner` in `file`.
pub fn symbol_of(owner: &BodyOwner, file: &MirFile) -> String {
    match owner {
        BodyOwner::Function(path) => function_symbol(path),
        BodyOwner::Value(path) => initializer_symbol(path),
        BodyOwner::Default { function, name, .. } => default_symbol(function, name),
        BodyOwner::Closure { parent, index } => {
            let parent = &file.bodies[*parent].owner;
            format!("{}#closure{}", symbol_of(parent, file), index)
        }
    }
}

/// The symbol a call of `callee` calls through its slot: `None` for a built-in, which is not a
/// generated function, and for a function value.
pub fn callee_symbol(callee: &Callee) -> Option<String> {
    match callee {
        Callee::Function(path) => Some(function_symbol(path)),
        Callee::Default { function, name, .. } => Some(default_symbol(function, name)),
        Callee::Builtin(_) | Callee::Value(..) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{lower_files, lower_source};
    use prism_mir::TerminatorKind;

    fn symbols(file: &MirFile) -> Vec<String> {
        file.bodies
            .iter()
            .map(|(_, body)| symbol_of(&body.owner, file))
            .collect()
    }

    #[test]
    fn symbols_of_a_file() {
        // The scenario "Symbols of a file" of the `code-generation` spec.
        let files = lower_files(&[(
            "lib/math",
            "package lib\n\
             public fun Int.double(): Int = this * 2\n\
             public fun scale(a: Int, b: Int = 2): Int = a * b\n\
             public val BASE: Int = 3\n",
        )]);
        assert_eq!(
            symbols(&files[0]),
            [
                "lib/math::Int.double",
                "lib/math::scale",
                "lib/math::scale#default(b)",
                "lib/math::BASE#init",
            ]
        );
        let base = DeclarationPath::new("lib/math", "BASE");
        assert_eq!(accessor_symbol(&base), "lib/math::BASE#get");
        assert_eq!(storage_symbol(&base), "lib/math::BASE#storage");
        assert_eq!(slot_symbol("lib/math::scale"), "lib/math::scale#slot");
        assert_eq!(slot_owner("lib/math::scale#slot"), Some("lib/math::scale"));
    }

    #[test]
    fn an_extension_on_a_struct_of_another_module() {
        let files = lower_files(&[
            (
                "lib/team",
                "package lib\npublic struct Team { val size: Int }\n",
            ),
            (
                "app/main",
                "package app\nimport /lib/team\n\
                 fun Team.grow(): Int = 1\n\
                 fun f(t: Team): Int = t.grow()\n",
            ),
        ]);
        let main = &files[1];
        assert_eq!(symbols(main)[0], "app/main::lib/team.Team.grow");
        let call = main
            .bodies
            .iter()
            .flat_map(|(_, body)| body.blocks.iter())
            .find_map(|(_, block)| match &block.terminator.kind {
                TerminatorKind::Call { callee, .. } => callee_symbol(callee),
                _ => None,
            });
        assert_eq!(call.as_deref(), Some("app/main::lib/team.Team.grow"));
    }

    #[test]
    fn closures_are_named_after_their_parent() {
        let file = lower_source("fun f(): Int { val g = { 1 }; return g() }\n");
        assert_eq!(symbols(&file), ["main::f", "main::f#closure0"]);
    }
}

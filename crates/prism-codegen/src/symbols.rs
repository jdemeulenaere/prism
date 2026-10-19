//! Symbols of generated functions and data, derived from declaration paths alone (D5 of
//! `codegen-core`).
//!
//! | Body | Symbol |
//! |---|---|
//! | function `name` of module `m` | `m::name` |
//! | extension `name` on receiver `R` of module `m` | `m::R.name`, `R` as the ABI writes it |
//! | extension `name` on `R` with context `C` | `m::R.name#context(C)`, `C` written as `R` is |
//! | generator of the default of parameter `p` | `<function symbol>#default(p)` |
//! | initializer and accessor of top-level `val` `name` | `m::name#init`, `m::name#get` |
//! | instance of a generic function or extension for type arguments `A, B` | `<symbol><A, B>` |
//! | instance of a default generator | `<instance symbol>#default(p)` |
//! | closure `i` of a body | `<parent symbol>#closure<i>` |
//! | descriptor of a closure | `<closure symbol>#descriptor` |
//! | slot of any of these functions in the function table | `<symbol>#slot` |
//! | storage of top-level `val` `name` | `m::name#storage`, `m::name#state` |
//!
//! A closure is named after the body it is lowered in and its index there, so a closure of an
//! instance after the instance (`main::apply<Int>#closure0`, D4 and D11 of `closures`). An
//! instance's type arguments are written as glue names types: every struct and enum qualified by
//! its module (`lib/box::keep<lib/user::User>`, D4 of `monomorphization`).

use crate::glue::qualified_name;
use prism_mir::{Body, BodyOwner, Callee, MirFile};
use prism_typed_tree::ArenaId;
use prism_typed_tree::{DeclarationPath, Type};

/// The symbol of the function or extension `path`.
pub fn function_symbol(path: &DeclarationPath) -> String {
    match &path.receiver {
        Some(receiver) => format!(
            "{}::{}.{}{}",
            path.module,
            receiver.receiver_constructor(),
            path.name,
            path.context_suffix(Type::receiver_constructor)
        ),
        None => format!("{}::{}", path.module, path.name),
    }
}

/// The symbol of the instance of the generic function or extension `path` for
/// `type_arguments`, or the function's own symbol when there are none.
pub fn instance_symbol(path: &DeclarationPath, type_arguments: &[Type]) -> String {
    let symbol = function_symbol(path);
    if type_arguments.is_empty() {
        return symbol;
    }
    let arguments: Vec<String> = type_arguments.iter().map(qualified_name).collect();
    format!("{}<{}>", symbol, arguments.join(", "))
}

/// The symbol of the generator of the default value of the parameter `parameter` of `function`.
pub fn default_symbol(function: &DeclarationPath, parameter: &str) -> String {
    default_instance_symbol(function, &[], parameter)
}

/// The symbol of the instance for `type_arguments` of the generator of the default value of the
/// parameter `parameter` of `function`.
pub fn default_instance_symbol(
    function: &DeclarationPath,
    type_arguments: &[Type],
    parameter: &str,
) -> String {
    format!(
        "{}#default({})",
        instance_symbol(function, type_arguments),
        parameter
    )
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

/// The drop function of a closure's allocated environment, which its descriptor names (D8 of
/// `escaping-closures`): `<closure symbol>#drop-environment`.
pub fn environment_drop_symbol(closure_symbol: &str) -> String {
    format!("{}#drop-environment", closure_symbol)
}

/// The symbol of the descriptor of the closure whose code is `closure_symbol` (D3 of
/// `closures`): static data holding the address of the closure's slot and its environment's
/// drop entry.
pub fn descriptor_symbol(closure_symbol: &str) -> String {
    format!("{}#descriptor", closure_symbol)
}

/// The function symbol a slot symbol stands for, if `name` is one.
pub fn slot_owner(name: &str) -> Option<&str> {
    name.strip_suffix("#slot")
}

/// The symbol of `body` of `file`, an instance's with its type arguments.
pub fn symbol_of(body: &Body, file: &MirFile) -> String {
    match &body.owner {
        BodyOwner::Function(path) => instance_symbol(path, &body.type_arguments),
        BodyOwner::Value(path) => initializer_symbol(path),
        BodyOwner::Default { function, name, .. } => {
            default_instance_symbol(function, &body.type_arguments, name)
        }
        BodyOwner::Closure { parent, index } => {
            format!(
                "{}#closure{}",
                symbol_of(&file.bodies[*parent], file),
                index
            )
        }
        // Named by what they belong to, so inserting a closure never shifts them (D19 of
        // `compose-transform`).
        BodyOwner::Restart { function } => {
            format!("{}#restart", symbol_of(&file.bodies[*function], file))
        }
        BodyOwner::RootContent { parent } => {
            format!("{}#root-content", symbol_of(&file.bodies[*parent], file))
        }
    }
}

/// The symbol of every body of `file`, indexed by its id. Each symbol is built once, a
/// closure's by extending its parent's, so naming every body of closures nested `d` deep costs
/// the length of their symbols, where calling [`symbol_of`] for each would rebuild every
/// parent's symbol again for each of its closures.
pub fn symbols_of(file: &MirFile) -> Vec<String> {
    let mut symbols: Vec<Option<String>> = vec![None; file.bodies.len()];
    for (id, _) in file.bodies.iter() {
        // The bodies from `id` up to the first whose symbol is known or that is no closure,
        // named outermost first.
        let mut chain = vec![id];
        while let Some(parent) =
            prism_mir::record::parent_of(&file.bodies[*chain.last().unwrap()].owner)
        {
            if symbols[parent.index()].is_some() {
                break;
            }
            chain.push(parent);
        }
        for body_id in chain.into_iter().rev() {
            if symbols[body_id.index()].is_some() {
                continue;
            }
            let body = &file.bodies[body_id];
            let parent_symbol = |parent: &prism_mir::BodyId| {
                symbols[parent.index()]
                    .as_deref()
                    .expect("a parent is named before its closures")
                    .to_string()
            };
            let symbol = match &body.owner {
                BodyOwner::Closure { parent, index } => {
                    format!("{}#closure{}", parent_symbol(parent), index)
                }
                BodyOwner::Restart { function } => format!("{}#restart", parent_symbol(function)),
                BodyOwner::RootContent { parent } => {
                    format!("{}#root-content", parent_symbol(parent))
                }
                _ => symbol_of(body, file),
            };
            symbols[body_id.index()] = Some(symbol);
        }
    }
    symbols
        .into_iter()
        .map(|symbol| symbol.expect("every body is named"))
        .collect()
}

/// The symbol a call of `callee` with `type_arguments` calls through its slot, an instance's for
/// a generic callee: `None` for a built-in, which is not a generated function, for an external,
/// which is called by its C symbol and has no slot (D5 of `standard-library`), and for a
/// function value.
pub fn callee_symbol(callee: &Callee, type_arguments: &[Type]) -> Option<String> {
    match callee {
        Callee::Function(path) => Some(instance_symbol(path, type_arguments)),
        Callee::Default { function, name, .. } => {
            Some(default_instance_symbol(function, type_arguments, name))
        }
        Callee::Builtin(_)
        | Callee::External(_)
        | Callee::Value(..)
        | Callee::Composer(_)
        | Callee::State(_) => None,
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
            .map(|(_, body)| symbol_of(body, file))
            .collect()
    }

    /// D19 of `compose-transform`: a restart function and a root content adapter are named
    /// after the body they belong to, an instance's after the instance.
    #[test]
    fn restart_and_root_content_symbols() {
        let mut file = lower_source("fun <T> show(x: T) {}\nfun main() {}\n");
        let show = prism_mir::BodyId(0);
        let main = prism_mir::BodyId(1);
        let mut restart = file.bodies[show].clone();
        restart.owner = BodyOwner::Restart { function: show };
        file.bodies.alloc(restart);
        let mut adapter = file.bodies[main].clone();
        adapter.owner = BodyOwner::RootContent { parent: main };
        file.bodies.alloc(adapter);
        let mut instance = file.bodies[show].clone();
        instance.type_parameters.clear();
        instance.type_arguments = vec![Type::Int];
        let instance = file.bodies.alloc(instance);
        let mut instance_restart = file.bodies[show].clone();
        instance_restart.owner = BodyOwner::Restart { function: instance };
        file.bodies.alloc(instance_restart);
        let expected = [
            "main::show",
            "main::main",
            "main::show#restart",
            "main::main#root-content",
            "main::show<Int>",
            "main::show<Int>#restart",
        ];
        assert_eq!(symbols(&file), expected);
        assert_eq!(symbols_of(&file), expected);
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
                TerminatorKind::Call {
                    callee,
                    type_arguments,
                    ..
                } => callee_symbol(callee, type_arguments),
                _ => None,
            });
        assert_eq!(call.as_deref(), Some("app/main::lib/team.Team.grow"));
    }

    #[test]
    fn symbols_of_instances() {
        // The names of the scenario "Symbols of instances" of the `code-generation` spec.
        let user = Type::plain_struct(prism_typed_tree::NominalId::new("lib/user", "User"));
        let keep = DeclarationPath::new("lib/box", "keep");
        assert_eq!(
            instance_symbol(&keep, std::slice::from_ref(&user)),
            "lib/box::keep<lib/user::User>"
        );
        let boxed = Type::Struct {
            id: prism_typed_tree::NominalId::new("lib/box", "Box"),
            arguments: vec![Type::Int],
        };
        let get = DeclarationPath::extension("lib/box", boxed, "get");
        assert_eq!(
            instance_symbol(&get, &[Type::Int]),
            "lib/box::lib/box.Box.get<Int>"
        );
        let pad = DeclarationPath::new("lib/box", "pad");
        assert_eq!(
            default_instance_symbol(&pad, &[Type::Int], "y"),
            "lib/box::pad<Int>#default(y)"
        );
        assert_eq!(
            instance_symbol(
                &DeclarationPath::new("main", "pair"),
                &[Type::Int.nullable(), Type::Heap(Box::new(user))]
            ),
            "main::pair<Int?, Heap<lib/user::User>>"
        );
        assert_eq!(instance_symbol(&keep, &[]), "lib/box::keep");
    }

    #[test]
    fn closures_are_named_after_their_parent() {
        let file = lower_source("fun f(): Int { val g = { 1 }; return g() }\n");
        assert_eq!(symbols(&file), ["main::f", "main::f#closure0"]);
        assert_eq!(
            descriptor_symbol("main::f#closure0"),
            "main::f#closure0#descriptor"
        );
    }

    #[test]
    fn symbols_of_nested_and_default_closures() {
        // The names of the scenario "Symbols of nested and default closures" of the
        // `code-generation` spec.
        let file = lower_source(
            "fun run(f: () -> Int): Int = f()\n\
             fun pick(n: Int, f: () -> Int = { 3 }): Int = n + f()\n\
             fun main() { println(run { run { 1 } }); println(pick(1)) }\n",
        );
        assert_eq!(
            symbols(&file),
            [
                "main::run",
                "main::pick",
                "main::pick#default(f)",
                "main::pick#default(f)#closure0",
                "main::main",
                "main::main#closure0",
                "main::main#closure0#closure0",
            ]
        );
    }
}

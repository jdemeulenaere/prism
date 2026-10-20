//! The intrinsic catalog (D6 of `standard-library`): every function and property the compiler
//! provides to the bundled library, by declaration path, with the built-in its calls lower to
//! and the canonical text of the declaration the library must give it.
//!
//! A library file's `intrinsic` declaration is checked against its entry: an unknown path or a
//! different signature is `E4029` (`InvalidLibraryDeclaration`). So the declaration, the code
//! that types its calls and the code that lowers them cannot drift apart. Calls of a
//! signature-typed intrinsic are checked as calls of its declaration; a rule-typed one is typed
//! by its own checker, selected by its built-in, because its behavior needs a feature Prism
//! lacks (overloading, arguments of different types, traits).

use prism_ast::decl::{ExtensionPropertyDecl, FunctionDecl};
use prism_ast::types::{Type, TypeKind};
use prism_typed_tree::BuiltinFunction;

/// One intrinsic the compiler provides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Intrinsic {
    /// The library module declaring it (`prism/core`).
    pub module: &'static str,
    /// The receiver type constructor of an intrinsic extension (`Int`, `List`), `None` for a
    /// plain function.
    pub receiver: Option<&'static str>,
    pub name: &'static str,
    pub builtin: BuiltinFunction,
    /// The declaration as [`declaration_text`] renders it: visibility, modifiers, signature,
    /// and no default value.
    pub signature: &'static str,
    /// Whether calls are typed by a rule rather than by the declared signature.
    pub rule_typed: bool,
}

const CORE: &str = "prism/core";

/// The member module (D3 of `core-library-in-prism`): every extension it declares on a built-in
/// type, intrinsic or written in Prism, is a member of that type.
pub(crate) const MEMBER_MODULE: &str = CORE;
const COMPOSE_RUNTIME: &str = prism_typed_tree::COMPOSE_RUNTIME_MODULE;

const fn entry(
    module: &'static str,
    receiver: Option<&'static str>,
    name: &'static str,
    builtin: BuiltinFunction,
    signature: &'static str,
    rule_typed: bool,
) -> Intrinsic {
    Intrinsic {
        module,
        receiver,
        name,
        builtin,
        signature,
        rule_typed,
    }
}

/// Every intrinsic, in the order of the library's declarations.
pub const CATALOG: &[Intrinsic] = &[
    // `/prism/core`: `print` and `println` take any argument with a text form (D9), and
    // `println` also none, until traits and overloading exist.
    entry(
        CORE,
        None,
        "print",
        BuiltinFunction::Print,
        "public intrinsic fun <T> print(value: &T): Unit",
        true,
    ),
    entry(
        CORE,
        None,
        "println",
        BuiltinFunction::PrintLine,
        "public intrinsic fun <T> println(value: &T): Unit",
        true,
    ),
    entry(
        CORE,
        Some("String"),
        "length",
        BuiltinFunction::StringLength,
        "public intrinsic val String.length: Int",
        false,
    ),
    entry(
        CORE,
        Some("List"),
        "size",
        BuiltinFunction::ListSize,
        "public intrinsic val <T> List<T>.size: Int",
        false,
    ),
    // `contains` and `indexOf` also need `==` on the element type at each call, until traits.
    entry(
        CORE,
        Some("List"),
        "contains",
        BuiltinFunction::ListContains,
        "public intrinsic operator fun <T> List<T>.contains(element: &T): Bool",
        true,
    ),
    entry(
        CORE,
        Some("List"),
        "indexOf",
        BuiltinFunction::ListIndexOf,
        "public intrinsic fun <T> List<T>.indexOf(element: &T): Int",
        true,
    ),
    entry(
        CORE,
        Some("List"),
        "add",
        BuiltinFunction::ListAdd,
        "public intrinsic mut fun <T> List<T>.add(element: T): Unit",
        false,
    ),
    entry(
        CORE,
        Some("List"),
        "insert",
        BuiltinFunction::ListInsert,
        "public intrinsic mut fun <T> List<T>.insert(index: Int, element: T): Unit",
        false,
    ),
    entry(
        CORE,
        Some("List"),
        "removeAt",
        BuiltinFunction::ListRemoveAt,
        "public intrinsic mut fun <T> List<T>.removeAt(index: Int): T",
        false,
    ),
    entry(
        CORE,
        Some("List"),
        "removeLast",
        BuiltinFunction::ListRemoveLast,
        "public intrinsic mut fun <T> List<T>.removeLast(): T",
        false,
    ),
    entry(
        CORE,
        Some("List"),
        "reverse",
        BuiltinFunction::ListReverse,
        "public intrinsic mut fun <T> List<T>.reverse(): Unit",
        false,
    ),
    entry(
        CORE,
        Some("List"),
        "clear",
        BuiltinFunction::ListClear,
        "public intrinsic mut fun <T> List<T>.clear(): Unit",
        false,
    ),
    // `/prism/compose/runtime` (D15 of `compose-transform`, D11 of `compose-library-in-prism`):
    // the compose transform lowers every call to the compose runtime's protocol. `remember`
    // also takes up to three keys before its calculation.
    entry(
        COMPOSE_RUNTIME,
        None,
        "remember",
        BuiltinFunction::Remember,
        "public intrinsic compose fun <T> remember(calculation: () -> T): T",
        true,
    ),
    entry(
        COMPOSE_RUNTIME,
        None,
        "key",
        BuiltinFunction::Key,
        "public intrinsic compose fun key(value: Int, content: compose () -> Unit): Unit",
        false,
    ),
    // State objects (D2 of `state-handles`, D1 of `owned-state`). `value` is typed by its own
    // rule: a copy for a `Copy` value type, a place otherwise, writable on `MutableState` only
    // (D5 of `state-handles`). `weak` is the capture function of both state types (D12 of
    // `owned-state`).
    entry(
        COMPOSE_RUNTIME,
        None,
        "mutableStateOf",
        BuiltinFunction::MutableStateOf,
        "public intrinsic fun <T> mutableStateOf(value: T): MutableState<T>",
        false,
    ),
    entry(
        COMPOSE_RUNTIME,
        Some("State"),
        "value",
        BuiltinFunction::StateValue,
        "public intrinsic val <T> State<T>.value: T",
        true,
    ),
    entry(
        COMPOSE_RUNTIME,
        Some("MutableState"),
        "value",
        BuiltinFunction::StateValue,
        "public intrinsic var <T> MutableState<T>.value: T",
        true,
    ),
    entry(
        COMPOSE_RUNTIME,
        Some("MutableState"),
        "update",
        BuiltinFunction::StateUpdate,
        "public intrinsic fun <T> MutableState<T>.update(transform: (T) -> T): Unit",
        false,
    ),
    entry(
        COMPOSE_RUNTIME,
        Some("State"),
        "weak",
        BuiltinFunction::StateWeak,
        "public intrinsic capture fun <T> State<T>.weak(): WeakReference<State<T>>",
        false,
    ),
    entry(
        COMPOSE_RUNTIME,
        Some("MutableState"),
        "weak",
        BuiltinFunction::StateWeak,
        "public intrinsic capture fun <T> MutableState<T>.weak(): WeakReference<MutableState<T>>",
        false,
    ),
    entry(
        COMPOSE_RUNTIME,
        Some("WeakReference"),
        "get",
        BuiltinFunction::WeakGet,
        "public intrinsic fun <T> WeakReference<T>.get(): (&T)?",
        false,
    ),
    // The composer of the running body, Compose's `currentComposer`: the one composer-access
    // primitive (D3 of `compose-library-in-prism`).
    entry(
        COMPOSE_RUNTIME,
        None,
        "currentComposer",
        BuiltinFunction::CurrentComposer,
        "public intrinsic compose fun currentComposer(): Int",
        false,
    ),
];

/// The intrinsic of `module` named `name`, on the receiver type constructor `receiver` for an
/// extension. A linear search over a few dozen entries, done once per intrinsic declaration
/// and per ingested intrinsic symbol.
pub fn lookup(module: &str, receiver: Option<&str>, name: &str) -> Option<&'static Intrinsic> {
    CATALOG
        .iter()
        .find(|entry| entry.module == module && entry.receiver == receiver && entry.name == name)
}

/// The type constructor of a written receiver type: its name, without type arguments, with
/// `Float` spelled `Double`, the name the catalog uses.
pub fn constructor_name(ty: &Type) -> Option<&str> {
    match &ty.kind {
        TypeKind::Named { name, .. } if name == "Float" => Some("Double"),
        TypeKind::Named { name, .. } => Some(name),
        _ => None,
    }
}

/// The type parameter list of a declaration, `<T, R : Copy> ` with a trailing space, or
/// nothing.
fn type_parameter_text(parameters: &[prism_ast::decl::TypeParameter]) -> String {
    if parameters.is_empty() {
        return String::new();
    }
    let parameters: Vec<String> = parameters
        .iter()
        .map(|p| match p.bound {
            Some(bound) => format!("{} : {}", p.name, bound.as_str()),
            None => p.name.clone(),
        })
        .collect();
    format!("<{}> ", parameters.join(", "))
}

/// The canonical text of an intrinsic or external function declaration, as `prism abi` shows
/// it and the catalog records it: visibility, modifiers, type parameters, receiver, name,
/// parameters without defaults, and the result (`Unit` when none is written).
pub fn declaration_text(f: &FunctionDecl) -> String {
    let mut text = format!("{} ", f.visibility);
    if let Some(library) = f.library {
        text.push_str(library.kind.as_str());
        text.push(' ');
    }
    if f.is_operator {
        text.push_str("operator ");
    }
    if f.is_capture {
        text.push_str("capture ");
    }
    if f.is_compose {
        text.push_str("compose ");
    }
    if let Some(receiver) = &f.receiver {
        if !receiver.mode.as_str().is_empty() {
            text.push_str(receiver.mode.as_str());
            text.push(' ');
        }
    }
    text.push_str("fun ");
    text.push_str(&type_parameter_text(&f.type_parameters));
    if let Some(receiver) = &f.receiver {
        text.push_str(&format!("{}.", receiver.ty));
    }
    let parameters: Vec<String> = f
        .params
        .iter()
        .map(|p| {
            // A vararg is compared with the catalog like the rest of the signature (D9 of
            // `vararg`).
            let modifier = if p.is_vararg { "vararg " } else { "" };
            format!("{}{}: {}{}", modifier, p.name, p.kind, p.ty)
        })
        .collect();
    let result = f
        .return_type
        .as_ref()
        .map_or_else(|| "Unit".to_string(), ToString::to_string);
    text.push_str(&format!(
        "{}({}): {}",
        f.name,
        parameters.join(", "),
        result
    ));
    text
}

/// The canonical text of an intrinsic extension property declaration.
pub fn property_text(p: &ExtensionPropertyDecl) -> String {
    format!(
        "{} intrinsic {} {}{}.{}: {}",
        p.visibility,
        p.keyword(),
        type_parameter_text(&p.type_parameters),
        p.receiver,
        p.name,
        p.ty
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_entry_is_found_by_its_path_and_paths_are_distinct() {
        for entry in CATALOG {
            assert_eq!(
                lookup(entry.module, entry.receiver, entry.name),
                Some(entry),
                "{}",
                entry.signature
            );
        }
    }

    #[test]
    fn list_member_entries_match_the_built_in_signatures() {
        for builtin in BuiltinFunction::LIST_MEMBERS {
            let entry = lookup(CORE, Some("List"), builtin.name()).expect("an entry");
            assert_eq!(entry.builtin, builtin);
            let signature = builtin.list_member_signature().expect("a list member");
            let mode = match signature.mode.as_str() {
                "" => String::new(),
                mode => format!("{} ", mode),
            };
            let parameters: Vec<String> = signature
                .parameters
                .iter()
                .map(|(name, ty)| format!("{}: {}", name, ty))
                .collect();
            let expected = match builtin {
                BuiltinFunction::ListSize => format!(
                    "public intrinsic val <T> List<T>.{}: {}",
                    builtin.name(),
                    signature.result
                ),
                // `contains` serves `in` (D8 of `operator-conventions`).
                BuiltinFunction::ListContains => format!(
                    "public intrinsic operator {}fun <T> List<T>.{}({}): {}",
                    mode,
                    builtin.name(),
                    parameters.join(", "),
                    signature.result
                ),
                _ => format!(
                    "public intrinsic {}fun <T> List<T>.{}({}): {}",
                    mode,
                    builtin.name(),
                    parameters.join(", "),
                    signature.result
                ),
            };
            assert_eq!(entry.signature, expected);
        }
    }
}

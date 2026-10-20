//! The library index behind the import help of unresolved names (D8 of `explicit-library`):
//! which library modules declare each public name, built from the library's own sources on the
//! first diagnostic that needs it, so a program without such a diagnostic never builds it.

use prism_ast::{Declaration, Type as AstType, TypeKind, Visibility};
use prism_library::Library;
use prism_typechecker::{Type, TypeError};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

/// The receiver of a library extension, as the index matches it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReceiverName {
    /// A type constructor named this (`List`, `Int`, `RecomposeScope`).
    Named(String),
    /// A type parameter of the extension: any receiver.
    Any,
}

/// The public declarations of every library module, by name.
#[derive(Debug, Default)]
pub(crate) struct LibraryIndex {
    /// Functions without a receiver, top-level values, structs and enums: the module paths
    /// (`prism/core`) declaring each name.
    names: HashMap<String, Vec<String>>,
    /// Extension functions and properties: each declaring module with its receiver.
    extensions: HashMap<String, Vec<(ReceiverName, String)>>,
}

impl LibraryIndex {
    /// The index of `library`: built once per process for the bundled library, afresh for a
    /// library with injected test modules.
    pub(crate) fn of(library: &Library) -> Arc<LibraryIndex> {
        static BUNDLED: OnceLock<Arc<LibraryIndex>> = OnceLock::new();
        if library.is_bundled() {
            Arc::clone(BUNDLED.get_or_init(|| Arc::new(Self::build(Library::bundled()))))
        } else {
            Arc::new(Self::build(library))
        }
    }

    /// Parses every module of `library` and walks its public top-level declarations. A module
    /// that does not parse adds nothing (its own error is reported where it is imported).
    fn build(library: &Library) -> LibraryIndex {
        let mut index = LibraryIndex::default();
        for module in library.modules() {
            let Ok(file) = prism_parser::parse_source(&module.source) else {
                continue;
            };
            for declaration in &file.declarations {
                if declaration.visibility() != Visibility::Public {
                    continue;
                }
                match declaration {
                    Declaration::Function(function) => match &function.receiver {
                        Some(receiver) => {
                            let parameters: Vec<&str> = function
                                .type_parameters
                                .iter()
                                .map(|parameter| parameter.name.as_str())
                                .collect();
                            index.add_extension(
                                &function.name,
                                receiver_name(&receiver.ty, &parameters),
                                &module.path,
                            )
                        }
                        None => index.add_name(&function.name, &module.path),
                    },
                    Declaration::ExtensionProperty(property) => {
                        let parameters: Vec<&str> = property
                            .type_parameters
                            .iter()
                            .map(|parameter| parameter.name.as_str())
                            .collect();
                        index.add_extension(
                            &property.name,
                            receiver_name(&property.receiver, &parameters),
                            &module.path,
                        )
                    }
                    Declaration::Struct(structure) => index.add_name(&structure.name, &module.path),
                    Declaration::Enum(enumeration) => {
                        index.add_name(&enumeration.name, &module.path)
                    }
                    Declaration::Var(value) => index.add_name(&value.name, &module.path),
                }
            }
        }
        index
    }

    fn add_name(&mut self, name: &str, module: &str) {
        let modules = self.names.entry(name.to_string()).or_default();
        if !modules.iter().any(|known| known == module) {
            modules.push(module.to_string());
        }
    }

    fn add_extension(&mut self, name: &str, receiver: ReceiverName, module: &str) {
        let entries = self.extensions.entry(name.to_string()).or_default();
        let entry = (receiver, module.to_string());
        if !entries.contains(&entry) {
            entries.push(entry);
        }
    }

    /// The modules declaring a function, value, struct or enum named `name`.
    pub(crate) fn modules_declaring(&self, name: &str) -> Vec<String> {
        self.names.get(name).cloned().unwrap_or_default()
    }

    /// The modules declaring an extension `name` that a receiver whose type constructor is
    /// named `receiver` (`None` for a receiver without one, such as a function type) can call.
    pub(crate) fn modules_extending(&self, receiver: Option<&str>, name: &str) -> Vec<String> {
        let mut modules: Vec<String> = Vec::new();
        for (entry, module) in self.extensions.get(name).into_iter().flatten() {
            let matches = match entry {
                ReceiverName::Any => true,
                ReceiverName::Named(named) => Some(named.as_str()) == receiver,
            };
            if matches && !modules.contains(module) {
                modules.push(module.clone());
            }
        }
        modules
    }
}

/// The name an extension's receiver type matches by: its type constructor after references
/// and `Heap`, or any receiver for a type parameter of the extension.
fn receiver_name(ty: &AstType, type_parameters: &[&str]) -> ReceiverName {
    match &ty.kind {
        TypeKind::Reference { inner, .. } => receiver_name(inner, type_parameters),
        TypeKind::Named { name, type_args } if name == "Heap" && type_args.len() == 1 => {
            receiver_name(&type_args[0], type_parameters)
        }
        TypeKind::Named { name, .. } if type_parameters.contains(&name.as_str()) => {
            ReceiverName::Any
        }
        TypeKind::Named { name, .. } => ReceiverName::Named(name.clone()),
        TypeKind::Nullable(_) => ReceiverName::Named("?".to_string()),
        TypeKind::Unit => ReceiverName::Named("Unit".to_string()),
        TypeKind::Function { .. } => ReceiverName::Named("->".to_string()),
    }
}

/// The name of the type constructor of `ty` after references and `Heap`, as a library
/// extension's receiver names it; an opaque type by its written name.
fn type_constructor_name(ty: &Type) -> Option<String> {
    match ty {
        Type::Reference { inner, .. } | Type::Heap(inner) => type_constructor_name(inner),
        Type::Int => Some("Int".to_string()),
        Type::Float => Some("Double".to_string()),
        Type::Bool => Some("Bool".to_string()),
        Type::Char => Some("Char".to_string()),
        Type::String => Some("String".to_string()),
        Type::Unit => Some("Unit".to_string()),
        Type::Nothing => Some("Nothing".to_string()),
        Type::List(_) => Some("List".to_string()),
        Type::Struct { id, .. } | Type::Enum { id, .. } => Some(id.name.clone()),
        Type::Opaque(name) => Some(name.split('<').next().unwrap_or(name).to_string()),
        _ => None,
    }
}

/// Fills the import suggestions of the unresolved names (`E4001`) and members (`E4006`) of
/// `errors` that a library module declares publicly, building the index of `library` only when
/// one of them is present. A module the file already imports never declares such a name, or it
/// would have resolved.
pub(crate) fn suggest_library_imports(library: &Library, errors: &mut [TypeError]) {
    let needs_index = errors.iter().any(|error| {
        matches!(
            error,
            TypeError::UnresolvedSymbol { .. } | TypeError::UnknownField { .. }
        )
    });
    if !needs_index {
        return;
    }
    let index = LibraryIndex::of(library);
    for error in errors.iter_mut() {
        match error {
            TypeError::UnresolvedSymbol {
                name,
                import_suggestions,
                ..
            } => {
                *import_suggestions = index.modules_declaring(name);
            }
            TypeError::UnknownField {
                target_type,
                field_name,
                import_suggestions,
                ..
            } => {
                let receiver = type_constructor_name(target_type);
                *import_suggestions = index.modules_extending(receiver.as_deref(), field_name);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_index_holds_the_public_declarations_of_each_module() {
        let index = LibraryIndex::of(Library::bundled());
        assert_eq!(index.modules_declaring("println"), vec!["prism/core"]);
        assert_eq!(
            index.modules_extending(Some("List"), "sumOf"),
            vec!["prism/collections"]
        );
        assert!(index.modules_extending(Some("Int"), "sumOf").is_empty());
        assert_eq!(
            index.modules_extending(Some("Int"), "dp"),
            vec!["prism/compose/ui"]
        );
        assert_eq!(
            index.modules_declaring("Column"),
            vec!["prism/compose/layout"]
        );
        assert!(index.modules_declaring("printn").is_empty());
        // Private and `package` declarations are not suggested.
        assert!(index.modules_declaring("NodeId").is_empty());
        assert!(index.modules_declaring("LeafNode").is_empty());
    }

    #[test]
    fn suggestions_fill_unresolved_names_and_members_only() {
        let span = prism_ast::Span::dummy();
        let mut errors = vec![
            TypeError::UnresolvedSymbol {
                name: "println".to_string(),
                span,
                import_suggestions: Vec::new(),
            },
            TypeError::UnresolvedSymbol {
                name: "printn".to_string(),
                span,
                import_suggestions: Vec::new(),
            },
            TypeError::UnknownField {
                target_type: Type::Reference {
                    mutable: false,
                    inner: Box::new(Type::List(Box::new(Type::Int))),
                },
                field_name: "sumOf".to_string(),
                span,
                help: None,
                import_suggestions: Vec::new(),
            },
            TypeError::UnknownField {
                target_type: Type::Int,
                field_name: "sumOf".to_string(),
                span,
                help: None,
                import_suggestions: Vec::new(),
            },
            TypeError::UnknownField {
                target_type: Type::Opaque("RecomposeScope".to_string()),
                field_name: "invalidate".to_string(),
                span,
                help: None,
                import_suggestions: Vec::new(),
            },
        ];
        suggest_library_imports(Library::bundled(), &mut errors);
        let suggestions: Vec<Vec<String>> = errors
            .iter()
            .map(|error| match error {
                TypeError::UnresolvedSymbol {
                    import_suggestions, ..
                }
                | TypeError::UnknownField {
                    import_suggestions, ..
                } => import_suggestions.clone(),
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(
            suggestions,
            vec![
                vec!["prism/core".to_string()],
                vec![],
                vec!["prism/collections".to_string()],
                vec![],
                vec!["prism/compose/runtime".to_string()],
            ]
        );
    }
}

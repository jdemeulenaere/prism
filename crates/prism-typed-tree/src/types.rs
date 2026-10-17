//! The semantic type representation shared by the type checker and the typed tree.
//!
//! The rules over these types (lowering from annotations and ABIs, assignability, joins) live in
//! the type checker; this module holds only the representation and its rendering.

use prism_ast::TypeMarker;
use std::collections::BTreeMap;
use std::fmt;

/// Module path of the compiler's built-in nominal types (such as `Range`).
pub const BUILTIN_MODULE: &str = "<builtin>";

/// Canonical identity of a struct or enum: its defining module (the root-relative file path
/// without extension, such as `a/lib`) and its name. Two nominal types are equal exactly when
/// their identities are equal.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NominalId {
    pub module: String,
    pub name: String,
}

impl NominalId {
    pub fn new(module: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            module: module.into(),
            name: name.into(),
        }
    }

    /// The built-in `Range` type produced by `a..b`.
    pub fn builtin_range() -> Self {
        Self::new(BUILTIN_MODULE, "Range")
    }
}

/// The definition behind a [`NominalId`], kept in a [`TypeTable`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeDef {
    /// Struct fields as `(name, is_var, type)`.
    Struct { fields: Vec<(String, bool, Type)> },
    /// Enum variants as `(name, payload types)`.
    Enum { variants: Vec<(String, Vec<Type>)> },
}

/// Every struct and enum known to one file (D4 of `mir-lowering`): its own declarations, those its
/// direct imports export, the types an imported signature mentions, and the built-in `Range`.
///
/// A type may have a marker without a definition: a type that reached the file through an
/// imported signature, from a module the file does not import, has a known marker but unknown
/// fields. The first definition and the first marker recorded for an id are kept.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TypeTable {
    definitions: BTreeMap<NominalId, TypeDef>,
    markers: BTreeMap<NominalId, Option<TypeMarker>>,
}

impl TypeTable {
    /// An empty table.
    pub fn new() -> Self {
        Self::default()
    }

    /// The table of a file that knows only the built-in types: the `Copy` struct `Range` with
    /// fields `start` and `end`.
    pub fn with_builtins() -> Self {
        let mut table = Self::new();
        let range = NominalId::builtin_range();
        table.insert_definition(
            range.clone(),
            TypeDef::Struct {
                fields: vec![
                    ("start".to_string(), false, Type::Int),
                    ("end".to_string(), false, Type::Int),
                ],
            },
        );
        table.insert_marker(range, Some(TypeMarker::Copy));
        table
    }

    /// Records the definition of `id`, unless one is already recorded.
    pub fn insert_definition(&mut self, id: NominalId, definition: TypeDef) {
        self.definitions.entry(id).or_insert(definition);
    }

    /// Replaces the definition of `id`.
    pub fn replace_definition(&mut self, id: NominalId, definition: TypeDef) {
        self.definitions.insert(id, definition);
    }

    /// Records the marker of `id`, unless one is already recorded.
    pub fn insert_marker(&mut self, id: NominalId, marker: Option<TypeMarker>) {
        self.markers.entry(id).or_insert(marker);
    }

    /// The definition of `id`, if the file knows it.
    pub fn definition(&self, id: &NominalId) -> Option<&TypeDef> {
        self.definitions.get(id)
    }

    /// The marker of `id`, or `None` when it has none or is unknown.
    pub fn marker(&self, id: &NominalId) -> Option<TypeMarker> {
        self.markers.get(id).copied().flatten()
    }

    /// Whether the table records `id`, with a definition or a marker.
    pub fn knows(&self, id: &NominalId) -> bool {
        self.definitions.contains_key(id) || self.markers.contains_key(id)
    }

    /// Every recorded id, in [`NominalId`] order.
    pub fn ids(&self) -> impl Iterator<Item = &NominalId> {
        let mut ids: Vec<&NominalId> = self.definitions.keys().chain(self.markers.keys()).collect();
        ids.sort();
        ids.dedup();
        ids.into_iter()
    }

    /// The fields of struct `id`, if its definition is known.
    pub fn struct_fields(&self, id: &NominalId) -> Option<&[(String, bool, Type)]> {
        match self.definitions.get(id) {
            Some(TypeDef::Struct { fields }) => Some(fields),
            _ => None,
        }
    }

    /// The variants of enum `id`, if its definition is known.
    pub fn enum_variants(&self, id: &NominalId) -> Option<&[(String, Vec<Type>)]> {
        match self.definitions.get(id) {
            Some(TypeDef::Enum { variants }) => Some(variants),
            _ => None,
        }
    }

    /// Whether values of `ty` are duplicated implicitly by a bitwise copy (the `ownership` spec,
    /// "Built-In Copy Types"). The error type counts as `Copy`, so an earlier error is not
    /// reported again as a move.
    pub fn is_copy(&self, ty: &Type) -> bool {
        match ty {
            Type::Int
            | Type::Float
            | Type::Bool
            | Type::Char
            | Type::Unit
            | Type::Nothing
            | Type::Error => true,
            Type::Reference { mutable, .. } => !mutable,
            Type::Nullable(inner) => self.is_copy(inner),
            Type::Struct(id) | Type::Enum(id) => self.marker(id) == Some(TypeMarker::Copy),
            Type::String
            | Type::Heap(_)
            | Type::Fn { .. }
            | Type::TypeParam(_)
            | Type::Opaque(_) => false,
        }
    }

    /// Whether values of `ty` can be duplicated by an explicit `.clone()` (the `ownership` spec,
    /// "Built-In Clone Types"). Every `Copy` type is `Clone`.
    pub fn is_clone(&self, ty: &Type) -> bool {
        match ty {
            Type::String => true,
            Type::Nullable(inner) | Type::Heap(inner) => self.is_clone(inner),
            Type::Struct(id) | Type::Enum(id) => self.marker(id).is_some(),
            _ => self.is_copy(ty),
        }
    }
}

/// Semantic representation of a Prism type.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Type {
    /// 64-bit signed integer.
    Int,
    /// 64-bit IEEE floating point.
    Float,
    /// Boolean (`true` or `false`).
    Bool,
    /// Unicode scalar value character literal (`'a'`).
    Char,
    /// UTF-8 string.
    String,
    /// Unit type `()`, equivalent to `void`.
    Unit,
    /// Bottom type `Nothing`, representing expressions that diverge or return.
    Nothing,
    /// Stack-scoped borrowed reference (`&T` or `&mut T`).
    Reference { mutable: bool, inner: Box<Type> },
    /// Nullable type `T?`.
    Nullable(Box<Type>),
    /// Single-owner heap boxed indirection `Heap<T>` for recursive types.
    Heap(Box<Type>),
    /// Nominal struct type. Its fields live in the checker's type table.
    Struct(NominalId),
    /// Nominal enum type. Its variants live in the checker's type table.
    Enum(NominalId),
    /// A type name with no struct or enum declaration in scope. Opaque types compare by name,
    /// and have no members.
    Opaque(String),
    /// First-class function type.
    Fn {
        is_compose: bool,
        params: Vec<Type>,
        ret: Box<Type>,
    },
    /// Generic type parameter (e.g. `T`).
    TypeParam(String),
    /// Sentinel type for error recovery and downstream error suppression.
    Error,
}

impl Type {
    /// Makes a type nullable. `Error` stays `Error`, and nullable types never nest (`T??` is
    /// `T?`).
    pub fn nullable(self) -> Type {
        match self {
            Type::Error | Type::Nullable(_) => self,
            other => Type::Nullable(Box::new(other)),
        }
    }

    /// Rewrites every opaque type name inside `self` with `bind`, keeping names it returns
    /// `None` for. Used to bind names lowered from annotations or ABI types to the struct or enum
    /// they denote.
    pub fn bind_names(self, bind: &dyn Fn(&str) -> Option<Type>) -> Type {
        match self {
            Type::Opaque(name) => bind(&name).unwrap_or(Type::Opaque(name)),
            Type::Reference { mutable, inner } => Type::Reference {
                mutable,
                inner: Box::new(inner.bind_names(bind)),
            },
            Type::Nullable(inner) => Type::Nullable(Box::new(inner.bind_names(bind))),
            Type::Heap(inner) => Type::Heap(Box::new(inner.bind_names(bind))),
            Type::Fn {
                is_compose,
                params,
                ret,
            } => Type::Fn {
                is_compose,
                params: params.into_iter().map(|p| p.bind_names(bind)).collect(),
                ret: Box::new(ret.bind_names(bind)),
            },
            other => other,
        }
    }
}

impl Type {
    /// Renders `expected` and `found` for a diagnostic. Nominal types show their bare names,
    /// unless both render the same, in which case each nominal type is qualified with its
    /// defining module (`a/lib::User` versus `b/lib::User`).
    pub fn render_pair(expected: &Type, found: &Type) -> (String, String) {
        let (e, f) = (expected.to_string(), found.to_string());
        if e == f && expected != found {
            (expected.qualified(), found.qualified())
        } else {
            (e, f)
        }
    }

    /// Renders the type with every nominal type qualified by its defining module.
    pub fn qualified(&self) -> String {
        match self {
            Type::Struct(id) | Type::Enum(id) => format!("{}::{}", id.module, id.name),
            Type::Reference { mutable, inner } => {
                format!(
                    "{}{}",
                    if *mutable { "&mut " } else { "&" },
                    inner.qualified()
                )
            }
            Type::Nullable(inner) if needs_parentheses(inner) => {
                format!("({})?", inner.qualified())
            }
            Type::Nullable(inner) => format!("{}?", inner.qualified()),
            Type::Heap(inner) => format!("Heap<{}>", inner.qualified()),
            Type::Fn {
                is_compose,
                params,
                ret,
            } => format!(
                "{}({}) -> {}",
                if *is_compose { "compose " } else { "" },
                params
                    .iter()
                    .map(Type::qualified)
                    .collect::<Vec<_>>()
                    .join(", "),
                ret.qualified()
            ),
            other => other.to_string(),
        }
    }
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Type::Int => write!(f, "Int"),
            Type::Float => write!(f, "Float"),
            Type::Bool => write!(f, "Bool"),
            Type::Char => write!(f, "Char"),
            Type::String => write!(f, "String"),
            Type::Unit => write!(f, "Unit"),
            Type::Nothing => write!(f, "Nothing"),
            Type::Reference { mutable, inner } => {
                if *mutable {
                    write!(f, "&mut {}", inner)
                } else {
                    write!(f, "&{}", inner)
                }
            }
            Type::Nullable(inner) if needs_parentheses(inner) => write!(f, "({})?", inner),
            Type::Nullable(inner) => write!(f, "{}?", inner),
            Type::Heap(inner) => write!(f, "Heap<{}>", inner),
            Type::Struct(id) | Type::Enum(id) => write!(f, "{}", id.name),
            Type::Opaque(name) => write!(f, "{}", name),
            Type::Fn {
                is_compose,
                params,
                ret,
            } => {
                let comp = if *is_compose { "compose " } else { "" };
                write!(f, "{}(", comp)?;
                for (i, p) in params.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", p)?;
                }
                write!(f, ") -> {}", ret)
            }
            Type::TypeParam(name) => write!(f, "{}", name),
            Type::Error => write!(f, "<error>"),
        }
    }
}

/// Whether a nullable of `inner` is written with parentheses, as in source: `(&T)?` is an
/// optional borrow, while `&T?` borrows a nullable, and `(() -> T)?` is an optional function.
fn needs_parentheses(inner: &Type) -> bool {
    matches!(inner, Type::Reference { .. } | Type::Fn { .. })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reference(mutable: bool, inner: Type) -> Type {
        Type::Reference {
            mutable,
            inner: Box::new(inner),
        }
    }

    #[test]
    fn copy_and_clone_of_built_in_types() {
        let table = TypeTable::with_builtins();
        for ty in [Type::Int, Type::Float, Type::Bool, Type::Char, Type::Unit] {
            assert!(table.is_copy(&ty) && table.is_clone(&ty), "{}", ty);
        }
        assert!(table.is_copy(&Type::Struct(NominalId::builtin_range())));
        assert!(!table.is_copy(&Type::String));
        assert!(table.is_clone(&Type::String));
        assert!(table.is_clone(&Type::Heap(Box::new(Type::String))));
        assert!(!table.is_copy(&Type::Heap(Box::new(Type::Int))));
        assert!(table.is_copy(&reference(false, Type::String)));
        assert!(!table.is_copy(&reference(true, Type::Int)));
        assert!(!table.is_clone(&reference(true, Type::Int)));
        assert!(table.is_copy(&Type::Int.nullable()));
        assert!(!table.is_copy(&Type::String.nullable()));
        assert!(table.is_clone(&Type::String.nullable()));
    }

    #[test]
    fn copy_and_clone_follow_markers() {
        let mut table = TypeTable::new();
        let point = NominalId::new("main", "Point");
        let user = NominalId::new("main", "User");
        let file = NominalId::new("main", "File");
        table.insert_marker(point.clone(), Some(TypeMarker::Copy));
        table.insert_marker(user.clone(), Some(TypeMarker::Clone));
        table.insert_marker(file.clone(), None);
        // The first marker is kept.
        table.insert_marker(file.clone(), Some(TypeMarker::Copy));
        assert!(table.is_copy(&Type::Struct(point.clone())));
        assert!(table.is_copy(&Type::Struct(point).nullable()));
        assert!(!table.is_copy(&Type::Struct(user.clone())));
        assert!(table.is_clone(&Type::Enum(user)));
        assert!(!table.is_clone(&Type::Struct(file.clone())));
        assert!(table.knows(&file));
        assert_eq!(table.marker(&file), None);
        assert!(!table.is_copy(&Type::Struct(NominalId::new("main", "Unknown"))));
    }

    #[test]
    fn nullable_references_and_functions_render_as_written() {
        let shared_int = Type::Reference {
            mutable: false,
            inner: Box::new(Type::Int),
        };
        assert_eq!(shared_int.clone().nullable().to_string(), "(&Int)?");
        let borrowed_nullable = Type::Reference {
            mutable: false,
            inner: Box::new(Type::Int.nullable()),
        };
        assert_eq!(borrowed_nullable.to_string(), "&Int?");
        let function = Type::Fn {
            is_compose: false,
            params: vec![],
            ret: Box::new(Type::Int),
        };
        assert_eq!(function.nullable().qualified(), "(() -> Int)?");
    }
}

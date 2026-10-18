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

/// A type parameter of a generic function, struct or enum: its position in its declaration's
/// list, its name and its bound. A type only ever mentions the parameters of one declaration, so
/// the index identifies the parameter; the name is for rendering.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TypeParameter {
    pub index: u32,
    pub name: String,
    pub bound: Option<TypeMarker>,
}

impl TypeParameter {
    /// The parameter as a type, [`Type::Parameter`].
    pub fn as_type(&self) -> Type {
        Type::Parameter(self.clone())
    }

    /// Renders a declaration's type parameter list as written (`<T, U : Copy>`), or nothing for
    /// a declaration without type parameters.
    pub fn render_list(parameters: &[TypeParameter]) -> String {
        if parameters.is_empty() {
            return String::new();
        }
        let rendered: Vec<String> = parameters.iter().map(TypeParameter::to_string).collect();
        format!("<{}>", rendered.join(", "))
    }
}

impl fmt::Display for TypeParameter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.bound {
            Some(bound) => write!(f, "{} : {}", self.name, bound.as_str()),
            None => write!(f, "{}", self.name),
        }
    }
}

/// The definition behind a [`NominalId`], kept in a [`TypeTable`]. Field and payload types are
/// the declared ones, which mention the declaration's own type `parameters`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeDef {
    /// Struct fields as `(name, is_var, type)`.
    Struct {
        parameters: Vec<TypeParameter>,
        fields: Vec<(String, bool, Type)>,
    },
    /// Enum variants as `(name, payload types)`.
    Enum {
        parameters: Vec<TypeParameter>,
        variants: Vec<(String, Vec<Type>)>,
    },
}

impl TypeDef {
    /// The declaration's type parameters.
    pub fn parameters(&self) -> &[TypeParameter] {
        match self {
            TypeDef::Struct { parameters, .. } | TypeDef::Enum { parameters, .. } => parameters,
        }
    }
}

/// The marker of a struct or enum and its marker parameters: the indices of the type parameters
/// that occur in its field or payload types, which condition the marker (D6 of `generics`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NominalMarker {
    pub marker: Option<TypeMarker>,
    pub parameters: Vec<u32>,
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
    markers: BTreeMap<NominalId, NominalMarker>,
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
                parameters: Vec::new(),
                fields: vec![
                    ("start".to_string(), false, Type::Int),
                    ("end".to_string(), false, Type::Int),
                ],
            },
        );
        table.insert_marker(range, Some(TypeMarker::Copy), Vec::new());
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

    /// Records the marker of `id` and its marker parameters, unless a marker is already
    /// recorded.
    pub fn insert_marker(
        &mut self,
        id: NominalId,
        marker: Option<TypeMarker>,
        marker_parameters: Vec<u32>,
    ) {
        self.markers.entry(id).or_insert(NominalMarker {
            marker,
            parameters: marker_parameters,
        });
    }

    /// The definition of `id`, if the file knows it.
    pub fn definition(&self, id: &NominalId) -> Option<&TypeDef> {
        self.definitions.get(id)
    }

    /// The marker of `id`, or `None` when it has none or is unknown.
    pub fn marker(&self, id: &NominalId) -> Option<TypeMarker> {
        self.markers.get(id).and_then(|marker| marker.marker)
    }

    /// The marker parameters of `id`: the type parameters that condition its marker.
    pub fn marker_parameters(&self, id: &NominalId) -> &[u32] {
        self.markers
            .get(id)
            .map_or(&[][..], |marker| &marker.parameters[..])
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
            Some(TypeDef::Struct { fields, .. }) => Some(fields),
            _ => None,
        }
    }

    /// The variants of enum `id`, if its definition is known.
    pub fn enum_variants(&self, id: &NominalId) -> Option<&[(String, Vec<Type>)]> {
        match self.definitions.get(id) {
            Some(TypeDef::Enum { variants, .. }) => Some(variants),
            _ => None,
        }
    }

    /// The type parameters of `id`, empty when it is not generic or unknown.
    pub fn parameters(&self, id: &NominalId) -> &[TypeParameter] {
        self.definitions.get(id).map_or(&[], TypeDef::parameters)
    }

    /// The fields of struct `id` with `arguments` substituted for its type parameters, if its
    /// definition is known.
    pub fn instantiated_fields(
        &self,
        id: &NominalId,
        arguments: &[Type],
    ) -> Option<Vec<(String, bool, Type)>> {
        self.struct_fields(id).map(|fields| {
            fields
                .iter()
                .map(|(name, is_var, ty)| (name.clone(), *is_var, ty.substitute(arguments)))
                .collect()
        })
    }

    /// The variants of enum `id` with `arguments` substituted for its type parameters, if its
    /// definition is known.
    pub fn instantiated_variants(
        &self,
        id: &NominalId,
        arguments: &[Type],
    ) -> Option<Vec<(String, Vec<Type>)>> {
        self.enum_variants(id).map(|variants| {
            variants
                .iter()
                .map(|(name, payloads)| {
                    let payloads = payloads.iter().map(|ty| ty.substitute(arguments)).collect();
                    (name.clone(), payloads)
                })
                .collect()
        })
    }

    /// Whether every marker parameter of `id` has an argument satisfying `satisfies`. A missing
    /// argument, which only an earlier arity error leaves, satisfies it.
    fn marker_arguments_satisfy(
        &self,
        id: &NominalId,
        arguments: &[Type],
        satisfies: impl Fn(&Type) -> bool,
    ) -> bool {
        self.marker_parameters(id)
            .iter()
            .all(|index| arguments.get(*index as usize).is_none_or(&satisfies))
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
            Type::Struct { id, arguments } | Type::Enum { id, arguments } => {
                self.marker(id) == Some(TypeMarker::Copy)
                    && self.marker_arguments_satisfy(id, arguments, |ty| self.is_copy(ty))
            }
            Type::Parameter(parameter) => parameter.bound == Some(TypeMarker::Copy),
            Type::String | Type::Heap(_) | Type::Fn { .. } | Type::Opaque(_) => false,
        }
    }

    /// Whether values of `ty` can be duplicated by an explicit `.clone()` (the `ownership` spec,
    /// "Built-In Clone Types"). Every `Copy` type is `Clone`.
    pub fn is_clone(&self, ty: &Type) -> bool {
        match ty {
            Type::String => true,
            Type::Nullable(inner) | Type::Heap(inner) => self.is_clone(inner),
            Type::Struct { id, arguments } | Type::Enum { id, arguments } => {
                self.marker(id).is_some()
                    && self.marker_arguments_satisfy(id, arguments, |ty| self.is_clone(ty))
            }
            // `Copy` implies `Clone`.
            Type::Parameter(parameter) => parameter.bound.is_some(),
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
    /// Nominal struct type, with one argument per type parameter of a generic struct. Its fields
    /// live in the checker's type table.
    Struct { id: NominalId, arguments: Vec<Type> },
    /// Nominal enum type, with one argument per type parameter of a generic enum. Its variants
    /// live in the checker's type table.
    Enum { id: NominalId, arguments: Vec<Type> },
    /// A type name with no struct or enum declaration in scope. Opaque types compare by name,
    /// and have no members.
    Opaque(String),
    /// First-class function type.
    Fn {
        is_compose: bool,
        params: Vec<Type>,
        ret: Box<Type>,
    },
    /// A type parameter of the enclosing generic declaration (`T` in `fun <T> id(x: T)`).
    Parameter(TypeParameter),
    /// Sentinel type for error recovery and downstream error suppression.
    Error,
}

impl Type {
    /// The non-generic struct `id`.
    pub fn plain_struct(id: NominalId) -> Type {
        Type::Struct {
            id,
            arguments: Vec::new(),
        }
    }

    /// The non-generic enum `id`.
    pub fn plain_enum(id: NominalId) -> Type {
        Type::Enum {
            id,
            arguments: Vec::new(),
        }
    }

    /// The nominal identity of a struct or enum type.
    pub fn nominal_id(&self) -> Option<&NominalId> {
        match self {
            Type::Struct { id, .. } | Type::Enum { id, .. } => Some(id),
            _ => None,
        }
    }

    /// The type with `arguments[i]` in place of each type parameter of index `i`, normalized
    /// through [`Type::nullable`], so `T?` with `T = String?` is `String?` (D4 of `generics`). A
    /// parameter without an argument stays as it is.
    pub fn substitute(&self, arguments: &[Type]) -> Type {
        if arguments.is_empty() {
            return self.clone();
        }
        match self {
            Type::Parameter(parameter) => arguments
                .get(parameter.index as usize)
                .cloned()
                .unwrap_or_else(|| self.clone()),
            Type::Reference { mutable, inner } => Type::Reference {
                mutable: *mutable,
                inner: Box::new(inner.substitute(arguments)),
            },
            Type::Nullable(inner) => inner.substitute(arguments).nullable(),
            Type::Heap(inner) => Type::Heap(Box::new(inner.substitute(arguments))),
            Type::Struct { id, arguments: own } => Type::Struct {
                id: id.clone(),
                arguments: own.iter().map(|ty| ty.substitute(arguments)).collect(),
            },
            Type::Enum { id, arguments: own } => Type::Enum {
                id: id.clone(),
                arguments: own.iter().map(|ty| ty.substitute(arguments)).collect(),
            },
            Type::Fn {
                is_compose,
                params,
                ret,
            } => Type::Fn {
                is_compose: *is_compose,
                params: params.iter().map(|ty| ty.substitute(arguments)).collect(),
                ret: Box::new(ret.substitute(arguments)),
            },
            other => other.clone(),
        }
    }

    /// Whether the type mentions a type parameter anywhere, function types included.
    pub fn mentions_parameter(&self) -> bool {
        match self {
            Type::Parameter(_) => true,
            Type::Reference { inner, .. } | Type::Nullable(inner) | Type::Heap(inner) => {
                inner.mentions_parameter()
            }
            Type::Struct { arguments, .. } | Type::Enum { arguments, .. } => {
                arguments.iter().any(Type::mentions_parameter)
            }
            Type::Fn { params, ret, .. } => {
                params.iter().any(Type::mentions_parameter) || ret.mentions_parameter()
            }
            _ => false,
        }
    }

    /// Whether the type mentions a type parameter outside every function type: a value of it may
    /// hold a value of the parameter's type (a generic slot, D7 of `generics`).
    pub fn mentions_parameter_outside_functions(&self) -> bool {
        match self {
            Type::Parameter(_) => true,
            Type::Reference { inner, .. } | Type::Nullable(inner) | Type::Heap(inner) => {
                inner.mentions_parameter_outside_functions()
            }
            Type::Struct { arguments, .. } | Type::Enum { arguments, .. } => arguments
                .iter()
                .any(Type::mentions_parameter_outside_functions),
            _ => false,
        }
    }

    /// The first reference the type contains outside every function type, if any: a type
    /// argument must not contain one (D7 of `generics`).
    pub fn reference_outside_functions(&self) -> Option<&Type> {
        match self {
            Type::Reference { .. } => Some(self),
            Type::Nullable(inner) | Type::Heap(inner) => inner.reference_outside_functions(),
            Type::Struct { arguments, .. } | Type::Enum { arguments, .. } => {
                arguments.iter().find_map(Type::reference_outside_functions)
            }
            _ => None,
        }
    }

    /// Makes a type nullable. `Error` stays `Error`, and nullable types never nest (`T??` is
    /// `T?`).
    pub fn nullable(self) -> Type {
        match self {
            Type::Error | Type::Nullable(_) => self,
            other => Type::Nullable(Box::new(other)),
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

    /// The constructor of an extension's receiver type, as `prism_abi::receiver_constructor`
    /// writes it for the same type in an ABI (`lib/team.Team`, `Int`, `Range`), so that a MIR
    /// callee finds its ABI symbol and the code generator names the extension's symbol.
    pub fn receiver_constructor(&self) -> String {
        match self {
            Type::Struct { id, .. } | Type::Enum { id, .. }
                if *id == NominalId::builtin_range() =>
            {
                "Range".to_string()
            }
            Type::Struct { id, .. } | Type::Enum { id, .. } => format!("{}.{}", id.module, id.name),
            Type::Opaque(name) => name.clone(),
            Type::Unit => "Unit".to_string(),
            other => other.to_string(),
        }
    }

    /// Renders the type with every nominal type qualified by its defining module.
    pub fn qualified(&self) -> String {
        match self {
            Type::Struct { id, arguments } | Type::Enum { id, arguments } => format!(
                "{}::{}{}",
                id.module,
                id.name,
                render_arguments(arguments, Type::qualified)
            ),
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
            Type::Struct { id, arguments } | Type::Enum { id, arguments } => write!(
                f,
                "{}{}",
                id.name,
                render_arguments(arguments, Type::to_string)
            ),
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
            Type::Parameter(parameter) => write!(f, "{}", parameter.name),
            Type::Error => write!(f, "<error>"),
        }
    }
}

/// Renders a nominal type's arguments (`<Int, String>`), or nothing when it has none.
fn render_arguments(arguments: &[Type], render: impl Fn(&Type) -> String) -> String {
    if arguments.is_empty() {
        return String::new();
    }
    let rendered: Vec<String> = arguments.iter().map(render).collect();
    format!("<{}>", rendered.join(", "))
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
        assert!(table.is_copy(&Type::plain_struct(NominalId::builtin_range())));
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
        table.insert_marker(point.clone(), Some(TypeMarker::Copy), Vec::new());
        table.insert_marker(user.clone(), Some(TypeMarker::Clone), Vec::new());
        table.insert_marker(file.clone(), None, Vec::new());
        // The first marker is kept.
        table.insert_marker(file.clone(), Some(TypeMarker::Copy), Vec::new());
        assert!(table.is_copy(&Type::plain_struct(point.clone())));
        assert!(table.is_copy(&Type::plain_struct(point).nullable()));
        assert!(!table.is_copy(&Type::plain_struct(user.clone())));
        assert!(table.is_clone(&Type::plain_enum(user)));
        assert!(!table.is_clone(&Type::plain_struct(file.clone())));
        assert!(table.knows(&file));
        assert_eq!(table.marker(&file), None);
        assert!(!table.is_copy(&Type::plain_struct(NominalId::new("main", "Unknown"))));
    }

    fn parameter(index: u32, name: &str, bound: Option<TypeMarker>) -> Type {
        Type::Parameter(TypeParameter {
            index,
            name: name.to_string(),
            bound,
        })
    }

    fn nominal(id: &NominalId, arguments: Vec<Type>) -> Type {
        Type::Struct {
            id: id.clone(),
            arguments,
        }
    }

    #[test]
    fn substitution_flattens_nullables() {
        let t = parameter(0, "T", None);
        assert_eq!(
            t.clone().nullable().substitute(&[Type::String.nullable()]),
            Type::String.nullable()
        );
        let boxed = NominalId::new("main", "Box");
        let function = Type::Fn {
            is_compose: false,
            params: vec![t.clone()],
            ret: Box::new(nominal(&boxed, vec![t.clone()])),
        };
        assert_eq!(
            function.substitute(&[Type::Int]).to_string(),
            "(Int) -> Box<Int>"
        );
        assert!(function.mentions_parameter());
        assert!(!function.mentions_parameter_outside_functions());
        assert!(Type::Heap(Box::new(t)).mentions_parameter_outside_functions());
    }

    #[test]
    fn markers_of_generic_types_are_conditional() {
        let mut table = TypeTable::new();
        let pair = NominalId::new("main", "Pair");
        let handle = NominalId::new("main", "Handle");
        table.insert_marker(pair.clone(), Some(TypeMarker::Copy), vec![0, 1]);
        table.insert_marker(handle.clone(), Some(TypeMarker::Copy), Vec::new());
        let ints = nominal(&pair, vec![Type::Int, Type::Int]);
        assert!(table.is_copy(&ints) && table.is_clone(&ints));
        let strings = nominal(&pair, vec![Type::String, Type::Int]);
        assert!(!table.is_copy(&strings) && table.is_clone(&strings));
        let functions = nominal(
            &pair,
            vec![
                Type::Fn {
                    is_compose: false,
                    params: Vec::new(),
                    ret: Box::new(Type::Unit),
                },
                Type::Int,
            ],
        );
        assert!(!table.is_clone(&functions));
        assert!(table.is_copy(&nominal(&handle, vec![Type::String])));
        assert_eq!(
            nominal(&pair, vec![Type::Int, Type::String]).qualified(),
            "main::Pair<Int, String>"
        );
    }

    #[test]
    fn type_parameters_follow_their_bounds() {
        let table = TypeTable::new();
        let copy = parameter(0, "T", Some(TypeMarker::Copy));
        let clone = parameter(0, "T", Some(TypeMarker::Clone));
        let unbounded = parameter(0, "T", None);
        assert!(table.is_copy(&copy) && table.is_clone(&copy));
        assert!(!table.is_copy(&clone) && table.is_clone(&clone));
        assert!(!table.is_copy(&unbounded) && !table.is_clone(&unbounded));
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

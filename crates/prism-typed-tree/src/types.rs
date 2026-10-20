//! The semantic type representation shared by the type checker and the typed tree.
//!
//! The rules over these types (lowering from annotations and ABIs, assignability, joins) live in
//! the type checker; this module holds only the representation and its rendering.

pub use prism_ast::ParameterKind;
use prism_ast::TypeMarker;
use std::collections::{BTreeMap, HashMap};
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

    /// The intrinsic struct this names, when it names one.
    pub fn intrinsic(&self) -> Option<IntrinsicStruct> {
        IntrinsicStruct::of(self)
    }
}

/// The compose runtime module, `/prism/compose/runtime` (D15 of `compose-transform`).
pub const COMPOSE_RUNTIME_MODULE: &str = "prism/compose/runtime";

/// The UI toolkit's base module, `/prism/compose/ui` (D21 of `ui-toolkit`).
pub const COMPOSE_UI_MODULE: &str = "prism/compose/ui";

/// A struct whose representation, glue and operations the compiler provides, declared in the
/// bundled library as an `intrinsic struct` without fields (D16 of `compose-transform`). No
/// program constructs one, reads a field of one or takes one apart: its intrinsic functions
/// make and use its values. Each is one word, a runtime handle, which `==` compares.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IntrinsicStruct {
    /// An owned composition, neither `Copy` nor `Clone`, disposed by its drop.
    Composition,
    /// A `Copy` handle of a restart scope.
    RecomposeScope,
    /// `State<T>`, an owned, read-only state object: one word naming the state cell it owns,
    /// retired by its drop (D1 and D5 of `owned-state`).
    State,
    /// `MutableState<T>`, an owned state object that may be written; it converts to
    /// `State<T>`.
    MutableState,
    /// `NodeId`, a `Copy` handle of a node of the UI tree, `package` to the toolkit (D3 of
    /// `ui-toolkit`).
    NodeId,
    /// `HandlerId`, a `Copy` handle of an entry of the UI handler table, compared by the
    /// runtime entry `prism_ui_handlers_equal` (D23 of `ui-toolkit`).
    HandlerId,
    /// `WeakReference<T>`, a `Copy` word naming a state without keeping it (D11 of
    /// `owned-state`).
    WeakReference,
}

impl IntrinsicStruct {
    /// Every intrinsic struct, in the order of the library's declarations.
    pub const ALL: [IntrinsicStruct; 7] = [
        IntrinsicStruct::Composition,
        IntrinsicStruct::RecomposeScope,
        IntrinsicStruct::State,
        IntrinsicStruct::MutableState,
        IntrinsicStruct::WeakReference,
        IntrinsicStruct::NodeId,
        IntrinsicStruct::HandlerId,
    ];

    /// The intrinsic struct `id` names, if any: by its defining module and name.
    pub fn of(id: &NominalId) -> Option<Self> {
        if id.module != COMPOSE_RUNTIME_MODULE && id.module != COMPOSE_UI_MODULE {
            return None;
        }
        Self::ALL
            .into_iter()
            .find(|intrinsic| intrinsic.module() == id.module && intrinsic.name() == id.name)
    }

    /// Its name in the library.
    pub fn name(self) -> &'static str {
        match self {
            IntrinsicStruct::Composition => "Composition",
            IntrinsicStruct::RecomposeScope => "RecomposeScope",
            IntrinsicStruct::State => "State",
            IntrinsicStruct::MutableState => "MutableState",
            IntrinsicStruct::NodeId => "NodeId",
            IntrinsicStruct::HandlerId => "HandlerId",
            IntrinsicStruct::WeakReference => "WeakReference",
        }
    }

    /// The names of its type parameters: `T`, the value type, for the state handles.
    pub fn type_parameters(self) -> &'static [&'static str] {
        match self {
            IntrinsicStruct::Composition
            | IntrinsicStruct::RecomposeScope
            | IntrinsicStruct::NodeId
            | IntrinsicStruct::HandlerId => &[],
            IntrinsicStruct::State
            | IntrinsicStruct::MutableState
            | IntrinsicStruct::WeakReference => &["T"],
        }
    }

    /// Whether it is a state object, `State<T>` or `MutableState<T>`.
    pub fn is_state_handle(self) -> bool {
        matches!(self, IntrinsicStruct::State | IntrinsicStruct::MutableState)
    }

    /// Whether its values keep attached function values past the call that receives them (D24
    /// of `owned-state`): a *keeper*, whose values carry the borrows of the weak captures of
    /// the content they keep. Only `Composition` is one: `runApp` returns once its app has
    /// ended, and the toolkit's handlers are kept by nodes of the composition.
    pub fn is_keeper(self) -> bool {
        self == IntrinsicStruct::Composition
    }

    /// The library module declaring it.
    pub fn module(self) -> &'static str {
        match self {
            IntrinsicStruct::NodeId | IntrinsicStruct::HandlerId => COMPOSE_UI_MODULE,
            _ => COMPOSE_RUNTIME_MODULE,
        }
    }

    /// The visibility its declaration must write: `package` for `NodeId`, which no public
    /// API hands out yet (D3 of `ui-toolkit`), `public` for the others.
    pub fn visibility(self) -> &'static str {
        match self {
            IntrinsicStruct::NodeId => "package",
            _ => "public",
        }
    }

    /// The runtime entry that compares two of its values, `(word, word) -> Bool`, when `==`
    /// is not the comparison of their words (D23 of `ui-toolkit`).
    pub fn equality_entry(self) -> Option<&'static str> {
        match self {
            IntrinsicStruct::HandlerId => Some("prism_ui_handlers_equal"),
            _ => None,
        }
    }

    /// Its identity.
    pub fn id(self) -> NominalId {
        NominalId::new(self.module(), self.name())
    }

    /// Its marker, which its declaration must write.
    pub fn marker(self) -> Option<TypeMarker> {
        match self {
            // A state object owns its state, so it is move-only whatever its value type (D1 of
            // `owned-state`).
            IntrinsicStruct::Composition
            | IntrinsicStruct::State
            | IntrinsicStruct::MutableState => None,
            // The markers of an intrinsic struct hold for every type argument: a weak reference
            // is `Copy` whatever it names.
            IntrinsicStruct::RecomposeScope
            | IntrinsicStruct::WeakReference
            | IntrinsicStruct::NodeId
            | IntrinsicStruct::HandlerId => Some(TypeMarker::Copy),
        }
    }

    /// The canonical text of its declaration, as `prism abi` shows it.
    pub fn declaration_text(self) -> String {
        let parameters = self.type_parameters();
        let parameters = if parameters.is_empty() {
            String::new()
        } else {
            format!("<{}>", parameters.join(", "))
        };
        match self.marker() {
            Some(marker) => format!(
                "{} intrinsic struct {}{} : {}",
                self.visibility(),
                self.name(),
                parameters,
                marker.as_str()
            ),
            None => format!(
                "{} intrinsic struct {}{}",
                self.visibility(),
                self.name(),
                parameters
            ),
        }
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
    /// The position of each field of a struct, or variant of an enum, by name (the first of a
    /// repeated name), so that a member is found in constant time however many there are.
    member_indices: BTreeMap<NominalId, HashMap<String, usize>>,
    /// The types a `capture` function of the library captures (D12 of `owned-state`), found
    /// from declarations, never from names: closures given to `attached` parameters capture
    /// their values weakly, and composables compare borrowed ones by identity.
    captured_weakly: std::collections::BTreeSet<NominalId>,
    /// The structs and enums with an `equals` operator, which `==` calls wherever their values
    /// are compared (D6 of `operator-conventions`), from their declarations and ABI records.
    equality_operators: std::collections::BTreeSet<NominalId>,
}

/// The position of each member of `definition` by name, the first of a repeated name.
fn member_indices(definition: &TypeDef) -> HashMap<String, usize> {
    let names: Vec<&String> = match definition {
        TypeDef::Struct { fields, .. } => fields.iter().map(|(name, _, _)| name).collect(),
        TypeDef::Enum { variants, .. } => variants.iter().map(|(name, _)| name).collect(),
    };
    let mut indices = HashMap::with_capacity(names.len());
    for (index, name) in names.into_iter().enumerate() {
        indices.entry(name.clone()).or_insert(index);
    }
    indices
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
        if !self.definitions.contains_key(&id) {
            self.member_indices
                .insert(id.clone(), member_indices(&definition));
            self.definitions.insert(id, definition);
        }
    }

    /// Replaces the definition of `id`.
    pub fn replace_definition(&mut self, id: NominalId, definition: TypeDef) {
        self.member_indices
            .insert(id.clone(), member_indices(&definition));
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

    /// The marker entry recorded for `id`, which may hold no marker: what the record codec
    /// of `instance-cache` stores beside the definitions.
    pub fn marker_entry(&self, id: &NominalId) -> Option<&NominalMarker> {
        self.markers.get(id)
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

    /// The position of the field `name` of struct `id`, if its definition is known and has one.
    pub fn field_index(&self, id: &NominalId, name: &str) -> Option<usize> {
        self.struct_fields(id)?;
        self.member_indices.get(id)?.get(name).copied()
    }

    /// The position of the variant `name` of enum `id`, if its definition is known and has one.
    pub fn variant_index(&self, id: &NominalId, name: &str) -> Option<usize> {
        self.enum_variants(id)?;
        self.member_indices.get(id)?.get(name).copied()
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

    /// The payload types of variant `index` of enum `id` with `arguments` substituted for its
    /// type parameters, if its definition is known and has that variant. Only that variant is
    /// instantiated, so a lookup costs its payloads, not the enum's variants.
    pub fn instantiated_variant(
        &self,
        id: &NominalId,
        arguments: &[Type],
        index: usize,
    ) -> Option<Vec<Type>> {
        let (_, payloads) = self.enum_variants(id)?.get(index)?;
        Some(payloads.iter().map(|ty| ty.substitute(arguments)).collect())
    }

    /// The type of field `index` of struct `id` with `arguments` substituted for its type
    /// parameters, if its definition is known and has that field.
    pub fn instantiated_field(
        &self,
        id: &NominalId,
        arguments: &[Type],
        index: usize,
    ) -> Option<Type> {
        let (_, _, ty) = self.struct_fields(id)?.get(index)?;
        Some(ty.substitute(arguments))
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

    /// The table of the structs and enums that `types` mention, with their definitions and
    /// markers, closed over the field and payload types of those definitions, and the built-in
    /// `Range` as this table defines it (D2 of `monomorphization`). An id this table knows only
    /// by its marker keeps only its marker. Linear in the definitions reached.
    pub fn restricted_to(&self, types: &[Type]) -> TypeTable {
        let mut ids = Vec::new();
        for ty in types {
            ty.collect_nominal_ids(&mut ids);
        }
        self.restricted_to_ids(ids)
    }

    /// [`TypeTable::restricted_to`] the structs and enums `ids`.
    pub fn restricted_to_ids(&self, ids: Vec<NominalId>) -> TypeTable {
        let mut restricted = TypeTable::new();
        let mut pending = ids;
        pending.push(NominalId::builtin_range());
        let mut seen = std::collections::HashSet::new();
        while let Some(id) = pending.pop() {
            if !seen.insert(id.clone()) {
                continue;
            }
            if let Some(definition) = self.definitions.get(&id) {
                match definition {
                    TypeDef::Struct { fields, .. } => {
                        for (_, _, ty) in fields {
                            ty.collect_nominal_ids(&mut pending);
                        }
                    }
                    TypeDef::Enum { variants, .. } => {
                        for (_, payloads) in variants {
                            for ty in payloads {
                                ty.collect_nominal_ids(&mut pending);
                            }
                        }
                    }
                }
                restricted.insert_definition(id.clone(), definition.clone());
            }
            if self.captured_weakly.contains(&id) {
                restricted.captured_weakly.insert(id.clone());
            }
            if self.equality_operators.contains(&id) {
                restricted.equality_operators.insert(id.clone());
            }
            if let Some(marker) = self.markers.get(&id) {
                restricted.markers.insert(id, marker.clone());
            }
        }
        restricted
    }

    /// Records that the type `id` has a capture function (D12 of `owned-state`).
    pub fn insert_capture_function(&mut self, id: NominalId) {
        self.captured_weakly.insert(id);
    }

    /// Whether values of `ty` have a capture function: a struct whose declarations include one.
    /// A set lookup, constant time per captured path and per parameter type.
    pub fn has_capture_function(&self, ty: &Type) -> bool {
        match ty {
            Type::Struct { id, .. } => self.captured_weakly.contains(id),
            _ => false,
        }
    }

    /// The types with a capture function, in order.
    pub fn capture_function_types(&self) -> impl Iterator<Item = &NominalId> {
        self.captured_weakly.iter()
    }

    /// Records that the struct or enum `id` has an `equals` operator (D6 of
    /// `operator-conventions`).
    pub fn insert_equality_operator(&mut self, id: NominalId) {
        self.equality_operators.insert(id);
    }

    /// Whether `==` on values of `ty` calls an `equals` operator: a struct or enum declared
    /// with one. A set lookup.
    pub fn has_equality_operator(&self, ty: &Type) -> bool {
        match ty {
            Type::Struct { id, .. } | Type::Enum { id, .. } => self.equality_operators.contains(id),
            _ => false,
        }
    }

    /// The types with an `equals` operator, in order.
    pub fn equality_operator_types(&self) -> impl Iterator<Item = &NominalId> {
        self.equality_operators.iter()
    }

    /// Adds the definitions and markers of `other` that this table does not record. Two tables
    /// of one program never disagree about a type, since a [`NominalId`] is unique in a program
    /// and every definition comes from the same declarations (D5 of `monomorphization`).
    pub fn extend_from(&mut self, other: &TypeTable) {
        self.captured_weakly
            .extend(other.captured_weakly.iter().cloned());
        self.equality_operators
            .extend(other.equality_operators.iter().cloned());
        for (id, definition) in &other.definitions {
            debug_assert!(
                self.definitions
                    .get(id)
                    .is_none_or(|existing| existing == definition),
                "two definitions of {}::{}",
                id.module,
                id.name
            );
            self.insert_definition(id.clone(), definition.clone());
        }
        for (id, marker) in &other.markers {
            self.markers
                .entry(id.clone())
                .or_insert_with(|| marker.clone());
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
            Type::Struct { id, arguments } | Type::Enum { id, arguments } => {
                self.marker(id) == Some(TypeMarker::Copy)
                    && self.marker_arguments_satisfy(id, arguments, |ty| self.is_copy(ty))
            }
            Type::Parameter(parameter) => parameter.bound == Some(TypeMarker::Copy),
            Type::String | Type::Heap(_) | Type::List(_) | Type::Fn { .. } | Type::Opaque(_) => {
                false
            }
        }
    }

    /// Whether values of `ty` can be duplicated by an explicit `.clone()` (the `ownership` spec,
    /// "Built-In Clone Types"). Every `Copy` type is `Clone`.
    pub fn is_clone(&self, ty: &Type) -> bool {
        match ty {
            Type::String => true,
            Type::Nullable(inner) | Type::Heap(inner) | Type::List(inner) => self.is_clone(inner),
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
    /// The built-in growable list `List<T>` (D2 of `collections`): a single owner of a buffer of
    /// elements, and an indirection, like `Heap<T>`.
    List(Box<Type>),
    /// Nominal struct type, with one argument per type parameter of a generic struct. Its fields
    /// live in the checker's type table.
    Struct { id: NominalId, arguments: Vec<Type> },
    /// Nominal enum type, with one argument per type parameter of a generic enum. Its variants
    /// live in the checker's type table.
    Enum { id: NominalId, arguments: Vec<Type> },
    /// A type name with no struct or enum declaration in scope. Opaque types compare by name,
    /// and have no members.
    Opaque(String),
    /// First-class function type, with each parameter's kind (plain, `escaping` or `attached`):
    /// one per parameter, always as many as `params`. With `receiver`, a function type with a
    /// receiver (`ColumnScope.(Int) -> Unit`, D2 of `receiver-lambdas`): `params[0]` is the
    /// receiver's borrow, `&T` or `&mut T` for `mut T.() -> R`, and is always plain. The kinds
    /// are part of the type's identity, but not of its representation: `converts_in_place`
    /// relates them by [`ParameterKind::converts_to`].
    Fn {
        is_compose: bool,
        receiver: bool,
        params: Vec<Type>,
        kinds: Vec<ParameterKind>,
        ret: Box<Type>,
    },
    /// A type parameter of the enclosing generic declaration (`T` in `fun <T> id(x: T)`).
    Parameter(TypeParameter),
    /// Sentinel type for error recovery and downstream error suppression.
    Error,
}

impl Type {
    /// The function type with `params` and `ret`, every parameter plain.
    pub fn function(is_compose: bool, params: Vec<Type>, ret: Type) -> Type {
        Type::Fn {
            is_compose,
            receiver: false,
            kinds: vec![ParameterKind::Plain; params.len()],
            params,
            ret: Box::new(ret),
        }
    }

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
            Type::List(inner) => Type::List(Box::new(inner.substitute(arguments))),
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
                receiver,
                params,
                kinds,
                ret,
            } => Type::Fn {
                is_compose: *is_compose,
                receiver: *receiver,
                params: params.iter().map(|ty| ty.substitute(arguments)).collect(),
                kinds: kinds.clone(),
                ret: Box::new(ret.substitute(arguments)),
            },
            other => other.clone(),
        }
    }

    /// Pushes the id of every struct and enum the type mentions, function types included.
    pub fn collect_nominal_ids(&self, out: &mut Vec<NominalId>) {
        match self {
            Type::Struct { id, arguments } | Type::Enum { id, arguments } => {
                out.push(id.clone());
                for argument in arguments {
                    argument.collect_nominal_ids(out);
                }
            }
            Type::Reference { inner, .. }
            | Type::Nullable(inner)
            | Type::Heap(inner)
            | Type::List(inner) => inner.collect_nominal_ids(out),
            Type::Fn { params, ret, .. } => {
                for param in params {
                    param.collect_nominal_ids(out);
                }
                ret.collect_nominal_ids(out);
            }
            _ => {}
        }
    }

    /// Whether the type mentions a type parameter anywhere, function types included.
    pub fn mentions_parameter(&self) -> bool {
        match self {
            Type::Parameter(_) => true,
            Type::Reference { inner, .. }
            | Type::Nullable(inner)
            | Type::Heap(inner)
            | Type::List(inner) => inner.mentions_parameter(),
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
            Type::Reference { inner, .. }
            | Type::Nullable(inner)
            | Type::Heap(inner)
            | Type::List(inner) => inner.mentions_parameter_outside_functions(),
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
            Type::Nullable(inner) | Type::Heap(inner) | Type::List(inner) => {
                inner.reference_outside_functions()
            }
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

    /// Whether a value of `self` can be used as a value of `to` without changing its
    /// representation (D8 of `closures`): the types are equal, or differ only by `&mut T`
    /// weakening to `&T` (behind shared borrows too, inside a nullable, and in the parameters
    /// and result of a function type, contravariantly in the parameters). Compose flags must
    /// match, and a parameter's kind may change by [`ParameterKind::converts_to`] (D3 of
    /// `escaping-closures`, D9 of `owned-state`): a function that accepts any function value
    /// also accepts an owned one, and both move their argument. An
    /// error on either side converts, so earlier errors are not reported again.
    ///
    /// The type checker allows a function value other than a named function or variant
    /// constructor to convert only by this relation, and the backend lowers such a `Subtype`
    /// coercion as a plain copy.
    pub fn converts_in_place(&self, to: &Type) -> bool {
        if self == to || *self == Type::Error || *to == Type::Error {
            return true;
        }
        match (self, to) {
            // Behind a shared borrow nothing is written, so the referent may weaken too.
            (
                Type::Reference { inner, .. },
                Type::Reference {
                    mutable: false,
                    inner: target,
                },
            ) => inner.converts_in_place(target),
            (
                Type::Reference {
                    mutable: true,
                    inner,
                },
                Type::Reference {
                    mutable: true,
                    inner: target,
                },
            ) => inner.converts_in_place(target) && target.converts_in_place(inner),
            (Type::Nullable(inner), Type::Nullable(target)) => inner.converts_in_place(target),
            (Type::Heap(inner), Type::Heap(target)) | (Type::List(inner), Type::List(target)) => {
                inner.converts_in_place(target) && target.converts_in_place(inner)
            }
            (
                Type::Struct { id, arguments },
                Type::Struct {
                    id: target,
                    arguments: target_arguments,
                },
            )
            | (
                Type::Enum { id, arguments },
                Type::Enum {
                    id: target,
                    arguments: target_arguments,
                },
            ) => {
                id == target
                    && arguments.len() == target_arguments.len()
                    && arguments
                        .iter()
                        .zip(target_arguments)
                        .all(|(a, b)| a.converts_in_place(b) && b.converts_in_place(a))
            }
            (
                // Whether either type has a receiver is ignored: `T.(A) -> R` and `(&T, A) -> R`
                // share a representation (D2 of `receiver-lambdas`).
                Type::Fn {
                    is_compose,
                    params,
                    kinds,
                    ret,
                    ..
                },
                Type::Fn {
                    is_compose: target_compose,
                    params: target_params,
                    kinds: target_kinds,
                    ret: target_ret,
                    ..
                },
            ) => {
                is_compose == target_compose
                    && params.len() == target_params.len()
                    && kinds
                        .iter()
                        .zip(target_kinds)
                        .all(|(&kind, &target)| kind.converts_to(target))
                    && target_params
                        .iter()
                        .zip(params)
                        .all(|(target, param)| target.converts_in_place(param))
                    && ret.converts_in_place(target_ret)
            }
            _ => false,
        }
    }

    /// Whether the type is a keeper (D24 of `owned-state`), or a nullable or `Heap<T>` of one:
    /// a value that may hold the borrows of the weak captures of the attached content it keeps.
    pub fn contains_keeper(&self) -> bool {
        match self {
            Type::Struct { id, .. } => id.intrinsic().is_some_and(IntrinsicStruct::is_keeper),
            Type::Nullable(inner) | Type::Heap(inner) => inner.contains_keeper(),
            _ => false,
        }
    }

    /// Whether the type is a function type or a nullable of one.
    pub fn is_function_value(&self) -> bool {
        match self {
            Type::Fn { .. } => true,
            Type::Nullable(inner) => matches!(**inner, Type::Fn { .. }),
            _ => false,
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
            Type::List(_) => "List".to_string(),
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
            Type::List(inner) => format!("List<{}>", inner.qualified()),
            Type::Fn {
                is_compose,
                receiver,
                params,
                kinds,
                ret,
            } => render_function(*is_compose, *receiver, params, kinds, ret, Type::qualified),
            other => other.to_string(),
        }
    }
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Type::Int => write!(f, "Int"),
            Type::Float => write!(f, "Double"),
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
            Type::List(inner) => write!(f, "List<{}>", inner),
            Type::Struct { id, arguments } | Type::Enum { id, arguments } => write!(
                f,
                "{}{}",
                id.name,
                render_arguments(arguments, Type::to_string)
            ),
            Type::Opaque(name) => write!(f, "{}", name),
            Type::Fn {
                is_compose,
                receiver,
                params,
                kinds,
                ret,
            } => write!(
                f,
                "{}",
                render_function(*is_compose, *receiver, params, kinds, ret, Type::to_string)
            ),
            Type::Parameter(parameter) => write!(f, "{}", parameter.name),
            Type::Error => write!(f, "<error>"),
        }
    }
}

/// Renders a function type as written: `compose `, then the receiver (`mut Builder.`) when it
/// has one, then its other parameters and its result.
fn render_function(
    is_compose: bool,
    receiver: bool,
    params: &[Type],
    kinds: &[ParameterKind],
    ret: &Type,
    render: impl Fn(&Type) -> String,
) -> String {
    let mut text = String::new();
    if is_compose {
        text.push_str("compose ");
    }
    let mut skipped = 0;
    if receiver {
        if let Some(Type::Reference { mutable, inner }) = params.first() {
            if *mutable {
                text.push_str("mut ");
            }
            text.push_str(&render(inner));
            text.push('.');
            skipped = 1;
        }
    }
    let rendered: Vec<String> = params
        .iter()
        .enumerate()
        .skip(skipped)
        .map(|(index, param)| {
            format!(
                "{}{}",
                kinds.get(index).copied().unwrap_or_default(),
                render(param)
            )
        })
        .collect();
    text.push_str(&format!("({}) -> {}", rendered.join(", "), render(ret)));
    text
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

    fn function(params: Vec<Type>, ret: Type) -> Type {
        Type::function(false, params, ret)
    }

    fn escaping_function(params: Vec<Type>, ret: Type) -> Type {
        Type::Fn {
            is_compose: false,
            receiver: false,
            kinds: vec![ParameterKind::Escaping; params.len()],
            params,
            ret: Box::new(ret),
        }
    }

    #[test]
    fn escaping_parameters_convert_in_place_one_way() {
        let handler = function(Vec::new(), Type::Unit);
        let lending = function(vec![handler.clone()], Type::Unit);
        let keeping = escaping_function(vec![handler.clone()], Type::Unit);
        // A function accepting any function value accepts an owned one.
        assert!(lending.converts_in_place(&keeping));
        assert!(!keeping.converts_in_place(&lending));
        // Contravariantly: as a parameter, the flags go the other way.
        let takes_lending = function(vec![lending.clone()], Type::Unit);
        let takes_keeping = function(vec![keeping.clone()], Type::Unit);
        assert!(takes_keeping.converts_in_place(&takes_lending));
        assert!(!takes_lending.converts_in_place(&takes_keeping));
        assert_eq!(keeping.to_string(), "(escaping () -> Unit) -> Unit");
    }

    /// `T.(params) -> ret`, or `mut T.(params) -> ret` with `mutable`.
    fn receiver_function(mutable: bool, receiver: Type, params: Vec<Type>, ret: Type) -> Type {
        let mut all = vec![reference(mutable, receiver)];
        all.extend(params);
        Type::Fn {
            is_compose: false,
            receiver: true,
            kinds: vec![ParameterKind::Plain; all.len()],
            params: all,
            ret: Box::new(ret),
        }
    }

    #[test]
    fn receiver_and_plain_function_types_convert_in_place() {
        let scope = Type::plain_struct(NominalId::new("main", "Scope"));
        let with_receiver = receiver_function(false, scope.clone(), vec![Type::Int], Type::Unit);
        let plain = function(vec![reference(false, scope.clone()), Type::Int], Type::Unit);
        // One representation: both directions, though the types differ.
        assert_ne!(with_receiver, plain);
        assert!(with_receiver.converts_in_place(&plain));
        assert!(plain.converts_in_place(&with_receiver));
        // A shared receiver may become a mutable one, never the reverse.
        let mutable = receiver_function(true, scope.clone(), vec![Type::Int], Type::Unit);
        assert!(with_receiver.converts_in_place(&mutable));
        assert!(!mutable.converts_in_place(&with_receiver));
        let plain_mutable = function(vec![reference(true, scope.clone()), Type::Int], Type::Unit);
        assert!(mutable.converts_in_place(&plain_mutable));
        assert!(!plain_mutable.converts_in_place(&with_receiver));
        // Rendering keeps the receiver as written.
        assert_eq!(with_receiver.to_string(), "Scope.(Int) -> Unit");
        assert_eq!(mutable.to_string(), "mut Scope.(Int) -> Unit");
        assert_eq!(plain.to_string(), "(&Scope, Int) -> Unit");
        assert_eq!(
            function(vec![with_receiver.clone()], Type::Unit).to_string(),
            "(Scope.(Int) -> Unit) -> Unit"
        );
        assert_eq!(with_receiver.qualified(), "main::Scope.(Int) -> Unit");
    }

    #[test]
    fn conversions_in_place() {
        let shared = reference(false, Type::Int);
        let mutable = reference(true, Type::Int);
        // Equal types and errors.
        assert!(Type::Int.converts_in_place(&Type::Int));
        assert!(Type::Error.converts_in_place(&Type::String));
        assert!(Type::String.converts_in_place(&Type::Error));
        // `&mut T` to `&T`, inside a nullable and behind shared borrows, never the reverse.
        assert!(mutable.converts_in_place(&shared));
        assert!(!shared.converts_in_place(&mutable));
        assert!(mutable
            .clone()
            .nullable()
            .converts_in_place(&shared.clone().nullable()));
        assert!(
            reference(false, mutable.clone()).converts_in_place(&reference(false, shared.clone()))
        );
        assert!(
            !reference(true, mutable.clone()).converts_in_place(&reference(true, shared.clone()))
        );
        // Steps that change a representation.
        assert!(!Type::Int.converts_in_place(&Type::Int.nullable()));
        assert!(!Type::Nothing.converts_in_place(&Type::Int));
        assert!(!Type::Heap(Box::new(Type::Int))
            .converts_in_place(&Type::Heap(Box::new(Type::Int.nullable()))));
        // Function types: contravariant parameters, covariant results, equal compose flags.
        let show = function(vec![Type::Int.nullable()], Type::Unit);
        let each = function(vec![Type::Int], Type::Unit);
        assert!(!show.converts_in_place(&each));
        assert!(
            !function(vec![], Type::Int).converts_in_place(&function(vec![], Type::Int.nullable()))
        );
        assert!(function(vec![shared.clone()], Type::Int)
            .converts_in_place(&function(vec![mutable.clone()], Type::Int)));
        assert!(!function(vec![mutable.clone()], Type::Int)
            .converts_in_place(&function(vec![shared.clone()], Type::Int)));
        assert!(
            function(vec![], mutable.clone()).converts_in_place(&function(vec![], shared.clone()))
        );
        assert!(show
            .clone()
            .nullable()
            .converts_in_place(&show.clone().nullable()));
        let composable = Type::function(true, vec![], Type::Unit);
        assert!(!composable.converts_in_place(&function(vec![], Type::Unit)));
        assert!(show.is_function_value() && show.clone().nullable().is_function_value());
        assert!(!Type::Int.is_function_value());
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
    fn a_restricted_table_holds_what_its_types_reach() {
        let mut table = TypeTable::with_builtins();
        let secret = NominalId::new("lib/box", "Secret");
        let holder = NominalId::new("lib/box", "Holder");
        let user = NominalId::new("lib/user", "User");
        let unrelated = NominalId::new("lib/box", "Unrelated");
        table.insert_definition(
            secret.clone(),
            TypeDef::Struct {
                parameters: Vec::new(),
                fields: vec![("n".to_string(), false, Type::Int)],
            },
        );
        table.insert_definition(
            holder.clone(),
            TypeDef::Struct {
                parameters: Vec::new(),
                fields: vec![(
                    "s".to_string(),
                    false,
                    Type::Heap(Box::new(Type::plain_struct(secret.clone()))),
                )],
            },
        );
        table.insert_definition(
            unrelated.clone(),
            TypeDef::Struct {
                parameters: Vec::new(),
                fields: Vec::new(),
            },
        );
        // A type of another module known by its marker alone keeps its marker.
        table.insert_marker(user.clone(), Some(TypeMarker::Clone), Vec::new());
        table.insert_marker(secret.clone(), Some(TypeMarker::Copy), Vec::new());
        let restricted = table.restricted_to(&[
            Type::plain_struct(holder.clone()).nullable(),
            Type::plain_struct(user.clone()),
        ]);
        assert!(restricted.definition(&holder).is_some());
        assert!(restricted.definition(&secret).is_some());
        assert_eq!(restricted.marker(&secret), Some(TypeMarker::Copy));
        assert!(restricted.definition(&user).is_none());
        assert_eq!(restricted.marker(&user), Some(TypeMarker::Clone));
        assert!(!restricted.knows(&unrelated));
        assert!(restricted.definition(&NominalId::builtin_range()).is_some());
        let mut union = TypeTable::new();
        union.extend_from(&restricted);
        union.extend_from(&table.restricted_to(&[Type::plain_struct(unrelated.clone())]));
        assert!(union.knows(&unrelated) && union.knows(&holder));
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
        let function = Type::function(false, vec![t.clone()], nominal(&boxed, vec![t.clone()]));
        assert_eq!(
            function.substitute(&[Type::Int]).to_string(),
            "(Int) -> Box<Int>"
        );
        assert!(function.mentions_parameter());
        assert!(!function.mentions_parameter_outside_functions());
        assert!(Type::Heap(Box::new(t)).mentions_parameter_outside_functions());
    }

    #[test]
    fn lists_render_substitute_and_follow_their_elements_markers() {
        let table = TypeTable::with_builtins();
        let t = parameter(0, "T", None);
        let list = |element: Type| Type::List(Box::new(element));
        assert_eq!(list(Type::Int.nullable()).to_string(), "List<Int?>");
        assert_eq!(list(list(Type::String)).qualified(), "List<List<String>>");
        assert_eq!(list(t.clone()).receiver_constructor(), "List");
        assert_eq!(
            list(t.clone()).substitute(&[Type::String]),
            list(Type::String)
        );
        assert!(list(t.clone()).mentions_parameter_outside_functions());
        assert!(list(reference(false, Type::Int))
            .reference_outside_functions()
            .is_some());
        // Never `Copy`; `Clone` exactly when the element is.
        assert!(!table.is_copy(&list(Type::Int)));
        assert!(table.is_clone(&list(Type::Int)) && table.is_clone(&list(Type::String)));
        assert!(!table.is_clone(&list(Type::function(false, Vec::new(), Type::Unit))));
        assert!(!table.is_clone(&list(t)));
        // Invariant: a list never converts to a list of another element type.
        assert!(!list(Type::Int).converts_in_place(&list(Type::Int.nullable())));
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
            vec![Type::function(false, Vec::new(), Type::Unit), Type::Int],
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
        let function = Type::function(false, vec![], Type::Int);
        assert_eq!(function.nullable().qualified(), "(() -> Int)?");
    }
}

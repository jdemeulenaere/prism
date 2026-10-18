//! Surface ABI signature extraction and hashing engine.

use crate::abi_type::{
    hash_marker, hash_marker_parameters, hash_type_parameters, receiver_constructor, AbiType,
    AbiTypeParameter, NameBinder, NominalHeader, NominalKind,
};
use crate::error::AbiError;
use crate::hasher::{ContentHash, DeterministicHasher};
use prism_ast::*;
use std::collections::HashMap;

/// The receiver of an exported extension function: its mode and its type, bound like any
/// signature type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbiReceiver {
    pub mode: ReceiverMode,
    pub ty: AbiType,
}

/// Represents an exported symbol in a file's ABI boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportedSymbol {
    Function {
        visibility: Visibility,
        is_compose: bool,
        /// The receiver of an extension function (`fun User.greet()`), `None` for a plain one.
        receiver: Option<AbiReceiver>,
        name: String,
        type_parameters: Vec<AbiTypeParameter>,
        params: Vec<(String, AbiType, Option<String>)>, // (name, type, default_value_expr)
        return_type: AbiType,
        /// The Monomorphization Hash of a generic function's body, `None` for a non-generic one.
        generic_body_hash: Option<ContentHash>,
    },
    Struct {
        visibility: Visibility,
        name: String,
        type_parameters: Vec<AbiTypeParameter>,
        marker: Option<TypeMarker>,
        /// The type parameters occurring in field types, which condition the marker.
        marker_parameters: Vec<u32>,
        fields: Vec<(String, bool, AbiType)>, // (name, is_var, type)
    },
    Enum {
        visibility: Visibility,
        name: String,
        type_parameters: Vec<AbiTypeParameter>,
        marker: Option<TypeMarker>,
        /// The type parameters occurring in payload types, which condition the marker.
        marker_parameters: Vec<u32>,
        variants: Vec<(String, Vec<AbiType>)>, // (name, payload types)
    },
    /// An exported top-level `val` (top-level declarations are always immutable).
    Var {
        visibility: Visibility,
        name: String,
        ty: AbiType,
    },
}

impl ExportedSymbol {
    /// Feeds what the symbol shows importers into `hasher`: everything the ABI records about it
    /// but a generic function's Monomorphization Hash. The file's interface hash is this stream
    /// for every symbol in order.
    fn hash_interface_into(&self, hasher: &mut DeterministicHasher) {
        match self {
            ExportedSymbol::Function {
                visibility,
                is_compose,
                receiver,
                name,
                type_parameters,
                params,
                return_type,
                generic_body_hash: _,
            } => {
                hasher.write_str("fun");
                hasher.write_str(visibility.as_str());
                hasher.write_u8(if *is_compose { 1 } else { 0 });
                match receiver {
                    Some(receiver) => {
                        hasher.write_u8(1);
                        hasher.write_u8(match receiver.mode {
                            ReceiverMode::Shared => 0,
                            ReceiverMode::Mutable => 1,
                            ReceiverMode::Consuming => 2,
                        });
                        receiver.ty.hash_into(hasher);
                    }
                    None => hasher.write_u8(0),
                }
                hasher.write_str(name);
                hash_type_parameters(type_parameters, hasher);
                hasher.write_u64(params.len() as u64);
                for (pname, pty, pdef) in params {
                    hasher.write_str(pname);
                    pty.hash_into(hasher);
                    if let Some(def) = pdef {
                        hasher.write_u8(1);
                        hasher.write_str(def);
                    } else {
                        hasher.write_u8(0);
                    }
                }
                return_type.hash_into(hasher);
            }
            ExportedSymbol::Struct {
                visibility,
                name,
                type_parameters,
                marker,
                marker_parameters,
                fields,
            } => {
                hasher.write_str("struct");
                hasher.write_str(visibility.as_str());
                hasher.write_str(name);
                hash_marker(*marker, hasher);
                hash_marker_parameters(marker_parameters, hasher);
                hash_type_parameters(type_parameters, hasher);
                hasher.write_u64(fields.len() as u64);
                for (fname, is_var, fty) in fields {
                    hasher.write_str(fname);
                    hasher.write_u8(if *is_var { 1 } else { 0 });
                    fty.hash_into(hasher);
                }
            }
            ExportedSymbol::Enum {
                visibility,
                name,
                type_parameters,
                marker,
                marker_parameters,
                variants,
            } => {
                hasher.write_str("enum");
                hasher.write_str(visibility.as_str());
                hasher.write_str(name);
                hash_marker(*marker, hasher);
                hash_marker_parameters(marker_parameters, hasher);
                hash_type_parameters(type_parameters, hasher);
                // Write variants count to eliminate canonicalization collisions
                hasher.write_u64(variants.len() as u64);
                for (vname, vflds) in variants {
                    hasher.write_str(vname);
                    // Write field count for this variant
                    hasher.write_u64(vflds.len() as u64);
                    for fty in vflds {
                        fty.hash_into(hasher);
                    }
                }
            }
            ExportedSymbol::Var {
                visibility,
                name,
                ty,
            } => {
                hasher.write_str("val");
                hasher.write_str(visibility.as_str());
                hasher.write_str(name);
                ty.hash_into(hasher);
            }
        }
    }

    /// The hash of [`ExportedSymbol::hash_interface_into`]'s stream for this symbol alone: a
    /// generic function's signature, which with its Monomorphization Hash keys its instances
    /// (D9 of `abi-cache`).
    pub fn interface_hash(&self) -> ContentHash {
        let mut hasher = DeterministicHasher::new();
        self.hash_interface_into(&mut hasher);
        hasher.finish()
    }

    /// The Monomorphization Hash of a generic function or extension, `None` for any other
    /// symbol.
    pub fn generic_body_hash(&self) -> Option<ContentHash> {
        match self {
            ExportedSymbol::Function {
                generic_body_hash, ..
            } => *generic_body_hash,
            _ => None,
        }
    }

    /// The key symbols are sorted by: kind, name, then a function's receiver type, qualified by
    /// its module (empty for a plain function), so extensions sharing a name keep a fixed order.
    fn sort_key(&self) -> (u8, &str, String) {
        match self {
            ExportedSymbol::Var { name, .. } => (0, name.as_str(), String::new()),
            ExportedSymbol::Struct { name, .. } => (1, name.as_str(), String::new()),
            ExportedSymbol::Enum { name, .. } => (2, name.as_str(), String::new()),
            ExportedSymbol::Function { name, receiver, .. } => (
                3,
                name.as_str(),
                receiver
                    .as_ref()
                    .map_or_else(String::new, |r| qualified_type_text(&r.ty)),
            ),
        }
    }
}

/// The text of `ty` with every nominal name qualified by its defining module (`models/user.User`).
fn qualified_type_text(ty: &AbiType) -> String {
    match ty {
        AbiType::Nominal {
            module, name, args, ..
        } => {
            let mut text = format!("{}.{}", module, name);
            if !args.is_empty() {
                let args: Vec<String> = args.iter().map(qualified_type_text).collect();
                text.push_str(&format!("<{}>", args.join(", ")));
            }
            text
        }
        other => other.to_string(),
    }
}

/// The marker parameters of a struct or enum with `type_parameters` and field or payload types
/// `member_types`: the indices, in order, of the type parameters that occur syntactically in a
/// member type (D6 of `generics`). Read from the AST alone, so Phase 1a and the type checker
/// compute the same answer.
pub fn marker_parameters(type_parameters: &[TypeParameter], member_types: &[&Type]) -> Vec<u32> {
    fn mentions(ty: &Type, name: &str) -> bool {
        match &ty.kind {
            TypeKind::Named {
                name: written,
                type_args,
            } => {
                (written == name && type_args.is_empty())
                    || type_args.iter().any(|arg| mentions(arg, name))
            }
            TypeKind::Nullable(inner) | TypeKind::Reference { inner, .. } => mentions(inner, name),
            TypeKind::Unit => false,
            TypeKind::Function {
                params,
                return_type,
                ..
            } => params.iter().any(|p| mentions(p, name)) || mentions(return_type, name),
        }
    }
    type_parameters
        .iter()
        .enumerate()
        .filter(|(_, parameter)| member_types.iter().any(|ty| mentions(ty, &parameter.name)))
        .map(|(index, _)| index as u32)
        .collect()
}

/// The header of struct `s`, read from its declaration alone.
fn struct_header(s: &StructDecl) -> NominalHeader {
    let members: Vec<&Type> = s.fields.iter().map(|field| &field.ty).collect();
    NominalHeader {
        kind: NominalKind::Struct,
        marker: s.marker,
        marker_parameters: marker_parameters(&s.type_parameters, &members),
    }
}

/// The header of enum `e`, read from its declaration alone.
fn enum_header(e: &EnumDecl) -> NominalHeader {
    let members: Vec<&Type> = e.variants.iter().flat_map(|v| &v.fields).collect();
    NominalHeader {
        kind: NominalKind::Enum,
        marker: e.marker,
        marker_parameters: marker_parameters(&e.type_parameters, &members),
    }
}

/// The type parameters of a declaration as the ABI records them.
fn abi_type_parameters(parameters: &[TypeParameter]) -> Vec<AbiTypeParameter> {
    parameters
        .iter()
        .map(|p| AbiTypeParameter {
            name: p.name.clone(),
            bound: p.bound,
        })
        .collect()
}

/// The structs and enums a file declares, with their headers, read from its AST alone (Phase 1a).
/// With `exported_only`, private declarations are left out: those are what importers can bind.
pub fn declared_types(file: &SourceFile, exported_only: bool) -> HashMap<&str, NominalHeader> {
    file.declarations
        .iter()
        .filter(|decl| !exported_only || decl.visibility() != Visibility::Private)
        .filter_map(|decl| match decl {
            Declaration::Struct(s) => Some((s.name.as_str(), struct_header(s))),
            Declaration::Enum(e) => Some((e.name.as_str(), enum_header(e))),
            _ => None,
        })
        .collect()
}

/// The fields of struct `s`, as `(name, is_var, type)`, each type bound with `bind` under the
/// struct's own type parameters.
fn struct_fields(s: &StructDecl, bind: &NameBinder) -> Vec<(String, bool, AbiType)> {
    let parameter_names: Vec<&str> = s.type_parameters.iter().map(|p| p.name.as_str()).collect();
    s.fields
        .iter()
        .map(|field| {
            let ty = AbiType::from_ast(&field.ty, &parameter_names, bind);
            (field.name.clone(), field.is_var, ty)
        })
        .collect()
}

/// The variants of enum `e`, as `(name, payload types)`, each type bound with `bind` under the
/// enum's own type parameters.
fn enum_variants(e: &EnumDecl, bind: &NameBinder) -> Vec<(String, Vec<AbiType>)> {
    let parameter_names: Vec<&str> = e.type_parameters.iter().map(|p| p.name.as_str()).collect();
    e.variants
        .iter()
        .map(|variant| {
            let payloads = variant
                .fields
                .iter()
                .map(|ty| AbiType::from_ast(ty, &parameter_names, bind))
                .collect();
            (variant.name.clone(), payloads)
        })
        .collect()
}

/// The fields of a struct or the variants of an enum, in a [`AbiTypeDefinition`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AbiTypeMembers {
    /// A struct's fields as `(name, is_var, type)`.
    Fields(Vec<(String, bool, AbiType)>),
    /// An enum's variants as `(name, payload types)`.
    Variants(Vec<(String, Vec<AbiType>)>),
}

/// The definition of a struct or enum that a value reached through a file's exports can hold
/// (D2 of `codegen-aggregates`): what an importer needs to lay out values of it, read from its
/// defining file's declarations alone, with every type bound as signature types are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbiTypeDefinition {
    /// The defining module.
    pub module: String,
    pub name: String,
    pub type_parameters: Vec<AbiTypeParameter>,
    pub marker: Option<TypeMarker>,
    pub marker_parameters: Vec<u32>,
    pub members: AbiTypeMembers,
}

impl AbiTypeDefinition {
    /// Whether the definition is a struct's or an enum's.
    pub fn kind(&self) -> NominalKind {
        match self.members {
            AbiTypeMembers::Fields(_) => NominalKind::Struct,
            AbiTypeMembers::Variants(_) => NominalKind::Enum,
        }
    }

    /// Every field or payload type, in declaration order.
    pub fn member_types(&self) -> Vec<&AbiType> {
        match &self.members {
            AbiTypeMembers::Fields(fields) => fields.iter().map(|(_, _, ty)| ty).collect(),
            AbiTypeMembers::Variants(variants) => {
                variants.iter().flat_map(|(_, payloads)| payloads).collect()
            }
        }
    }

    fn hash_into(&self, hasher: &mut DeterministicHasher) {
        hasher.write_str("definition");
        hasher.write_str(&self.module);
        hasher.write_str(&self.name);
        hash_marker(self.marker, hasher);
        hash_marker_parameters(&self.marker_parameters, hasher);
        hash_type_parameters(&self.type_parameters, hasher);
        match &self.members {
            AbiTypeMembers::Fields(fields) => {
                hasher.write_u8(0);
                hasher.write_u64(fields.len() as u64);
                for (name, is_var, ty) in fields {
                    hasher.write_str(name);
                    hasher.write_u8(u8::from(*is_var));
                    ty.hash_into(hasher);
                }
            }
            AbiTypeMembers::Variants(variants) => {
                hasher.write_u8(1);
                hasher.write_u64(variants.len() as u64);
                for (name, payloads) in variants {
                    hasher.write_str(name);
                    hasher.write_u64(payloads.len() as u64);
                    for ty in payloads {
                        ty.hash_into(hasher);
                    }
                }
            }
        }
    }
}

/// The binder of a file's own type names: its own structs and enums (private ones included)
/// first, bound to `module`, then `bind_import`, as Phase 1 binds every signature type.
fn own_binder<'a>(
    file: &'a SourceFile,
    module: &'a str,
    bind_import: &'a NameBinder<'a>,
) -> impl Fn(&str) -> Option<(NominalHeader, String)> + 'a {
    let own_types = declared_types(file, false);
    move |name: &str| match own_types.get(name) {
        Some(header) => Some((header.clone(), module.to_string())),
        None => bind_import(name),
    }
}

/// The definition of every struct and enum `file` declares, private ones included, in
/// declaration order (Phase 1a, D3 of `codegen-aggregates`). Types are bound as
/// [`FileAbi::extract`] binds them: to the file's own declarations first, then with
/// `bind_import`. A definition depends only on the file's AST and its direct imports' headers.
pub fn type_definitions(
    file: &SourceFile,
    module: &str,
    bind_import: &NameBinder,
) -> Vec<AbiTypeDefinition> {
    let bind = own_binder(file, module, bind_import);
    file.declarations
        .iter()
        .filter_map(|decl| match decl {
            Declaration::Struct(s) => Some(AbiTypeDefinition {
                module: module.to_string(),
                name: s.name.clone(),
                type_parameters: abi_type_parameters(&s.type_parameters),
                marker: s.marker,
                marker_parameters: struct_header(s).marker_parameters,
                members: AbiTypeMembers::Fields(struct_fields(s, &bind)),
            }),
            Declaration::Enum(e) => Some(AbiTypeDefinition {
                module: module.to_string(),
                name: e.name.clone(),
                type_parameters: abi_type_parameters(&e.type_parameters),
                marker: e.marker,
                marker_parameters: enum_header(e).marker_parameters,
                members: AbiTypeMembers::Variants(enum_variants(e, &bind)),
            }),
            _ => None,
        })
        .collect()
}

/// Feeds the defining module and name of every struct and enum occurring in `ty` into `out`:
/// the type itself, its type arguments, and the insides of nullables, references, `Heap<T>`
/// and function types.
fn nominal_types_in<'t>(ty: &'t AbiType, out: &mut Vec<(&'t str, &'t str)>) {
    match ty {
        AbiType::Nominal {
            module, name, args, ..
        } => {
            out.push((module, name));
            args.iter().for_each(|arg| nominal_types_in(arg, out));
        }
        AbiType::Unbound { args, .. } => args.iter().for_each(|arg| nominal_types_in(arg, out)),
        AbiType::Nullable(inner) | AbiType::Heap(inner) | AbiType::Ref { inner, .. } => {
            nominal_types_in(inner, out)
        }
        AbiType::Fn { params, ret, .. } => {
            params.iter().for_each(|param| nominal_types_in(param, out));
            nominal_types_in(ret, out);
        }
        AbiType::Prim(_) | AbiType::Unit | AbiType::Range | AbiType::Parameter { .. } => {}
    }
}

/// Every type an exported symbol shows: parameter, receiver, return and value types, and the
/// fields and payloads of exported structs and enums.
fn symbol_types(symbol: &ExportedSymbol) -> Vec<&AbiType> {
    match symbol {
        ExportedSymbol::Function {
            receiver,
            params,
            return_type,
            ..
        } => receiver
            .iter()
            .map(|r| &r.ty)
            .chain(params.iter().map(|(_, ty, _)| ty))
            .chain(std::iter::once(return_type))
            .collect(),
        ExportedSymbol::Struct { fields, .. } => fields.iter().map(|(_, _, ty)| ty).collect(),
        ExportedSymbol::Enum { variants, .. } => {
            variants.iter().flat_map(|(_, payloads)| payloads).collect()
        }
        ExportedSymbol::Var { ty, .. } => vec![ty],
    }
}

/// The definitions of the types reachable from `symbols` of the file `module` (D2 of
/// `codegen-aggregates`): the closure, over `definition`, of the nominal types the symbols
/// show, following each reached definition's fields and payloads, without the file's own
/// exported structs and enums (recorded as symbols). Sorted by module and name. A type
/// `definition` does not know ends the walk there.
fn reachable_definitions(
    module: &str,
    symbols: &[ExportedSymbol],
    definition: &dyn Fn(&str, &str) -> Option<AbiTypeDefinition>,
) -> Vec<AbiTypeDefinition> {
    let own_exports: std::collections::HashSet<&str> = symbols
        .iter()
        .filter_map(|symbol| match symbol {
            ExportedSymbol::Struct { name, .. } | ExportedSymbol::Enum { name, .. } => {
                Some(name.as_str())
            }
            _ => None,
        })
        .collect();
    let mut pending: Vec<(String, String)> = Vec::new();
    let mut roots = Vec::new();
    for ty in symbols.iter().flat_map(symbol_types) {
        nominal_types_in(ty, &mut roots);
    }
    pending.extend(
        roots
            .into_iter()
            .map(|(m, n)| (m.to_string(), n.to_string())),
    );
    let mut seen: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
    let mut found = Vec::new();
    while let Some((type_module, name)) = pending.pop() {
        if !seen.insert((type_module.clone(), name.clone())) {
            continue;
        }
        if type_module == module && own_exports.contains(name.as_str()) {
            continue;
        }
        let Some(reached) = definition(&type_module, &name) else {
            continue;
        };
        let mut members = Vec::new();
        for ty in reached.member_types() {
            nominal_types_in(ty, &mut members);
        }
        pending.extend(
            members
                .into_iter()
                .map(|(m, n)| (m.to_string(), n.to_string())),
        );
        found.push(reached);
    }
    found.sort_by(|a, b| (&a.module, &a.name).cmp(&(&b.module, &b.name)));
    found
}

/// The exported ABI boundary of a Prism source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileAbi {
    /// Package name, if any (e.g. `demo.math`).
    pub package: Option<String>,
    /// Module path: the file's root-relative path without extension (e.g. `demo/math/math`).
    /// It is the defining module of every struct and enum the file exports.
    pub module: String,
    /// All exported (`public` and `package`) symbols.
    pub symbols: Vec<ExportedSymbol>,
    /// The definitions of the structs and enums reachable from the symbols, other than the
    /// file's own exported ones, sorted by module and name (D2 of `codegen-aggregates`).
    pub type_definitions: Vec<AbiTypeDefinition>,
    /// The deterministic hash of the whole exported ABI: the interface hash and the
    /// Monomorphization Hashes of the exported generic bodies.
    pub abi_hash: ContentHash,
    /// The deterministic hash of the exported ABI without the Monomorphization Hashes: what an
    /// importer's Phase 2 depends on (D2 of `abi-cache`).
    pub interface_hash: ContentHash,
}

impl FileAbi {
    /// Computes the surface ABI and deterministic hash from a parsed [`SourceFile`] whose module
    /// path is `module`.
    ///
    /// Each nominal type name in an exported signature is bound to its defining module: first to
    /// the file's own top-level structs and enums (private ones included), then with
    /// `bind_import`, which looks the name up among the types exported by the file's direct
    /// imports. Binding never reads a function body. A name neither binds stays unbound.
    ///
    /// The reachable type definitions are looked up among the file's own declarations only;
    /// [`FileAbi::extract_with_definitions`] sees those of other files too.
    pub fn extract(
        file: &SourceFile,
        module: &str,
        bind_import: &NameBinder,
    ) -> Result<Self, AbiError> {
        let own = type_definitions(file, module, bind_import);
        let definition = |type_module: &str, name: &str| {
            own.iter()
                .find(|d| d.module == type_module && d.name == name)
                .cloned()
        };
        Self::extract_with_definitions(file, module, bind_import, &definition)
    }

    /// [`FileAbi::extract`], recording the definition of every type reachable from the exports
    /// that `definition` (by defining module and name) knows (D2 and D3 of
    /// `codegen-aggregates`).
    pub fn extract_with_definitions(
        file: &SourceFile,
        module: &str,
        bind_import: &NameBinder,
        definition: &dyn Fn(&str, &str) -> Option<AbiTypeDefinition>,
    ) -> Result<Self, AbiError> {
        let package = file.package.as_ref().map(|p| p.package_string());
        let mut symbols = Vec::new();

        let bind = own_binder(file, module, bind_import);

        for decl in &file.declarations {
            // Default visibility is `private`. Private declarations are EXCLUDED from the ABI.
            let vis = decl.visibility();
            if vis == Visibility::Private {
                continue;
            }

            // Each declaration's types are lowered with its own type parameters in scope.
            let parameter_names: Vec<&str> = match decl {
                Declaration::Function(f) => &f.type_parameters[..],
                Declaration::Struct(s) => &s.type_parameters[..],
                Declaration::Enum(e) => &e.type_parameters[..],
                Declaration::Var(_) => &[],
            }
            .iter()
            .map(|p| p.name.as_str())
            .collect();
            let lower = |ty: &Type| AbiType::from_ast(ty, &parameter_names, &bind);

            match decl {
                Declaration::Function(f) => {
                    // Check return type rule (single-file compilation unit invariant)
                    let return_type = match &f.return_type {
                        Some(ty) => lower(ty),
                        None => {
                            // If it's a block body or no body, return type is Unit `()`
                            // If it's an expression body `= expr` without return type, it is an inferred return type
                            // which is forbidden for exported ABI.
                            match &f.body {
                                Some(FunctionBody::Expr(_)) => {
                                    return Err(AbiError::MissingExplicitReturnType {
                                        function_name: f.name.clone(),
                                        visibility: vis.as_str().to_string(),
                                        span: f.span,
                                    });
                                }
                                _ => AbiType::Unit,
                            }
                        }
                    };

                    let params: Vec<(String, AbiType, Option<String>)> = f
                        .params
                        .iter()
                        .map(|p| {
                            // The default's whole syntax, span-free: its `Display` text summarizes blocks
                            // and `when` branches, which would leave their contents unhashed.
                            let def_str = p.default_value.as_ref().map(|d| {
                                let mut h = DeterministicHasher::new();
                                hash_expr_span_free(d, &mut h);
                                h.finish().to_hex()
                            });
                            (p.name.clone(), lower(&p.ty), def_str)
                        })
                        .collect();

                    // For generic functions and extensions, compute monomorphization hash of the body (`compilation-and-abi` spec)
                    // Hashing MUST be span-free to prevent comments/whitespace from altering ABI (hermetic builds invariant)
                    let generic_body_hash = if !f.type_parameters.is_empty() {
                        let mut h = DeterministicHasher::new();
                        if let Some(body) = &f.body {
                            hash_body_span_free(body, &mut h);
                        }
                        Some(h.finish())
                    } else {
                        None
                    };

                    let receiver = f.receiver.as_ref().map(|r| AbiReceiver {
                        mode: r.mode,
                        ty: lower(&r.ty),
                    });

                    symbols.push(ExportedSymbol::Function {
                        visibility: vis,
                        is_compose: f.is_compose,
                        receiver,
                        name: f.name.clone(),
                        type_parameters: abi_type_parameters(&f.type_parameters),
                        params,
                        return_type,
                        generic_body_hash,
                    });
                }

                Declaration::Struct(s) => {
                    let fields = struct_fields(s, &bind);

                    symbols.push(ExportedSymbol::Struct {
                        visibility: vis,
                        name: s.name.clone(),
                        type_parameters: abi_type_parameters(&s.type_parameters),
                        marker: s.marker,
                        marker_parameters: struct_header(s).marker_parameters,
                        fields,
                    });
                }

                Declaration::Enum(e) => {
                    let variants = enum_variants(e, &bind);

                    symbols.push(ExportedSymbol::Enum {
                        visibility: vis,
                        name: e.name.clone(),
                        type_parameters: abi_type_parameters(&e.type_parameters),
                        marker: e.marker,
                        marker_parameters: enum_header(e).marker_parameters,
                        variants,
                    });
                }

                Declaration::Var(v) => {
                    let ty = match &v.ty {
                        Some(t) => lower(t),
                        None => {
                            return Err(AbiError::MissingExplicitVarType {
                                var_name: v.name.clone(),
                                visibility: vis.as_str().to_string(),
                                span: v.span,
                            });
                        }
                    };

                    symbols.push(ExportedSymbol::Var {
                        visibility: vis,
                        name: v.name.clone(),
                        ty,
                    });
                }
            }
        }

        // Sort symbols deterministically by (kind, name) so declaration order does not perturb the hash
        symbols.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));

        let type_definitions = reachable_definitions(module, &symbols, definition);
        let (interface_hash, abi_hash) =
            Self::calculate_hashes(&package, module, &symbols, &type_definitions);

        Ok(FileAbi {
            package,
            module: module.to_string(),
            symbols,
            type_definitions,
            abi_hash,
            interface_hash,
        })
    }

    /// The interface hash and the ABI hash of `symbols` and `definitions`. The interface hash
    /// covers every symbol without its Monomorphization Hash, then every reachable type
    /// definition; the ABI hash covers the interface hash, then each symbol's Monomorphization
    /// Hash or its absence, in symbol order.
    fn calculate_hashes(
        package: &Option<String>,
        module: &str,
        symbols: &[ExportedSymbol],
        definitions: &[AbiTypeDefinition],
    ) -> (ContentHash, ContentHash) {
        let mut hasher = DeterministicHasher::new();

        if let Some(pkg) = package {
            hasher.write_str("package");
            hasher.write_str(pkg);
        }
        hasher.write_str("module");
        hasher.write_str(module);

        hasher.write_u64(symbols.len() as u64);

        for sym in symbols {
            sym.hash_interface_into(&mut hasher);
        }

        hasher.write_u64(definitions.len() as u64);
        for definition in definitions {
            definition.hash_into(&mut hasher);
        }

        let interface_hash = hasher.finish();

        let mut hasher = DeterministicHasher::new();
        hasher.write_str("abi");
        hasher.write_hash(&interface_hash);
        for sym in symbols {
            match sym {
                ExportedSymbol::Function {
                    generic_body_hash: Some(body_hash),
                    ..
                } => {
                    hasher.write_u8(1);
                    hasher.write_hash(body_hash);
                }
                _ => hasher.write_u8(0),
            }
        }
        (interface_hash, hasher.finish())
    }

    /// Formats the ABI hash as 64 hexadecimal digits.
    pub fn abi_hash_hex(&self) -> String {
        self.abi_hash.to_hex()
    }

    /// Formats the interface hash as 64 hexadecimal digits.
    pub fn interface_hash_hex(&self) -> String {
        self.interface_hash.to_hex()
    }

    /// The exported generic function or extension `name`, whose receiver's constructor is
    /// `receiver` ([`receiver_constructor`]) or which has no receiver. `None` when the file
    /// exports no such generic.
    pub fn generic_function_of(
        &self,
        name: &str,
        receiver: Option<&str>,
    ) -> Option<&ExportedSymbol> {
        self.symbols.iter().find(|symbol| match symbol {
            ExportedSymbol::Function {
                name: symbol_name,
                receiver: symbol_receiver,
                generic_body_hash: Some(_),
                ..
            } => {
                symbol_name == name
                    && symbol_receiver
                        .as_ref()
                        .map(|r| receiver_constructor(&r.ty))
                        .as_deref()
                        == receiver
            }
            _ => false,
        })
    }

    /// The Monomorphization Hash of the exported generic function or extension `name`, whose
    /// receiver's constructor is `receiver` ([`receiver_constructor`]) or which has no receiver.
    /// `None` when the file exports no such generic.
    pub fn generic_body_hash_of(&self, name: &str, receiver: Option<&str>) -> Option<ContentHash> {
        self.generic_function_of(name, receiver)?
            .generic_body_hash()
    }
}

/// Hashes a function body without inspecting `Span` coordinates.
fn hash_body_span_free(body: &FunctionBody, hasher: &mut DeterministicHasher) {
    match body {
        FunctionBody::Expr(e) => {
            hasher.write_u8(1);
            hash_expr_span_free(e, hasher);
        }
        FunctionBody::Block(b) => {
            hasher.write_u8(2);
            hash_block_span_free(b, hasher);
        }
    }
}

/// Hashes a block's statements and tail, ignoring all `Span` metadata.
fn hash_block_span_free(block: &Block, hasher: &mut DeterministicHasher) {
    hasher.write_u64(block.statements.len() as u64);
    for statement in &block.statements {
        hash_statement_span_free(statement, hasher);
    }
    if let Some(tail) = &block.tail {
        hasher.write_u8(1);
        hash_expr_span_free(tail, hasher);
    } else {
        hasher.write_u8(0);
    }
}

/// Hashes a statement's syntax structure, ignoring all `Span` metadata.
fn hash_statement_span_free(statement: &Statement, hasher: &mut DeterministicHasher) {
    match &statement.kind {
        StatementKind::Assign { target, value } => {
            hasher.write_u8(21);
            hash_expr_span_free(target, hasher);
            hash_expr_span_free(value, hasher);
        }
        StatementKind::CompoundAssign {
            operator,
            target,
            value,
        } => {
            hasher.write_u8(31);
            hasher.write_str(operator.as_str());
            hash_expr_span_free(target, hasher);
            hash_expr_span_free(value, hasher);
        }
        StatementKind::Local {
            is_var,
            name,
            ty,
            initializer,
        } => {
            hasher.write_u8(22);
            hasher.write_u8(if *is_var { 1 } else { 0 });
            hasher.write_str(name);
            if let Some(t) = ty {
                hasher.write_u8(1);
                hasher.write_str(&t.to_string());
            } else {
                hasher.write_u8(0);
            }
            hash_expr_span_free(initializer, hasher);
        }
        StatementKind::Destructure {
            is_var,
            entries,
            initializer,
        } => {
            hasher.write_u8(32);
            hasher.write_u8(if *is_var { 1 } else { 0 });
            hasher.write_u64(entries.len() as u64);
            for entry in entries {
                hasher.write_str(&entry.field);
                hasher.write_str(&entry.name);
            }
            hash_expr_span_free(initializer, hasher);
        }
        StatementKind::Expression(expr) => {
            hasher.write_u8(24);
            hash_expr_span_free(expr, hasher);
        }
        StatementKind::While { condition, body } => {
            hasher.write_u8(25);
            hash_expr_span_free(condition, hasher);
            hash_block_span_free(body, hasher);
        }
        StatementKind::DoWhile { body, condition } => {
            hasher.write_u8(26);
            hash_block_span_free(body, hasher);
            hash_expr_span_free(condition, hasher);
        }
        StatementKind::For {
            variable,
            iterable,
            body,
            ..
        } => {
            hasher.write_u8(27);
            hasher.write_str(variable);
            hash_expr_span_free(iterable, hasher);
            hash_block_span_free(body, hasher);
        }
    }
}

/// Hashes a pattern's syntax structure, ignoring all `Span` metadata.
fn hash_pattern_span_free(pattern: &Pattern, hasher: &mut DeterministicHasher) {
    match &pattern.kind {
        PatternKind::Value(e) => {
            hasher.write_u8(1);
            hash_expr_span_free(e, hasher);
        }
        PatternKind::InRange {
            start,
            end,
            end_inclusive,
        } => {
            hasher.write_u8(2);
            hash_expr_span_free(start, hasher);
            hash_expr_span_free(end, hasher);
            hasher.write_u8(if *end_inclusive { 1 } else { 0 });
        }
        PatternKind::Wildcard => hasher.write_u8(4),
        PatternKind::Binding { is_var, name } => {
            hasher.write_u8(5);
            hasher.write_u8(if *is_var { 1 } else { 0 });
            hasher.write_str(name);
        }
        PatternKind::Constructor { callee, arguments } => {
            hasher.write_u8(6);
            hash_expr_span_free(callee, hasher);
            hasher.write_u64(arguments.len() as u64);
            for argument in arguments {
                if let Some(name) = &argument.name {
                    hasher.write_u8(1);
                    hasher.write_str(name);
                } else {
                    hasher.write_u8(0);
                }
                hash_pattern_span_free(&argument.pattern, hasher);
            }
        }
    }
}

/// Recursively hashes expression syntax structure, ignoring all `Span` metadata.
fn hash_expr_span_free(expr: &Expr, hasher: &mut DeterministicHasher) {
    match &expr.kind {
        ExprKind::Literal(lit) => {
            hasher.write_u8(10);
            match lit {
                Literal::Int(v) => {
                    hasher.write_u8(1);
                    hasher.write_u64(*v as u64);
                }
                Literal::Float(v) => {
                    hasher.write_u8(2);
                    hasher.write_u64(v.to_bits());
                }
                Literal::String(s) => {
                    hasher.write_u8(3);
                    hasher.write_str(s);
                }
                Literal::Char(c) => {
                    hasher.write_u8(4);
                    hasher.write_u64(*c as u64);
                }
                Literal::Bool(b) => {
                    hasher.write_u8(5);
                    hasher.write_u8(if *b { 1 } else { 0 });
                }
                Literal::Null => {
                    hasher.write_u8(6);
                }
            }
        }
        ExprKind::Ident(id) => {
            hasher.write_u8(11);
            hasher.write_str(id);
        }
        ExprKind::Binary { op, left, right } => {
            hasher.write_u8(12);
            hasher.write_str(op.as_str());
            hash_expr_span_free(left, hasher);
            hash_expr_span_free(right, hasher);
        }
        ExprKind::Unary { op, expr } => {
            hasher.write_u8(13);
            hasher.write_str(op.as_str());
            hash_expr_span_free(expr, hasher);
        }
        ExprKind::Call {
            callee,
            type_arguments,
            args,
        } => {
            hasher.write_u8(14);
            hash_expr_span_free(callee, hasher);
            hasher.write_u64(type_arguments.len() as u64);
            for argument in type_arguments {
                hasher.write_str(&argument.to_string());
            }
            hasher.write_u64(args.len() as u64);
            for arg in args {
                if let Some(name) = &arg.name {
                    hasher.write_u8(1);
                    hasher.write_str(name);
                } else {
                    hasher.write_u8(0);
                }
                hash_expr_span_free(&arg.value, hasher);
            }
        }
        ExprKind::MemberAccess {
            target,
            member,
            is_safe_nav,
        } => {
            hasher.write_u8(15);
            hasher.write_str(member);
            hasher.write_u8(if *is_safe_nav { 1 } else { 0 });
            hash_expr_span_free(target, hasher);
        }
        ExprKind::Index { target, index } => {
            hasher.write_u8(16);
            hash_expr_span_free(target, hasher);
            hash_expr_span_free(index, hasher);
        }
        ExprKind::Block(b) => {
            hasher.write_u8(17);
            hash_block_span_free(b, hasher);
        }
        ExprKind::If {
            cond,
            then_branch,
            else_branch,
        } => {
            hasher.write_u8(18);
            hash_expr_span_free(cond, hasher);
            hash_expr_span_free(then_branch, hasher);
            if let Some(eb) = else_branch {
                hasher.write_u8(1);
                hash_expr_span_free(eb, hasher);
            } else {
                hasher.write_u8(0);
            }
        }
        ExprKind::When { target, branches } => {
            hasher.write_u8(19);
            if let Some(t) = target {
                hasher.write_u8(1);
                hash_expr_span_free(t, hasher);
            } else {
                hasher.write_u8(0);
            }
            hasher.write_u64(branches.len() as u64);
            for branch in branches {
                hasher.write_u64(branch.patterns.len() as u64);
                for pat in &branch.patterns {
                    match pat {
                        WhenPattern::Pattern(pattern) => {
                            hasher.write_u8(1);
                            hash_pattern_span_free(pattern, hasher);
                        }
                        WhenPattern::Else => {
                            hasher.write_u8(3);
                        }
                    }
                }
                if let Some(guard) = &branch.guard {
                    hasher.write_u8(1);
                    hash_expr_span_free(guard, hasher);
                } else {
                    hasher.write_u8(0);
                }
                hash_expr_span_free(&branch.body, hasher);
            }
        }
        ExprKind::Return(val) => {
            hasher.write_u8(20);
            if let Some(v) = val {
                hasher.write_u8(1);
                hash_expr_span_free(v, hasher);
            } else {
                hasher.write_u8(0);
            }
        }
        ExprKind::This => hasher.write_u8(30),
        ExprKind::Break => hasher.write_u8(28),
        ExprKind::Continue => hasher.write_u8(29),
        ExprKind::Closure { params, body } => {
            hasher.write_u8(23);
            hasher.write_u64(params.len() as u64);
            for p in params {
                hasher.write_str(&p.name);
            }
            hash_block_span_free(body, hasher);
        }
        ExprKind::Template(parts) => {
            hasher.write_u8(33);
            hasher.write_u64(parts.len() as u64);
            for part in parts {
                match part {
                    TemplatePart::Text(text) => {
                        hasher.write_u8(1);
                        hasher.write_str(text);
                    }
                    TemplatePart::Expression(part) => {
                        hasher.write_u8(2);
                        hash_expr_span_free(part, hasher);
                    }
                }
            }
        }
    }
}

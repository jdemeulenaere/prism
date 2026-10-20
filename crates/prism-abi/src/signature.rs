//! Surface ABI signature extraction and hashing engine.

use crate::abi_type::{
    hash_marker, hash_marker_parameters, hash_type_parameters, AbiType, AbiTypeParameter,
    NameBinder, NominalHeader, NominalKind,
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

/// A parameter of an exported function: its name, type, whether it is `escaping` or `attached`, the
/// span-free hash of its default value's syntax, if it has one, and the inputs that default
/// reads, which its generator takes (D26 of `ui-toolkit`). The inputs follow from the hashed
/// syntax, so they are not hashed again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbiParameter {
    pub name: String,
    pub ty: AbiType,
    pub kind: ParameterKind,
    pub default: Option<String>,
    pub default_inputs: prism_ast::DefaultInputs,
    /// Whether the parameter is `vararg` (D14 of `vararg`): `ty` is then the `List<E>` its
    /// body sees, and callers pack their elements into it.
    pub is_vararg: bool,
}

impl AbiParameter {
    /// The parameter as written in a signature: `name: Type`, `name: escaping () -> Unit`, or
    /// `vararg name: E` for a vararg of type `List<E>`.
    pub fn signature_text(&self) -> String {
        match (&self.ty, self.is_vararg) {
            (AbiType::List(element), true) => format!("vararg {}: {}", self.name, element),
            _ => format!("{}: {}{}", self.name, self.kind, self.ty),
        }
    }
}

/// What provides an exported function's code (D5 and D6 of `standard-library`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FunctionKind {
    /// A function with a Prism body.
    #[default]
    Prism,
    /// An `intrinsic` declaration, whose calls the compiler lowers.
    Intrinsic,
    /// An `external` declaration, a runtime entry point.
    External,
}

impl FunctionKind {
    /// The modifier written before the declaration, empty for an ordinary function.
    pub fn as_str(self) -> &'static str {
        match self {
            FunctionKind::Prism => "",
            FunctionKind::Intrinsic => "intrinsic",
            FunctionKind::External => "external",
        }
    }

    /// The kind of `f`, from its modifier.
    pub fn of(f: &FunctionDecl) -> Self {
        match f.library {
            None => FunctionKind::Prism,
            Some(LibraryModifier {
                kind: LibraryKind::Intrinsic,
                ..
            }) => FunctionKind::Intrinsic,
            Some(LibraryModifier {
                kind: LibraryKind::External,
                ..
            }) => FunctionKind::External,
        }
    }
}

/// Represents an exported symbol in a file's ABI boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportedSymbol {
    Function {
        visibility: Visibility,
        /// Whether the function has a body, or is intrinsic or external.
        kind: FunctionKind,
        is_compose: bool,
        /// Whether it is a `capture` function (D12 of `owned-state`).
        is_capture: bool,
        /// Whether it is an `operator` function, which operator syntax may call (D15 of
        /// `operator-conventions`).
        is_operator: bool,
        /// The receiver of an extension function (`fun User.greet()`), `None` for a plain one.
        receiver: Option<AbiReceiver>,
        /// The type of the context parameter (`context(scope: RowScope)`), bound like any
        /// signature type (D9 of `receiver-lambdas`).
        context: Option<AbiType>,
        name: String,
        type_parameters: Vec<AbiTypeParameter>,
        params: Vec<AbiParameter>,
        return_type: AbiType,
    },
    Struct {
        visibility: Visibility,
        name: String,
        type_parameters: Vec<AbiTypeParameter>,
        marker: Option<TypeMarker>,
        /// The type parameters occurring in field types, which condition the marker.
        marker_parameters: Vec<u32>,
        fields: Vec<(String, bool, AbiType)>, // (name, is_var, type)
        /// Whether it is an `intrinsic struct` of the bundled library, without fields, whose
        /// layout the compiler provides and is no part of the ABI (D16 of
        /// `compose-transform`).
        intrinsic: bool,
        /// Whether its file declares an `equals` operator for it, which `==` calls everywhere
        /// (D6 and D15 of `operator-conventions`).
        equality: bool,
        /// Whether its file declares a drop hook for it, which its drop glue calls everywhere
        /// (D10 of `drop-and-clone-hooks`).
        drop_hook: bool,
        /// Whether its file declares a clone hook for it, which its clone glue calls (D10 of
        /// `drop-and-clone-hooks`).
        clone_hook: bool,
    },
    Enum {
        visibility: Visibility,
        name: String,
        type_parameters: Vec<AbiTypeParameter>,
        marker: Option<TypeMarker>,
        /// The type parameters occurring in payload types, which condition the marker.
        marker_parameters: Vec<u32>,
        variants: Vec<(String, Vec<AbiType>)>, // (name, payload types)
        /// Whether its file declares an `equals` operator for it (D15 of
        /// `operator-conventions`).
        equality: bool,
        /// Whether its file declares a drop hook for it (D10 of `drop-and-clone-hooks`).
        drop_hook: bool,
        /// Whether its file declares a clone hook for it (D10 of `drop-and-clone-hooks`).
        clone_hook: bool,
    },
    /// An exported top-level `val` (top-level declarations are always immutable).
    Var {
        visibility: Visibility,
        name: String,
        ty: AbiType,
    },
    /// An exported intrinsic extension property (`public intrinsic val <T> List<T>.size: Int`,
    /// D6 of `standard-library`), writable when declared `intrinsic var` (D4 of
    /// `state-handles`).
    ExtensionProperty {
        visibility: Visibility,
        type_parameters: Vec<AbiTypeParameter>,
        receiver: AbiType,
        name: String,
        ty: AbiType,
        writable: bool,
    },
}

impl ExportedSymbol {
    /// Feeds what the symbol shows importers into `hasher`: everything the ABI records about it.
    /// No body is part of it, a generic one's included (D3 of `instance-cache`).
    fn hash_into(&self, hasher: &mut DeterministicHasher) {
        match self {
            ExportedSymbol::Function {
                visibility,
                kind,
                is_compose,
                is_capture,
                is_operator,
                receiver,
                context,
                name,
                type_parameters,
                params,
                return_type,
            } => {
                // An ordinary function keeps the tag it always had, so library kinds change no
                // existing hash.
                hasher.write_str(match kind {
                    FunctionKind::Prism => "fun",
                    FunctionKind::Intrinsic => "intrinsic fun",
                    FunctionKind::External => "external fun",
                });
                hasher.write_str(visibility.as_str());
                hasher.write_u8(if *is_compose { 1 } else { 0 });
                // A function that is no capture function keeps the bytes it always had.
                if *is_capture {
                    hasher.write_str("capture");
                }
                // So does a function that is no operator.
                if *is_operator {
                    hasher.write_str("operator");
                }
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
                // A function without a context keeps the bytes it always had.
                if let Some(context) = context {
                    hasher.write_str("context");
                    context.hash_into(hasher);
                }
                hasher.write_str(name);
                hash_type_parameters(type_parameters, hasher);
                hasher.write_u64(params.len() as u64);
                for parameter in params {
                    hasher.write_str(&parameter.name);
                    parameter.ty.hash_into(hasher);
                    // Plain and `escaping` keep the bytes they always had. A vararg is never
                    // `escaping` or `attached` (the parser rejects both), so it takes its own
                    // value of the same byte.
                    hasher.write_u8(match (parameter.is_vararg, parameter.kind) {
                        (true, _) => 3,
                        (false, ParameterKind::Plain) => 0,
                        (false, ParameterKind::Escaping) => 1,
                        (false, ParameterKind::Attached) => 2,
                    });
                    if let Some(def) = &parameter.default {
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
                intrinsic,
                equality,
                drop_hook,
                clone_hook,
            } => {
                // An ordinary struct keeps the tag it always had.
                hasher.write_str(if *intrinsic {
                    "intrinsic struct"
                } else {
                    "struct"
                });
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
                // A type without an equality operator keeps the bytes it always had.
                if *equality {
                    hasher.write_str("equals");
                }
                hash_hooks(*drop_hook, *clone_hook, hasher);
            }
            ExportedSymbol::Enum {
                visibility,
                name,
                type_parameters,
                marker,
                marker_parameters,
                variants,
                equality,
                drop_hook,
                clone_hook,
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
                if *equality {
                    hasher.write_str("equals");
                }
                hash_hooks(*drop_hook, *clone_hook, hasher);
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
            ExportedSymbol::ExtensionProperty {
                visibility,
                type_parameters,
                receiver,
                name,
                ty,
                writable,
            } => {
                hasher.write_str(if *writable {
                    "intrinsic var"
                } else {
                    "intrinsic val"
                });
                hasher.write_str(visibility.as_str());
                hash_type_parameters(type_parameters, hasher);
                receiver.hash_into(hasher);
                hasher.write_str(name);
                ty.hash_into(hasher);
            }
        }
    }

    /// The symbol's declaration as `prism abi` shows it and the intrinsic catalog compares it
    /// (D6 of `standard-library`): visibility, library modifier, `compose`, receiver mode, type
    /// parameters, receiver, name, parameters without their defaults and result, as in
    /// `public intrinsic mut fun <T> List<T>.add(element: T): Unit`, or an intrinsic struct's
    /// header (`public intrinsic struct RecomposeScope : Copy`). `None` for any other struct
    /// or enum.
    pub fn declaration_text(&self) -> Option<String> {
        match self {
            ExportedSymbol::Function {
                visibility,
                kind,
                is_compose,
                is_capture,
                is_operator,
                receiver,
                context,
                name,
                type_parameters,
                params,
                return_type,
            } => {
                let mut text = String::new();
                if let Some(context) = context {
                    text.push_str(&format!("context({}) ", context));
                }
                text.push_str(&format!("{} ", visibility));
                if *kind != FunctionKind::Prism {
                    text.push_str(kind.as_str());
                    text.push(' ');
                }
                if *is_operator {
                    text.push_str("operator ");
                }
                if *is_capture {
                    text.push_str("capture ");
                }
                if *is_compose {
                    text.push_str("compose ");
                }
                if let Some(receiver) = receiver {
                    if !receiver.mode.as_str().is_empty() {
                        text.push_str(receiver.mode.as_str());
                        text.push(' ');
                    }
                }
                text.push_str("fun ");
                text.push_str(&type_parameter_text(type_parameters));
                if let Some(receiver) = receiver {
                    text.push_str(&format!("{}.", receiver.ty));
                }
                let params: Vec<String> = params
                    .iter()
                    .map(|p| p.signature_text())
                    .collect();
                text.push_str(&format!("{}({}): {}", name, params.join(", "), return_type));
                Some(text)
            }
            ExportedSymbol::ExtensionProperty {
                visibility,
                type_parameters,
                receiver,
                name,
                ty,
                writable,
            } => Some(format!(
                "{} intrinsic {} {}{}.{}: {}",
                visibility,
                if *writable { "var" } else { "val" },
                type_parameter_text(type_parameters),
                receiver,
                name,
                ty
            )),
            ExportedSymbol::Var {
                visibility,
                name,
                ty,
            } => Some(format!("{} val {}: {}", visibility, name, ty)),
            // An intrinsic struct is all header (D16 of `compose-transform`).
            ExportedSymbol::Struct {
                visibility,
                name,
                type_parameters,
                marker,
                intrinsic: true,
                ..
            } => {
                let mut text = format!("{} intrinsic struct {}", visibility, name);
                if !type_parameters.is_empty() {
                    let names: Vec<&str> =
                        type_parameters.iter().map(|p| p.name.as_str()).collect();
                    text.push_str(&format!("<{}>", names.join(", ")));
                }
                if let Some(marker) = marker {
                    text.push_str(&format!(" : {}", marker.as_str()));
                }
                Some(text)
            }
            ExportedSymbol::Struct { .. } | ExportedSymbol::Enum { .. } => None,
        }
    }

    /// The key symbols are sorted by: kind, name, then a function's receiver type, qualified by
    /// its module (empty for a plain function), then its context type, qualified the same way
    /// (empty without one), then its receiver mode, so extensions sharing a name keep a fixed
    /// order (a shared and a `mut` accessor of one type, D9 of `operator-conventions`).
    fn sort_key(&self) -> (u8, &str, String, String, u8) {
        match self {
            ExportedSymbol::Var { name, .. } => (0, name.as_str(), String::new(), String::new(), 0),
            ExportedSymbol::Struct { name, .. } => {
                (1, name.as_str(), String::new(), String::new(), 0)
            }
            ExportedSymbol::Enum { name, .. } => {
                (2, name.as_str(), String::new(), String::new(), 0)
            }
            ExportedSymbol::Function {
                name,
                receiver,
                context,
                ..
            } => (
                3,
                name.as_str(),
                receiver
                    .as_ref()
                    .map_or_else(String::new, |r| qualified_type_text(&r.ty)),
                context
                    .as_ref()
                    .map_or_else(String::new, qualified_type_text),
                receiver.as_ref().map_or(0, |r| match r.mode {
                    ReceiverMode::Shared => 0,
                    ReceiverMode::Mutable => 1,
                    ReceiverMode::Consuming => 2,
                }),
            ),
            ExportedSymbol::ExtensionProperty { name, receiver, .. } => (
                4,
                name.as_str(),
                qualified_type_text(receiver),
                String::new(),
                0,
            ),
        }
    }
}

/// `<T, R : Copy> ` for type parameters, with a trailing space, or nothing.
fn type_parameter_text(parameters: &[AbiTypeParameter]) -> String {
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
                receiver,
                params,
                return_type,
                ..
            } => {
                receiver.as_ref().is_some_and(|r| mentions(&r.ty, name))
                    || params.iter().any(|p| mentions(p, name))
                    || mentions(return_type, name)
            }
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
    /// Whether the defining file declares an `equals` operator for the type, which equality of
    /// its values calls (D6 and D15 of `operator-conventions`).
    pub equality: bool,
    /// Whether the defining file declares a drop hook for the type, which drop glue of its
    /// values calls first (D10 of `drop-and-clone-hooks`).
    pub drop_hook: bool,
    /// Whether the defining file declares a clone hook for the type, which clone glue of its
    /// values calls instead of cloning field by field (D10 of `drop-and-clone-hooks`).
    pub clone_hook: bool,
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

    /// The structural hash of the definition alone, as the ABI hash covers it: what an
    /// instance key covers for each type its type arguments reach (D6 of `instance-cache`).
    pub fn hash(&self) -> ContentHash {
        let mut hasher = DeterministicHasher::new();
        self.hash_into(&mut hasher);
        hasher.finish()
    }

    /// The defining module and name of every struct and enum its fields or payloads mention.
    pub fn referenced_types(&self) -> Vec<(&str, &str)> {
        let mut out = Vec::new();
        for ty in self.member_types() {
            nominal_types_in(ty, &mut out);
        }
        out
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
        // A type without an equality operator keeps the bytes it always had.
        if self.equality {
            hasher.write_str("equals");
        }
        hash_hooks(self.drop_hook, self.clone_hook, hasher);
    }
}

/// Feeds a type's hook flags into `hasher`; a type without hooks keeps the bytes it always had
/// (D10 of `drop-and-clone-hooks`).
fn hash_hooks(drop_hook: bool, clone_hook: bool, hasher: &mut DeterministicHasher) {
    if drop_hook {
        hasher.write_str("drop hook");
    }
    if clone_hook {
        hasher.write_str("clone hook");
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
    let equality = equality_types(file);
    let hooks = hook_types(file);
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
                equality: equality.contains(s.name.as_str()),
                drop_hook: hooks.drop.contains(s.name.as_str()),
                clone_hook: hooks.clone.contains(s.name.as_str()),
            }),
            Declaration::Enum(e) => Some(AbiTypeDefinition {
                module: module.to_string(),
                name: e.name.clone(),
                type_parameters: abi_type_parameters(&e.type_parameters),
                marker: e.marker,
                marker_parameters: enum_header(e).marker_parameters,
                members: AbiTypeMembers::Variants(enum_variants(e, &bind)),
                equality: equality.contains(e.name.as_str()),
                drop_hook: hooks.drop.contains(e.name.as_str()),
                clone_hook: hooks.clone.contains(e.name.as_str()),
            }),
            _ => None,
        })
        .collect()
}

/// The names of the structs and enums of `file` for which it declares an equality operator,
/// `operator fun T.equals(...)` on a non-generic struct or enum `T` of the file, by a syntactic
/// match of the receiver's name (D15 of `operator-conventions`). The type checker checks the
/// declaration's shape (`E4035`).
pub fn equality_types(file: &SourceFile) -> std::collections::HashSet<&str> {
    let types: std::collections::HashSet<&str> = file
        .declarations
        .iter()
        .filter_map(|decl| match decl {
            Declaration::Struct(s) if s.type_parameters.is_empty() && !s.is_intrinsic() => {
                Some(s.name.as_str())
            }
            Declaration::Enum(e) if e.type_parameters.is_empty() => Some(e.name.as_str()),
            _ => None,
        })
        .collect();
    file.declarations
        .iter()
        .filter_map(|decl| match decl {
            Declaration::Function(f) if f.is_operator && f.name == "equals" => {
                match &f.receiver.as_ref()?.ty.kind {
                    prism_ast::types::TypeKind::Named { name, type_args }
                        if type_args.is_empty() && types.contains(name.as_str()) =>
                    {
                        Some(name.as_str())
                    }
                    _ => None,
                }
            }
            _ => None,
        })
        .collect()
}

/// The names of the structs and enums of `file` with a drop hook and with a clone hook.
#[derive(Debug, Default)]
pub struct HookTypes<'f> {
    /// Types with `operator mut fun T.drop()`.
    pub drop: std::collections::HashSet<&'f str>,
    /// Types with `operator fun T.clone()`.
    pub clone: std::collections::HashSet<&'f str>,
}

/// The structs and enums of `file` for which it declares a drop hook, an `operator` extension
/// named `drop` with a `mut` receiver, or a clone hook, one named `clone` with a shared
/// receiver, on a struct or enum of the file that is not intrinsic, by a syntactic match of the
/// receiver's name, generic or not (D3 and D10 of `drop-and-clone-hooks`). The type checker
/// checks the declarations' shapes (`E4035`).
pub fn hook_types(file: &SourceFile) -> HookTypes<'_> {
    let mut hooks = HookTypes::default();
    if !file.declarations.iter().any(|decl| {
        matches!(decl, Declaration::Function(f) if f.is_operator && (f.name == "drop" || f.name == "clone"))
    }) {
        return hooks;
    }
    let types: std::collections::HashSet<&str> = file
        .declarations
        .iter()
        .filter_map(|decl| match decl {
            Declaration::Struct(s) if !s.is_intrinsic() => Some(s.name.as_str()),
            Declaration::Enum(e) => Some(e.name.as_str()),
            _ => None,
        })
        .collect();
    for decl in &file.declarations {
        let Declaration::Function(f) = decl else {
            continue;
        };
        if !f.is_operator {
            continue;
        }
        let Some(receiver) = f.receiver.as_ref() else {
            continue;
        };
        let prism_ast::types::TypeKind::Named { name, .. } = &receiver.ty.kind else {
            continue;
        };
        if !types.contains(name.as_str()) {
            continue;
        }
        match (f.name.as_str(), receiver.mode) {
            ("drop", prism_ast::decl::ReceiverMode::Mutable) => {
                hooks.drop.insert(name.as_str());
            }
            ("clone", prism_ast::decl::ReceiverMode::Shared) => {
                hooks.clone.insert(name.as_str());
            }
            _ => {}
        }
    }
    hooks
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
        AbiType::Nullable(inner)
        | AbiType::Heap(inner)
        | AbiType::List(inner)
        | AbiType::Ref { inner, .. } => nominal_types_in(inner, out),
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
            context,
            params,
            return_type,
            ..
        } => receiver
            .iter()
            .map(|r| &r.ty)
            .chain(context.iter())
            .chain(params.iter().map(|parameter| &parameter.ty))
            .chain(std::iter::once(return_type))
            .collect(),
        ExportedSymbol::Struct { fields, .. } => fields.iter().map(|(_, _, ty)| ty).collect(),
        ExportedSymbol::Enum { variants, .. } => {
            variants.iter().flat_map(|(_, payloads)| payloads).collect()
        }
        ExportedSymbol::Var { ty, .. } => vec![ty],
        ExportedSymbol::ExtensionProperty { receiver, ty, .. } => vec![receiver, ty],
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
    /// The deterministic hash of the exported ABI: its signatures and reachable type
    /// definitions, never a body (D3 of `instance-cache`). What an importer's Phase 2 depends on.
    pub abi_hash: ContentHash,
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
        Self::extract_with_origin(file, module, bind_import, definition, false)
    }

    /// [`FileAbi::extract_with_definitions`] for a file of the bundled library when
    /// `in_library`, the only files that may declare `external` and `intrinsic` declarations: a
    /// workspace file using either is rejected at its first modifier with
    /// [`AbiError::LibraryOnlyDeclaration`] (`E3002`, D12 of `standard-library`), so no ABI ever
    /// carries a workspace intrinsic.
    pub fn extract_with_origin(
        file: &SourceFile,
        module: &str,
        bind_import: &NameBinder,
        definition: &dyn Fn(&str, &str) -> Option<AbiTypeDefinition>,
        in_library: bool,
    ) -> Result<Self, AbiError> {
        if !in_library {
            if let Some(modifier) = file
                .declarations
                .iter()
                .find_map(Declaration::library_modifier)
            {
                return Err(AbiError::LibraryOnlyDeclaration {
                    modifier: modifier.kind.as_str().to_string(),
                    span: modifier.span,
                });
            }
        }
        let package = file.package.as_ref().map(|p| p.package_string());
        let mut symbols = Vec::new();

        let bind = own_binder(file, module, bind_import);
        let equality = equality_types(file);
        let hooks = hook_types(file);

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
                Declaration::ExtensionProperty(p) => &p.type_parameters[..],
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
                                // A library declaration has no body to infer from, so it
                                // returns `Unit` unless it says otherwise.
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

                    let earlier: Vec<String> = f.params.iter().map(|p| p.name.clone()).collect();
                    let context_name = f.context.as_ref().map(|c| c.name.as_str());
                    let params: Vec<AbiParameter> = f
                        .params
                        .iter()
                        .enumerate()
                        .map(|(index, p)| {
                            // The default's whole syntax, span-free: its `Display` text summarizes blocks
                            // and `when` branches, which would leave their contents unhashed.
                            let def_str = p.default_value.as_ref().map(|d| {
                                let mut h = DeterministicHasher::new();
                                hash_expr_span_free(d, &mut h);
                                h.finish().to_hex()
                            });
                            let default_inputs = p.default_value.as_ref().map_or(
                                prism_ast::DefaultInputs::ALL,
                                |d| {
                                    prism_ast::default_inputs(
                                        d,
                                        context_name,
                                        f.receiver.is_some(),
                                        &earlier[..index],
                                    )
                                },
                            );
                            // A vararg's body sees the list of its written element type.
                            let ty = if p.is_vararg {
                                AbiType::List(Box::new(lower(&p.ty)))
                            } else {
                                lower(&p.ty)
                            };
                            AbiParameter {
                                name: p.name.clone(),
                                ty,
                                kind: p.kind,
                                default: def_str,
                                default_inputs,
                                is_vararg: p.is_vararg,
                            }
                        })
                        .collect();

                    let receiver = f.receiver.as_ref().map(|r| AbiReceiver {
                        mode: r.mode,
                        ty: lower(&r.ty),
                    });

                    symbols.push(ExportedSymbol::Function {
                        visibility: vis,
                        kind: FunctionKind::of(f),
                        is_compose: f.is_compose,
                        is_capture: f.is_capture,
                        is_operator: f.is_operator,
                        receiver,
                        context: f.context.as_ref().map(|c| lower(&c.ty)),
                        name: f.name.clone(),
                        type_parameters: abi_type_parameters(&f.type_parameters),
                        params,
                        return_type,
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
                        intrinsic: s.is_intrinsic(),
                        equality: equality.contains(s.name.as_str()),
                        drop_hook: hooks.drop.contains(s.name.as_str()),
                        clone_hook: hooks.clone.contains(s.name.as_str()),
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
                        equality: equality.contains(e.name.as_str()),
                        drop_hook: hooks.drop.contains(e.name.as_str()),
                        clone_hook: hooks.clone.contains(e.name.as_str()),
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

                Declaration::ExtensionProperty(p) => {
                    symbols.push(ExportedSymbol::ExtensionProperty {
                        visibility: vis,
                        type_parameters: abi_type_parameters(&p.type_parameters),
                        receiver: lower(&p.receiver),
                        name: p.name.clone(),
                        ty: lower(&p.ty),
                        writable: p.writable,
                    });
                }
            }
        }

        // Sort symbols deterministically by (kind, name) so declaration order does not perturb the hash
        symbols.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));

        let type_definitions = reachable_definitions(module, &symbols, definition);
        let abi_hash = Self::calculate_hash(&package, module, &symbols, &type_definitions);

        Ok(FileAbi {
            package,
            module: module.to_string(),
            symbols,
            type_definitions,
            abi_hash,
        })
    }

    /// The ABI hash of `symbols` and `definitions`: every symbol in order, then every reachable
    /// type definition.
    fn calculate_hash(
        package: &Option<String>,
        module: &str,
        symbols: &[ExportedSymbol],
        definitions: &[AbiTypeDefinition],
    ) -> ContentHash {
        let mut hasher = DeterministicHasher::new();

        if let Some(pkg) = package {
            hasher.write_str("package");
            hasher.write_str(pkg);
        }
        hasher.write_str("module");
        hasher.write_str(module);

        hasher.write_u64(symbols.len() as u64);

        for sym in symbols {
            sym.hash_into(&mut hasher);
        }

        hasher.write_u64(definitions.len() as u64);
        for definition in definitions {
            definition.hash_into(&mut hasher);
        }
        hasher.finish()
    }

    /// Formats the ABI hash as 64 hexadecimal digits.
    pub fn abi_hash_hex(&self) -> String {
        self.abi_hash.to_hex()
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

/// The initializer hash of the top-level value `declaration` (D8 of `hot-reload`): a hash of its
/// name, its declared type and its initializer expression that ignores positions, so moving or
/// reformatting the declaration keeps it, and editing the type or the initializer changes it.
/// One of the three parts of the key of the value's storage across reloads.
pub fn initializer_hash(declaration: &TopLevelVarDecl) -> ContentHash {
    let mut hasher = DeterministicHasher::new();
    hasher.write_str("initializer");
    hasher.write_str(&declaration.name);
    match &declaration.ty {
        Some(ty) => {
            hasher.write_u8(1);
            hasher.write_str(&ty.to_string());
        }
        None => hasher.write_u8(0),
    }
    hash_expr_span_free(&declaration.initializer, &mut hasher);
    hasher.finish()
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
        ExprKind::Index { target, indices } => {
            hasher.write_u8(16);
            hash_expr_span_free(target, hasher);
            hasher.write_u64(indices.len() as u64);
            for index in indices {
                hash_expr_span_free(index, hasher);
            }
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
        ExprKind::Closure {
            params,
            arrow,
            body,
        } => {
            hasher.write_u8(23);
            // `{ -> x }` and `{ x }` differ: only the second may have an implicit `it`.
            hasher.write_u8(u8::from(arrow.is_some()));
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

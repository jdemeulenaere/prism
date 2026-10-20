//! Structured, module-qualified types recorded in a file's ABI.

use crate::hasher::DeterministicHasher;
use prism_ast::{ParameterKind, Type as AstType, TypeKind, TypeMarker};
use std::fmt;

/// A built-in primitive type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PrimKind {
    Int,
    Float,
    Bool,
    Char,
    String,
    Nothing,
}

impl PrimKind {
    fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "Int" => PrimKind::Int,
            "Float" | "Double" => PrimKind::Float,
            "Bool" => PrimKind::Bool,
            "Char" => PrimKind::Char,
            "String" => PrimKind::String,
            "Nothing" => PrimKind::Nothing,
            _ => return None,
        })
    }

    fn as_str(self) -> &'static str {
        match self {
            PrimKind::Int => "Int",
            PrimKind::Float => "Double",
            PrimKind::Bool => "Bool",
            PrimKind::Char => "Char",
            PrimKind::String => "String",
            PrimKind::Nothing => "Nothing",
        }
    }
}

/// Whether a nominal type is a struct or an enum. Importers need it to type a value whose type is
/// defined in a module they do not import.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NominalKind {
    Struct,
    Enum,
}

/// What a struct or enum's declaration says about it, read from the AST alone: its kind, its
/// marker (`Copy`, `Clone` or none) and its marker parameters, the indices of the type parameters
/// that occur in its field or payload types and so condition the marker. Every reference to the
/// type in an ABI carries it, so an importer knows how values of the type may be duplicated
/// without importing its module.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NominalHeader {
    pub kind: NominalKind,
    pub marker: Option<TypeMarker>,
    pub marker_parameters: Vec<u32>,
}

/// A type parameter of an exported declaration: its name, recorded for diagnostics but not
/// hashed (type arguments are positional), and its bound.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AbiTypeParameter {
    pub name: String,
    pub bound: Option<TypeMarker>,
}

/// Feeds a declaration's type parameters into `hasher`: their number and bounds, not their names.
pub(crate) fn hash_type_parameters(
    parameters: &[AbiTypeParameter],
    hasher: &mut DeterministicHasher,
) {
    hasher.write_u64(parameters.len() as u64);
    for parameter in parameters {
        hash_marker(parameter.bound, hasher);
    }
}

/// Feeds a nominal type's marker parameters into `hasher`.
pub(crate) fn hash_marker_parameters(parameters: &[u32], hasher: &mut DeterministicHasher) {
    hasher.write_u64(parameters.len() as u64);
    for index in parameters {
        hasher.write_u64(u64::from(*index));
    }
}

/// Feeds a marker into `hasher` as one byte.
pub(crate) fn hash_marker(marker: Option<TypeMarker>, hasher: &mut DeterministicHasher) {
    hasher.write_u8(match marker {
        None => 0,
        Some(TypeMarker::Clone) => 1,
        Some(TypeMarker::Copy) => 2,
    });
}

/// A type in an exported signature, recorded as a tree so importers never re-parse it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AbiType {
    /// A built-in primitive.
    Prim(PrimKind),
    /// The unit type, written `Unit` or `()`.
    Unit,
    /// A struct or enum bound to its defining module (a root-relative path without extension),
    /// with the marker and marker parameters its declaration carries.
    Nominal {
        kind: NominalKind,
        marker: Option<TypeMarker>,
        marker_parameters: Vec<u32>,
        module: String,
        name: String,
        args: Vec<AbiType>,
    },
    /// The built-in `Range` type, written `Range` where no struct or enum of that name is in scope.
    Range,
    /// A type parameter of the declaration whose signature holds the type, by its position in
    /// the declaration's list; the name is for rendering.
    Parameter { index: u32, name: String },
    /// A type name bound to no module: unknown or ambiguous between imports.
    Unbound { name: String, args: Vec<AbiType> },
    /// A nullable type `T?`.
    Nullable(Box<AbiType>),
    /// A borrow `&T` or `&mut T`.
    Ref { mutable: bool, inner: Box<AbiType> },
    /// An owned heap indirection `Heap<T>`.
    Heap(Box<AbiType>),
    /// The built-in growable list `List<T>` (D2 of `collections`).
    List(Box<AbiType>),
    /// A function type, possibly composable, with each parameter's kind (plain, `escaping` or
    /// `attached`). With
    /// `receiver`, the first parameter is the receiver's borrow, `&T` for `T.() -> R` and
    /// `&mut T` for `mut T.() -> R` (D9 of `receiver-lambdas`).
    Fn {
        compose: bool,
        receiver: bool,
        params: Vec<AbiType>,
        kinds: Vec<ParameterKind>,
        ret: Box<AbiType>,
    },
}

/// Binds a nominal type name to the header and defining module of the struct or enum it denotes,
/// or returns `None` when the name is unknown or ambiguous.
pub type NameBinder<'a> = dyn Fn(&str) -> Option<(NominalHeader, String)> + 'a;

/// A binder for files with no imports: every name not declared in the file stays unbound.
pub fn no_imports(_name: &str) -> Option<(NominalHeader, String)> {
    None
}

impl AbiType {
    /// Lowers an AST type of a declaration whose type parameters are named `parameters`: a name
    /// without arguments that is one of them becomes [`AbiType::Parameter`], so a type parameter
    /// never binds to a struct of the same name. Every other nominal name is bound with `bind`.
    pub fn from_ast(ty: &AstType, parameters: &[&str], bind: &NameBinder) -> Self {
        let lower = |inner: &AstType| Self::from_ast(inner, parameters, bind);
        match &ty.kind {
            TypeKind::Named { name, type_args } => {
                if type_args.is_empty() {
                    if let Some(index) = parameters.iter().position(|p| p == name) {
                        return AbiType::Parameter {
                            index: index as u32,
                            name: name.clone(),
                        };
                    }
                    if let Some(prim) = PrimKind::from_name(name) {
                        return AbiType::Prim(prim);
                    }
                    if name == "Unit" {
                        return AbiType::Unit;
                    }
                }
                if name == "Heap" && type_args.len() == 1 {
                    return AbiType::Heap(Box::new(lower(&type_args[0])));
                }
                // `List` is a reserved built-in name (D2 of `collections`), never bound.
                if name == "List" && type_args.len() == 1 {
                    return AbiType::List(Box::new(lower(&type_args[0])));
                }
                let args = type_args.iter().map(lower).collect();
                match bind(name) {
                    Some((header, module)) => AbiType::Nominal {
                        kind: header.kind,
                        marker: header.marker,
                        marker_parameters: header.marker_parameters,
                        module,
                        name: name.clone(),
                        args,
                    },
                    // A user `Range` binds above; otherwise the name is the built-in type.
                    None if name == "Range" && args.is_empty() => AbiType::Range,
                    None => AbiType::Unbound {
                        name: name.clone(),
                        args,
                    },
                }
            }
            TypeKind::Nullable(inner) => AbiType::Nullable(Box::new(lower(inner))),
            TypeKind::Reference { mutable, inner } => AbiType::Ref {
                mutable: *mutable,
                inner: Box::new(lower(inner)),
            },
            TypeKind::Unit => AbiType::Unit,
            TypeKind::Function {
                is_compose,
                receiver,
                params,
                parameter_kinds,
                return_type,
            } => {
                let mut lowered = Vec::with_capacity(params.len() + 1);
                let mut flags = Vec::with_capacity(params.len() + 1);
                if let Some(receiver) = receiver {
                    lowered.push(AbiType::Ref {
                        mutable: receiver.mutable,
                        inner: Box::new(lower(&receiver.ty)),
                    });
                    flags.push(ParameterKind::Plain);
                }
                lowered.extend(params.iter().map(lower));
                flags.extend(parameter_kinds.iter().copied());
                AbiType::Fn {
                    compose: *is_compose,
                    receiver: receiver.is_some(),
                    params: lowered,
                    kinds: flags,
                    ret: Box::new(lower(return_type)),
                }
            }
        }
    }

    /// Feeds the type's structure into `hasher`, one tag byte per node.
    pub fn hash_into(&self, hasher: &mut DeterministicHasher) {
        match self {
            AbiType::Prim(prim) => {
                hasher.write_u8(1);
                hasher.write_str(prim.as_str());
            }
            AbiType::Unit => hasher.write_u8(2),
            AbiType::Nominal {
                kind,
                marker,
                marker_parameters,
                module,
                name,
                args,
            } => {
                hasher.write_u8(3);
                hasher.write_u8(match kind {
                    NominalKind::Struct => 0,
                    NominalKind::Enum => 1,
                });
                hash_marker(*marker, hasher);
                hash_marker_parameters(marker_parameters, hasher);
                hasher.write_str(module);
                hasher.write_str(name);
                Self::hash_list(args, hasher);
            }
            AbiType::Unbound { name, args } => {
                hasher.write_u8(4);
                hasher.write_str(name);
                Self::hash_list(args, hasher);
            }
            AbiType::Nullable(inner) => {
                hasher.write_u8(5);
                inner.hash_into(hasher);
            }
            AbiType::Ref { mutable, inner } => {
                hasher.write_u8(6);
                hasher.write_u8(u8::from(*mutable));
                inner.hash_into(hasher);
            }
            AbiType::Heap(inner) => {
                hasher.write_u8(7);
                inner.hash_into(hasher);
            }
            AbiType::Fn {
                compose,
                receiver,
                params,
                kinds,
                ret,
            } => {
                hasher.write_u8(8);
                hasher.write_u8(u8::from(*compose));
                // A plain function type keeps the bytes it always had.
                if *receiver {
                    hasher.write_u8(12);
                }
                Self::hash_list(params, hasher);
                // Plain and `escaping` keep the bytes they always had.
                for kind in kinds {
                    hasher.write_u8(match kind {
                        ParameterKind::Plain => 0,
                        ParameterKind::Escaping => 1,
                        ParameterKind::Attached => 2,
                    });
                }
                ret.hash_into(hasher);
            }
            AbiType::Range => hasher.write_u8(9),
            AbiType::List(inner) => {
                hasher.write_u8(11);
                inner.hash_into(hasher);
            }
            // By position only: renaming a type parameter changes nothing an importer can do.
            AbiType::Parameter { index, .. } => {
                hasher.write_u8(10);
                hasher.write_u64(u64::from(*index));
            }
        }
    }

    fn hash_list(types: &[AbiType], hasher: &mut DeterministicHasher) {
        hasher.write_u64(types.len() as u64);
        for ty in types {
            ty.hash_into(hasher);
        }
    }
}

/// The constructor of an extension's receiver type, which with the extension's module and name
/// identifies it (one extension per receiver constructor and name): `<module>.<Name>` for a
/// struct or enum, the name of a built-in type (`Int`, `Range`) or of an unbound one, and the
/// type's text otherwise. Type arguments are left out. The build cache matches MIR callees to
/// ABI symbols with it.
pub fn receiver_constructor(ty: &AbiType) -> String {
    match ty {
        AbiType::Nominal { module, name, .. } => format!("{}.{}", module, name),
        AbiType::Unbound { name, .. } => name.clone(),
        other => other.to_string(),
    }
}

impl fmt::Display for AbiType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AbiType::Prim(prim) => write!(f, "{}", prim.as_str()),
            AbiType::Unit => write!(f, "Unit"),
            AbiType::Range => write!(f, "Range"),
            AbiType::Parameter { name, .. } => write!(f, "{}", name),
            AbiType::Nominal { name, args, .. } | AbiType::Unbound { name, args } => {
                write!(f, "{}", name)?;
                if !args.is_empty() {
                    let args: Vec<String> = args.iter().map(ToString::to_string).collect();
                    write!(f, "<{}>", args.join(", "))?;
                }
                Ok(())
            }
            // `((Int) -> Int)?`: without parentheses the `?` would bind to the return type.
            // `(&T)?` would read as a reference to a `T?` without its parentheses.
            AbiType::Nullable(inner)
                if matches!(**inner, AbiType::Fn { .. } | AbiType::Ref { .. }) =>
            {
                write!(f, "({})?", inner)
            }
            AbiType::Nullable(inner) => write!(f, "{}?", inner),
            AbiType::Ref { mutable, inner } => {
                write!(f, "{}{}", if *mutable { "&mut " } else { "&" }, inner)
            }
            AbiType::Heap(inner) => write!(f, "Heap<{}>", inner),
            AbiType::List(inner) => write!(f, "List<{}>", inner),
            AbiType::Fn {
                compose,
                receiver,
                params,
                kinds,
                ret,
            } => {
                let mut receiver_text = String::new();
                let mut written = &params[..];
                if *receiver {
                    if let Some((AbiType::Ref { mutable, inner }, rest)) = params.split_first() {
                        receiver_text = format!("{}{}.", if *mutable { "mut " } else { "" }, inner);
                        written = rest;
                    }
                }
                let skipped = params.len() - written.len();
                let params: Vec<String> = written
                    .iter()
                    .enumerate()
                    .map(|(index, param)| {
                        let index = index + skipped;
                        let kind = kinds.get(index).copied().unwrap_or_default();
                        format!("{}{}", kind, param)
                    })
                    .collect();
                write!(
                    f,
                    "{}{}({}) -> {}",
                    if *compose { "compose " } else { "" },
                    receiver_text,
                    params.join(", "),
                    ret
                )
            }
        }
    }
}

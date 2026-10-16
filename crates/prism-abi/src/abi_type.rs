//! Structured, module-qualified types recorded in a file's ABI.

use crate::hasher::DeterministicHasher;
use prism_ast::{Type as AstType, TypeKind};
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
            PrimKind::Float => "Float",
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

/// A type in an exported signature, recorded as a tree so importers never re-parse it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AbiType {
    /// A built-in primitive.
    Prim(PrimKind),
    /// The unit type, written `Unit` or `()`.
    Unit,
    /// A struct or enum bound to its defining module (a root-relative path without extension).
    Nominal {
        kind: NominalKind,
        module: String,
        name: String,
        args: Vec<AbiType>,
    },
    /// A type name bound to no module: unknown, ambiguous between imports, or a type parameter.
    Unbound { name: String, args: Vec<AbiType> },
    /// A nullable type `T?`.
    Nullable(Box<AbiType>),
    /// A borrow `&T` or `&mut T`.
    Ref { mutable: bool, inner: Box<AbiType> },
    /// An owned heap indirection `Heap<T>`.
    Heap(Box<AbiType>),
    /// A function type, possibly composable.
    Fn {
        compose: bool,
        params: Vec<AbiType>,
        ret: Box<AbiType>,
    },
}

/// Binds a nominal type name to the kind and defining module of the struct or enum it denotes,
/// or returns `None` when the name is unknown or ambiguous.
pub type NameBinder<'a> = dyn Fn(&str) -> Option<(NominalKind, String)> + 'a;

/// A binder for files with no imports: every name not declared in the file stays unbound.
pub fn no_imports(_name: &str) -> Option<(NominalKind, String)> {
    None
}

impl AbiType {
    /// Lowers an AST type, binding each nominal name with `bind`.
    pub fn from_ast(ty: &AstType, bind: &NameBinder) -> Self {
        match &ty.kind {
            TypeKind::Named { name, type_args } => {
                if type_args.is_empty() {
                    if let Some(prim) = PrimKind::from_name(name) {
                        return AbiType::Prim(prim);
                    }
                    if name == "Unit" {
                        return AbiType::Unit;
                    }
                }
                if name == "Heap" && type_args.len() == 1 {
                    return AbiType::Heap(Box::new(Self::from_ast(&type_args[0], bind)));
                }
                let args = type_args.iter().map(|a| Self::from_ast(a, bind)).collect();
                match bind(name) {
                    Some((kind, module)) => AbiType::Nominal {
                        kind,
                        module,
                        name: name.clone(),
                        args,
                    },
                    None => AbiType::Unbound {
                        name: name.clone(),
                        args,
                    },
                }
            }
            TypeKind::Nullable(inner) => AbiType::Nullable(Box::new(Self::from_ast(inner, bind))),
            TypeKind::Reference { mutable, inner } => AbiType::Ref {
                mutable: *mutable,
                inner: Box::new(Self::from_ast(inner, bind)),
            },
            TypeKind::Unit => AbiType::Unit,
            TypeKind::Function {
                is_compose,
                params,
                return_type,
            } => AbiType::Fn {
                compose: *is_compose,
                params: params.iter().map(|p| Self::from_ast(p, bind)).collect(),
                ret: Box::new(Self::from_ast(return_type, bind)),
            },
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
                module,
                name,
                args,
            } => {
                hasher.write_u8(3);
                hasher.write_u8(match kind {
                    NominalKind::Struct => 0,
                    NominalKind::Enum => 1,
                });
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
                params,
                ret,
            } => {
                hasher.write_u8(8);
                hasher.write_u8(u8::from(*compose));
                Self::hash_list(params, hasher);
                ret.hash_into(hasher);
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

impl fmt::Display for AbiType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AbiType::Prim(prim) => write!(f, "{}", prim.as_str()),
            AbiType::Unit => write!(f, "Unit"),
            AbiType::Nominal { name, args, .. } | AbiType::Unbound { name, args } => {
                write!(f, "{}", name)?;
                if !args.is_empty() {
                    let args: Vec<String> = args.iter().map(ToString::to_string).collect();
                    write!(f, "<{}>", args.join(", "))?;
                }
                Ok(())
            }
            // `((Int) -> Int)?`: without parentheses the `?` would bind to the return type.
            AbiType::Nullable(inner) if matches!(**inner, AbiType::Fn { .. }) => {
                write!(f, "({})?", inner)
            }
            AbiType::Nullable(inner) => write!(f, "{}?", inner),
            AbiType::Ref { mutable, inner } => {
                write!(f, "{}{}", if *mutable { "&mut " } else { "&" }, inner)
            }
            AbiType::Heap(inner) => write!(f, "Heap<{}>", inner),
            AbiType::Fn {
                compose,
                params,
                ret,
            } => {
                let params: Vec<String> = params.iter().map(ToString::to_string).collect();
                write!(
                    f,
                    "{}({}) -> {}",
                    if *compose { "compose " } else { "" },
                    params.join(", "),
                    ret
                )
            }
        }
    }
}

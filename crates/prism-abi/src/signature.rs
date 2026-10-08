//! Surface ABI signature extraction and hashing engine.

use crate::error::AbiError;
use crate::hasher::DeterministicHasher;
use prism_ast::*;

/// Represents an exported symbol in a file's ABI boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportedSymbol {
    Function {
        visibility: Visibility,
        is_compose: bool,
        name: String,
        type_params: Vec<String>,
        params: Vec<(String, String)>, // (name, type_str)
        return_type: String,
        generic_body_hash: Option<u64>,
    },
    Struct {
        visibility: Visibility,
        name: String,
        type_params: Vec<String>,
        fields: Vec<(String, bool, String)>, // (name, is_var, type_str)
    },
    Enum {
        visibility: Visibility,
        name: String,
        type_params: Vec<String>,
        variants: Vec<(String, Vec<String>)>, // (name, field_types)
    },
    Var {
        visibility: Visibility,
        name: String,
        is_var: bool,
        ty: String,
    },
}

/// The exported ABI boundary of a Prism source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileAbi {
    /// Package name, if any (e.g. `demo.math`).
    pub package: Option<String>,
    /// All exported (`public` and `package`) symbols.
    pub symbols: Vec<ExportedSymbol>,
    /// 64-bit deterministic hash of the surface ABI.
    pub abi_hash: u64,
}

impl FileAbi {
    /// Computes the surface ABI and deterministic hash from a parsed [`SourceFile`].
    pub fn extract(file: &SourceFile) -> Result<Self, AbiError> {
        let package = file.package.as_ref().map(|p| p.package_string());
        let mut symbols = Vec::new();

        for decl in &file.declarations {
            // Default visibility is `private`. Private declarations are EXCLUDED from the ABI.
            let vis = decl.visibility();
            if vis == Visibility::Private {
                continue;
            }

            match decl {
                Declaration::Function(f) => {
                    // Check return type rule (Invariant 1)
                    let return_type_str = match &f.return_type {
                        Some(ty) => ty.to_string(),
                        None => {
                            // If it's a block body or no body, return type is Unit `()`
                            // If it's an expression body `= expr` without return type, it is an inferred return type
                            // which is forbidden for exported ABI (Invariant 1).
                            match &f.body {
                                Some(FunctionBody::Expr(_)) => {
                                    return Err(AbiError::MissingExplicitReturnType {
                                        function_name: f.name.clone(),
                                        visibility: vis.as_str().to_string(),
                                        span: f.span,
                                    });
                                }
                                _ => "()".to_string(),
                            }
                        }
                    };

                    let params: Vec<(String, String)> = f
                        .params
                        .iter()
                        .map(|p| (p.name.clone(), p.ty.to_string()))
                        .collect();

                    // For generic functions, compute monomorphization hash of the body (ADR-0003)
                    let generic_body_hash = if !f.type_params.is_empty() {
                        let mut h = DeterministicHasher::new();
                        if let Some(body) = &f.body {
                            match body {
                                FunctionBody::Expr(e) => h.write_str(&e.to_string()),
                                FunctionBody::Block(b) => h.write_str(&format!("{:?}", b)),
                            }
                        }
                        Some(h.finish())
                    } else {
                        None
                    };

                    symbols.push(ExportedSymbol::Function {
                        visibility: vis,
                        is_compose: f.is_compose,
                        name: f.name.clone(),
                        type_params: f.type_params.clone(),
                        params,
                        return_type: return_type_str,
                        generic_body_hash,
                    });
                }

                Declaration::Struct(s) => {
                    let fields: Vec<(String, bool, String)> = s
                        .fields
                        .iter()
                        .map(|fld| (fld.name.clone(), fld.is_var, fld.ty.to_string()))
                        .collect();

                    symbols.push(ExportedSymbol::Struct {
                        visibility: vis,
                        name: s.name.clone(),
                        type_params: s.type_params.clone(),
                        fields,
                    });
                }

                Declaration::Enum(e) => {
                    let variants: Vec<(String, Vec<String>)> = e
                        .variants
                        .iter()
                        .map(|v| {
                            let flds = v.fields.iter().map(|f| f.to_string()).collect();
                            (v.name.clone(), flds)
                        })
                        .collect();

                    symbols.push(ExportedSymbol::Enum {
                        visibility: vis,
                        name: e.name.clone(),
                        type_params: e.type_params.clone(),
                        variants,
                    });
                }

                Declaration::Var(v) => {
                    let ty_str = match &v.ty {
                        Some(t) => t.to_string(),
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
                        is_var: v.is_var,
                        ty: ty_str,
                    });
                }
            }
        }

        // Compute deterministic ABI hash
        let abi_hash = Self::calculate_hash(&package, &symbols);

        Ok(FileAbi {
            package,
            symbols,
            abi_hash,
        })
    }

    fn calculate_hash(package: &Option<String>, symbols: &[ExportedSymbol]) -> u64 {
        let mut hasher = DeterministicHasher::new();

        if let Some(pkg) = package {
            hasher.write_str("package");
            hasher.write_str(pkg);
        }

        for sym in symbols {
            match sym {
                ExportedSymbol::Function {
                    visibility,
                    is_compose,
                    name,
                    type_params,
                    params,
                    return_type,
                    generic_body_hash,
                } => {
                    hasher.write_str("fun");
                    hasher.write_str(visibility.as_str());
                    hasher.write_u8(if *is_compose { 1 } else { 0 });
                    hasher.write_str(name);
                    for tp in type_params {
                        hasher.write_str(tp);
                    }
                    for (pname, pty) in params {
                        hasher.write_str(pname);
                        hasher.write_str(pty);
                    }
                    hasher.write_str(return_type);
                    if let Some(gh) = generic_body_hash {
                        hasher.write_u64(*gh);
                    }
                }
                ExportedSymbol::Struct {
                    visibility,
                    name,
                    type_params,
                    fields,
                } => {
                    hasher.write_str("struct");
                    hasher.write_str(visibility.as_str());
                    hasher.write_str(name);
                    for tp in type_params {
                        hasher.write_str(tp);
                    }
                    for (fname, is_var, fty) in fields {
                        hasher.write_str(fname);
                        hasher.write_u8(if *is_var { 1 } else { 0 });
                        hasher.write_str(fty);
                    }
                }
                ExportedSymbol::Enum {
                    visibility,
                    name,
                    type_params,
                    variants,
                } => {
                    hasher.write_str("enum");
                    hasher.write_str(visibility.as_str());
                    hasher.write_str(name);
                    for tp in type_params {
                        hasher.write_str(tp);
                    }
                    for (vname, vflds) in variants {
                        hasher.write_str(vname);
                        for fty in vflds {
                            hasher.write_str(fty);
                        }
                    }
                }
                ExportedSymbol::Var {
                    visibility,
                    name,
                    is_var,
                    ty,
                } => {
                    hasher.write_str("var");
                    hasher.write_str(visibility.as_str());
                    hasher.write_str(name);
                    hasher.write_u8(if *is_var { 1 } else { 0 });
                    hasher.write_str(ty);
                }
            }
        }

        hasher.finish()
    }

    /// Formats the ABI hash as a 16-character hexadecimal string.
    pub fn abi_hash_hex(&self) -> String {
        format!("{:016x}", self.abi_hash)
    }
}

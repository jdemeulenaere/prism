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
        params: Vec<(String, String, Option<String>)>, // (name, type_str, default_value_expr)
        return_type: String,
        generic_body_hash: Option<u128>,
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

impl ExportedSymbol {
    fn sort_key(&self) -> (u8, &str) {
        match self {
            ExportedSymbol::Var { name, .. } => (0, name.as_str()),
            ExportedSymbol::Struct { name, .. } => (1, name.as_str()),
            ExportedSymbol::Enum { name, .. } => (2, name.as_str()),
            ExportedSymbol::Function { name, .. } => (3, name.as_str()),
        }
    }
}

/// The exported ABI boundary of a Prism source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileAbi {
    /// Package name, if any (e.g. `demo.math`).
    pub package: Option<String>,
    /// All exported (`public` and `package`) symbols.
    pub symbols: Vec<ExportedSymbol>,
    /// 128-bit deterministic hash of the surface ABI.
    pub abi_hash: u128,
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
                    // Check return type rule (single-file compilation unit invariant)
                    let return_type_str = match &f.return_type {
                        Some(ty) => ty.to_string(),
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
                                _ => "()".to_string(),
                            }
                        }
                    };

                    let params: Vec<(String, String, Option<String>)> = f
                        .params
                        .iter()
                        .map(|p| {
                            let def_str = p.default_value.as_ref().map(|d| d.to_string());
                            (p.name.clone(), p.ty.to_string(), def_str)
                        })
                        .collect();

                    // For generic functions, compute monomorphization hash of the body (`compilation-and-abi` spec)
                    // Hashing MUST be span-free to prevent comments/whitespace from altering ABI (hermetic builds invariant)
                    let generic_body_hash = if !f.type_params.is_empty() {
                        let mut h = DeterministicHasher::new();
                        if let Some(body) = &f.body {
                            hash_body_span_free(body, &mut h);
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

        // Sort symbols deterministically by (kind, name) so declaration order does not perturb the hash
        symbols.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));

        // Compute deterministic ABI hash
        let abi_hash = Self::calculate_hash(&package, &symbols);

        Ok(FileAbi {
            package,
            symbols,
            abi_hash,
        })
    }

    fn calculate_hash(package: &Option<String>, symbols: &[ExportedSymbol]) -> u128 {
        let mut hasher = DeterministicHasher::new();

        if let Some(pkg) = package {
            hasher.write_str("package");
            hasher.write_str(pkg);
        }

        hasher.write_u64(symbols.len() as u64);

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
                    hasher.write_u64(type_params.len() as u64);
                    for tp in type_params {
                        hasher.write_str(tp);
                    }
                    hasher.write_u64(params.len() as u64);
                    for (pname, pty, pdef) in params {
                        hasher.write_str(pname);
                        hasher.write_str(pty);
                        if let Some(def) = pdef {
                            hasher.write_u8(1);
                            hasher.write_str(def);
                        } else {
                            hasher.write_u8(0);
                        }
                    }
                    hasher.write_str(return_type);
                    if let Some(gh) = generic_body_hash {
                        hasher.write_u8(1);
                        hasher.write_u128(*gh);
                    } else {
                        hasher.write_u8(0);
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
                    hasher.write_u64(type_params.len() as u64);
                    for tp in type_params {
                        hasher.write_str(tp);
                    }
                    hasher.write_u64(fields.len() as u64);
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
                    hasher.write_u64(type_params.len() as u64);
                    for tp in type_params {
                        hasher.write_str(tp);
                    }
                    // Write variants count to eliminate canonicalization collisions
                    hasher.write_u64(variants.len() as u64);
                    for (vname, vflds) in variants {
                        hasher.write_str(vname);
                        // Write field count for this variant
                        hasher.write_u64(vflds.len() as u64);
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

    /// Formats the ABI hash as a 32-character hexadecimal string.
    pub fn abi_hash_hex(&self) -> String {
        format!("{:032x}", self.abi_hash)
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
            hasher.write_u64(b.stmts.len() as u64);
            for s in &b.stmts {
                hash_expr_span_free(s, hasher);
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
        ExprKind::Call { callee, args } => {
            hasher.write_u8(14);
            hash_expr_span_free(callee, hasher);
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
            hasher.write_u64(b.stmts.len() as u64);
            for s in &b.stmts {
                hash_expr_span_free(s, hasher);
            }
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
                        WhenPattern::Expr(e) => {
                            hasher.write_u8(1);
                            hash_expr_span_free(e, hasher);
                        }
                        WhenPattern::InRange { start, end } => {
                            hasher.write_u8(2);
                            hash_expr_span_free(start, hasher);
                            hash_expr_span_free(end, hasher);
                        }
                        WhenPattern::Else => {
                            hasher.write_u8(3);
                        }
                    }
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
        ExprKind::Assign { target, value } => {
            hasher.write_u8(21);
            hash_expr_span_free(target, hasher);
            hash_expr_span_free(value, hasher);
        }
        ExprKind::LocalVar {
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
        ExprKind::Closure { params, body } => {
            hasher.write_u8(23);
            hasher.write_u64(params.len() as u64);
            for p in params {
                hasher.write_str(p);
            }
            hash_expr_span_free(body, hasher);
        }
    }
}

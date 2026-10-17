//! Surface ABI signature extraction and hashing engine.

use crate::abi_type::{hash_marker, AbiType, NameBinder, NominalHeader, NominalKind};
use crate::error::AbiError;
use crate::hasher::DeterministicHasher;
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
        type_params: Vec<String>,
        params: Vec<(String, AbiType, Option<String>)>, // (name, type, default_value_expr)
        return_type: AbiType,
        generic_body_hash: Option<u128>,
    },
    Struct {
        visibility: Visibility,
        name: String,
        type_params: Vec<String>,
        marker: Option<TypeMarker>,
        fields: Vec<(String, bool, AbiType)>, // (name, is_var, type)
    },
    Enum {
        visibility: Visibility,
        name: String,
        type_params: Vec<String>,
        marker: Option<TypeMarker>,
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

/// The structs and enums a file declares, with their headers, read from its AST alone (Phase 1a).
/// With `exported_only`, private declarations are left out: those are what importers can bind.
pub fn declared_types(file: &SourceFile, exported_only: bool) -> HashMap<&str, NominalHeader> {
    file.declarations
        .iter()
        .filter(|decl| !exported_only || decl.visibility() != Visibility::Private)
        .filter_map(|decl| match decl {
            Declaration::Struct(s) => Some((
                s.name.as_str(),
                NominalHeader {
                    kind: NominalKind::Struct,
                    marker: s.marker,
                },
            )),
            Declaration::Enum(e) => Some((
                e.name.as_str(),
                NominalHeader {
                    kind: NominalKind::Enum,
                    marker: e.marker,
                },
            )),
            _ => None,
        })
        .collect()
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
    /// 128-bit deterministic hash of the surface ABI.
    pub abi_hash: u128,
}

impl FileAbi {
    /// Computes the surface ABI and deterministic hash from a parsed [`SourceFile`] whose module
    /// path is `module`.
    ///
    /// Each nominal type name in an exported signature is bound to its defining module: first to
    /// the file's own top-level structs and enums (private ones included), then with
    /// `bind_import`, which looks the name up among the types exported by the file's direct
    /// imports. Binding never reads a function body. A name neither binds stays unbound.
    pub fn extract(
        file: &SourceFile,
        module: &str,
        bind_import: &NameBinder,
    ) -> Result<Self, AbiError> {
        let package = file.package.as_ref().map(|p| p.package_string());
        let mut symbols = Vec::new();

        let own_types = declared_types(file, false);
        let bind = |name: &str| match own_types.get(name) {
            Some(header) => Some((*header, module.to_string())),
            None => bind_import(name),
        };
        let lower = |ty: &Type| AbiType::from_ast(ty, &bind);

        for decl in &file.declarations {
            // Default visibility is `private`. Private declarations are EXCLUDED from the ABI.
            let vis = decl.visibility();
            if vis == Visibility::Private {
                continue;
            }

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
                            let def_str = p.default_value.as_ref().map(|d| d.to_string());
                            (p.name.clone(), lower(&p.ty), def_str)
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

                    let receiver = f.receiver.as_ref().map(|r| AbiReceiver {
                        mode: r.mode,
                        ty: lower(&r.ty),
                    });

                    symbols.push(ExportedSymbol::Function {
                        visibility: vis,
                        is_compose: f.is_compose,
                        receiver,
                        name: f.name.clone(),
                        type_params: f.type_params.clone(),
                        params,
                        return_type,
                        generic_body_hash,
                    });
                }

                Declaration::Struct(s) => {
                    let fields: Vec<(String, bool, AbiType)> = s
                        .fields
                        .iter()
                        .map(|fld| (fld.name.clone(), fld.is_var, lower(&fld.ty)))
                        .collect();

                    symbols.push(ExportedSymbol::Struct {
                        visibility: vis,
                        name: s.name.clone(),
                        type_params: s.type_params.clone(),
                        marker: s.marker,
                        fields,
                    });
                }

                Declaration::Enum(e) => {
                    let variants: Vec<(String, Vec<AbiType>)> = e
                        .variants
                        .iter()
                        .map(|v| (v.name.clone(), v.fields.iter().map(lower).collect()))
                        .collect();

                    symbols.push(ExportedSymbol::Enum {
                        visibility: vis,
                        name: e.name.clone(),
                        type_params: e.type_params.clone(),
                        marker: e.marker,
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

        // Compute deterministic ABI hash
        let abi_hash = Self::calculate_hash(&package, module, &symbols);

        Ok(FileAbi {
            package,
            module: module.to_string(),
            symbols,
            abi_hash,
        })
    }

    fn calculate_hash(package: &Option<String>, module: &str, symbols: &[ExportedSymbol]) -> u128 {
        let mut hasher = DeterministicHasher::new();

        if let Some(pkg) = package {
            hasher.write_str("package");
            hasher.write_str(pkg);
        }
        hasher.write_str("module");
        hasher.write_str(module);

        hasher.write_u64(symbols.len() as u64);

        for sym in symbols {
            match sym {
                ExportedSymbol::Function {
                    visibility,
                    is_compose,
                    receiver,
                    name,
                    type_params,
                    params,
                    return_type,
                    generic_body_hash,
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
                            receiver.ty.hash_into(&mut hasher);
                        }
                        None => hasher.write_u8(0),
                    }
                    hasher.write_str(name);
                    hasher.write_u64(type_params.len() as u64);
                    for tp in type_params {
                        hasher.write_str(tp);
                    }
                    hasher.write_u64(params.len() as u64);
                    for (pname, pty, pdef) in params {
                        hasher.write_str(pname);
                        pty.hash_into(&mut hasher);
                        if let Some(def) = pdef {
                            hasher.write_u8(1);
                            hasher.write_str(def);
                        } else {
                            hasher.write_u8(0);
                        }
                    }
                    return_type.hash_into(&mut hasher);
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
                    marker,
                    fields,
                } => {
                    hasher.write_str("struct");
                    hasher.write_str(visibility.as_str());
                    hasher.write_str(name);
                    hash_marker(*marker, &mut hasher);
                    hasher.write_u64(type_params.len() as u64);
                    for tp in type_params {
                        hasher.write_str(tp);
                    }
                    hasher.write_u64(fields.len() as u64);
                    for (fname, is_var, fty) in fields {
                        hasher.write_str(fname);
                        hasher.write_u8(if *is_var { 1 } else { 0 });
                        fty.hash_into(&mut hasher);
                    }
                }
                ExportedSymbol::Enum {
                    visibility,
                    name,
                    type_params,
                    marker,
                    variants,
                } => {
                    hasher.write_str("enum");
                    hasher.write_str(visibility.as_str());
                    hasher.write_str(name);
                    hash_marker(*marker, &mut hasher);
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
                            fty.hash_into(&mut hasher);
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
                    ty.hash_into(&mut hasher);
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
                        WhenPattern::Expr(e) => {
                            hasher.write_u8(1);
                            hash_expr_span_free(e, hasher);
                        }
                        WhenPattern::InRange {
                            start,
                            end,
                            end_inclusive,
                        } => {
                            hasher.write_u8(2);
                            hash_expr_span_free(start, hasher);
                            hash_expr_span_free(end, hasher);
                            hasher.write_u8(if *end_inclusive { 1 } else { 0 });
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
        ExprKind::This => hasher.write_u8(30),
        ExprKind::Break => hasher.write_u8(28),
        ExprKind::Continue => hasher.write_u8(29),
        ExprKind::Closure { params, body } => {
            hasher.write_u8(23);
            hasher.write_u64(params.len() as u64);
            for p in params {
                hasher.write_str(p);
            }
            hash_block_span_free(body, hasher);
        }
    }
}

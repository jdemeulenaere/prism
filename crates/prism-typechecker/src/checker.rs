//! Main TypeChecker implementation: Pass 1 declaration registration and Pass 2 body checking.

use crate::error::TypeError;
use crate::scope::{is_type_symbol, ImportLookup, Mutability, ScopeStack, Symbol, SymbolKind};
use crate::types::{NominalId, Type, TypeDef, BUILTIN_MODULE};
use prism_abi::{AbiType, ExportedSymbol, FileAbi};
use prism_ast::decl::{Declaration, FunctionBody, FunctionDecl, TopLevelVarDecl};
use prism_ast::expr::{
    BinaryOp, BlockExpr, CallArg, Expr, ExprKind, Literal, UnaryOp, WhenBranch, WhenPattern,
};
use prism_ast::module::SourceFile;
use prism_ast::{Span, Visibility};
use std::collections::{HashMap, HashSet};

/// Helper to peel reference (`&T`, `&mut T`) and `Heap<T>` indirection layers to access the underlying nominal type.
pub fn peel_indirection(mut ty: &Type) -> &Type {
    loop {
        match ty {
            Type::Reference { inner, .. } => ty = inner,
            Type::Heap(inner) => ty = inner,
            _ => return ty,
        }
    }
}

/// Every type recorded in an exported symbol's signature or definition.
fn abi_symbol_types(sym: &ExportedSymbol) -> Vec<&AbiType> {
    match sym {
        ExportedSymbol::Function {
            params,
            return_type,
            ..
        } => params
            .iter()
            .map(|(_, ty, _)| ty)
            .chain(std::iter::once(return_type))
            .collect(),
        ExportedSymbol::Struct { fields, .. } => fields.iter().map(|(_, _, ty)| ty).collect(),
        ExportedSymbol::Enum { variants, .. } => variants.iter().flat_map(|(_, p)| p).collect(),
        ExportedSymbol::Var { ty, .. } => vec![ty],
    }
}

/// Peels reference (`&T`, `&mut T`) layers, keeping `Heap<T>`.
fn peel_references(mut ty: &Type) -> &Type {
    while let Type::Reference { inner, .. } = ty {
        ty = inner;
    }
    ty
}

/// Where the expected type of the value being checked comes from. It picks the code of an
/// assignability mismatch: a returned value reports `E4005`, an argument `E4004`, and any other
/// expectation (annotation, assignment, operand, default value) `E4002`.
///
/// The origin is inherited by `if`/`when` branches and a block's final expression, which produce
/// the checked value, and reset to `Value` by every other nested expectation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Origin {
    Value,
    Return,
    Argument,
}

/// Inference state of an unannotated private top-level declaration, whose type comes from its
/// body or initializer. Such a declaration is inferred on demand, the first time it is used, so
/// its type does not depend on declaration order (D5 of `typechecker-soundness`).
#[derive(Debug, Clone)]
enum Inference {
    /// Not inferred yet.
    Pending(Box<PendingDecl>),
    /// Being inferred: reaching it again means its type depends on itself.
    InProgress,
    /// Inferred; its symbol holds the type.
    Done,
}

/// A declaration waiting for on-demand inference.
#[derive(Debug, Clone)]
enum PendingDecl {
    Function(FunctionDecl),
    Var(TopLevelVarDecl),
}

/// The state of the body being checked, swapped out while another declaration is inferred on
/// demand.
struct BodyContext {
    locals: Vec<HashMap<String, Symbol>>,
    in_compose_stack: Vec<bool>,
    current_expected_return: Option<Type>,
    in_function: bool,
    origin: Origin,
}

/// How the value of a `when` is used.
#[derive(Debug, Clone, Copy)]
enum WhenUse<'a> {
    /// The value is discarded: no `else` or common branch type is needed.
    Statement,
    /// The value is used, and its type is the join of the branch types.
    Infer,
    /// The value is used, and every branch is checked against the expected type.
    Check(&'a Type),
}

/// Static type checker and semantic analyzer for Prism source modules.
pub struct TypeChecker {
    /// Lexical scope environment.
    pub env: ScopeStack,
    /// Canonical package path of the current file (e.g. `Some("app.ui")`).
    pub current_package: Option<String>,
    /// Module path of the current file: its root-relative path without extension (`app/main`).
    /// Nominal types declared in this file are identified by it.
    pub module_path: String,
    /// Definitions of every nominal type known to this file, keyed by identity.
    types: HashMap<NominalId, TypeDef>,
    /// Origin of the expected type currently being checked against.
    origin: Origin,
    /// Unannotated private top-level declarations, by name, and how far their inference is.
    inference: HashMap<String, Inference>,
    /// Declarations already reported as depending on themselves (`E4013`).
    recursive_inference: HashSet<String>,
    /// Module paths of the file's direct imports, whose ABIs this checker has ingested.
    direct_imports: HashSet<String>,
    /// Every struct and enum named by an ingested ABI, by bare name. Some are defined in modules
    /// the file does not import, which can be named in an `E4014` hint.
    seen_nominals: HashMap<String, Vec<NominalId>>,
    /// Accumulated semantic errors.
    pub errors: Vec<TypeError>,
    /// Ambient composable calling context stack (tracks `is_compose` across functions and closures).
    pub in_compose_stack: Vec<bool>,
    /// Expected return type of the currently enclosing function.
    pub current_expected_return: Option<Type>,
    /// Private functions with omitted return types needing inference in Pass 2.
    pub unannotated_private_fns: Vec<String>,
    /// Whether the checker is currently evaluating within a function body.
    pub in_function: bool,
    /// The enum that owns each module-level bare variant name (`Red` for `Color.Red`).
    variant_owners: HashMap<String, String>,
    /// Bare variant names declared by more than one enum. They are not bound at module level and
    /// must be qualified (`B.None`), except as a `when` pattern over one of those enums.
    ambiguous_variants: HashSet<String>,
}

impl TypeChecker {
    /// Creates a new `TypeChecker` for the file with module path `module_path` (its root-relative
    /// path without extension, such as `app/main`) in the given package.
    pub fn new(current_package: Option<String>, module_path: impl Into<String>) -> Self {
        let mut types = HashMap::new();
        types.insert(
            NominalId::builtin_range(),
            TypeDef::Struct {
                fields: vec![
                    ("start".to_string(), false, Type::Int),
                    ("end".to_string(), false, Type::Int),
                ],
            },
        );
        Self {
            env: ScopeStack::new(),
            current_package,
            module_path: module_path.into(),
            types,
            origin: Origin::Value,
            inference: HashMap::new(),
            recursive_inference: HashSet::new(),
            direct_imports: HashSet::new(),
            seen_nominals: HashMap::new(),
            errors: Vec::new(),
            in_compose_stack: vec![false],
            current_expected_return: None,
            unannotated_private_fns: Vec::new(),
            in_function: false,
            variant_owners: HashMap::new(),
            ambiguous_variants: HashSet::new(),
        }
    }

    /// Returns `true` if the current evaluation context is composable (`compose fun` or composable closure).
    pub fn is_in_compose(&self) -> bool {
        *self.in_compose_stack.last().unwrap_or(&false)
    }

    /// Pushes a composable context flag when entering a function or closure.
    pub fn push_compose_context(&mut self, is_compose: bool) {
        self.in_compose_stack.push(is_compose);
    }

    /// Pops the composable context flag when exiting a function or closure.
    pub fn pop_compose_context(&mut self) {
        if self.in_compose_stack.len() > 1 {
            self.in_compose_stack.pop();
        }
    }

    /// The definition of the nominal type `id`, if this file knows it.
    pub fn type_def(&self, id: &NominalId) -> Option<&TypeDef> {
        self.types.get(id)
    }

    /// The fields of struct `id`, if its definition is known.
    fn struct_fields(&self, id: &NominalId) -> Option<Vec<(String, bool, Type)>> {
        match self.types.get(id) {
            Some(TypeDef::Struct { fields }) => Some(fields.clone()),
            _ => None,
        }
    }

    /// The variants of enum `id`, if its definition is known.
    fn enum_variants(&self, id: &NominalId) -> Option<Vec<(String, Vec<Type>)>> {
        match self.types.get(id) {
            Some(TypeDef::Enum { variants }) => Some(variants.clone()),
            _ => None,
        }
    }

    /// Binds a type name in the current scope: a declared or imported struct or enum gives its
    /// identity, and `Range` with no user declaration gives the built-in range type.
    /// An ambiguous imported type name gives the error type (`E4012` is reported by
    /// [`Self::check_annotation_names`]).
    fn bind_type_name(&self, name: &str) -> Option<Type> {
        if let Some(sym) = self.env.lookup_type(name) {
            return Some(sym.ty());
        }
        match self.env.lookup_import(name, is_type_symbol) {
            ImportLookup::Ambiguous(_) => Some(Type::Error),
            _ if name == "Range" => Some(Type::Struct(NominalId::builtin_range())),
            // Reported as `E4014` by `check_annotation_names`; `Error` avoids a follow-on `E4006`.
            _ if self.unimported_type_named(name).is_some() => Some(Type::Error),
            _ => None,
        }
    }

    /// Lowers an annotation to a semantic type, binding each nominal name to the struct or enum
    /// it denotes where the annotation appears. Names with no declaration stay opaque.
    pub fn lower_type(&self, ast_ty: &prism_ast::types::Type) -> Type {
        Type::from_ast(ast_ty).bind_names(&|name| self.bind_type_name(name))
    }

    /// Resolves an identifier, reporting `E4001` when it is unknown and `E4007` when it names a
    /// `package` symbol of another package.
    fn resolve_ident(&mut self, name: &str, span: Span) -> Option<Symbol> {
        match self.env.resolve(name, span) {
            Ok(sym) => {
                let sym = sym.clone();
                self.check_visibility(&sym, span);
                Some(sym)
            }
            Err(TypeError::UnresolvedSymbol { .. })
                if self.unimported_type_named(name).is_some() =>
            {
                // The name is a type that reached this file through an import's signature: to
                // construct it or name its variants, the file must import its module.
                let id = self.unimported_type_named(name).expect("checked above");
                self.errors.push(TypeError::MemberOfUnimportedType {
                    type_name: id.name,
                    module: id.module,
                    span,
                });
                None
            }
            Err(err) => {
                self.errors.push(err);
                None
            }
        }
    }

    /// Records every struct and enum `ty` names, so `E4014` can name types that reached this
    /// file through an import's signature.
    fn record_nominals(&mut self, ty: &AbiType) {
        match ty {
            AbiType::Nominal {
                module, name, args, ..
            } => {
                let id = NominalId::new(module.clone(), name.clone());
                let ids = self.seen_nominals.entry(name.clone()).or_default();
                if !ids.contains(&id) {
                    ids.push(id);
                }
                args.iter().for_each(|a| self.record_nominals(a));
            }
            AbiType::Unbound { args, .. } => args.iter().for_each(|a| self.record_nominals(a)),
            AbiType::Nullable(inner) | AbiType::Heap(inner) | AbiType::Ref { inner, .. } => {
                self.record_nominals(inner)
            }
            AbiType::Fn { params, ret, .. } => {
                params.iter().for_each(|p| self.record_nominals(p));
                self.record_nominals(ret);
            }
            AbiType::Prim(_) | AbiType::Unit => {}
        }
    }

    /// Whether the module defining `id` is out of this file's reach: neither the file itself,
    /// the built-in module, nor a direct import (D9 of `typechecker-soundness`).
    fn is_unimported(&self, id: &NominalId) -> bool {
        id.module != self.module_path
            && id.module != BUILTIN_MODULE
            && !self.direct_imports.contains(&id.module)
    }

    /// The single type named `name` that reached this file through an import's signature but is
    /// defined in a module the file does not import.
    fn unimported_type_named(&self, name: &str) -> Option<NominalId> {
        let ids: Vec<&NominalId> = self
            .seen_nominals
            .get(name)?
            .iter()
            .filter(|id| self.is_unimported(id))
            .collect();
        match ids.as_slice() {
            [id] => Some((*id).clone()),
            _ => None,
        }
    }

    /// Reports `E4014` and returns `true` if `ty` is a struct or enum whose defining module is
    /// not a direct import, so its members cannot be used.
    fn reject_unimported_members(&mut self, ty: &Type, span: Span) -> bool {
        match ty {
            Type::Struct(id) | Type::Enum(id) if self.is_unimported(id) => {
                self.errors.push(TypeError::MemberOfUnimportedType {
                    type_name: id.name.clone(),
                    module: id.module.clone(),
                    span,
                });
                true
            }
            _ => false,
        }
    }

    /// Reports `E4007` if `sym` has `package` visibility and is defined in another package
    /// (single-file compilation unit invariant). Returns whether the symbol is visible.
    fn check_visibility(&mut self, sym: &Symbol, span: Span) -> bool {
        if sym.visibility() == Visibility::Package {
            let def_pkg = sym.defining_package().unwrap_or("");
            let curr_pkg = self.current_package.as_deref().unwrap_or("");
            if def_pkg != curr_pkg {
                self.errors.push(TypeError::VisibilityViolation {
                    symbol: sym.name.clone(),
                    defining_package: def_pkg.to_string(),
                    accessing_package: curr_pkg.to_string(),
                    span,
                });
                return false;
            }
        }
        true
    }

    /// Reports `E4012` for every nominal type name in an annotation that several imports export,
    /// and `E4007` for every one that has `package`
    /// visibility in another package.
    fn check_annotation_names(&mut self, ast_ty: &prism_ast::types::Type) {
        use prism_ast::types::TypeKind;
        match &ast_ty.kind {
            TypeKind::Named { name, type_args } => {
                if let Some(sym) = self.env.lookup_type(name).cloned() {
                    self.check_visibility(&sym, ast_ty.span);
                } else if let ImportLookup::Ambiguous(modules) =
                    self.env.lookup_import(name, is_type_symbol)
                {
                    self.errors.push(TypeError::AmbiguousImport {
                        name: name.clone(),
                        modules,
                        span: ast_ty.span,
                    });
                } else if let Some(id) = self.unimported_type_named(name) {
                    // The name is a type this file only knows through an import's signature:
                    // naming it requires importing its module, like using its members.
                    self.errors.push(TypeError::MemberOfUnimportedType {
                        type_name: id.name,
                        module: id.module,
                        span: ast_ty.span,
                    });
                }
                for arg in type_args {
                    self.check_annotation_names(arg);
                }
            }
            TypeKind::Nullable(inner) | TypeKind::Reference { inner, .. } => {
                self.check_annotation_names(inner)
            }
            TypeKind::Function {
                params,
                return_type,
                ..
            } => {
                for param in params {
                    self.check_annotation_names(param);
                }
                self.check_annotation_names(return_type);
            }
            TypeKind::Unit => {}
        }
    }

    /// Returns the struct or enum symbol `expr` names, if `expr` is an identifier whose innermost
    /// binding is a type declaration rather than a value.
    fn type_name_symbol(&self, expr: &Expr) -> Option<Symbol> {
        if let ExprKind::Ident(name) = &expr.kind {
            if let Some(sym) = self.env.lookup(name) {
                if matches!(
                    sym.kind,
                    SymbolKind::Struct { .. } | SymbolKind::Enum { .. }
                ) {
                    return Some(sym.clone());
                }
            }
        }
        None
    }

    /// Types `member` on `target` when `target` names a type: an enum name gives access to its
    /// variants, while a struct name has no members (`E4006`). Returns `None` when `target` is
    /// not a type name.
    fn type_name_member(
        &mut self,
        target: &Expr,
        member: &str,
        span: Span,
    ) -> Option<(Type, Option<bool>)> {
        let sym = self.type_name_symbol(target)?;
        if !self.check_visibility(&sym, target.span) {
            return Some((Type::Error, None));
        }
        let ty = sym.ty();
        if let SymbolKind::Struct { .. } = sym.kind {
            self.errors.push(TypeError::UnknownField {
                target_type: ty,
                field_name: member.to_string(),
                span,
            });
            return Some((Type::Error, None));
        }
        Some(self.member_type(&ty, member, false, span))
    }

    /// Binds the bare name of a variant of `enum_name` at module level.
    ///
    /// Variants are namespaced by their enum, so when two enums declare the same variant name the
    /// bare name is unbound and recorded as ambiguous rather than reported as a duplicate. Any
    /// other clash is a duplicate symbol, returned as `E4010`.
    #[allow(clippy::result_large_err)]
    fn define_variant(&mut self, enum_name: &str, symbol: Symbol) -> Result<(), TypeError> {
        let name = symbol.name.clone();
        if self.ambiguous_variants.contains(&name) {
            return Ok(());
        }
        if let Some(owner) = self.variant_owners.get(&name) {
            if owner != enum_name {
                self.env.remove(&name);
                self.variant_owners.remove(&name);
                self.ambiguous_variants.insert(name);
                return Ok(());
            }
        }
        self.env.define(symbol)?;
        self.variant_owners.insert(name, enum_name.to_string());
        Ok(())
    }

    /// Ingests exported symbols from an imported file's Phase 1 FileAbi.
    ///
    /// Each exported struct and enum is identified by the exporter's module path. Types in the
    /// exported signatures are already bound to their defining modules by Phase 1, so they are
    /// converted as they are.
    pub fn ingest_file_abi(&mut self, abi: &FileAbi) {
        self.direct_imports.insert(abi.module.clone());
        for ty in abi.symbols.iter().flat_map(abi_symbol_types) {
            self.record_nominals(ty);
        }
        for sym in &abi.symbols {
            let symbol = match sym {
                ExportedSymbol::Struct {
                    visibility, name, ..
                } => Symbol::struct_decl(
                    NominalId::new(abi.module.clone(), name.clone()),
                    *visibility,
                    abi.package.clone(),
                    Span::dummy(),
                ),
                ExportedSymbol::Enum {
                    visibility, name, ..
                } => Symbol::enum_decl(
                    NominalId::new(abi.module.clone(), name.clone()),
                    *visibility,
                    abi.package.clone(),
                    Span::dummy(),
                ),
                _ => continue,
            };
            self.env.define_import(&abi.module, symbol);
        }
        for sym in &abi.symbols {
            match sym {
                ExportedSymbol::Function {
                    visibility,
                    is_compose,
                    name,
                    params,
                    return_type,
                    ..
                } => {
                    let param_types = params
                        .iter()
                        .map(|(_, ty_str, _)| Type::from_abi(ty_str))
                        .collect();
                    let param_names = params.iter().map(|(n, _, _)| n.clone()).collect();
                    let has_default = params.iter().map(|(_, _, d)| d.is_some()).collect();
                    let fn_ty = Type::Fn {
                        is_compose: *is_compose,
                        params: param_types,
                        ret: Box::new(Type::from_abi(return_type)),
                    };
                    let symbol = Symbol::function_with_defaults(
                        name.clone(),
                        fn_ty,
                        param_names,
                        has_default,
                        *visibility,
                        abi.package.clone(),
                        Span::dummy(),
                    );
                    self.env.define_import(&abi.module, symbol);
                }
                ExportedSymbol::Struct { name, fields, .. } => {
                    let fields = fields
                        .iter()
                        .map(|(f_name, is_var, f_ty)| {
                            (f_name.clone(), *is_var, Type::from_abi(f_ty))
                        })
                        .collect();
                    let id = NominalId::new(abi.module.clone(), name.clone());
                    self.types.entry(id).or_insert(TypeDef::Struct { fields });
                }
                ExportedSymbol::Enum {
                    visibility,
                    name,
                    variants,
                    ..
                } => {
                    let variant_types: Vec<(String, Vec<Type>)> = variants
                        .iter()
                        .map(|(v_name, f_types)| {
                            (v_name.clone(), f_types.iter().map(Type::from_abi).collect())
                        })
                        .collect();
                    let id = NominalId::new(abi.module.clone(), name.clone());
                    let enum_ty = Type::Enum(id.clone());
                    self.types.entry(id).or_insert(TypeDef::Enum {
                        variants: variant_types.clone(),
                    });

                    for (v_name, f_types) in &variant_types {
                        let v_sym = if f_types.is_empty() {
                            Symbol::top_level_variable(
                                v_name.clone(),
                                enum_ty.clone(),
                                Mutability::Val,
                                *visibility,
                                abi.package.clone(),
                                Span::dummy(),
                            )
                        } else {
                            Symbol::function(
                                v_name.clone(),
                                Type::Fn {
                                    is_compose: false,
                                    params: f_types.clone(),
                                    ret: Box::new(enum_ty.clone()),
                                },
                                *visibility,
                                abi.package.clone(),
                                Span::dummy(),
                            )
                        };
                        self.define_variant(name, v_sym).ok();
                    }
                }
                ExportedSymbol::Var {
                    visibility,
                    name,
                    is_var,
                    ty,
                } => {
                    let mutability = if *is_var {
                        Mutability::Var
                    } else {
                        Mutability::Val
                    };
                    let symbol = Symbol::top_level_variable(
                        name.clone(),
                        Type::from_abi(ty),
                        mutability,
                        *visibility,
                        abi.package.clone(),
                        Span::dummy(),
                    );
                    self.env.define_import(&abi.module, symbol);
                }
            }
        }
    }

    /// Pass 1: Registers all top-level symbols (functions, structs, enums, variables) in module scope.
    ///
    /// Enables forward references so functions can call sibling functions declared later in the file.
    /// Every struct and enum name is declared before any annotation is lowered, so field and
    /// signature types may name types declared later in the file.
    pub fn register_module_declarations(&mut self, source_file: &SourceFile) {
        for decl in &source_file.declarations {
            let symbol = match decl {
                Declaration::Struct(s) => {
                    let mut seen_fields = HashSet::new();
                    for f in &s.fields {
                        if !seen_fields.insert(f.name.clone()) {
                            self.errors.push(TypeError::DuplicateSymbol {
                                name: format!(
                                    "duplicate field '{}' in struct '{}'",
                                    f.name, s.name
                                ),
                                span: f.span,
                            });
                        }
                    }
                    let id = NominalId::new(self.module_path.clone(), s.name.clone());
                    Symbol::struct_decl(id, s.visibility, self.current_package.clone(), s.span)
                }
                Declaration::Enum(e) => {
                    let mut seen_variants = HashSet::new();
                    for v in &e.variants {
                        if !seen_variants.insert(v.name.clone()) {
                            self.errors.push(TypeError::DuplicateSymbol {
                                name: format!(
                                    "duplicate variant '{}' in enum '{}'",
                                    v.name, e.name
                                ),
                                span: v.span,
                            });
                        }
                    }
                    let id = NominalId::new(self.module_path.clone(), e.name.clone());
                    Symbol::enum_decl(id, e.visibility, self.current_package.clone(), e.span)
                }
                _ => continue,
            };
            self.define_top_level(symbol);
        }

        for decl in &source_file.declarations {
            match decl {
                Declaration::Struct(s) => {
                    for f in &s.fields {
                        self.check_annotation_names(&f.ty);
                    }
                    let fields = s
                        .fields
                        .iter()
                        .map(|f| (f.name.clone(), f.is_var, self.lower_type(&f.ty)))
                        .collect();
                    let id = NominalId::new(self.module_path.clone(), s.name.clone());
                    self.types.entry(id).or_insert(TypeDef::Struct { fields });
                }
                Declaration::Enum(e) => {
                    for payload in e.variants.iter().flat_map(|v| &v.fields) {
                        self.check_annotation_names(payload);
                    }
                    let variants: Vec<(String, Vec<Type>)> = e
                        .variants
                        .iter()
                        .map(|v| {
                            (
                                v.name.clone(),
                                v.fields.iter().map(|f| self.lower_type(f)).collect(),
                            )
                        })
                        .collect();
                    let id = NominalId::new(self.module_path.clone(), e.name.clone());
                    let enum_ty = Type::Enum(id.clone());
                    self.types.entry(id).or_insert(TypeDef::Enum {
                        variants: variants.clone(),
                    });

                    for (v_name, f_types) in &variants {
                        let v_sym = if f_types.is_empty() {
                            Symbol::top_level_variable(
                                v_name.clone(),
                                enum_ty.clone(),
                                Mutability::Val,
                                e.visibility,
                                self.current_package.clone(),
                                e.span,
                            )
                        } else {
                            Symbol::function_with_defaults(
                                v_name.clone(),
                                Type::Fn {
                                    is_compose: false,
                                    params: f_types.clone(),
                                    ret: Box::new(enum_ty.clone()),
                                },
                                vec![String::new(); f_types.len()],
                                vec![false; f_types.len()],
                                e.visibility,
                                self.current_package.clone(),
                                e.span,
                            )
                        };
                        if let Err(err) = self.define_variant(&e.name, v_sym) {
                            self.errors.push(err);
                        }
                    }
                }
                Declaration::Function(f) => {
                    self.register_function_decl(f);
                }
                Declaration::Var(v) => {
                    let ty = if let Some(annot) = &v.ty {
                        self.check_annotation_names(annot);
                        self.lower_type(annot)
                    } else if v.visibility != Visibility::Private {
                        self.errors.push(TypeError::TypeMismatch {
                            expected: Type::Error,
                            found: Type::Error,
                            span: v.span,
                        });
                        Type::Error
                    } else {
                        Type::Error
                    };

                    let mutability = if v.is_var {
                        Mutability::Var
                    } else {
                        Mutability::Val
                    };
                    let sym = Symbol::top_level_variable(
                        v.name.clone(),
                        ty,
                        mutability,
                        v.visibility,
                        self.current_package.clone(),
                        v.span,
                    );
                    self.define_top_level(sym);
                }
            }
        }
    }

    /// Defines a top-level declaration in module scope. A declaration with the same name as an
    /// imported symbol shadows the import: lookups check module scope before the import layer.
    fn define_top_level(&mut self, symbol: Symbol) {
        if let Err(err) = self.env.define(symbol) {
            self.errors.push(err);
        }
    }

    fn register_function_decl(&mut self, f: &FunctionDecl) {
        for annot in f.params.iter().map(|p| &p.ty).chain(&f.return_type) {
            self.check_annotation_names(annot);
        }
        let params: Vec<Type> = f.params.iter().map(|p| self.lower_type(&p.ty)).collect();
        let param_names: Vec<String> = f.params.iter().map(|p| p.name.clone()).collect();
        let has_default: Vec<bool> = f.params.iter().map(|p| p.default_value.is_some()).collect();

        let ret = if let Some(annot) = &f.return_type {
            self.lower_type(annot)
        } else if f.visibility != Visibility::Private {
            self.errors.push(TypeError::MissingReturnType {
                function_name: f.name.clone(),
                span: f.span,
            });
            Type::Error
        } else if matches!(f.body, Some(FunctionBody::Block(_)) | None) {
            Type::Unit
        } else {
            self.unannotated_private_fns.push(f.name.clone());
            Type::Error
        };

        let fn_ty = Type::Fn {
            is_compose: f.is_compose,
            params,
            ret: Box::new(ret),
        };

        let sym = Symbol::function_with_defaults(
            f.name.clone(),
            fn_ty,
            param_names,
            has_default,
            f.visibility,
            self.current_package.clone(),
            f.span,
        );
        self.define_top_level(sym);
    }

    /// Pass 2: Typechecks the bodies of all top-level declarations in the module.
    pub fn check_module(&mut self, source_file: &SourceFile) {
        self.register_module_declarations(source_file);

        // Unannotated private declarations get their type from their body, inferred on demand.
        let mut inferred_spans = HashSet::new();
        for decl in &source_file.declarations {
            let pending = match decl {
                Declaration::Function(f)
                    if f.visibility == Visibility::Private
                        && f.return_type.is_none()
                        && matches!(f.body, Some(FunctionBody::Expr(_))) =>
                {
                    Box::new(PendingDecl::Function(f.clone()))
                }
                Declaration::Var(v) if v.visibility == Visibility::Private && v.ty.is_none() => {
                    Box::new(PendingDecl::Var(v.clone()))
                }
                _ => continue,
            };
            // A duplicate declaration (`E4010`) keeps the first one's inference.
            if !self.inference.contains_key(decl.name()) {
                self.inference
                    .insert(decl.name().to_string(), Inference::Pending(pending));
                inferred_spans.insert(decl.span());
            }
        }

        for decl in &source_file.declarations {
            if inferred_spans.contains(&decl.span()) {
                // Inferred on first use, or here if never used before.
                self.infer_declaration(decl.name());
                continue;
            }
            match decl {
                Declaration::Function(f) => {
                    self.check_function_body(f);
                }
                Declaration::Var(v) => self.check_top_level_var(v),
                _ => {}
            }
        }
    }

    /// Checks a top-level variable's initializer. An unannotated private variable takes the
    /// initializer's type.
    fn check_top_level_var(&mut self, v: &TopLevelVarDecl) {
        let inferred = self.infer_expr(&v.initializer);
        if let Some(annot) = &v.ty {
            let expected = self.lower_type(annot);
            if !inferred.is_assignable_to(&expected) {
                self.errors.push(TypeError::TypeMismatch {
                    expected,
                    found: inferred.clone(),
                    span: v.span,
                });
            }
        } else if v.visibility == Visibility::Private {
            self.env.update_type(&v.name, inferred);
        }
    }

    /// Infers the unannotated private declaration `name` if it is still pending. Its body is
    /// checked in a fresh body context (module scope only), so the body being checked when the
    /// declaration is first used does not leak into it.
    fn infer_declaration(&mut self, name: &str) {
        let pending = match self.inference.get(name) {
            Some(Inference::Pending(pending)) => pending.clone(),
            _ => return,
        };
        self.inference
            .insert(name.to_string(), Inference::InProgress);

        let saved = BodyContext {
            locals: self.env.take_locals(),
            in_compose_stack: std::mem::replace(&mut self.in_compose_stack, vec![false]),
            current_expected_return: self.current_expected_return.take(),
            in_function: std::mem::replace(&mut self.in_function, false),
            origin: std::mem::replace(&mut self.origin, Origin::Value),
        };
        match pending.as_ref() {
            PendingDecl::Function(f) => self.check_function_body(f),
            PendingDecl::Var(v) => self.check_top_level_var(v),
        }
        self.env.restore_locals(saved.locals);
        self.in_compose_stack = saved.in_compose_stack;
        self.current_expected_return = saved.current_expected_return;
        self.in_function = saved.in_function;
        self.origin = saved.origin;

        self.inference.insert(name.to_string(), Inference::Done);
    }

    /// Before `name` is read at `span`, infers it if it is a pending unannotated declaration.
    /// Reaching a declaration whose inference is in progress means its type depends on itself,
    /// which reports `E4013` once per declaration. The use then sees the declaration's provisional
    /// type, whose inferred part (a function's return type, a variable's type) is the error type.
    fn ensure_inferred(&mut self, name: &str, span: Span) {
        if !self.env.resolves_at_module_level(name) {
            return;
        }
        match self.inference.get(name) {
            Some(Inference::Pending(_)) => self.infer_declaration(name),
            Some(Inference::InProgress) if self.recursive_inference.insert(name.to_string()) => {
                self.errors.push(TypeError::CannotInferType {
                    message: format!(
                        "cannot infer the type of '{}' because it depends on itself; add an explicit type",
                        name
                    ),
                    span,
                });
            }
            _ => {}
        }
    }

    /// Typechecks an individual function declaration body.
    pub fn check_function_body(&mut self, f: &FunctionDecl) {
        let prev_in_func = self.in_function;
        self.in_function = true;
        self.env.enter_scope();
        self.push_compose_context(f.is_compose);

        // Bind parameters in local scope. A default value is checked against its parameter type
        // before the parameter is bound, so it may refer to earlier parameters only.
        for p in &f.params {
            let p_ty = self.lower_type(&p.ty);
            if let Some(default) = &p.default_value {
                self.check_expr(default, &p_ty);
            }
            let sym = Symbol::variable(p.name.clone(), p_ty, Mutability::Val, p.span);
            if let Err(err) = self.env.define(sym) {
                self.errors.push(err);
            }
        }

        let declared_ret = f.return_type.as_ref().map(|t| self.lower_type(t)).or({
            if matches!(f.body, Some(FunctionBody::Block(_)) | None) {
                Some(Type::Unit)
            } else {
                None
            }
        });
        let prev_ret = self.current_expected_return.take();
        self.current_expected_return = declared_ret.clone();

        if let Some(body) = &f.body {
            match body {
                FunctionBody::Block(block) => {
                    // The final expression of a non-`Unit` body is the returned value.
                    let origin = match &declared_ret {
                        Some(Type::Unit) | None => Origin::Value,
                        Some(_) => Origin::Return,
                    };
                    let block_ty =
                        self.with_origin(origin, |c| c.check_block(block, declared_ret.as_ref()));
                    if let Some(expected) = &declared_ret {
                        // A `Unit` block body discards its final value (see `check_block`).
                        if expected != &Type::Unit && !block_ty.is_assignable_to(expected) {
                            self.errors.push(TypeError::ReturnTypeMismatch {
                                expected: expected.clone(),
                                found: block_ty,
                                span: block.span,
                            });
                        }
                    }
                }
                FunctionBody::Expr(expr) => {
                    let expr_ty = if let Some(expected) = &declared_ret {
                        self.check_expr_at(expr, expected, Origin::Return)
                    } else {
                        self.infer_expr(expr)
                    };

                    if declared_ret.is_none() {
                        self.env.update_type(
                            &f.name,
                            Type::Fn {
                                is_compose: f.is_compose,
                                params: f.params.iter().map(|p| self.lower_type(&p.ty)).collect(),
                                ret: Box::new(expr_ty),
                            },
                        );
                    }
                }
            }
        }

        self.current_expected_return = prev_ret;
        self.pop_compose_context();
        self.env.exit_scope();
        self.in_function = prev_in_func;
    }

    /// Runs `f` with the expectation origin set to `origin`, restoring it afterwards.
    fn with_origin<T>(&mut self, origin: Origin, f: impl FnOnce(&mut Self) -> T) -> T {
        let prev = std::mem::replace(&mut self.origin, origin);
        let result = f(self);
        self.origin = prev;
        result
    }

    /// Checks `expr` against `expected` with the expectation origin `origin`.
    fn check_expr_at(&mut self, expr: &Expr, expected: &Type, origin: Origin) -> Type {
        self.with_origin(origin, |checker| checker.check_expr(expr, expected))
    }

    /// Infers an operator operand or a condition. Operators read through references (D1 of
    /// `typechecker-soundness`), so every `&` and `&mut` layer is peeled: with `a: &Int`,
    /// `a + 1` is an `Int`. `Heap<T>` is not peeled.
    fn infer_operand(&mut self, expr: &Expr) -> Type {
        let ty = self.with_origin(Origin::Value, |c| c.infer_expr(expr));
        peel_references(&ty).clone()
    }

    /// Checks an operator operand or a condition against `expected`, reading through references
    /// like [`Self::infer_operand`]. A mismatch is a plain `E4002`.
    fn check_operand(&mut self, expr: &Expr, expected: &Type) -> Type {
        let ty = self.infer_operand(expr);
        if ty.is_assignable_to(expected) {
            return ty;
        }
        if ty != Type::Error {
            self.with_origin(Origin::Value, |c| {
                c.report_mismatch(expected, ty, expr.span)
            });
        }
        Type::Error
    }

    /// Reports that `found` is not assignable to `expected`, with the code of the current
    /// expectation origin.
    fn report_mismatch(&mut self, expected: &Type, found: Type, span: Span) {
        let expected = expected.clone();
        self.errors.push(match self.origin {
            Origin::Value => TypeError::TypeMismatch {
                expected,
                found,
                span,
            },
            Origin::Return => TypeError::ReturnTypeMismatch {
                expected,
                found,
                span,
            },
            Origin::Argument => TypeError::ArgumentTypeMismatch {
                expected,
                found,
                span,
            },
        });
    }

    /// Checks an expression against an expected type in bidirectional checking mode.
    pub fn check_expr(&mut self, expr: &Expr, expected: &Type) -> Type {
        if expected == &Type::Error {
            return Type::Error;
        }

        // Special handling for null literal
        if let ExprKind::Literal(Literal::Null) = &expr.kind {
            if let Type::Nullable(_) = expected {
                return expected.clone();
            } else {
                self.report_mismatch(expected, Type::Nullable(Box::new(Type::Nothing)), expr.span);
                return Type::Error;
            }
        }

        // Special handling for closures checked against expected function type
        if let ExprKind::Closure { params, body } = &expr.kind {
            if let Type::Fn {
                is_compose,
                params: exp_params,
                ret: exp_ret,
            } = expected
            {
                if params.len() != exp_params.len() {
                    self.errors.push(TypeError::CallSignatureMismatch {
                        message: format!(
                            "closure expected {} parameters, but has {}",
                            exp_params.len(),
                            params.len()
                        ),
                        span: expr.span,
                    });
                    return Type::Error;
                }

                self.env.enter_scope();
                self.push_compose_context(*is_compose);
                let prev_in_func = self.in_function;
                self.in_function = true;
                let prev_ret = self.current_expected_return.take();
                self.current_expected_return = Some((**exp_ret).clone());

                for (name, param_ty) in params.iter().zip(exp_params.iter()) {
                    let sym = Symbol::variable(
                        name.clone(),
                        param_ty.clone(),
                        Mutability::Val,
                        expr.span,
                    );
                    if let Err(err) = self.env.define(sym) {
                        self.errors.push(err);
                    }
                }

                // The body's value is what the closure returns.
                let body_ty =
                    self.with_origin(Origin::Return, |c| c.check_block(body, Some(exp_ret)));

                self.current_expected_return = prev_ret;
                self.in_function = prev_in_func;
                self.pop_compose_context();
                self.env.exit_scope();

                return Type::Fn {
                    is_compose: *is_compose,
                    params: exp_params.clone(),
                    ret: Box::new(body_ty),
                };
            }
        }

        // Special handling for if/else in check mode
        if let ExprKind::If {
            cond,
            then_branch,
            else_branch,
        } = &expr.kind
        {
            self.check_operand(cond, &Type::Bool);
            let then_ty = self.check_expr(then_branch, expected);
            if let Some(else_b) = else_branch {
                let else_ty = self.check_expr(else_b, expected);
                if then_ty.is_assignable_to(expected) && else_ty.is_assignable_to(expected) {
                    return expected.clone();
                }
            } else if expected == &Type::Unit {
                return Type::Unit;
            }
        }

        // Special handling for when in check mode: each branch produces the checked value
        if let ExprKind::When { target, branches } = &expr.kind {
            return self.infer_when_expr(
                target.as_deref(),
                branches,
                WhenUse::Check(expected),
                expr.span,
            );
        }

        // Default: infer and verify assignability. Expectations nested inside the inferred
        // expression have their own origin.
        let inferred = self.with_origin(Origin::Value, |c| c.infer_expr(expr));
        if !inferred.is_assignable_to(expected) {
            if inferred != Type::Error && expected != &Type::Error {
                self.report_mismatch(expected, inferred.clone(), expr.span);
            }
            return Type::Error;
        }
        inferred
    }

    /// Types an identifier read as a value.
    fn infer_ident(&mut self, name: &str, span: Span) -> Type {
        match self.resolve_ident(name, span) {
            Some(sym) => match sym.kind {
                SymbolKind::Struct { .. } | SymbolKind::Enum { .. } => {
                    self.errors.push(TypeError::TypeNameAsValue {
                        name: name.to_string(),
                        span,
                    });
                    Type::Error
                }
                _ => sym.ty(),
            },
            None => Type::Error,
        }
    }

    /// Infers the type of an expression in inference mode.
    pub fn infer_expr(&mut self, expr: &Expr) -> Type {
        match &expr.kind {
            ExprKind::Literal(lit) => match lit {
                Literal::Int(_) => Type::Int,
                Literal::Float(_) => Type::Float,
                Literal::String(_) => Type::String,
                Literal::Char(_) => Type::Char,
                Literal::Bool(_) => Type::Bool,
                Literal::Null => Type::Nullable(Box::new(Type::Nothing)),
            },

            ExprKind::Ident(name) => {
                self.ensure_inferred(name, expr.span);
                self.infer_ident(name, expr.span)
            }

            ExprKind::Binary { op, left, right } => {
                self.infer_binary_op(*op, left, right, expr.span)
            }

            ExprKind::Unary { op, expr: inner } => self.infer_unary_op(*op, inner, expr.span),

            ExprKind::LocalVar {
                is_var,
                name,
                ty: annot,
                initializer,
            } => {
                let var_ty = if let Some(a) = annot {
                    self.check_annotation_names(a);
                    let expected = self.lower_type(a);
                    self.check_expr(initializer, &expected);
                    expected
                } else {
                    self.infer_expr(initializer)
                };

                let mutability = if *is_var {
                    Mutability::Var
                } else {
                    Mutability::Val
                };
                let sym = Symbol::variable(name.clone(), var_ty, mutability, expr.span);
                if let Err(err) = self.env.define(sym) {
                    self.errors.push(err);
                }
                Type::Unit
            }

            ExprKind::Assign { target, value } => self.check_assignment(target, value, expr.span),

            ExprKind::Block(block) => self.check_block(block, None),

            ExprKind::If {
                cond,
                then_branch,
                else_branch,
            } => {
                self.check_operand(cond, &Type::Bool);
                let then_ty = self.infer_expr(then_branch);
                if let Some(else_b) = else_branch {
                    let else_ty = self.infer_expr(else_b);
                    if let Some(joined) = then_ty.join(&else_ty) {
                        joined
                    } else {
                        self.errors.push(TypeError::TypeMismatch {
                            expected: then_ty.clone(),
                            found: else_ty,
                            span: expr.span,
                        });
                        Type::Error
                    }
                } else {
                    Type::Unit
                }
            }

            ExprKind::When { target, branches } => {
                self.infer_when_expr(target.as_deref(), branches, WhenUse::Infer, expr.span)
            }

            ExprKind::Call { callee, args } => self.infer_call_expr(callee, args, expr.span),

            ExprKind::MemberAccess {
                target,
                member,
                is_safe_nav,
            } => self.infer_member_access(target, member, *is_safe_nav, expr.span),

            ExprKind::Index { target, index } => {
                self.check_expr(index, &Type::Int);
                let target_ty = self.infer_expr(target);
                let peeled = peel_indirection(&target_ty);
                match peeled {
                    Type::String => Type::Char,
                    Type::Error => Type::Error,
                    other => {
                        self.errors.push(TypeError::TypeMismatch {
                            expected: Type::String,
                            found: other.clone(),
                            span: expr.span,
                        });
                        Type::Error
                    }
                }
            }

            ExprKind::Return(opt_expr) => {
                if !self.in_function {
                    self.errors.push(TypeError::ReturnTypeMismatch {
                        expected: Type::Unit,
                        found: Type::Error,
                        span: expr.span,
                    });
                    return Type::Nothing;
                }
                let expected = self.current_expected_return.clone();
                if let Some(e) = opt_expr {
                    if let Some(exp) = &expected {
                        self.check_expr_at(e, exp, Origin::Return);
                    } else {
                        self.infer_expr(e);
                    }
                } else if let Some(exp) = &expected {
                    if exp != &Type::Unit && exp != &Type::Error {
                        self.errors.push(TypeError::ReturnTypeMismatch {
                            expected: exp.clone(),
                            found: Type::Unit,
                            span: expr.span,
                        });
                    }
                }
                Type::Nothing
            }

            ExprKind::Closure { params, body } => {
                self.env.enter_scope();
                self.push_compose_context(false);
                let prev_in_func = self.in_function;
                self.in_function = true;
                let prev_ret = self.current_expected_return.take();

                // Without an expected function type, nothing gives the parameters a type (D6 of
                // `typechecker-soundness`). They are bound to the error type so the body is still
                // checked for other errors.
                let mut param_types = Vec::new();
                for name in params {
                    let sym =
                        Symbol::variable(name.clone(), Type::Error, Mutability::Val, expr.span);
                    match self.env.define(sym) {
                        Ok(()) => self.errors.push(TypeError::CannotInferType {
                            message: format!(
                                "cannot infer the type of closure parameter '{}'; pass the closure where a function type is expected",
                                name
                            ),
                            span: expr.span,
                        }),
                        Err(err) => self.errors.push(err),
                    }
                    param_types.push(Type::Error);
                }
                let body_ty = self.check_block(body, None);

                self.current_expected_return = prev_ret;
                self.in_function = prev_in_func;
                self.pop_compose_context();
                self.env.exit_scope();

                Type::Fn {
                    is_compose: false,
                    params: param_types,
                    ret: Box::new(body_ty),
                }
            }
        }
    }

    fn infer_binary_op(&mut self, op: BinaryOp, left: &Expr, right: &Expr, span: Span) -> Type {
        match op {
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod => {
                let left_ty = self.infer_operand(left);
                let right_ty = self.infer_operand(right);

                if left_ty == Type::Error || right_ty == Type::Error {
                    return Type::Error;
                }

                if left_ty == Type::Int && right_ty == Type::Int {
                    return Type::Int;
                }

                if left_ty == Type::Float && right_ty == Type::Float {
                    return Type::Float;
                }

                if op == BinaryOp::Add && left_ty == Type::String && right_ty == Type::String {
                    return Type::String;
                }

                // Strict typing: no implicit widening (e.g. Int + Float rejected)
                self.errors.push(TypeError::TypeMismatch {
                    expected: left_ty,
                    found: right_ty,
                    span,
                });
                Type::Error
            }

            BinaryOp::Eq | BinaryOp::NotEq => {
                let left_ty = self.infer_operand(left);
                let right_ty = self.infer_operand(right);

                if left_ty == Type::Error || right_ty == Type::Error {
                    return Type::Bool;
                }

                if left_ty.is_assignable_to(&right_ty) || right_ty.is_assignable_to(&left_ty) {
                    Type::Bool
                } else {
                    self.errors.push(TypeError::TypeMismatch {
                        expected: left_ty,
                        found: right_ty,
                        span,
                    });
                    Type::Error
                }
            }

            BinaryOp::Lt | BinaryOp::LtEq | BinaryOp::Gt | BinaryOp::GtEq => {
                let left_ty = self.infer_operand(left);
                let right_ty = self.infer_operand(right);

                if left_ty == Type::Error || right_ty == Type::Error {
                    return Type::Bool;
                }

                if (left_ty == Type::Int && right_ty == Type::Int)
                    || (left_ty == Type::Float && right_ty == Type::Float)
                    || (left_ty == Type::Char && right_ty == Type::Char)
                {
                    Type::Bool
                } else {
                    self.errors.push(TypeError::TypeMismatch {
                        expected: left_ty,
                        found: right_ty,
                        span,
                    });
                    Type::Error
                }
            }

            BinaryOp::And | BinaryOp::Or => {
                self.check_operand(left, &Type::Bool);
                self.check_operand(right, &Type::Bool);
                Type::Bool
            }

            BinaryOp::Elvis => {
                let left_ty = self.infer_expr(left);
                // The right side is checked even when the left side cannot be null.
                let inner = match left_ty {
                    Type::Nullable(inner) => *inner,
                    other => other,
                };
                let right_ty = self.infer_expr(right);
                if inner == Type::Error || right_ty == Type::Error {
                    Type::Error
                } else if right_ty == Type::Nothing {
                    inner
                } else if inner == Type::Nothing {
                    right_ty
                } else if right_ty.is_assignable_to(&inner) {
                    inner
                } else if inner.is_assignable_to(&right_ty) {
                    right_ty
                } else {
                    self.errors.push(TypeError::TypeMismatch {
                        expected: inner.clone(),
                        found: right_ty,
                        span,
                    });
                    inner
                }
            }

            BinaryOp::Range => {
                self.check_operand(left, &Type::Int);
                self.check_operand(right, &Type::Int);
                Type::Struct(NominalId::builtin_range())
            }
        }
    }

    fn infer_unary_op(&mut self, op: UnaryOp, inner: &Expr, span: Span) -> Type {
        match op {
            UnaryOp::Neg => {
                let ty = self.infer_operand(inner);
                if ty == Type::Int || ty == Type::Float || ty == Type::Error {
                    ty
                } else {
                    self.errors.push(TypeError::TypeMismatch {
                        expected: Type::Int,
                        found: ty,
                        span,
                    });
                    Type::Error
                }
            }
            UnaryOp::Not => {
                self.check_operand(inner, &Type::Bool);
                Type::Bool
            }
            UnaryOp::Borrow => match self.infer_expr(inner) {
                Type::Error => Type::Error,
                ty => Type::Reference {
                    mutable: false,
                    inner: Box::new(ty),
                },
            },
            UnaryOp::BorrowMut => {
                let (ty, writable) = self.infer_place(inner);
                if ty == Type::Error {
                    return Type::Error;
                }
                match &inner.kind {
                    ExprKind::Ident(name) => {
                        // A `val` holding `&mut T` may be reborrowed mutably.
                        let is_val = matches!(
                            self.env.lookup(name).map(|sym| &sym.kind),
                            Some(SymbolKind::Variable {
                                mutability: Mutability::Val,
                                ..
                            })
                        );
                        if is_val && !matches!(ty, Type::Reference { mutable: true, .. }) {
                            self.errors.push(TypeError::ImmutabilityViolation {
                                message: format!(
                                    "cannot take mutable borrow '&mut' of immutable 'val' binding '{}'",
                                    name
                                ),
                                span,
                            });
                        }
                    }
                    ExprKind::MemberAccess { member, .. } if !writable => {
                        self.errors.push(TypeError::ImmutabilityViolation {
                            message: format!(
                                "cannot take mutable borrow '&mut' of field '{}': it is a 'val' field or is reached through a 'val' binding, 'val' field or shared '&' borrow",
                                member
                            ),
                            span,
                        });
                    }
                    _ => {}
                }
                Type::Reference {
                    mutable: true,
                    inner: Box::new(ty),
                }
            }
        }
    }

    fn check_assignment(&mut self, target: &Expr, value: &Expr, span: Span) -> Type {
        let (target_ty, is_mutable) = match &target.kind {
            ExprKind::Ident(name) => match self.env.resolve(name, target.span) {
                Ok(sym) => match &sym.kind {
                    SymbolKind::Variable { ty, mutability, .. } => {
                        (ty.clone(), *mutability == Mutability::Var)
                    }
                    _ => {
                        self.errors.push(TypeError::ImmutabilityViolation {
                            message: format!("cannot assign to non-variable '{}'", name),
                            span,
                        });
                        (Type::Error, false)
                    }
                },
                Err(err) => {
                    self.errors.push(err);
                    (Type::Error, false)
                }
            },
            ExprKind::MemberAccess {
                target: base,
                member,
                is_safe_nav: false,
            } => {
                let (base_ty, base_writable, field_ty, is_var) =
                    match self.type_name_member(base, member, target.span) {
                        Some((field_ty, is_var)) => (field_ty.clone(), false, field_ty, is_var),
                        None => {
                            let (base_ty, base_writable) = self.infer_place(base);
                            let (field_ty, is_var) =
                                self.member_type(&base_ty, member, false, target.span);
                            (base_ty, base_writable, field_ty, is_var)
                        }
                    };
                match is_var {
                    Some(false) => {
                        self.errors.push(TypeError::ImmutabilityViolation {
                            message: format!(
                                "cannot assign to immutable 'val' struct field '{}'",
                                member
                            ),
                            span,
                        });
                        (field_ty, false)
                    }
                    Some(true) if !contents_writable(&base_ty, base_writable) => {
                        self.errors.push(TypeError::ImmutabilityViolation {
                            message: format!(
                                "cannot assign to field '{}' of a value that is not mutable (a 'val' binding or field, or a shared '&' borrow)",
                                member
                            ),
                            span,
                        });
                        (field_ty, false)
                    }
                    Some(true) => (field_ty, true),
                    None => {
                        if field_ty != Type::Error {
                            self.errors.push(TypeError::UnknownField {
                                target_type: base_ty.clone(),
                                field_name: member.clone(),
                                span,
                            });
                        }
                        (Type::Error, false)
                    }
                }
            }
            _ => {
                self.errors.push(TypeError::ImmutabilityViolation {
                    message: "invalid assignment target".to_string(),
                    span,
                });
                (Type::Error, false)
            }
        };

        if !is_mutable && target_ty != Type::Error {
            if let ExprKind::Ident(name) = &target.kind {
                self.errors.push(TypeError::ImmutabilityViolation {
                    message: format!("cannot reassign to immutable 'val' binding '{}'", name),
                    span,
                });
            }
        }

        self.check_expr(value, &target_ty);
        Type::Unit
    }

    /// Typechecks an expression whose value is discarded. Only `when` differs from value
    /// position: it needs neither an `else` nor a common branch type.
    fn infer_statement(&mut self, stmt: &Expr) -> Type {
        match &stmt.kind {
            ExprKind::When { target, branches } => {
                self.infer_when_expr(target.as_deref(), branches, WhenUse::Statement, stmt.span)
            }
            _ => self.infer_expr(stmt),
        }
    }

    /// Typechecks a block. With an expected type its final expression is checked against it,
    /// except that a block expected to produce `Unit` (the body of a `Unit` function or of a
    /// closure expected to return `Unit`) discards its final value: the expression is
    /// typechecked as a statement and its type ignored (D8 of `typechecker-soundness`).
    fn check_block(&mut self, block: &BlockExpr, expected: Option<&Type>) -> Type {
        self.env.enter_scope();
        if block.stmts.is_empty() {
            self.env.exit_scope();
            return Type::Unit;
        }

        let num_stmts = block.stmts.len();
        self.with_origin(Origin::Value, |checker| {
            for stmt in &block.stmts[..num_stmts - 1] {
                checker.infer_statement(stmt);
            }
        });

        // The tail of a block whose expected type is `Unit` is a statement too. A checked tail
        // produces the block's value, so it keeps the expectation origin.
        let last_stmt = &block.stmts[num_stmts - 1];
        let result_ty = match expected {
            Some(Type::Unit) => {
                self.with_origin(Origin::Value, |c| c.infer_statement(last_stmt));
                Type::Unit
            }
            Some(exp) => self.check_expr(last_stmt, exp),
            None => self.with_origin(Origin::Value, |c| c.infer_expr(last_stmt)),
        };

        self.env.exit_scope();
        result_ty
    }

    /// Typechecks a `when`. When its value is used it must be exhaustive (`E4009`), and its
    /// branches either join to a common type (`WhenUse::Infer`) or are each checked against the
    /// expected type (`WhenUse::Check`). As a statement its value is discarded, so neither
    /// applies and its type is `Unit`.
    fn infer_when_expr(
        &mut self,
        target: Option<&Expr>,
        branches: &[WhenBranch],
        use_: WhenUse,
        span: Span,
    ) -> Type {
        let value_position = !matches!(use_, WhenUse::Statement);
        // The subject and the patterns are operands, so their expectations are plain values.
        let branch_origin = self.origin;
        self.origin = Origin::Value;
        // Patterns match the borrowed value, so a `&Enum` or `&mut Enum` subject matches as `Enum`.
        let subject_ty = target.map(|t| {
            let ty = self.infer_expr(t);
            let ty = peel_references(&ty).clone();
            // Matching on variants needs the enum's definition, which needs its module.
            if self.reject_unimported_members(&ty, t.span) {
                Type::Error
            } else {
                ty
            }
        });
        let subject_variants = match &subject_ty {
            Some(Type::Enum(id)) => self
                .enum_variants(id)
                .map(|variants| (id.clone(), variants)),
            _ => None,
        };
        let mut has_else = false;
        let mut unified_ty: Option<Type> = None;
        let mut matched_variants = HashSet::new();

        for branch in branches {
            for pattern in &branch.patterns {
                match pattern {
                    WhenPattern::Else => {
                        has_else = true;
                    }
                    WhenPattern::Expr(p_expr) => {
                        let covered = subject_variants.as_ref().and_then(|(id, variants)| {
                            self.unit_variant_pattern(p_expr, id, variants)
                        });
                        if let Some(v_name) = covered {
                            matched_variants.insert(v_name);
                        } else if let Some(s_ty) = &subject_ty {
                            self.check_expr(p_expr, s_ty);
                        } else {
                            self.check_operand(p_expr, &Type::Bool);
                        }
                    }
                    WhenPattern::InRange { start, end } => {
                        if let Some(s_ty) = &subject_ty {
                            self.check_expr(start, s_ty);
                            self.check_expr(end, s_ty);
                        }
                    }
                }
            }

            if let WhenUse::Check(expected) = use_ {
                self.check_expr_at(&branch.body, expected, branch_origin);
                continue;
            }
            let branch_ty = self.infer_expr(&branch.body);
            if value_position && branch_ty != Type::Nothing {
                if let Some(current) = &unified_ty {
                    if let Some(joined) = current.join(&branch_ty) {
                        unified_ty = Some(joined);
                    } else {
                        self.errors.push(TypeError::TypeMismatch {
                            expected: current.clone(),
                            found: branch_ty.clone(),
                            span: branch.span,
                        });
                    }
                } else {
                    unified_ty = Some(branch_ty);
                }
            }
        }

        self.origin = branch_origin;
        if !value_position {
            return Type::Unit;
        }

        // Exhaustiveness check: a `when` in value position needs an `else` or every variant. A
        // subject whose type is already an error has nothing more to report.
        let is_exhaustive = has_else
            || subject_ty == Some(Type::Error)
            || subject_variants.is_some_and(|(_, variants)| {
                !variants.is_empty()
                    && variants
                        .iter()
                        .all(|(v_name, _)| matched_variants.contains(v_name))
            });

        if !is_exhaustive {
            self.errors.push(TypeError::NonExhaustiveWhen {
                message: "'when' expression in value position must have an 'else' branch or be exhaustive"
                    .to_string(),
                span,
            });
        }

        match use_ {
            WhenUse::Check(expected) => expected.clone(),
            _ => unified_ty.unwrap_or(Type::Unit),
        }
    }

    /// If `pattern` names a payload-free variant of the subject enum `enum_id`, returns that
    /// variant's name. Such a pattern covers the whole variant. A qualified path `Enum.Variant`
    /// counts only when `Enum` is the subject's enum, and a bare `Variant` counts unless a local
    /// binding shadows it. Values that merely equal a variant (`Shape.Circle(1)`, `h.Green`) never
    /// count.
    fn unit_variant_pattern(
        &self,
        pattern: &Expr,
        enum_id: &NominalId,
        variants: &[(String, Vec<Type>)],
    ) -> Option<String> {
        let v_name = match &pattern.kind {
            ExprKind::Ident(name) => {
                let shadowed =
                    self.env.lookup(name).is_some() && !self.env.resolves_at_module_level(name);
                if shadowed {
                    return None;
                }
                name
            }
            ExprKind::MemberAccess {
                target,
                member,
                is_safe_nav: false,
            } => match self.type_name_symbol(target).map(|sym| sym.kind) {
                Some(SymbolKind::Enum { id, .. }) if &id == enum_id => member,
                _ => return None,
            },
            _ => return None,
        };
        variants
            .iter()
            .any(|(name, payloads)| name == v_name && payloads.is_empty())
            .then(|| v_name.clone())
    }

    fn infer_call_expr(&mut self, callee: &Expr, args: &[CallArg], span: Span) -> Type {
        // A struct name is its constructor; an enum name is only a namespace for its variants.
        let type_name = self.type_name_symbol(callee);
        let callee_ty = match &type_name {
            Some(sym) if !self.check_visibility(sym, callee.span) => Type::Error,
            Some(sym) => match sym.kind {
                SymbolKind::Struct { .. } => sym.ty(),
                _ => {
                    self.errors.push(TypeError::CallSignatureMismatch {
                        message: format!(
                            "enum '{}' cannot be instantiated directly without a variant",
                            sym.name
                        ),
                        span,
                    });
                    Type::Error
                }
            },
            None => self.infer_expr(callee),
        };
        let is_constructor = type_name.is_some();

        match callee_ty {
            Type::Fn {
                is_compose,
                params,
                ret,
            } => {
                // Composable context enforcement (first-class compose invariant, E4008)
                if is_compose && !self.is_in_compose() {
                    let callee_name = if let ExprKind::Ident(n) = &callee.kind {
                        n.clone()
                    } else {
                        "<closure>".to_string()
                    };
                    self.errors.push(TypeError::InvalidComposeContext {
                        callee: callee_name,
                        span,
                    });
                }

                let (param_names, has_default) = if let ExprKind::Ident(callee_name) = &callee.kind
                {
                    if let Some(sym) = self.env.lookup(callee_name) {
                        if let SymbolKind::Function {
                            param_names,
                            has_default,
                            ..
                        } = &sym.kind
                        {
                            (param_names.clone(), has_default.clone())
                        } else {
                            (Vec::new(), Vec::new())
                        }
                    } else {
                        (Vec::new(), Vec::new())
                    }
                } else {
                    (Vec::new(), Vec::new())
                };

                let n_params = params.len();
                let min_args = if has_default.len() == n_params {
                    has_default.iter().filter(|&&d| !d).count()
                } else {
                    n_params
                };

                let has_named_args = args.iter().any(|a| a.name.is_some());

                if !has_named_args {
                    if args.len() < min_args || args.len() > n_params {
                        let msg = if min_args == n_params {
                            format!(
                                "function expected {} arguments, found {}",
                                n_params,
                                args.len()
                            )
                        } else {
                            format!(
                                "function expected between {} and {} arguments, found {}",
                                min_args,
                                n_params,
                                args.len()
                            )
                        };
                        self.errors
                            .push(TypeError::CallSignatureMismatch { message: msg, span });
                        return *ret;
                    }

                    for (arg, param_ty) in args.iter().zip(params.iter()) {
                        self.check_expr_at(&arg.value, param_ty, Origin::Argument);
                    }
                } else {
                    let mut param_assigned = vec![false; n_params];
                    let mut seen_names = HashSet::new();
                    let mut positional_idx = 0;

                    for arg in args {
                        if let Some(arg_name) = &arg.name {
                            if !seen_names.insert(arg_name.clone()) {
                                self.errors.push(TypeError::CallSignatureMismatch {
                                    message: format!("duplicate argument '{}' provided", arg_name),
                                    span: arg.span,
                                });
                                continue;
                            }
                            if let Some(pos) = param_names.iter().position(|pn| pn == arg_name) {
                                if param_assigned[pos] {
                                    self.errors.push(TypeError::CallSignatureMismatch {
                                        message: format!(
                                            "parameter '{}' is already given a positional argument",
                                            arg_name
                                        ),
                                        span: arg.span,
                                    });
                                }
                                param_assigned[pos] = true;
                                self.check_expr_at(&arg.value, &params[pos], Origin::Argument);
                            } else {
                                self.errors.push(TypeError::CallSignatureMismatch {
                                    message: format!("unknown parameter '{}'", arg_name),
                                    span: arg.span,
                                });
                            }
                        } else {
                            if positional_idx < n_params {
                                if param_assigned[positional_idx] {
                                    self.errors.push(TypeError::CallSignatureMismatch {
                                        message: format!(
                                            "positional argument assigns parameter '{}', which is already given by name",
                                            param_names
                                                .get(positional_idx)
                                                .map(String::as_str)
                                                .unwrap_or("parameter")
                                        ),
                                        span: arg.span,
                                    });
                                }
                                param_assigned[positional_idx] = true;
                                self.check_expr_at(
                                    &arg.value,
                                    &params[positional_idx],
                                    Origin::Argument,
                                );
                                positional_idx += 1;
                            } else {
                                self.errors.push(TypeError::CallSignatureMismatch {
                                    message: format!(
                                        "too many positional arguments (expected at most {})",
                                        n_params
                                    ),
                                    span: arg.span,
                                });
                            }
                        }
                    }

                    for (i, &assigned) in param_assigned.iter().enumerate().take(n_params) {
                        let has_def = has_default.get(i).copied().unwrap_or(false);
                        if !has_def && !assigned {
                            let p_name = param_names
                                .get(i)
                                .map(|s| s.as_str())
                                .unwrap_or("parameter");
                            self.errors.push(TypeError::CallSignatureMismatch {
                                message: format!("missing required argument '{}'", p_name),
                                span,
                            });
                        }
                    }
                }

                *ret
            }
            Type::Struct(id) if is_constructor => {
                let name = id.name.clone();
                let fields = self.struct_fields(&id).unwrap_or_default();
                let has_named = args.iter().any(|a| a.name.is_some());
                if !has_named {
                    if args.len() != fields.len() {
                        self.errors.push(TypeError::CallSignatureMismatch {
                            message: format!(
                                "struct '{}' expected {} arguments, found {}",
                                name,
                                fields.len(),
                                args.len()
                            ),
                            span,
                        });
                        return Type::Struct(id);
                    }
                    for (arg, (_, _, f_ty)) in args.iter().zip(fields.iter()) {
                        self.check_expr_at(&arg.value, f_ty, Origin::Argument);
                    }
                } else {
                    let mut field_assigned = vec![false; fields.len()];
                    let mut seen_fields = HashSet::new();
                    let mut pos_idx = 0;

                    for arg in args {
                        if let Some(arg_name) = &arg.name {
                            if !seen_fields.insert(arg_name.clone()) {
                                self.errors.push(TypeError::CallSignatureMismatch {
                                    message: format!(
                                        "duplicate field '{}' provided in struct '{}' instantiation",
                                        arg_name, name
                                    ),
                                    span: arg.span,
                                });
                                continue;
                            }
                            if let Some(idx) = fields.iter().position(|(n, _, _)| n == arg_name) {
                                if field_assigned[idx] {
                                    self.errors.push(TypeError::CallSignatureMismatch {
                                        message: format!(
                                            "field '{}' of struct '{}' is already given a positional argument",
                                            arg_name, name
                                        ),
                                        span: arg.span,
                                    });
                                }
                                field_assigned[idx] = true;
                                self.check_expr_at(&arg.value, &fields[idx].2, Origin::Argument);
                            } else {
                                self.errors.push(TypeError::UnknownField {
                                    target_type: Type::Struct(id.clone()),
                                    field_name: arg_name.clone(),
                                    span: arg.span,
                                });
                            }
                        } else {
                            if pos_idx < fields.len() {
                                if field_assigned[pos_idx] {
                                    self.errors.push(TypeError::CallSignatureMismatch {
                                        message: format!(
                                            "positional argument assigns field '{}' of struct '{}', which is already given by name",
                                            fields[pos_idx].0, name
                                        ),
                                        span: arg.span,
                                    });
                                }
                                field_assigned[pos_idx] = true;
                                self.check_expr_at(
                                    &arg.value,
                                    &fields[pos_idx].2,
                                    Origin::Argument,
                                );
                                pos_idx += 1;
                            } else {
                                self.errors.push(TypeError::CallSignatureMismatch {
                                    message: format!(
                                        "struct '{}' expected at most {} arguments, found more",
                                        name,
                                        fields.len()
                                    ),
                                    span: arg.span,
                                });
                            }
                        }
                    }

                    for (idx, (f_name, _, _)) in fields.iter().enumerate() {
                        if !field_assigned[idx] {
                            self.errors.push(TypeError::CallSignatureMismatch {
                                message: format!(
                                    "missing required field '{}' in struct '{}' instantiation",
                                    f_name, name
                                ),
                                span,
                            });
                        }
                    }
                }

                Type::Struct(id)
            }
            Type::Error => Type::Error,
            other => {
                self.errors.push(TypeError::CallSignatureMismatch {
                    message: format!("type '{}' cannot be called as a function", other),
                    span,
                });
                Type::Error
            }
        }
    }

    fn infer_member_access(
        &mut self,
        target: &Expr,
        member: &str,
        is_safe_nav: bool,
        span: Span,
    ) -> Type {
        if let Some((ty, _)) = self.type_name_member(target, member, span) {
            return ty;
        }
        let target_ty = self.infer_expr(target);
        self.member_type(&target_ty, member, is_safe_nav, span).0
    }

    /// Types `member` accessed on a value of type `target_ty`, reporting unknown members.
    ///
    /// Returns the member's type and, when the member is a struct field, whether that field is
    /// declared `var`.
    fn member_type(
        &mut self,
        target_ty: &Type,
        member: &str,
        is_safe_nav: bool,
        span: Span,
    ) -> (Type, Option<bool>) {
        if target_ty == &Type::Error {
            return (Type::Error, None);
        }

        let (base_ty, is_nullable) = match target_ty {
            Type::Nullable(inner) => {
                if is_safe_nav {
                    (inner.as_ref(), true)
                } else {
                    self.errors.push(TypeError::UnknownField {
                        target_type: target_ty.clone(),
                        field_name: member.to_string(),
                        span,
                    });
                    return (Type::Error, None);
                }
            }
            other => (other, false),
        };
        let wrap = |ty: Type| if is_nullable { ty.nullable() } else { ty };

        let peeled_ty = peel_indirection(base_ty).clone();
        if self.reject_unimported_members(&peeled_ty, span) {
            return (Type::Error, None);
        }
        let def = match &peeled_ty {
            Type::Struct(id) | Type::Enum(id) => self.types.get(id).cloned(),
            _ => None,
        };

        match (&peeled_ty, def) {
            (Type::Struct(_), Some(TypeDef::Struct { fields })) => {
                if let Some((_, is_var, f_ty)) = fields.iter().find(|(n, _, _)| n == member) {
                    (wrap(f_ty.clone()), Some(*is_var))
                } else {
                    self.errors.push(TypeError::UnknownField {
                        target_type: peeled_ty.clone(),
                        field_name: member.to_string(),
                        span,
                    });
                    (Type::Error, None)
                }
            }
            (Type::Enum(id), Some(TypeDef::Enum { variants })) => {
                if let Some((_, payloads)) = variants.iter().find(|(v_name, _)| v_name == member) {
                    let enum_ty = Type::Enum(id.clone());
                    if payloads.is_empty() {
                        (wrap(enum_ty), None)
                    } else {
                        let ctor = Type::Fn {
                            is_compose: false,
                            params: payloads.clone(),
                            ret: Box::new(enum_ty),
                        };
                        (ctor, None)
                    }
                } else {
                    self.errors.push(TypeError::UnknownField {
                        target_type: peeled_ty.clone(),
                        field_name: member.to_string(),
                        span,
                    });
                    (Type::Error, None)
                }
            }
            (other, _) => {
                self.errors.push(TypeError::UnknownField {
                    target_type: other.clone(),
                    field_name: member.to_string(),
                    span,
                });
                (Type::Error, None)
            }
        }
    }

    /// Infers the type of a place expression (a binding or a field path such as `o.user.name`)
    /// together with whether the place itself may be written.
    ///
    /// A binding is writable when it is `var`. A field is writable when it is `var` and the value
    /// holding it is writable, where a `&mut` hop grants and a `&` hop denies write access
    /// regardless of the binding that holds the reference. Any other expression is a temporary,
    /// which is writable unless it is reached through a shared reference.
    fn infer_place(&mut self, expr: &Expr) -> (Type, bool) {
        match &expr.kind {
            ExprKind::Ident(name) => {
                let ty = self.infer_expr(expr);
                let writable = matches!(
                    self.env.lookup(name).map(|sym| &sym.kind),
                    Some(SymbolKind::Variable {
                        mutability: Mutability::Var,
                        ..
                    })
                );
                (ty, writable)
            }
            ExprKind::MemberAccess {
                target,
                member,
                is_safe_nav,
            } => {
                if let Some((ty, _)) = self.type_name_member(target, member, expr.span) {
                    return (ty, true);
                }
                let (base_ty, base_writable) = self.infer_place(target);
                let (ty, is_var) = self.member_type(&base_ty, member, *is_safe_nav, expr.span);
                // An enum variant is a fresh temporary rather than a field.
                let writable = match is_var {
                    Some(is_var) => is_var && contents_writable(&base_ty, base_writable),
                    None => true,
                };
                (ty, writable)
            }
            _ => (self.infer_expr(expr), true),
        }
    }
}

/// Whether the value behind a place of type `ty` may be mutated, given whether the place itself
/// is writable. The first reference layer decides (`&mut` grants, `&` denies), and every further
/// reference layer must also be `&mut`. `Heap<T>` owns its contents, so it is transparent.
fn contents_writable(mut ty: &Type, place_writable: bool) -> bool {
    let mut writable = place_writable;
    let mut behind_reference = false;
    loop {
        match ty {
            Type::Reference { mutable, inner } => {
                writable = if behind_reference {
                    writable && *mutable
                } else {
                    *mutable
                };
                behind_reference = true;
                ty = inner;
            }
            Type::Heap(inner) => ty = inner,
            Type::Nullable(inner) => ty = inner,
            _ => return writable,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prism_abi::{AbiType, PrimKind};

    /// Declares a struct with the given fields in `checker`'s module, returning its type.
    fn declare_struct(
        checker: &mut TypeChecker,
        name: &str,
        fields: Vec<(String, bool, Type)>,
    ) -> Type {
        let id = NominalId::new(checker.module_path.clone(), name);
        checker.types.insert(id.clone(), TypeDef::Struct { fields });
        checker
            .env
            .define(Symbol::struct_decl(
                id.clone(),
                Visibility::Public,
                None,
                Span::dummy(),
            ))
            .unwrap();
        Type::Struct(id)
    }

    /// Declares an enum with the given variants in `checker`'s module, returning its type.
    fn declare_enum(
        checker: &mut TypeChecker,
        name: &str,
        variants: Vec<(String, Vec<Type>)>,
    ) -> Type {
        let id = NominalId::new(checker.module_path.clone(), name);
        checker.types.insert(id.clone(), TypeDef::Enum { variants });
        checker
            .env
            .define(Symbol::enum_decl(
                id.clone(),
                Visibility::Public,
                None,
                Span::dummy(),
            ))
            .unwrap();
        Type::Enum(id)
    }

    // Helper to wrap expression in Expr
    fn expr(kind: ExprKind) -> Expr {
        Expr::new(kind, Span::dummy())
    }

    #[test]
    fn test_strict_numeric_operations_reject_widening() {
        let mut checker = TypeChecker::new(None, "<test>");
        let plus = expr(ExprKind::Binary {
            op: BinaryOp::Add,
            left: Box::new(expr(ExprKind::Literal(Literal::Int(1)))),
            right: Box::new(expr(ExprKind::Literal(Literal::Float(2.0)))),
        });

        let res = checker.infer_expr(&plus);
        assert_eq!(res, Type::Error);
        assert_eq!(checker.errors.len(), 1);
        assert_eq!(checker.errors[0].code(), "E4002");
    }

    #[test]
    fn test_char_literal_and_indexing() {
        let mut checker = TypeChecker::new(None, "<test>");
        let char_expr = expr(ExprKind::Literal(Literal::Char('z')));
        assert_eq!(checker.infer_expr(&char_expr), Type::Char);

        let index_expr = expr(ExprKind::Index {
            target: Box::new(expr(ExprKind::Literal(Literal::String(
                "hello".to_string(),
            )))),
            index: Box::new(expr(ExprKind::Literal(Literal::Int(0)))),
        });
        assert_eq!(checker.infer_expr(&index_expr), Type::Char);
    }

    #[test]
    fn test_val_reassignment_rejected_e4003() {
        let mut checker = TypeChecker::new(None, "<test>");
        let init = expr(ExprKind::LocalVar {
            is_var: false,
            name: "x".to_string(),
            ty: Some(prism_ast::types::Type::named("Int", Span::dummy())),
            initializer: Box::new(expr(ExprKind::Literal(Literal::Int(42)))),
        });
        checker.infer_expr(&init);

        let reassign = expr(ExprKind::Assign {
            target: Box::new(expr(ExprKind::Ident("x".to_string()))),
            value: Box::new(expr(ExprKind::Literal(Literal::Int(100)))),
        });
        checker.infer_expr(&reassign);

        assert_eq!(checker.errors.len(), 1);
        assert_eq!(checker.errors[0].code(), "E4003");
    }

    #[test]
    fn test_error_sentinel_suppresses_cascade() {
        let mut checker = TypeChecker::new(None, "<test>");
        // undefined + 1
        let err_expr = expr(ExprKind::Binary {
            op: BinaryOp::Add,
            left: Box::new(expr(ExprKind::Ident("not_found".to_string()))),
            right: Box::new(expr(ExprKind::Literal(Literal::Int(1)))),
        });
        let res = checker.infer_expr(&err_expr);
        assert_eq!(res, Type::Error);
        // Only 1 error (E4001: unresolved symbol), no cascade E4002 error
        assert_eq!(checker.errors.len(), 1);
        assert_eq!(checker.errors[0].code(), "E4001");
    }

    #[test]
    fn test_compose_context_enforcement_e4008() {
        let mut checker = TypeChecker::new(None, "<test>");
        let compose_fn_sym = Symbol::function(
            "MyView",
            Type::Fn {
                is_compose: true,
                params: Vec::new(),
                ret: Box::new(Type::Unit),
            },
            Visibility::Public,
            None,
            Span::dummy(),
        );
        checker.env.define(compose_fn_sym).unwrap();

        // Calling MyView in non-compose context
        let call = expr(ExprKind::Call {
            callee: Box::new(expr(ExprKind::Ident("MyView".to_string()))),
            args: Vec::new(),
        });
        checker.infer_expr(&call);

        assert_eq!(checker.errors.len(), 1);
        assert_eq!(checker.errors[0].code(), "E4008");
    }

    #[test]
    fn test_when_expression_exhaustiveness_e4009() {
        let mut checker = TypeChecker::new(None, "<test>");
        let when_no_else = expr(ExprKind::When {
            target: Some(Box::new(expr(ExprKind::Literal(Literal::Int(1))))),
            branches: vec![WhenBranch {
                patterns: vec![WhenPattern::Expr(expr(ExprKind::Literal(Literal::Int(1))))],
                body: expr(ExprKind::Literal(Literal::String("one".to_string()))),
                span: Span::dummy(),
            }],
        });
        checker.infer_expr(&when_no_else);

        assert_eq!(checker.errors.len(), 1);
        assert_eq!(checker.errors[0].code(), "E4009");
    }

    #[test]
    fn test_block_unit_and_trailing_expr() {
        let mut checker = TypeChecker::new(None, "<test>");
        let block_empty = expr(ExprKind::Block(BlockExpr {
            stmts: Vec::new(),
            span: Span::dummy(),
        }));
        assert_eq!(checker.infer_expr(&block_empty), Type::Unit);

        let block_trailing = expr(ExprKind::Block(BlockExpr {
            stmts: vec![
                expr(ExprKind::LocalVar {
                    is_var: false,
                    name: "a".to_string(),
                    ty: None,
                    initializer: Box::new(expr(ExprKind::Literal(Literal::Int(10)))),
                }),
                expr(ExprKind::Ident("a".to_string())),
            ],
            span: Span::dummy(),
        }));
        assert_eq!(checker.infer_expr(&block_trailing), Type::Int);
    }

    #[test]
    fn test_call_signature_mismatch_e4004() {
        let mut checker = TypeChecker::new(None, "<test>");
        let fn_sym = Symbol::function(
            "add",
            Type::Fn {
                is_compose: false,
                params: vec![Type::Int, Type::Int],
                ret: Box::new(Type::Int),
            },
            Visibility::Public,
            None,
            Span::dummy(),
        );
        checker.env.define(fn_sym).unwrap();

        // Wrong arity
        let call_wrong_arity = expr(ExprKind::Call {
            callee: Box::new(expr(ExprKind::Ident("add".to_string()))),
            args: vec![CallArg {
                name: None,
                value: expr(ExprKind::Literal(Literal::Int(1))),
                span: Span::dummy(),
            }],
        });
        checker.infer_expr(&call_wrong_arity);
        assert_eq!(checker.errors.len(), 1);
        assert_eq!(checker.errors[0].code(), "E4004");

        // Wrong argument type
        let call_wrong_type = expr(ExprKind::Call {
            callee: Box::new(expr(ExprKind::Ident("add".to_string()))),
            args: vec![
                CallArg {
                    name: None,
                    value: expr(ExprKind::Literal(Literal::Int(1))),
                    span: Span::dummy(),
                },
                CallArg {
                    name: None,
                    value: expr(ExprKind::Literal(Literal::String("bad".to_string()))),
                    span: Span::dummy(),
                },
            ],
        });
        checker.infer_expr(&call_wrong_type);
        assert_eq!(checker.errors.len(), 2);
        assert_eq!(checker.errors[1].code(), "E4004");
    }

    #[test]
    fn test_closure_checking_against_fn_type() {
        let mut checker = TypeChecker::new(None, "<test>");
        let closure = expr(ExprKind::Closure {
            params: vec!["n".to_string()],
            body: BlockExpr {
                stmts: vec![expr(ExprKind::Binary {
                    op: BinaryOp::Add,
                    left: Box::new(expr(ExprKind::Ident("n".to_string()))),
                    right: Box::new(expr(ExprKind::Literal(Literal::Int(1)))),
                })],
                span: Span::dummy(),
            },
        });

        let expected_fn = Type::Fn {
            is_compose: false,
            params: vec![Type::Int],
            ret: Box::new(Type::Int),
        };

        let checked = checker.check_expr(&closure, &expected_fn);
        assert_eq!(checked, expected_fn);
        assert!(checker.errors.is_empty());
    }

    #[test]
    fn test_when_diverging_branch_with_nothing() {
        let mut checker = TypeChecker::new(None, "<test>");
        checker.in_function = true;
        checker.current_expected_return = Some(Type::Int);
        let when_expr = expr(ExprKind::When {
            target: Some(Box::new(expr(ExprKind::Literal(Literal::Int(0))))),
            branches: vec![
                WhenBranch {
                    patterns: vec![WhenPattern::Expr(expr(ExprKind::Literal(Literal::Int(0))))],
                    body: expr(ExprKind::Return(Some(Box::new(expr(ExprKind::Literal(
                        Literal::Int(99),
                    )))))),
                    span: Span::dummy(),
                },
                WhenBranch {
                    patterns: vec![WhenPattern::Else],
                    body: expr(ExprKind::Literal(Literal::Int(42))),
                    span: Span::dummy(),
                },
            ],
        });

        // The 'return' branch has Nothing, so unified type is Int
        let ty = checker.infer_expr(&when_expr);
        assert_eq!(ty, Type::Int);
        assert!(checker.errors.is_empty());
    }

    #[test]
    fn test_struct_instantiation_and_field_access() {
        let mut checker = TypeChecker::new(None, "<test>");
        declare_struct(
            &mut checker,
            "Point",
            vec![
                ("x".to_string(), false, Type::Int),
                ("y".to_string(), true, Type::Int),
            ],
        );

        // Valid positional instantiation
        let call_pos = expr(ExprKind::Call {
            callee: Box::new(expr(ExprKind::Ident("Point".to_string()))),
            args: vec![
                CallArg {
                    name: None,
                    value: expr(ExprKind::Literal(Literal::Int(10))),
                    span: Span::dummy(),
                },
                CallArg {
                    name: None,
                    value: expr(ExprKind::Literal(Literal::Int(20))),
                    span: Span::dummy(),
                },
            ],
        });
        let res_pos = checker.infer_expr(&call_pos);
        assert!(matches!(res_pos, Type::Struct(ref id) if id.name == "Point"));

        // Valid named field instantiation
        let call_named = expr(ExprKind::Call {
            callee: Box::new(expr(ExprKind::Ident("Point".to_string()))),
            args: vec![
                CallArg {
                    name: Some("x".to_string()),
                    value: expr(ExprKind::Literal(Literal::Int(10))),
                    span: Span::dummy(),
                },
                CallArg {
                    name: Some("y".to_string()),
                    value: expr(ExprKind::Literal(Literal::Int(20))),
                    span: Span::dummy(),
                },
            ],
        });
        let res_named = checker.infer_expr(&call_named);
        assert!(matches!(res_named, Type::Struct(ref id) if id.name == "Point"));

        // Field access: p.x -> Int
        let field_access = expr(ExprKind::MemberAccess {
            target: Box::new(call_pos),
            member: "x".to_string(),
            is_safe_nav: false,
        });
        assert_eq!(checker.infer_expr(&field_access), Type::Int);

        // Unknown field access: p.z -> E4006
        let unknown_field = expr(ExprKind::MemberAccess {
            target: Box::new(expr(ExprKind::Ident("Point".to_string()))),
            member: "z".to_string(),
            is_safe_nav: false,
        });
        assert_eq!(checker.infer_expr(&unknown_field), Type::Error);
        assert!(checker.errors.iter().any(|e| e.code() == "E4006"));
    }

    #[test]
    fn test_struct_field_mutability_validation_e4003() {
        let mut checker = TypeChecker::new(None, "<test>");
        declare_struct(
            &mut checker,
            "User",
            vec![
                ("id".to_string(), false, Type::Int),
                ("age".to_string(), true, Type::Int),
            ],
        );

        // val u = User(1, 20)
        let init_val = expr(ExprKind::LocalVar {
            is_var: false,
            name: "u".to_string(),
            ty: None,
            initializer: Box::new(expr(ExprKind::Call {
                callee: Box::new(expr(ExprKind::Ident("User".to_string()))),
                args: vec![
                    CallArg {
                        name: None,
                        value: expr(ExprKind::Literal(Literal::Int(1))),
                        span: Span::dummy(),
                    },
                    CallArg {
                        name: None,
                        value: expr(ExprKind::Literal(Literal::Int(20))),
                        span: Span::dummy(),
                    },
                ],
            })),
        });
        checker.infer_expr(&init_val);

        // u.id = 2 -> E4003 (immutable field)
        let assign_val_field = expr(ExprKind::Assign {
            target: Box::new(expr(ExprKind::MemberAccess {
                target: Box::new(expr(ExprKind::Ident("u".to_string()))),
                member: "id".to_string(),
                is_safe_nav: false,
            })),
            value: Box::new(expr(ExprKind::Literal(Literal::Int(2)))),
        });
        checker.infer_expr(&assign_val_field);
        assert!(checker.errors.iter().any(|e| e.code() == "E4003"));

        // u.age = 21 -> E4003 (immutable binding 'u')
        let assign_val_binding = expr(ExprKind::Assign {
            target: Box::new(expr(ExprKind::MemberAccess {
                target: Box::new(expr(ExprKind::Ident("u".to_string()))),
                member: "age".to_string(),
                is_safe_nav: false,
            })),
            value: Box::new(expr(ExprKind::Literal(Literal::Int(21)))),
        });
        checker.infer_expr(&assign_val_binding);
        assert_eq!(
            checker
                .errors
                .iter()
                .filter(|e| e.code() == "E4003")
                .count(),
            2
        );

        // var u2 = User(2, 30)
        let init_var = expr(ExprKind::LocalVar {
            is_var: true,
            name: "u2".to_string(),
            ty: None,
            initializer: Box::new(expr(ExprKind::Call {
                callee: Box::new(expr(ExprKind::Ident("User".to_string()))),
                args: vec![
                    CallArg {
                        name: None,
                        value: expr(ExprKind::Literal(Literal::Int(2))),
                        span: Span::dummy(),
                    },
                    CallArg {
                        name: None,
                        value: expr(ExprKind::Literal(Literal::Int(30))),
                        span: Span::dummy(),
                    },
                ],
            })),
        });
        checker.infer_expr(&init_var);

        // u2.age = 31 -> OK (mutable field on mutable var binding)
        let initial_err_count = checker.errors.len();
        let assign_var = expr(ExprKind::Assign {
            target: Box::new(expr(ExprKind::MemberAccess {
                target: Box::new(expr(ExprKind::Ident("u2".to_string()))),
                member: "age".to_string(),
                is_safe_nav: false,
            })),
            value: Box::new(expr(ExprKind::Literal(Literal::Int(31)))),
        });
        checker.infer_expr(&assign_var);
        assert_eq!(checker.errors.len(), initial_err_count);
    }

    #[test]
    fn test_enum_variant_constructor_typing() {
        let mut checker = TypeChecker::new(None, "<test>");
        declare_enum(
            &mut checker,
            "Option",
            vec![
                ("None".to_string(), Vec::new()),
                ("Some".to_string(), vec![Type::Int]),
            ],
        );

        // Qualified unit variant: Option.None -> Option
        let none_access = expr(ExprKind::MemberAccess {
            target: Box::new(expr(ExprKind::Ident("Option".to_string()))),
            member: "None".to_string(),
            is_safe_nav: false,
        });
        let res_none = checker.infer_expr(&none_access);
        assert!(matches!(res_none, Type::Enum(ref id) if id.name == "Option"));

        // Qualified variant constructor call: Option.Some(42) -> Option
        let some_call = expr(ExprKind::Call {
            callee: Box::new(expr(ExprKind::MemberAccess {
                target: Box::new(expr(ExprKind::Ident("Option".to_string()))),
                member: "Some".to_string(),
                is_safe_nav: false,
            })),
            args: vec![CallArg {
                name: None,
                value: expr(ExprKind::Literal(Literal::Int(42))),
                span: Span::dummy(),
            }],
        });
        let res_some = checker.infer_expr(&some_call);
        assert!(matches!(res_some, Type::Enum(ref id) if id.name == "Option"));

        // Option.Some("bad") -> E4004 (argument type mismatch)
        let some_bad = expr(ExprKind::Call {
            callee: Box::new(expr(ExprKind::MemberAccess {
                target: Box::new(expr(ExprKind::Ident("Option".to_string()))),
                member: "Some".to_string(),
                is_safe_nav: false,
            })),
            args: vec![CallArg {
                name: None,
                value: expr(ExprKind::Literal(Literal::String("bad".to_string()))),
                span: Span::dummy(),
            }],
        });
        checker.infer_expr(&some_bad);
        assert!(checker.errors.iter().any(|e| e.code() == "E4004"));
    }

    #[test]
    fn test_reference_and_borrow_typing() {
        let mut checker = TypeChecker::new(None, "<test>");
        let init_val = expr(ExprKind::LocalVar {
            is_var: false,
            name: "x".to_string(),
            ty: None,
            initializer: Box::new(expr(ExprKind::Literal(Literal::Int(42)))),
        });
        checker.infer_expr(&init_val);

        // Shared borrow: &x -> &Int
        let shared_borrow = expr(ExprKind::Unary {
            op: UnaryOp::Borrow,
            expr: Box::new(expr(ExprKind::Ident("x".to_string()))),
        });
        assert_eq!(
            checker.infer_expr(&shared_borrow),
            Type::Reference {
                mutable: false,
                inner: Box::new(Type::Int),
            }
        );

        // Mutable borrow on val: &mut x -> E4003
        let mut_borrow_val = expr(ExprKind::Unary {
            op: UnaryOp::BorrowMut,
            expr: Box::new(expr(ExprKind::Ident("x".to_string()))),
        });
        checker.infer_expr(&mut_borrow_val);
        assert!(checker.errors.iter().any(|e| e.code() == "E4003"));

        // Mutable borrow on var: &mut y -> &mut Int
        let init_var = expr(ExprKind::LocalVar {
            is_var: true,
            name: "y".to_string(),
            ty: None,
            initializer: Box::new(expr(ExprKind::Literal(Literal::Int(100)))),
        });
        checker.infer_expr(&init_var);
        let mut_borrow_var = expr(ExprKind::Unary {
            op: UnaryOp::BorrowMut,
            expr: Box::new(expr(ExprKind::Ident("y".to_string()))),
        });
        assert_eq!(
            checker.infer_expr(&mut_borrow_var),
            Type::Reference {
                mutable: true,
                inner: Box::new(Type::Int),
            }
        );
    }

    #[test]
    fn test_nullability_safe_nav_and_elvis() {
        let mut checker = TypeChecker::new(None, "<test>");
        let point_ty = declare_struct(
            &mut checker,
            "Point",
            vec![("x".to_string(), false, Type::Int)],
        );
        let opt_point_ty = Type::Nullable(Box::new(point_ty));
        let init_opt = expr(ExprKind::LocalVar {
            is_var: false,
            name: "opt".to_string(),
            ty: None,
            initializer: Box::new(expr(ExprKind::Literal(Literal::Null))),
        });
        checker.infer_expr(&init_opt);
        checker.env.update_type("opt", opt_point_ty);

        // Safe nav: opt?.x -> Int?
        let safe_nav = expr(ExprKind::MemberAccess {
            target: Box::new(expr(ExprKind::Ident("opt".to_string()))),
            member: "x".to_string(),
            is_safe_nav: true,
        });
        assert_eq!(
            checker.infer_expr(&safe_nav),
            Type::Nullable(Box::new(Type::Int))
        );

        // Elvis: opt_int ?: 0 -> Int
        let elvis = expr(ExprKind::Binary {
            op: BinaryOp::Elvis,
            left: Box::new(expr(ExprKind::Literal(Literal::Null))),
            right: Box::new(expr(ExprKind::Literal(Literal::Int(0)))),
        });
        assert_eq!(checker.infer_expr(&elvis), Type::Int);
    }

    #[test]
    fn test_heap_indirection_field_access() {
        let mut checker = TypeChecker::new(None, "<test>");
        let point_ty = Type::Heap(Box::new(declare_struct(
            &mut checker,
            "Point",
            vec![("x".to_string(), false, Type::Int)],
        )));
        checker
            .env
            .define(Symbol::variable(
                "h",
                point_ty,
                Mutability::Val,
                Span::dummy(),
            ))
            .unwrap();

        // Transparent dereference on Heap<Point>: h.x -> Int
        let field_access = expr(ExprKind::MemberAccess {
            target: Box::new(expr(ExprKind::Ident("h".to_string()))),
            member: "x".to_string(),
            is_safe_nav: false,
        });
        assert_eq!(checker.infer_expr(&field_access), Type::Int);
    }

    #[test]
    fn test_cross_module_abi_ingestion_and_package_visibility() {
        // ABI defined in package 'core.math'
        let abi = FileAbi {
            package: Some("core.math".to_string()),
            module: "core/math/math".to_string(),
            symbols: vec![
                ExportedSymbol::Function {
                    visibility: Visibility::Public,
                    is_compose: false,
                    name: "add".to_string(),
                    type_params: Vec::new(),
                    params: vec![
                        ("a".to_string(), AbiType::Prim(PrimKind::Int), None),
                        ("b".to_string(), AbiType::Prim(PrimKind::Int), None),
                    ],
                    return_type: AbiType::Prim(PrimKind::Int),
                    generic_body_hash: None,
                },
                ExportedSymbol::Function {
                    visibility: Visibility::Package,
                    is_compose: false,
                    name: "internal_calc".to_string(),
                    type_params: Vec::new(),
                    params: Vec::new(),
                    return_type: AbiType::Prim(PrimKind::Int),
                    generic_body_hash: None,
                },
            ],
            abi_hash: 12345,
        };

        // Case 1: Ingested from external package 'app.ui'
        let mut ext_checker = TypeChecker::new(Some("app.ui".to_string()), "<test>");
        ext_checker.ingest_file_abi(&abi);

        // Calling public 'add' -> OK
        let call_pub = expr(ExprKind::Call {
            callee: Box::new(expr(ExprKind::Ident("add".to_string()))),
            args: vec![
                CallArg {
                    name: None,
                    value: expr(ExprKind::Literal(Literal::Int(1))),
                    span: Span::dummy(),
                },
                CallArg {
                    name: None,
                    value: expr(ExprKind::Literal(Literal::Int(2))),
                    span: Span::dummy(),
                },
            ],
        });
        assert_eq!(ext_checker.infer_expr(&call_pub), Type::Int);
        assert!(ext_checker.errors.is_empty());

        // Calling package-visible 'internal_calc' from external package -> E4007
        let call_pkg = expr(ExprKind::Call {
            callee: Box::new(expr(ExprKind::Ident("internal_calc".to_string()))),
            args: Vec::new(),
        });
        ext_checker.infer_expr(&call_pkg);
        assert!(ext_checker.errors.iter().any(|e| e.code() == "E4007"));

        // Case 2: Ingested from identical package 'core.math'
        let mut same_checker = TypeChecker::new(Some("core.math".to_string()), "<test>");
        same_checker.ingest_file_abi(&abi);
        assert_eq!(same_checker.infer_expr(&call_pkg), Type::Int);
        assert!(same_checker.errors.is_empty());
    }

    #[test]
    fn test_default_param_omission_and_named_args() {
        let mut checker = TypeChecker::new(None, "<test>");
        let fn_ty = Type::Fn {
            is_compose: false,
            params: vec![Type::String, Type::Int, Type::String],
            ret: Box::new(Type::String),
        };
        let sym = Symbol::function_with_defaults(
            "greet",
            fn_ty,
            vec!["msg".to_string(), "count".to_string(), "prefix".to_string()],
            vec![false, true, true],
            Visibility::Public,
            None,
            Span::dummy(),
        );
        checker.env.define(sym).unwrap();

        // 1. Omit default parameters: greet("hello") -> OK
        let call_one = expr(ExprKind::Call {
            callee: Box::new(expr(ExprKind::Ident("greet".to_string()))),
            args: vec![CallArg {
                name: None,
                value: expr(ExprKind::Literal(Literal::String("hello".to_string()))),
                span: Span::dummy(),
            }],
        });
        assert_eq!(checker.infer_expr(&call_one), Type::String);
        assert!(checker.errors.is_empty());

        // 2. Named arguments: greet(msg = "hi", prefix = ">>") -> OK
        let call_named = expr(ExprKind::Call {
            callee: Box::new(expr(ExprKind::Ident("greet".to_string()))),
            args: vec![
                CallArg {
                    name: Some("msg".to_string()),
                    value: expr(ExprKind::Literal(Literal::String("hi".to_string()))),
                    span: Span::dummy(),
                },
                CallArg {
                    name: Some("prefix".to_string()),
                    value: expr(ExprKind::Literal(Literal::String(">>".to_string()))),
                    span: Span::dummy(),
                },
            ],
        });
        assert_eq!(checker.infer_expr(&call_named), Type::String);
        assert!(checker.errors.is_empty());

        // 3. Duplicate named argument -> E4004
        let call_dup = expr(ExprKind::Call {
            callee: Box::new(expr(ExprKind::Ident("greet".to_string()))),
            args: vec![
                CallArg {
                    name: Some("msg".to_string()),
                    value: expr(ExprKind::Literal(Literal::String("a".to_string()))),
                    span: Span::dummy(),
                },
                CallArg {
                    name: Some("msg".to_string()),
                    value: expr(ExprKind::Literal(Literal::String("b".to_string()))),
                    span: Span::dummy(),
                },
            ],
        });
        checker.infer_expr(&call_dup);
        assert!(checker.errors.iter().any(|e| e.code() == "E4004"));
        checker.errors.clear();

        // 4. Missing required parameter -> E4004
        let call_missing = expr(ExprKind::Call {
            callee: Box::new(expr(ExprKind::Ident("greet".to_string()))),
            args: vec![CallArg {
                name: Some("count".to_string()),
                value: expr(ExprKind::Literal(Literal::Int(2))),
                span: Span::dummy(),
            }],
        });
        checker.infer_expr(&call_missing);
        assert!(checker.errors.iter().any(|e| e.code() == "E4004"));
    }

    #[test]
    fn test_struct_named_args_and_validation() {
        let mut checker = TypeChecker::new(None, "<test>");
        declare_struct(
            &mut checker,
            "Point",
            vec![
                ("x".to_string(), false, Type::Int),
                ("y".to_string(), false, Type::Int),
            ],
        );

        // Valid named: Point(x = 1, y = 2) -> OK
        let call_ok = expr(ExprKind::Call {
            callee: Box::new(expr(ExprKind::Ident("Point".to_string()))),
            args: vec![
                CallArg {
                    name: Some("x".to_string()),
                    value: expr(ExprKind::Literal(Literal::Int(1))),
                    span: Span::dummy(),
                },
                CallArg {
                    name: Some("y".to_string()),
                    value: expr(ExprKind::Literal(Literal::Int(2))),
                    span: Span::dummy(),
                },
            ],
        });
        checker.infer_expr(&call_ok);
        assert!(checker.errors.is_empty());

        // Duplicate field: Point(x = 1, x = 2) -> E4004
        let call_dup = expr(ExprKind::Call {
            callee: Box::new(expr(ExprKind::Ident("Point".to_string()))),
            args: vec![
                CallArg {
                    name: Some("x".to_string()),
                    value: expr(ExprKind::Literal(Literal::Int(1))),
                    span: Span::dummy(),
                },
                CallArg {
                    name: Some("x".to_string()),
                    value: expr(ExprKind::Literal(Literal::Int(2))),
                    span: Span::dummy(),
                },
            ],
        });
        checker.infer_expr(&call_dup);
        assert!(checker.errors.iter().any(|e| e.code() == "E4004"));
        checker.errors.clear();

        // Missing field: Point(x = 1) -> E4004
        let call_missing = expr(ExprKind::Call {
            callee: Box::new(expr(ExprKind::Ident("Point".to_string()))),
            args: vec![CallArg {
                name: Some("x".to_string()),
                value: expr(ExprKind::Literal(Literal::Int(1))),
                span: Span::dummy(),
            }],
        });
        checker.infer_expr(&call_missing);
        assert!(checker.errors.iter().any(|e| e.code() == "E4004"));
    }

    #[test]
    fn test_exhaustive_enum_when_without_else() {
        let mut checker = TypeChecker::new(None, "<test>");
        let color_ty = declare_enum(
            &mut checker,
            "Color",
            vec![
                ("Red".to_string(), Vec::new()),
                ("Green".to_string(), Vec::new()),
                ("Blue".to_string(), Vec::new()),
            ],
        );
        checker
            .env
            .define(Symbol::variable(
                "c",
                color_ty.clone(),
                Mutability::Val,
                Span::dummy(),
            ))
            .unwrap();
        checker
            .env
            .define(Symbol::top_level_variable(
                "Red",
                color_ty.clone(),
                Mutability::Val,
                Visibility::Public,
                None,
                Span::dummy(),
            ))
            .unwrap();
        checker
            .env
            .define(Symbol::top_level_variable(
                "Green",
                color_ty.clone(),
                Mutability::Val,
                Visibility::Public,
                None,
                Span::dummy(),
            ))
            .unwrap();
        checker
            .env
            .define(Symbol::top_level_variable(
                "Blue",
                color_ty.clone(),
                Mutability::Val,
                Visibility::Public,
                None,
                Span::dummy(),
            ))
            .unwrap();

        // Exhaustive: covers Red, Green, Blue without else -> OK
        let when_exhaustive = expr(ExprKind::When {
            target: Some(Box::new(expr(ExprKind::Ident("c".to_string())))),
            branches: vec![
                WhenBranch {
                    patterns: vec![WhenPattern::Expr(expr(ExprKind::Ident("Red".to_string())))],
                    body: expr(ExprKind::Literal(Literal::Int(1))),
                    span: Span::dummy(),
                },
                WhenBranch {
                    patterns: vec![WhenPattern::Expr(expr(ExprKind::Ident(
                        "Green".to_string(),
                    )))],
                    body: expr(ExprKind::Literal(Literal::Int(2))),
                    span: Span::dummy(),
                },
                WhenBranch {
                    patterns: vec![WhenPattern::Expr(expr(ExprKind::Ident("Blue".to_string())))],
                    body: expr(ExprKind::Literal(Literal::Int(3))),
                    span: Span::dummy(),
                },
            ],
        });
        let ty = checker.infer_expr(&when_exhaustive);
        assert_eq!(ty, Type::Int);
        assert!(checker.errors.is_empty());

        // Incomplete: covers only Red, Green without else -> E4009
        let when_incomplete = expr(ExprKind::When {
            target: Some(Box::new(expr(ExprKind::Ident("c".to_string())))),
            branches: vec![
                WhenBranch {
                    patterns: vec![WhenPattern::Expr(expr(ExprKind::Ident("Red".to_string())))],
                    body: expr(ExprKind::Literal(Literal::Int(1))),
                    span: Span::dummy(),
                },
                WhenBranch {
                    patterns: vec![WhenPattern::Expr(expr(ExprKind::Ident(
                        "Green".to_string(),
                    )))],
                    body: expr(ExprKind::Literal(Literal::Int(2))),
                    span: Span::dummy(),
                },
            ],
        });
        checker.infer_expr(&when_incomplete);
        assert!(checker.errors.iter().any(|e| e.code() == "E4009"));
    }

    #[test]
    fn test_top_level_return_rejected() {
        let mut checker = TypeChecker::new(None, "<test>");
        let ret_expr = expr(ExprKind::Return(Some(Box::new(expr(ExprKind::Literal(
            Literal::Int(42),
        ))))));
        checker.infer_expr(&ret_expr);
        assert!(checker.errors.iter().any(|e| e.code() == "E4005"));
    }

    #[test]
    fn test_compose_isolation_across_closure() {
        let mut checker = TypeChecker::new(None, "<test>");
        let comp_fn = Symbol::function(
            "MyView",
            Type::Fn {
                is_compose: true,
                params: Vec::new(),
                ret: Box::new(Type::Unit),
            },
            Visibility::Public,
            None,
            Span::dummy(),
        );
        checker.env.define(comp_fn).unwrap();

        // Enclosing context is composable
        checker.push_compose_context(true);

        // A standard (non-composable) closure: { MyView() }
        let closure = expr(ExprKind::Closure {
            params: Vec::new(),
            body: BlockExpr {
                stmts: vec![expr(ExprKind::Call {
                    callee: Box::new(expr(ExprKind::Ident("MyView".to_string()))),
                    args: Vec::new(),
                })],
                span: Span::dummy(),
            },
        });

        // Inferring the closure should not leak the outer compose context -> emits E4008!
        checker.infer_expr(&closure);
        assert!(checker.errors.iter().any(|e| e.code() == "E4008"));
    }

    #[test]
    fn test_duplicate_struct_fields_and_enum_variants_rejected() {
        let mut checker = TypeChecker::new(None, "<test>");
        let source_file = SourceFile {
            package: None,
            imports: Vec::new(),
            declarations: vec![
                Declaration::Struct(prism_ast::StructDecl {
                    visibility: Visibility::Public,
                    name: "DupPoint".to_string(),
                    type_params: Vec::new(),
                    fields: vec![
                        prism_ast::StructField {
                            name: "x".to_string(),
                            is_var: false,
                            ty: prism_ast::Type::named("Int", Span::dummy()),
                            span: Span::dummy(),
                        },
                        prism_ast::StructField {
                            name: "x".to_string(),
                            is_var: false,
                            ty: prism_ast::Type::named("Int", Span::dummy()),
                            span: Span::dummy(),
                        },
                    ],
                    span: Span::dummy(),
                }),
                Declaration::Enum(prism_ast::EnumDecl {
                    visibility: Visibility::Public,
                    name: "DupEnum".to_string(),
                    type_params: Vec::new(),
                    variants: vec![
                        prism_ast::EnumVariant {
                            name: "Val".to_string(),
                            fields: Vec::new(),
                            span: Span::dummy(),
                        },
                        prism_ast::EnumVariant {
                            name: "Val".to_string(),
                            fields: Vec::new(),
                            span: Span::dummy(),
                        },
                    ],
                    span: Span::dummy(),
                }),
            ],
            span: Span::dummy(),
        };

        checker.register_module_declarations(&source_file);
        assert_eq!(
            checker
                .errors
                .iter()
                .filter(|e| e.code() == "E4010")
                .count(),
            3
        );
    }

    #[test]
    fn test_indexing_indirection_peeling_and_heap_rejection() {
        let mut checker = TypeChecker::new(None, "<test>");
        // Borrowed string: &String
        let ref_str_ty = Type::Reference {
            mutable: false,
            inner: Box::new(Type::String),
        };
        checker
            .env
            .define(Symbol::variable(
                "s",
                ref_str_ty,
                Mutability::Val,
                Span::dummy(),
            ))
            .unwrap();

        let index_expr = expr(ExprKind::Index {
            target: Box::new(expr(ExprKind::Ident("s".to_string()))),
            index: Box::new(expr(ExprKind::Literal(Literal::Int(0)))),
        });
        assert_eq!(checker.infer_expr(&index_expr), Type::Char);
        assert!(checker.errors.is_empty());

        // Non-collection Heap: Heap<Int> -> cannot index -> E4002
        let heap_int_ty = Type::Heap(Box::new(Type::Int));
        checker
            .env
            .define(Symbol::variable(
                "h",
                heap_int_ty,
                Mutability::Val,
                Span::dummy(),
            ))
            .unwrap();
        let bad_index = expr(ExprKind::Index {
            target: Box::new(expr(ExprKind::Ident("h".to_string()))),
            index: Box::new(expr(ExprKind::Literal(Literal::Int(0)))),
        });
        checker.infer_expr(&bad_index);
        assert!(checker.errors.iter().any(|e| e.code() == "E4002"));
    }

    /// Parses and fully checks a Prism source file, returning the accumulated errors.
    fn check_source(src: &str) -> Vec<TypeError> {
        let file = prism_parser::parse_source(src).expect("source must parse");
        let mut checker =
            TypeChecker::new(file.package.as_ref().map(|p| p.package_string()), "<test>");
        checker.check_module(&file);
        checker.errors
    }

    fn error_codes(errors: &[TypeError]) -> Vec<&'static str> {
        errors.iter().map(|e| e.code()).collect()
    }

    const USER_STRUCT: &str = "struct User {\n    val id: Int\n    var name: String\n}\n";

    #[test]
    fn test_field_access_on_annotated_param_by_value() {
        let src = format!(
            "{}fun byValue(user: User): String = user.name\n",
            USER_STRUCT
        );
        assert!(check_source(&src).is_empty());
    }

    #[test]
    fn test_field_access_through_shared_reference() {
        let src = format!(
            "{}fun byRef(user: &User): String = user.name\nfun idOf(user: &User): Int = user.id\n",
            USER_STRUCT
        );
        assert!(check_source(&src).is_empty());
    }

    #[test]
    fn test_field_assignment_through_mutable_reference() {
        let ok = format!(
            "{}fun byMut(user: &mut User, n: String): Unit {{\n    user.name = n\n}}\n",
            USER_STRUCT
        );
        assert!(check_source(&ok).is_empty());

        // Assigning to a `val` field through `&mut` is still an immutability violation.
        let val_field = format!(
            "{}fun setId(user: &mut User): Unit {{\n    user.id = 1\n}}\n",
            USER_STRUCT
        );
        assert_eq!(error_codes(&check_source(&val_field)), vec!["E4003"]);

        // Mutable borrow of a `val` field resolves the struct and is rejected.
        let borrow_val = format!(
            "{}fun borrowId(user: &mut User): Unit {{\n    val r = &mut user.id\n}}\n",
            USER_STRUCT
        );
        assert_eq!(error_codes(&check_source(&borrow_val)), vec!["E4003"]);

        // Assigning a mismatched value type to a resolved field is rejected.
        let mismatch = format!(
            "{}fun setName(user: &mut User): Unit {{\n    user.name = 42\n}}\n",
            USER_STRUCT
        );
        assert_eq!(error_codes(&check_source(&mismatch)), vec!["E4002"]);
    }

    #[test]
    fn test_safe_navigation_on_annotated_nullable_struct() {
        let ok = format!(
            "{}fun nameOf(user: User?): String? = user?.name\n",
            USER_STRUCT
        );
        assert!(check_source(&ok).is_empty());

        // `user?.name` is `String?`, so it is not assignable to the `String` return type.
        let not_null = format!(
            "{}fun nameOf(user: User?): String = user?.name\n",
            USER_STRUCT
        );
        assert_eq!(error_codes(&check_source(&not_null)), vec!["E4005"]);
    }

    #[test]
    fn test_field_access_on_annotated_local_val() {
        let src = "struct Point {\n    val x: Int\n    val y: Int\n}\nfun main(): Int {\n    val p: Point = Point(1, 2)\n    return p.x + p.y\n}\n";
        assert!(check_source(src).is_empty());
    }

    #[test]
    fn test_chained_field_access_through_struct_typed_field() {
        // `Order` references `User` before it is declared (forward reference in Pass 1).
        let src = format!(
            "struct Order {{\n    val user: User\n}}\n{}fun buyer(order: &Order): String = order.user.name\n",
            USER_STRUCT
        );
        assert!(check_source(&src).is_empty());
    }

    #[test]
    fn test_enum_typed_param_when_exhaustiveness() {
        let enum_decl = "enum Status {\n    Active,\n    Inactive\n}\n";
        let exhaustive = format!(
            "{}fun describe(s: Status): Int = when (s) {{\n    Active -> 1,\n    Inactive -> 0\n}}\n",
            enum_decl
        );
        assert!(check_source(&exhaustive).is_empty());

        let partial = format!(
            "{}fun describe(s: Status): Int = when (s) {{\n    Active -> 1\n}}\n",
            enum_decl
        );
        assert_eq!(error_codes(&check_source(&partial)), vec!["E4009"]);
    }

    #[test]
    fn test_unknown_nominal_type_stays_opaque() {
        // The annotation itself is accepted; member access on the opaque type is E4006.
        assert!(check_source("fun id(x: Missing): Missing = x\n").is_empty());
        assert_eq!(
            error_codes(&check_source("fun size(x: Missing): Int = x.size\n")),
            vec!["E4006"]
        );

        // Unknown fields on a resolved struct are still rejected.
        let src = format!("{}fun bad(user: User): Int = user.age\n", USER_STRUCT);
        assert_eq!(error_codes(&check_source(&src)), vec!["E4006"]);
    }

    #[test]
    fn test_field_access_on_struct_imported_from_abi() {
        let models = prism_parser::parse_source(
            "package demo.models\npublic struct Address {\n    val city: String\n}\npublic struct User {\n    var name: String\n    val email: String?\n    val home: Address\n}\n",
        )
        .expect("models must parse");
        let abi = FileAbi::extract(&models, "demo/models/models", &prism_abi::no_imports)
            .expect("models ABI must extract");

        let app = prism_parser::parse_source(
            "package demo.app\nfun nameOf(user: &User): String = user.name\nfun rename(user: &mut User, n: String): Unit {\n    user.name = n\n}\nfun emailOf(user: &User): String? = user.email\nfun cityOf(user: User): String = user.home.city\n",
        )
        .expect("app must parse");
        let mut checker = TypeChecker::new(Some("demo.app".to_string()), "<test>");
        checker.ingest_file_abi(&abi);
        checker.check_module(&app);
        assert!(checker.errors.is_empty(), "{:?}", checker.errors);
    }

    #[test]
    fn test_exported_function_missing_return_type_e4011() {
        for src in [
            "public fun greet(name: String) {\n}\n",
            "package fun helper(x: Int) {\n}\n",
        ] {
            assert_eq!(error_codes(&check_source(src)), vec!["E4011"], "{}", src);
        }
        // Private functions may still omit the return type.
        assert!(check_source("fun local(x: Int) {\n}\n").is_empty());
    }
}

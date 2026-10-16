//! Main TypeChecker implementation: Pass 1 declaration registration and Pass 2 body checking.

use crate::error::TypeError;
use crate::scope::{Mutability, ScopeStack, Symbol, SymbolKind};
use crate::types::Type;
use prism_abi::{ExportedSymbol, FileAbi};
use prism_ast::decl::{Declaration, FunctionBody, FunctionDecl};
use prism_ast::expr::{
    BinaryOp, BlockExpr, CallArg, Expr, ExprKind, Literal, UnaryOp, WhenBranch, WhenPattern,
};
use prism_ast::module::SourceFile;
use prism_ast::{Span, Visibility};

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

fn extract_variant_name(p_expr: &Expr) -> Option<&str> {
    match &p_expr.kind {
        ExprKind::Ident(name) => Some(name.as_str()),
        ExprKind::MemberAccess { member, .. } => Some(member.as_str()),
        ExprKind::Call { callee, .. } => match &callee.kind {
            ExprKind::Ident(name) => Some(name.as_str()),
            ExprKind::MemberAccess { member, .. } => Some(member.as_str()),
            _ => None,
        },
        _ => None,
    }
}

/// Static type checker and semantic analyzer for Prism source modules.
pub struct TypeChecker {
    /// Lexical scope environment.
    pub env: ScopeStack,
    /// Canonical package path of the current file (e.g. `Some("app.ui")`).
    pub current_package: Option<String>,
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
}

impl TypeChecker {
    /// Creates a new `TypeChecker` for a file in the given package.
    pub fn new(current_package: Option<String>) -> Self {
        Self {
            env: ScopeStack::new(),
            current_package,
            errors: Vec::new(),
            in_compose_stack: vec![false],
            current_expected_return: None,
            unannotated_private_fns: Vec::new(),
            in_function: false,
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

    /// Resolves a nominal type reference lowered from an annotation or ABI type string
    /// (a field-less `Type::Struct { name }`) to the struct or enum declared under that name.
    ///
    /// Types that already carry members, and names with no struct/enum declaration in scope
    /// (opaque or generic type names), are returned unchanged.
    pub fn resolve_nominal(&self, ty: &Type) -> Type {
        if let Type::Struct { name, fields } = ty {
            if fields.is_empty() {
                if let Some(sym) = self.env.lookup_type(name) {
                    return sym.ty();
                }
            }
        }
        ty.clone()
    }

    /// Ingests exported symbols from an imported file's Phase 1 FileAbi.
    pub fn ingest_file_abi(&mut self, abi: &FileAbi) {
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
                        .map(|(_, ty_str, _)| Type::parse_str(ty_str))
                        .collect();
                    let param_names = params.iter().map(|(n, _, _)| n.clone()).collect();
                    let has_default = params.iter().map(|(_, _, d)| d.is_some()).collect();
                    let ret_ty = Type::parse_str(return_type);
                    let fn_ty = Type::Fn {
                        is_compose: *is_compose,
                        params: param_types,
                        ret: Box::new(ret_ty),
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
                    self.env.define(symbol).ok();
                }
                ExportedSymbol::Struct {
                    visibility,
                    name,
                    fields,
                    ..
                } => {
                    let field_types = fields
                        .iter()
                        .map(|(f_name, is_var, f_ty)| {
                            (f_name.clone(), *is_var, Type::parse_str(f_ty))
                        })
                        .collect();
                    let symbol = Symbol::struct_decl(
                        name.clone(),
                        field_types,
                        *visibility,
                        abi.package.clone(),
                        Span::dummy(),
                    );
                    self.env.define(symbol).ok();
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
                            (
                                v_name.clone(),
                                f_types.iter().map(|s| Type::parse_str(s)).collect(),
                            )
                        })
                        .collect();
                    let enum_ty = Type::Enum {
                        name: name.clone(),
                        variants: variant_types.clone(),
                    };
                    let symbol = Symbol::enum_decl(
                        name.clone(),
                        variant_types.clone(),
                        *visibility,
                        abi.package.clone(),
                        Span::dummy(),
                    );
                    self.env.define(symbol).ok();

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
                        self.env.define(v_sym).ok();
                    }
                }
                ExportedSymbol::Var {
                    visibility,
                    name,
                    is_var,
                    ty,
                } => {
                    let var_ty = Type::parse_str(ty);
                    let mutability = if *is_var {
                        Mutability::Var
                    } else {
                        Mutability::Val
                    };
                    let symbol = Symbol::top_level_variable(
                        name.clone(),
                        var_ty,
                        mutability,
                        *visibility,
                        abi.package.clone(),
                        Span::dummy(),
                    );
                    self.env.define(symbol).ok();
                }
            }
        }
    }

    /// Pass 1: Registers all top-level symbols (functions, structs, enums, variables) in module scope.
    ///
    /// Enables forward references so functions can call sibling functions declared later in the file.
    pub fn register_module_declarations(&mut self, source_file: &SourceFile) {
        for decl in &source_file.declarations {
            match decl {
                Declaration::Struct(s) => {
                    let mut seen_fields = std::collections::HashSet::new();
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
                    let fields = s
                        .fields
                        .iter()
                        .map(|f| (f.name.clone(), f.is_var, Type::from_ast(&f.ty)))
                        .collect();
                    let sym = Symbol::struct_decl(
                        s.name.clone(),
                        fields,
                        s.visibility,
                        self.current_package.clone(),
                        s.span,
                    );
                    if let Err(err) = self.env.define(sym) {
                        self.errors.push(err);
                    }
                }
                Declaration::Enum(e) => {
                    let mut seen_variants = std::collections::HashSet::new();
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
                    let variants: Vec<(String, Vec<Type>)> = e
                        .variants
                        .iter()
                        .map(|v| {
                            (
                                v.name.clone(),
                                v.fields.iter().map(Type::from_ast).collect(),
                            )
                        })
                        .collect();
                    let enum_ty = Type::Enum {
                        name: e.name.clone(),
                        variants: variants.clone(),
                    };
                    let sym = Symbol::enum_decl(
                        e.name.clone(),
                        variants.clone(),
                        e.visibility,
                        self.current_package.clone(),
                        e.span,
                    );
                    if let Err(err) = self.env.define(sym) {
                        self.errors.push(err);
                    }

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
                        if let Err(err) = self.env.define(v_sym) {
                            self.errors.push(err);
                        }
                    }
                }
                Declaration::Function(f) => {
                    self.register_function_decl(f);
                }
                Declaration::Var(v) => {
                    let ty = if let Some(annot) = &v.ty {
                        Type::from_ast(annot)
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
                    if let Err(err) = self.env.define(sym) {
                        self.errors.push(err);
                    }
                }
            }
        }
    }

    fn register_function_decl(&mut self, f: &FunctionDecl) {
        let params: Vec<Type> = f.params.iter().map(|p| Type::from_ast(&p.ty)).collect();
        let param_names: Vec<String> = f.params.iter().map(|p| p.name.clone()).collect();
        let has_default: Vec<bool> = f.params.iter().map(|p| p.default_value.is_some()).collect();

        let ret = if let Some(annot) = &f.return_type {
            Type::from_ast(annot)
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
        if let Err(err) = self.env.define(sym) {
            self.errors.push(err);
        }
    }

    /// Pass 2: Typechecks the bodies of all top-level declarations in the module.
    pub fn check_module(&mut self, source_file: &SourceFile) {
        self.register_module_declarations(source_file);

        for decl in &source_file.declarations {
            match decl {
                Declaration::Function(f) => {
                    self.check_function_body(f);
                }
                Declaration::Var(v) => {
                    let inferred = self.infer_expr(&v.initializer);
                    if let Some(annot) = &v.ty {
                        let expected = Type::from_ast(annot);
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
                _ => {}
            }
        }
    }

    /// Typechecks an individual function declaration body.
    pub fn check_function_body(&mut self, f: &FunctionDecl) {
        let prev_in_func = self.in_function;
        self.in_function = true;
        self.env.enter_scope();
        self.push_compose_context(f.is_compose);

        // Bind parameters in local scope
        for p in &f.params {
            let p_ty = Type::from_ast(&p.ty);
            let sym = Symbol::variable(p.name.clone(), p_ty, Mutability::Val, p.span);
            if let Err(err) = self.env.define(sym) {
                self.errors.push(err);
            }
        }

        let declared_ret = f.return_type.as_ref().map(Type::from_ast).or({
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
                    let block_ty = self.check_block(block, declared_ret.as_ref());
                    if let Some(expected) = &declared_ret {
                        // For block bodies returning Unit, trailing expressions are discarded.
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
                        self.check_expr(expr, expected)
                    } else {
                        self.infer_expr(expr)
                    };

                    if declared_ret.is_none() {
                        self.env.update_type(
                            &f.name,
                            Type::Fn {
                                is_compose: f.is_compose,
                                params: f.params.iter().map(|p| Type::from_ast(&p.ty)).collect(),
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
                self.errors.push(TypeError::TypeMismatch {
                    expected: expected.clone(),
                    found: Type::Nullable(Box::new(Type::Nothing)),
                    span: expr.span,
                });
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

                let body_ty = self.check_expr(body, exp_ret);

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
            self.check_expr(cond, &Type::Bool);
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

        // Default: infer and verify assignability
        let inferred = self.infer_expr(expr);
        if !inferred.is_assignable_to(expected) {
            if inferred != Type::Error && expected != &Type::Error {
                self.errors.push(TypeError::TypeMismatch {
                    expected: expected.clone(),
                    found: inferred.clone(),
                    span: expr.span,
                });
            }
            return Type::Error;
        }
        inferred
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

            ExprKind::Ident(name) => match self.env.resolve(name, expr.span) {
                Ok(sym) => {
                    // Check visibility across packages (single-file compilation unit invariant, E4007)
                    if sym.visibility() == Visibility::Package {
                        let def_pkg = sym.defining_package().unwrap_or("");
                        let curr_pkg = self.current_package.as_deref().unwrap_or("");
                        if def_pkg != curr_pkg {
                            self.errors.push(TypeError::VisibilityViolation {
                                symbol: name.clone(),
                                defining_package: def_pkg.to_string(),
                                accessing_package: curr_pkg.to_string(),
                                span: expr.span,
                            });
                            return Type::Error;
                        }
                    }
                    sym.ty()
                }
                Err(err) => {
                    self.errors.push(err);
                    Type::Error
                }
            },

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
                    let expected = Type::from_ast(a);
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
                self.check_expr(cond, &Type::Bool);
                let then_ty = self.infer_expr(then_branch);
                if let Some(else_b) = else_branch {
                    let else_ty = self.infer_expr(else_b);
                    if then_ty == Type::Nothing {
                        else_ty
                    } else if else_ty == Type::Nothing {
                        then_ty
                    } else if then_ty.is_assignable_to(&else_ty) {
                        else_ty
                    } else if else_ty.is_assignable_to(&then_ty) {
                        then_ty
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
                self.infer_when_expr(target.as_deref(), branches, expr.span)
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
                        self.check_expr(e, exp);
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

                let mut param_types = Vec::new();
                for name in params {
                    let sym =
                        Symbol::variable(name.clone(), Type::Error, Mutability::Val, expr.span);
                    self.env.define(sym).ok();
                    param_types.push(Type::Error);
                }
                let body_ty = self.infer_expr(body);

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
                let left_ty = self.infer_expr(left);
                let right_ty = self.infer_expr(right);

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
                let left_ty = self.infer_expr(left);
                let right_ty = self.infer_expr(right);

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
                let left_ty = self.infer_expr(left);
                let right_ty = self.infer_expr(right);

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
                self.check_expr(left, &Type::Bool);
                self.check_expr(right, &Type::Bool);
                Type::Bool
            }

            BinaryOp::Elvis => {
                let left_ty = self.infer_expr(left);
                if let Type::Nullable(inner) = left_ty {
                    let right_ty = self.infer_expr(right);
                    if right_ty == Type::Nothing {
                        *inner
                    } else if *inner == Type::Nothing {
                        right_ty
                    } else if right_ty.is_assignable_to(&inner) {
                        *inner
                    } else if (*inner).is_assignable_to(&right_ty) {
                        right_ty
                    } else {
                        self.errors.push(TypeError::TypeMismatch {
                            expected: *inner.clone(),
                            found: right_ty,
                            span,
                        });
                        *inner
                    }
                } else {
                    left_ty
                }
            }

            BinaryOp::Range => {
                self.check_expr(left, &Type::Int);
                self.check_expr(right, &Type::Int);
                Type::Struct {
                    name: "Range".to_string(),
                    fields: vec![
                        ("start".to_string(), false, Type::Int),
                        ("end".to_string(), false, Type::Int),
                    ],
                }
            }
        }
    }

    fn infer_unary_op(&mut self, op: UnaryOp, inner: &Expr, span: Span) -> Type {
        match op {
            UnaryOp::Neg => {
                let ty = self.infer_expr(inner);
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
                self.check_expr(inner, &Type::Bool);
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
                let (base_ty, base_writable) = self.infer_place(base);
                let (field_ty, is_var) = self.member_type(&base_ty, member, false, target.span);
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

    fn check_block(&mut self, block: &BlockExpr, expected: Option<&Type>) -> Type {
        self.env.enter_scope();
        if block.stmts.is_empty() {
            self.env.exit_scope();
            return Type::Unit;
        }

        let num_stmts = block.stmts.len();
        for stmt in &block.stmts[..num_stmts - 1] {
            self.infer_expr(stmt);
        }

        let last_stmt = &block.stmts[num_stmts - 1];
        let result_ty = if let Some(exp) = expected {
            self.check_expr(last_stmt, exp)
        } else {
            self.infer_expr(last_stmt)
        };

        self.env.exit_scope();
        result_ty
    }

    fn infer_when_expr(
        &mut self,
        target: Option<&Expr>,
        branches: &[WhenBranch],
        span: Span,
    ) -> Type {
        let subject_ty = target.map(|t| self.infer_expr(t));
        let mut has_else = false;
        let mut unified_ty: Option<Type> = None;
        let mut matched_variants = std::collections::HashSet::new();

        for branch in branches {
            for pattern in &branch.patterns {
                match pattern {
                    WhenPattern::Else => {
                        has_else = true;
                    }
                    WhenPattern::Expr(p_expr) => {
                        if let Some(s_ty) = &subject_ty {
                            self.check_expr(p_expr, s_ty);
                            if let Some(v_name) = extract_variant_name(p_expr) {
                                matched_variants.insert(v_name.to_string());
                            }
                        } else {
                            self.check_expr(p_expr, &Type::Bool);
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

            let branch_ty = self.infer_expr(&branch.body);
            if branch_ty != Type::Nothing {
                if let Some(current) = &unified_ty {
                    if branch_ty.is_assignable_to(current) {
                        // keeps current
                    } else if current.is_assignable_to(&branch_ty) {
                        unified_ty = Some(branch_ty);
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

        // Exhaustiveness check: when in value position without else
        let is_exhaustive = if has_else {
            true
        } else if let Some(s_ty) = &subject_ty {
            let enum_variants = match self.resolve_nominal(peel_indirection(s_ty)) {
                Type::Enum { variants, .. } => Some(variants),
                _ => None,
            };

            if let Some(variants) = enum_variants {
                !variants.is_empty()
                    && variants
                        .iter()
                        .all(|(v_name, _)| matched_variants.contains(v_name))
            } else {
                false
            }
        } else {
            false
        };

        if !is_exhaustive {
            self.errors.push(TypeError::NonExhaustiveWhen {
                message: "'when' expression in value position must have an 'else' branch or be exhaustive"
                    .to_string(),
                span,
            });
        }

        unified_ty.unwrap_or(Type::Unit)
    }

    fn infer_call_expr(&mut self, callee: &Expr, args: &[CallArg], span: Span) -> Type {
        let callee_ty = self.infer_expr(callee);

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
                        self.check_expr(&arg.value, param_ty);
                    }
                } else {
                    let mut param_assigned = vec![false; n_params];
                    let mut seen_names = std::collections::HashSet::new();
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
                                param_assigned[pos] = true;
                                self.check_expr(&arg.value, &params[pos]);
                            } else {
                                self.errors.push(TypeError::CallSignatureMismatch {
                                    message: format!("unknown parameter '{}'", arg_name),
                                    span: arg.span,
                                });
                            }
                        } else {
                            if positional_idx < n_params {
                                param_assigned[positional_idx] = true;
                                self.check_expr(&arg.value, &params[positional_idx]);
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
            Type::Struct { name, fields } => {
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
                        return Type::Struct { name, fields };
                    }
                    for (arg, (_, _, f_ty)) in args.iter().zip(fields.iter()) {
                        self.check_expr(&arg.value, f_ty);
                    }
                } else {
                    let mut field_assigned = vec![false; fields.len()];
                    let mut seen_fields = std::collections::HashSet::new();
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
                                field_assigned[idx] = true;
                                self.check_expr(&arg.value, &fields[idx].2);
                            } else {
                                self.errors.push(TypeError::UnknownField {
                                    target_type: Type::Struct {
                                        name: name.clone(),
                                        fields: fields.clone(),
                                    },
                                    field_name: arg_name.clone(),
                                    span: arg.span,
                                });
                            }
                        } else {
                            if pos_idx < fields.len() {
                                field_assigned[pos_idx] = true;
                                self.check_expr(&arg.value, &fields[pos_idx].2);
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

                Type::Struct { name, fields }
            }
            Type::Enum { name, variants } => {
                if args.is_empty() {
                    Type::Enum { name, variants }
                } else {
                    self.errors.push(TypeError::CallSignatureMismatch {
                        message: format!(
                            "enum '{}' cannot be instantiated directly without a variant",
                            name
                        ),
                        span,
                    });
                    Type::Error
                }
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
        let wrap = |ty: Type| if is_nullable { nullable(ty) } else { ty };

        let peeled_ty = self.resolve_nominal(peel_indirection(base_ty));

        match &peeled_ty {
            Type::Struct { fields, .. } => {
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
            Type::Enum { name, variants } => {
                if let Some((_, payloads)) = variants.iter().find(|(v_name, _)| v_name == member) {
                    let enum_ty = Type::Enum {
                        name: name.clone(),
                        variants: variants.clone(),
                    };
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
            other => {
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

/// Makes `ty` nullable. `Error` stays `Error`.
fn nullable(ty: Type) -> Type {
    match ty {
        Type::Error => ty,
        other => Type::Nullable(Box::new(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Helper to wrap expression in Expr
    fn expr(kind: ExprKind) -> Expr {
        Expr::new(kind, Span::dummy())
    }

    #[test]
    fn test_strict_numeric_operations_reject_widening() {
        let mut checker = TypeChecker::new(None);
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
        let mut checker = TypeChecker::new(None);
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
        let mut checker = TypeChecker::new(None);
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
        let mut checker = TypeChecker::new(None);
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
        let mut checker = TypeChecker::new(None);
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
        let mut checker = TypeChecker::new(None);
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
        let mut checker = TypeChecker::new(None);
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
        let mut checker = TypeChecker::new(None);
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
        assert_eq!(checker.errors[1].code(), "E4002");
    }

    #[test]
    fn test_closure_checking_against_fn_type() {
        let mut checker = TypeChecker::new(None);
        let closure = expr(ExprKind::Closure {
            params: vec!["n".to_string()],
            body: Box::new(expr(ExprKind::Binary {
                op: BinaryOp::Add,
                left: Box::new(expr(ExprKind::Ident("n".to_string()))),
                right: Box::new(expr(ExprKind::Literal(Literal::Int(1)))),
            })),
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
        let mut checker = TypeChecker::new(None);
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
        let mut checker = TypeChecker::new(None);
        let point_sym = Symbol::struct_decl(
            "Point",
            vec![
                ("x".to_string(), false, Type::Int),
                ("y".to_string(), true, Type::Int),
            ],
            Visibility::Public,
            None,
            Span::dummy(),
        );
        checker.env.define(point_sym).unwrap();

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
        assert!(matches!(res_pos, Type::Struct { ref name, .. } if name == "Point"));

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
        assert!(matches!(res_named, Type::Struct { ref name, .. } if name == "Point"));

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
        let mut checker = TypeChecker::new(None);
        let user_sym = Symbol::struct_decl(
            "User",
            vec![
                ("id".to_string(), false, Type::Int),
                ("age".to_string(), true, Type::Int),
            ],
            Visibility::Public,
            None,
            Span::dummy(),
        );
        checker.env.define(user_sym.clone()).unwrap();

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
        let mut checker = TypeChecker::new(None);
        let opt_sym = Symbol::enum_decl(
            "Option",
            vec![
                ("None".to_string(), Vec::new()),
                ("Some".to_string(), vec![Type::Int]),
            ],
            Visibility::Public,
            None,
            Span::dummy(),
        );
        checker.env.define(opt_sym).unwrap();

        // Qualified unit variant: Option.None -> Option
        let none_access = expr(ExprKind::MemberAccess {
            target: Box::new(expr(ExprKind::Ident("Option".to_string()))),
            member: "None".to_string(),
            is_safe_nav: false,
        });
        let res_none = checker.infer_expr(&none_access);
        assert!(matches!(res_none, Type::Enum { ref name, .. } if name == "Option"));

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
        assert!(matches!(res_some, Type::Enum { ref name, .. } if name == "Option"));

        // Option.Some("bad") -> E4002
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
        assert!(checker.errors.iter().any(|e| e.code() == "E4002"));
    }

    #[test]
    fn test_reference_and_borrow_typing() {
        let mut checker = TypeChecker::new(None);
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
        let mut checker = TypeChecker::new(None);
        let point_sym = Symbol::struct_decl(
            "Point",
            vec![("x".to_string(), false, Type::Int)],
            Visibility::Public,
            None,
            Span::dummy(),
        );
        checker.env.define(point_sym).unwrap();

        let opt_point_ty = Type::Nullable(Box::new(Type::Struct {
            name: "Point".to_string(),
            fields: vec![("x".to_string(), false, Type::Int)],
        }));
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
        let mut checker = TypeChecker::new(None);
        let point_ty = Type::Heap(Box::new(Type::Struct {
            name: "Point".to_string(),
            fields: vec![("x".to_string(), false, Type::Int)],
        }));
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
            symbols: vec![
                ExportedSymbol::Function {
                    visibility: Visibility::Public,
                    is_compose: false,
                    name: "add".to_string(),
                    type_params: Vec::new(),
                    params: vec![
                        ("a".to_string(), "Int".to_string(), None),
                        ("b".to_string(), "Int".to_string(), None),
                    ],
                    return_type: "Int".to_string(),
                    generic_body_hash: None,
                },
                ExportedSymbol::Function {
                    visibility: Visibility::Package,
                    is_compose: false,
                    name: "internal_calc".to_string(),
                    type_params: Vec::new(),
                    params: Vec::new(),
                    return_type: "Int".to_string(),
                    generic_body_hash: None,
                },
            ],
            abi_hash: 12345,
        };

        // Case 1: Ingested from external package 'app.ui'
        let mut ext_checker = TypeChecker::new(Some("app.ui".to_string()));
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
        let mut same_checker = TypeChecker::new(Some("core.math".to_string()));
        same_checker.ingest_file_abi(&abi);
        assert_eq!(same_checker.infer_expr(&call_pkg), Type::Int);
        assert!(same_checker.errors.is_empty());
    }

    #[test]
    fn test_default_param_omission_and_named_args() {
        let mut checker = TypeChecker::new(None);
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
        let mut checker = TypeChecker::new(None);
        let point_sym = Symbol::struct_decl(
            "Point",
            vec![
                ("x".to_string(), false, Type::Int),
                ("y".to_string(), false, Type::Int),
            ],
            Visibility::Public,
            None,
            Span::dummy(),
        );
        checker.env.define(point_sym).unwrap();

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
        let mut checker = TypeChecker::new(None);
        let color_ty = Type::Enum {
            name: "Color".to_string(),
            variants: vec![
                ("Red".to_string(), Vec::new()),
                ("Green".to_string(), Vec::new()),
                ("Blue".to_string(), Vec::new()),
            ],
        };
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
        let mut checker = TypeChecker::new(None);
        let ret_expr = expr(ExprKind::Return(Some(Box::new(expr(ExprKind::Literal(
            Literal::Int(42),
        ))))));
        checker.infer_expr(&ret_expr);
        assert!(checker.errors.iter().any(|e| e.code() == "E4005"));
    }

    #[test]
    fn test_compose_isolation_across_closure() {
        let mut checker = TypeChecker::new(None);
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
            body: Box::new(expr(ExprKind::Call {
                callee: Box::new(expr(ExprKind::Ident("MyView".to_string()))),
                args: Vec::new(),
            })),
        });

        // Inferring the closure should not leak the outer compose context -> emits E4008!
        checker.infer_expr(&closure);
        assert!(checker.errors.iter().any(|e| e.code() == "E4008"));
    }

    #[test]
    fn test_duplicate_struct_fields_and_enum_variants_rejected() {
        let mut checker = TypeChecker::new(None);
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
        let mut checker = TypeChecker::new(None);
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
        let mut checker = TypeChecker::new(file.package.as_ref().map(|p| p.package_string()));
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

        // `user?.name` is `String?`, so it is not assignable to `String`.
        let not_null = format!(
            "{}fun nameOf(user: User?): String = user?.name\n",
            USER_STRUCT
        );
        assert_eq!(error_codes(&check_source(&not_null)), vec!["E4002"]);
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
        let abi = FileAbi::extract(&models).expect("models ABI must extract");

        let app = prism_parser::parse_source(
            "package demo.app\nfun nameOf(user: &User): String = user.name\nfun rename(user: &mut User, n: String): Unit {\n    user.name = n\n}\nfun emailOf(user: &User): String? = user.email\nfun cityOf(user: User): String = user.home.city\n",
        )
        .expect("app must parse");
        let mut checker = TypeChecker::new(Some("demo.app".to_string()));
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

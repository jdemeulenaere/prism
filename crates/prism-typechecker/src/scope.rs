//! Lexical scoping and symbol environments for semantic analysis.

use crate::error::TypeError;
use crate::types::Type;
use prism_ast::{Span, Visibility};
use std::collections::HashMap;

/// Declared mutability of a binding (`val` vs `var`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mutability {
    Val,
    Var,
}

/// The semantic classification of an identifier in the symbol table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SymbolKind {
    /// Local variable, constant, or function parameter binding.
    Variable {
        ty: Type,
        mutability: Mutability,
        visibility: Visibility,
        defining_package: Option<String>,
    },
    /// Function declaration.
    Function {
        fn_type: Type,
        param_names: Vec<String>,
        has_default: Vec<bool>,
        visibility: Visibility,
        defining_package: Option<String>,
    },
    /// Struct declaration with field definitions.
    Struct {
        fields: Vec<(String, bool, Type)>, // (name, is_var, type)
        visibility: Visibility,
        defining_package: Option<String>,
    },
    /// Enum declaration with variant constructors.
    Enum {
        variants: Vec<(String, Vec<Type>)>, // (name, payloads)
        visibility: Visibility,
        defining_package: Option<String>,
    },
}

/// A resolved symbol with its type, definition span, and mutability/visibility attributes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    pub span: Span,
}

impl Symbol {
    pub fn variable(name: impl Into<String>, ty: Type, mutability: Mutability, span: Span) -> Self {
        Self {
            name: name.into(),
            kind: SymbolKind::Variable {
                ty,
                mutability,
                visibility: Visibility::Private,
                defining_package: None,
            },
            span,
        }
    }

    pub fn top_level_variable(
        name: impl Into<String>,
        ty: Type,
        mutability: Mutability,
        visibility: Visibility,
        defining_package: Option<String>,
        span: Span,
    ) -> Self {
        Self {
            name: name.into(),
            kind: SymbolKind::Variable {
                ty,
                mutability,
                visibility,
                defining_package,
            },
            span,
        }
    }

    pub fn function(
        name: impl Into<String>,
        fn_type: Type,
        visibility: Visibility,
        defining_package: Option<String>,
        span: Span,
    ) -> Self {
        Self {
            name: name.into(),
            kind: SymbolKind::Function {
                fn_type,
                param_names: Vec::new(),
                has_default: Vec::new(),
                visibility,
                defining_package,
            },
            span,
        }
    }

    pub fn function_with_defaults(
        name: impl Into<String>,
        fn_type: Type,
        param_names: Vec<String>,
        has_default: Vec<bool>,
        visibility: Visibility,
        defining_package: Option<String>,
        span: Span,
    ) -> Self {
        Self {
            name: name.into(),
            kind: SymbolKind::Function {
                fn_type,
                param_names,
                has_default,
                visibility,
                defining_package,
            },
            span,
        }
    }

    pub fn struct_decl(
        name: impl Into<String>,
        fields: Vec<(String, bool, Type)>,
        visibility: Visibility,
        defining_package: Option<String>,
        span: Span,
    ) -> Self {
        Self {
            name: name.into(),
            kind: SymbolKind::Struct {
                fields,
                visibility,
                defining_package,
            },
            span,
        }
    }

    pub fn enum_decl(
        name: impl Into<String>,
        variants: Vec<(String, Vec<Type>)>,
        visibility: Visibility,
        defining_package: Option<String>,
        span: Span,
    ) -> Self {
        Self {
            name: name.into(),
            kind: SymbolKind::Enum {
                variants,
                visibility,
                defining_package,
            },
            span,
        }
    }

    pub fn visibility(&self) -> Visibility {
        match &self.kind {
            SymbolKind::Variable { visibility, .. } => *visibility,
            SymbolKind::Function { visibility, .. } => *visibility,
            SymbolKind::Struct { visibility, .. } => *visibility,
            SymbolKind::Enum { visibility, .. } => *visibility,
        }
    }

    pub fn defining_package(&self) -> Option<&str> {
        match &self.kind {
            SymbolKind::Variable {
                defining_package, ..
            } => defining_package.as_deref(),
            SymbolKind::Function {
                defining_package, ..
            } => defining_package.as_deref(),
            SymbolKind::Struct {
                defining_package, ..
            } => defining_package.as_deref(),
            SymbolKind::Enum {
                defining_package, ..
            } => defining_package.as_deref(),
        }
    }

    pub fn ty(&self) -> Type {
        match &self.kind {
            SymbolKind::Variable { ty, .. } => ty.clone(),
            SymbolKind::Function { fn_type, .. } => fn_type.clone(),
            SymbolKind::Struct { fields, .. } => Type::Struct {
                name: self.name.clone(),
                fields: fields.clone(),
            },
            SymbolKind::Enum { variants, .. } => Type::Enum {
                name: self.name.clone(),
                variants: variants.clone(),
            },
        }
    }
}

/// Lexical scope environment tracking nested block scopes and shadowing.
#[derive(Debug, Clone)]
pub struct ScopeStack {
    scopes: Vec<HashMap<String, Symbol>>,
}

impl ScopeStack {
    /// Initializes an environment with a global/module-level scope.
    pub fn new() -> Self {
        Self {
            scopes: vec![HashMap::new()],
        }
    }

    /// Pushes a new nested lexical scope level (e.g. entering a block or function body).
    pub fn enter_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    /// Pops the innermost lexical scope level.
    pub fn exit_scope(&mut self) {
        if self.scopes.len() > 1 {
            self.scopes.pop();
        }
    }

    /// Number of active scope levels (1 = module level).
    pub fn depth(&self) -> usize {
        self.scopes.len()
    }

    /// Defines a symbol in the current (innermost) scope level.
    ///
    /// Rejects duplicate definitions within the same scope with `E4010`.
    /// Shadowing a declaration from an enclosing outer scope is permitted.
    #[allow(clippy::result_large_err)]
    pub fn define(&mut self, symbol: Symbol) -> Result<(), TypeError> {
        let current_scope = self.scopes.last_mut().expect("scope stack cannot be empty");
        if current_scope.contains_key(&symbol.name) {
            return Err(TypeError::DuplicateSymbol {
                name: symbol.name,
                span: symbol.span,
            });
        }
        current_scope.insert(symbol.name.clone(), symbol);
        Ok(())
    }

    /// Looks up an identifier starting from the innermost scope outwards without emitting an error.
    pub fn lookup(&self, name: &str) -> Option<&Symbol> {
        for scope in self.scopes.iter().rev() {
            if let Some(symbol) = scope.get(name) {
                return Some(symbol);
            }
        }
        None
    }

    /// Looks up a nominal type name (struct or enum declaration) from the innermost scope outwards,
    /// skipping value symbols (variables, functions) that share the name.
    pub fn lookup_type(&self, name: &str) -> Option<&Symbol> {
        self.scopes
            .iter()
            .rev()
            .filter_map(|scope| scope.get(name))
            .find(|symbol| {
                matches!(
                    symbol.kind,
                    SymbolKind::Struct { .. } | SymbolKind::Enum { .. }
                )
            })
    }

    /// Resolves an identifier starting from the innermost scope outwards.
    ///
    /// Returns `Err(TypeError::UnresolvedSymbol)` (`E4001`) if not found in any scope level.
    #[allow(clippy::result_large_err)]
    pub fn resolve(&self, name: &str, span: Span) -> Result<&Symbol, TypeError> {
        self.lookup(name)
            .ok_or_else(|| TypeError::UnresolvedSymbol {
                name: name.to_string(),
                span,
            })
    }

    /// Updates the type of an existing symbol in the innermost scope that defines it
    /// (useful for inferred private expression bodies).
    pub fn update_type(&mut self, name: &str, new_type: Type) -> bool {
        for scope in self.scopes.iter_mut().rev() {
            if let Some(symbol) = scope.get_mut(name) {
                match &mut symbol.kind {
                    SymbolKind::Variable { ty, .. } => *ty = new_type,
                    SymbolKind::Function { fn_type, .. } => *fn_type = new_type,
                    _ => {}
                }
                return true;
            }
        }
        false
    }
}

impl Default for ScopeStack {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_define_and_resolve_local_symbol() {
        let mut env = ScopeStack::new();
        let sym = Symbol::variable("x", Type::Int, Mutability::Val, Span::dummy());
        env.define(sym.clone()).unwrap();

        let resolved = env.resolve("x", Span::dummy()).unwrap();
        assert_eq!(resolved.name, "x");
        assert_eq!(resolved.ty(), Type::Int);
    }

    #[test]
    fn test_duplicate_symbol_in_same_scope_rejected_e4010() {
        let mut env = ScopeStack::new();
        let sym1 = Symbol::variable("x", Type::Int, Mutability::Val, Span::dummy());
        let sym2 = Symbol::variable("x", Type::String, Mutability::Var, Span::dummy());

        env.define(sym1).unwrap();
        let err = env.define(sym2).unwrap_err();
        assert_eq!(err.code(), "E4010");
    }

    #[test]
    fn test_shadowing_across_nested_scopes_allowed() {
        let mut env = ScopeStack::new();
        let sym_outer = Symbol::variable("x", Type::Int, Mutability::Val, Span::dummy());
        env.define(sym_outer).unwrap();

        env.enter_scope();
        let sym_inner = Symbol::variable("x", Type::String, Mutability::Var, Span::dummy());
        env.define(sym_inner).unwrap();

        // Resolves to innermost
        let resolved = env.resolve("x", Span::dummy()).unwrap();
        assert_eq!(resolved.ty(), Type::String);

        env.exit_scope();
        // Resolves to outer
        let resolved_outer = env.resolve("x", Span::dummy()).unwrap();
        assert_eq!(resolved_outer.ty(), Type::Int);
    }

    #[test]
    fn test_lookup_type_skips_shadowing_value_symbols() {
        let mut env = ScopeStack::new();
        let user = Symbol::struct_decl(
            "User",
            vec![("name".to_string(), true, Type::String)],
            Visibility::Private,
            None,
            Span::dummy(),
        );
        env.define(user).unwrap();

        env.enter_scope();
        env.define(Symbol::variable(
            "User",
            Type::Int,
            Mutability::Val,
            Span::dummy(),
        ))
        .unwrap();

        // Value lookup finds the shadowing variable; type lookup still finds the struct.
        assert_eq!(env.lookup("User").unwrap().ty(), Type::Int);
        let ty_sym = env.lookup_type("User").unwrap();
        assert!(matches!(ty_sym.kind, SymbolKind::Struct { .. }));
        assert!(env.lookup_type("Missing").is_none());
    }

    #[test]
    fn test_unresolved_symbol_e4001() {
        let env = ScopeStack::new();
        let err = env.resolve("undefined_var", Span::dummy()).unwrap_err();
        assert_eq!(err.code(), "E4001");
    }
}

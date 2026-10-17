//! Lexical scoping and symbol environments for semantic analysis.

use crate::error::TypeError;
use crate::types::{NominalId, Type};
use prism_ast::{Span, Visibility};
use prism_typed_tree::LocalId;
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
    /// Local variable, constant, or function parameter binding, or a top-level `val`.
    Variable {
        ty: Type,
        mutability: Mutability,
        visibility: Visibility,
        defining_package: Option<String>,
        /// The binding of the body being checked, for a local; `None` for a top-level `val`.
        local: Option<LocalId>,
        /// The defining module, for a top-level `val` or a payload-free variant.
        module: Option<String>,
    },
    /// Function declaration.
    Function {
        fn_type: Type,
        param_names: Vec<String>,
        has_default: Vec<bool>,
        visibility: Visibility,
        defining_package: Option<String>,
        /// The defining module.
        module: String,
    },
    /// Struct declaration. Its fields live in the checker's type table under `id`.
    Struct {
        id: NominalId,
        visibility: Visibility,
        defining_package: Option<String>,
    },
    /// Enum declaration. Its variants live in the checker's type table under `id`.
    Enum {
        id: NominalId,
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
                local: None,
                module: None,
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
                local: None,
                module: None,
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
                module: String::new(),
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
                module: String::new(),
            },
            span,
        }
    }

    pub fn struct_decl(
        id: NominalId,
        visibility: Visibility,
        defining_package: Option<String>,
        span: Span,
    ) -> Self {
        Self {
            name: id.name.clone(),
            kind: SymbolKind::Struct {
                id,
                visibility,
                defining_package,
            },
            span,
        }
    }

    pub fn enum_decl(
        id: NominalId,
        visibility: Visibility,
        defining_package: Option<String>,
        span: Span,
    ) -> Self {
        Self {
            name: id.name.clone(),
            kind: SymbolKind::Enum {
                id,
                visibility,
                defining_package,
            },
            span,
        }
    }

    /// The symbol, bound to the binding `local` of the body being checked.
    pub fn with_local(mut self, id: LocalId) -> Self {
        if let SymbolKind::Variable { local, .. } = &mut self.kind {
            *local = Some(id);
        }
        self
    }

    /// The symbol, declared by module `defining_module`. Only top-level values and functions
    /// record their module.
    pub fn with_module(mut self, defining_module: impl Into<String>) -> Self {
        match &mut self.kind {
            SymbolKind::Variable { module, .. } => *module = Some(defining_module.into()),
            SymbolKind::Function { module, .. } => *module = defining_module.into(),
            SymbolKind::Struct { .. } | SymbolKind::Enum { .. } => {}
        }
        self
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
            SymbolKind::Struct { id, .. } => Type::Struct(id.clone()),
            SymbolKind::Enum { id, .. } => Type::Enum(id.clone()),
        }
    }
}

/// Lexical scope environment tracking nested block scopes and shadowing.
#[derive(Debug, Clone)]
pub struct ScopeStack {
    scopes: Vec<HashMap<String, Symbol>>,
    /// Symbols exported by direct imports, below the module scope. Each name maps to one
    /// candidate per exporting module; a name with several candidates is ambiguous at use.
    imports: HashMap<String, Vec<(String, Symbol)>>,
}

/// The result of looking a name up among the imports.
pub enum ImportLookup<'a> {
    /// No import exports the name.
    None,
    /// Exactly one import exports the name.
    One(&'a Symbol),
    /// Several imports export the name; holds their module paths.
    Ambiguous(Vec<String>),
}

impl ScopeStack {
    /// Initializes an environment with a global/module-level scope.
    pub fn new() -> Self {
        Self {
            scopes: vec![HashMap::new()],
            imports: HashMap::new(),
        }
    }

    /// Adds a symbol exported by the imported module `module` to the import layer. A module
    /// contributes at most one candidate per name.
    pub fn define_import(&mut self, module: &str, symbol: Symbol) {
        let candidates = self.imports.entry(symbol.name.clone()).or_default();
        if !candidates.iter().any(|(m, _)| m == module) {
            candidates.push((module.to_string(), symbol));
        }
    }

    /// Looks `name` up among the imports, keeping only candidates accepted by `keep`.
    pub fn lookup_import(&self, name: &str, keep: impl Fn(&Symbol) -> bool) -> ImportLookup<'_> {
        let candidates: Vec<&(String, Symbol)> = self
            .imports
            .get(name)
            .map(|c| c.iter().filter(|(_, sym)| keep(sym)).collect())
            .unwrap_or_default();
        match candidates.as_slice() {
            [] => ImportLookup::None,
            [(_, sym)] => ImportLookup::One(sym),
            many => ImportLookup::Ambiguous(many.iter().map(|(m, _)| m.clone()).collect()),
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
    ///
    /// Falls back to the imports when no scope declares the name; an ambiguous import gives
    /// `None` (use [`Self::resolve`] to report it).
    pub fn lookup(&self, name: &str) -> Option<&Symbol> {
        for scope in self.scopes.iter().rev() {
            if let Some(symbol) = scope.get(name) {
                return Some(symbol);
            }
        }
        match self.lookup_import(name, |_| true) {
            ImportLookup::One(symbol) => Some(symbol),
            _ => None,
        }
    }

    /// Looks up a nominal type name (struct or enum declaration) from the innermost scope outwards,
    /// skipping value symbols (variables, functions) that share the name, then among the imports.
    /// An ambiguous imported type name gives `None` (see [`Self::lookup_import`]).
    pub fn lookup_type(&self, name: &str) -> Option<&Symbol> {
        let in_scope = self
            .scopes
            .iter()
            .rev()
            .filter_map(|scope| scope.get(name))
            .find(|symbol| is_type_symbol(symbol));
        match in_scope {
            Some(symbol) => Some(symbol),
            None => match self.lookup_import(name, is_type_symbol) {
                ImportLookup::One(symbol) => Some(symbol),
                _ => None,
            },
        }
    }

    /// Resolves an identifier starting from the innermost scope outwards, then among the imports.
    ///
    /// Returns `Err(TypeError::UnresolvedSymbol)` (`E4001`) if no scope or import declares it,
    /// and `Err(TypeError::AmbiguousImport)` (`E4012`) if no scope declares it and several
    /// imports export it.
    #[allow(clippy::result_large_err)]
    pub fn resolve(&self, name: &str, span: Span) -> Result<&Symbol, TypeError> {
        for scope in self.scopes.iter().rev() {
            if let Some(symbol) = scope.get(name) {
                return Ok(symbol);
            }
        }
        match self.lookup_import(name, |_| true) {
            ImportLookup::One(symbol) => Ok(symbol),
            ImportLookup::Ambiguous(modules) => Err(TypeError::AmbiguousImport {
                name: name.to_string(),
                modules,
                span,
            }),
            ImportLookup::None => Err(TypeError::UnresolvedSymbol {
                name: name.to_string(),
                span,
            }),
        }
    }

    /// Detaches every scope level nested inside the module scope, leaving only the module scope.
    /// [`Self::restore_locals`] puts them back. Used to check a declaration's body on demand
    /// while another body is being checked.
    pub fn take_locals(&mut self) -> Vec<HashMap<String, Symbol>> {
        self.scopes.split_off(1)
    }

    /// Re-attaches scope levels detached by [`Self::take_locals`].
    pub fn restore_locals(&mut self, locals: Vec<HashMap<String, Symbol>>) {
        self.scopes.truncate(1);
        self.scopes.extend(locals);
    }

    /// Removes a symbol from the current (innermost) scope level, returning it if it was there.
    pub fn remove(&mut self, name: &str) -> Option<Symbol> {
        self.scopes
            .last_mut()
            .expect("scope stack cannot be empty")
            .remove(name)
    }

    /// Whether `name` currently resolves to a module-level symbol (it is declared at module
    /// level and no nested scope shadows it).
    pub fn resolves_at_module_level(&self, name: &str) -> bool {
        let innermost = self
            .scopes
            .iter()
            .rposition(|scope| scope.contains_key(name));
        innermost == Some(0)
    }

    /// Whether `name` currently resolves to a local: a symbol of a scope nested inside the module
    /// scope (a parameter, a local binding, a loop variable).
    pub fn resolves_to_local(&self, name: &str) -> bool {
        matches!(
            self.scopes.iter().rposition(|scope| scope.contains_key(name)),
            Some(level) if level > 0
        )
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

/// Whether `symbol` declares a nominal type (a struct or an enum).
pub fn is_type_symbol(symbol: &Symbol) -> bool {
    matches!(
        symbol.kind,
        SymbolKind::Struct { .. } | SymbolKind::Enum { .. }
    )
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
            NominalId::new("<test>", "User"),
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

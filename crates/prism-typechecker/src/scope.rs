//! Lexical scoping and symbol environments for semantic analysis.

use crate::error::TypeError;
use crate::types::{NominalId, Type};
use prism_ast::{Span, Visibility};
use prism_typed_tree::LocalId;
use prism_typed_tree::TypeParameter;
use std::collections::HashMap;
use std::sync::Arc;

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
    /// Function declaration, or a payload variant's constructor. The function type mentions the
    /// declaration's own `type_parameters`, if it is generic.
    Function {
        fn_type: Type,
        type_parameters: Vec<TypeParameter>,
        param_names: Vec<String>,
        defaults: Vec<Option<prism_ast::DefaultInputs>>,
        visibility: Visibility,
        defining_package: Option<String>,
        /// The defining module.
        module: String,
        /// The type of the context parameter (`context(scope: RowScope)`), which a call passes
        /// from its context (D6 of `receiver-lambdas`).
        context: Option<Type>,
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
                type_parameters: Vec::new(),
                param_names: Vec::new(),
                defaults: Vec::new(),
                visibility,
                defining_package,
                module: String::new(),
                context: None,
            },
            span,
        }
    }

    pub fn function_with_defaults(
        name: impl Into<String>,
        fn_type: Type,
        param_names: Vec<String>,
        defaults: Vec<Option<prism_ast::DefaultInputs>>,
        visibility: Visibility,
        defining_package: Option<String>,
        span: Span,
    ) -> Self {
        Self {
            name: name.into(),
            kind: SymbolKind::Function {
                fn_type,
                type_parameters: Vec::new(),
                param_names,
                defaults,
                visibility,
                defining_package,
                module: String::new(),
                context: None,
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

    /// The symbol of a function with the context parameter type `context`.
    pub fn with_context(mut self, context: Option<Type>) -> Self {
        if let SymbolKind::Function { context: own, .. } = &mut self.kind {
            *own = context;
        }
        self
    }

    /// The context parameter type of a function symbol, if it has one.
    pub fn context(&self) -> Option<&Type> {
        match &self.kind {
            SymbolKind::Function { context, .. } => context.as_ref(),
            _ => None,
        }
    }

    /// The symbol of a generic function or variant constructor with `parameters`.
    pub fn with_type_parameters(mut self, parameters: Vec<TypeParameter>) -> Self {
        if let SymbolKind::Function {
            type_parameters, ..
        } = &mut self.kind
        {
            *type_parameters = parameters;
        }
        self
    }

    /// The type parameters of a generic function or variant constructor, empty otherwise.
    pub fn type_parameters(&self) -> &[TypeParameter] {
        match &self.kind {
            SymbolKind::Function {
                type_parameters, ..
            } => type_parameters,
            _ => &[],
        }
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
            SymbolKind::Struct { id, .. } => Type::plain_struct(id.clone()),
            SymbolKind::Enum { id, .. } => Type::plain_enum(id.clone()),
        }
    }
}

/// Lexical scope environment tracking nested block scopes and shadowing.
///
/// A scoped symbol table: each name maps to its bindings, innermost last, each with the level
/// that declares it, so looking a name up hashes it once however deeply the use is nested
/// (closures nested through their calls' trailing lambdas nest scopes as deep as the source).
/// Each level lists the names it declares, which leaving it unbinds.
#[derive(Debug, Clone)]
pub struct ScopeStack {
    /// Each name's bindings, ordered by level, innermost last. Level 0 is the module scope.
    bindings: HashMap<String, Vec<(usize, Symbol)>>,
    /// The names each open level declares, the module scope first.
    levels: Vec<Vec<String>>,
    /// The innermost level [`Self::take_locals`] hides, `0` when none is hidden: a binding is
    /// visible when it is the module scope's or declared deeper than this.
    hidden_through: usize,
    /// Symbols exported by direct imports, below the module scope. Each name maps to one
    /// candidate per exporting module; a name with several candidates is ambiguous at use.
    imports: HashMap<String, Vec<(String, Symbol)>>,
    /// The names of the prelude modules, below the imports: found only when no scope and no
    /// import has the name, so any of them shadows a prelude name (D4 of `standard-library`).
    /// Shared by every checker of a process.
    prelude: Option<Arc<HashMap<String, Symbol>>>,
}

/// The nested levels [`ScopeStack::take_locals`] hid, which [`ScopeStack::restore_locals`]
/// shows again.
#[derive(Debug)]
pub struct DetachedLocals {
    /// The levels open when they were hidden.
    depth: usize,
    /// The innermost level hidden before.
    hidden_through: usize,
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
            bindings: HashMap::new(),
            levels: vec![Vec::new()],
            hidden_through: 0,
            imports: HashMap::new(),
            prelude: None,
        }
    }

    /// Puts the prelude's `names` below the imports.
    pub fn set_prelude(&mut self, names: Arc<HashMap<String, Symbol>>) {
        self.prelude = Some(names);
    }

    /// The prelude symbol named `name`, if a prelude is set and has one.
    fn lookup_prelude(&self, name: &str) -> Option<&Symbol> {
        self.prelude.as_ref()?.get(name)
    }

    /// The imported symbols, one per name: the first candidate of each.
    pub(crate) fn imported_names(&self) -> HashMap<String, Symbol> {
        self.imports
            .iter()
            .filter_map(|(name, candidates)| {
                let (_, symbol) = candidates.first()?;
                Some((name.clone(), symbol.clone()))
            })
            .collect()
    }

    /// Adds a symbol exported by the imported module `module` to the import layer. A module
    /// contributes at most one candidate per name, except a struct and its companion value
    /// (D4 of `ui-toolkit`), which it contributes both.
    pub fn define_import(&mut self, module: &str, symbol: Symbol) {
        let candidates = self.imports.entry(symbol.name.clone()).or_default();
        let companion = |other: &Symbol| {
            matches!(
                (&other.kind, &symbol.kind),
                (SymbolKind::Struct { .. }, SymbolKind::Variable { .. })
                    | (SymbolKind::Variable { .. }, SymbolKind::Struct { .. })
            )
        };
        if !candidates
            .iter()
            .any(|(m, other)| m == module && !companion(other))
        {
            candidates.push((module.to_string(), symbol));
        }
    }

    /// Looks `name` up among the imports, keeping only candidates accepted by `keep`. A module
    /// that exports a struct and its companion value under the name counts once, as its value:
    /// the struct is what [`Self::lookup_type`] and [`Self::lookup_constructor`] find.
    pub fn lookup_import(&self, name: &str, keep: impl Fn(&Symbol) -> bool) -> ImportLookup<'_> {
        let mut candidates: Vec<&(String, Symbol)> = self
            .imports
            .get(name)
            .map(|c| c.iter().filter(|(_, sym)| keep(sym)).collect())
            .unwrap_or_default();
        if candidates.len() == 2 && candidates[0].0 == candidates[1].0 {
            candidates.retain(|(_, symbol)| !is_type_symbol(symbol));
        }
        match candidates.as_slice() {
            [] => ImportLookup::None,
            [(_, sym)] => ImportLookup::One(sym),
            many => ImportLookup::Ambiguous(many.iter().map(|(m, _)| m.clone()).collect()),
        }
    }

    /// Pushes a new nested lexical scope level (e.g. entering a block or function body).
    pub fn enter_scope(&mut self) {
        self.levels.push(Vec::new());
    }

    /// Pops the innermost lexical scope level, unbinding the names it declares. The module
    /// scope and the levels [`Self::take_locals`] hides are never popped.
    pub fn exit_scope(&mut self) {
        let level = self.current_level();
        if level == 0 || level <= self.hidden_through {
            return;
        }
        for name in self.levels.pop().expect("a nested level") {
            if let Some(bindings) = self.bindings.get_mut(&name) {
                // A name `remove` already unbound, or declared twice after a removal, may
                // have no binding of this level left.
                if bindings.last().is_some_and(|(at, _)| *at == level) {
                    bindings.pop();
                }
            }
        }
    }

    /// Number of active scope levels (1 = module level).
    pub fn depth(&self) -> usize {
        self.levels.len()
    }

    /// The level of the innermost scope.
    fn current_level(&self) -> usize {
        self.levels.len() - 1
    }

    /// Whether a binding declared at `level` is visible.
    fn is_visible(&self, level: usize) -> bool {
        level == 0 || level > self.hidden_through
    }

    /// The visible bindings of `name`, innermost first, with their levels.
    fn visible(&self, name: &str) -> impl Iterator<Item = &(usize, Symbol)> {
        self.bindings
            .get(name)
            .into_iter()
            .flat_map(|bindings| bindings.iter().rev())
            .filter(|(level, _)| self.is_visible(*level))
    }

    /// The visible bindings of `name` that an inner binding of it shadows, innermost first: the
    /// receivers of enclosing extensions and lambdas, for `this` (D4 of `receiver-lambdas`).
    pub fn shadowed(&self, name: &str) -> impl Iterator<Item = &Symbol> {
        self.visible(name).skip(1).map(|(_, symbol)| symbol)
    }

    /// Defines a symbol in the current (innermost) scope level.
    ///
    /// Rejects duplicate definitions within the same scope with `E4010`.
    /// Shadowing a declaration from an enclosing outer scope is permitted.
    #[allow(clippy::result_large_err)]
    pub fn define(&mut self, symbol: Symbol) -> Result<(), TypeError> {
        let level = self.current_level();
        let bindings = self.bindings.entry(symbol.name.clone()).or_default();
        if bindings.last().is_some_and(|(at, _)| *at == level) {
            return Err(TypeError::DuplicateSymbol {
                name: symbol.name,
                span: symbol.span,
            });
        }
        self.levels[level].push(symbol.name.clone());
        bindings.push((level, symbol));
        Ok(())
    }

    /// Defines the top-level value `symbol` as the companion value of the struct of its name
    /// that the module scope already declares (D4 of `ui-toolkit`): the name then means the
    /// value in value positions, the struct in type positions, and its constructor as a callee.
    /// Without such a struct it is [`Self::define`], so every other pair is still `E4010`.
    #[allow(clippy::result_large_err)]
    pub fn define_companion(&mut self, symbol: Symbol) -> Result<(), TypeError> {
        let has_struct = self.current_level() == 0
            && self.bindings.get(&symbol.name).is_some_and(|bindings| {
                matches!(
                    bindings.as_slice(),
                    [(
                        0,
                        Symbol {
                            kind: SymbolKind::Struct { .. },
                            ..
                        }
                    )]
                )
            });
        if !has_struct {
            return self.define(symbol);
        }
        self.levels[0].push(symbol.name.clone());
        self.bindings
            .get_mut(&symbol.name)
            .expect("the struct's binding")
            .push((0, symbol));
        Ok(())
    }

    /// The struct a call `name(...)` constructs: the innermost binding of `name` when it is a
    /// struct, the struct of a companion value the name otherwise resolves to (D4 of
    /// `ui-toolkit`), or `None`.
    pub fn lookup_constructor(&self, name: &str) -> Option<&Symbol> {
        let mut visible = self.visible(name);
        if let Some((level, symbol)) = visible.next() {
            return match &symbol.kind {
                SymbolKind::Struct { .. } => Some(symbol),
                SymbolKind::Variable { local: None, .. } if *level == 0 => visible
                    .next()
                    .filter(|(level, other)| {
                        *level == 0 && matches!(other.kind, SymbolKind::Struct { .. })
                    })
                    .map(|(_, other)| other),
                _ => None,
            };
        }
        let candidates = self.imports.get(name);
        match candidates.map(Vec::as_slice) {
            Some([(_, symbol)]) => {
                matches!(symbol.kind, SymbolKind::Struct { .. }).then_some(symbol)
            }
            Some([(first, a), (second, b)]) if first == second => [a, b]
                .into_iter()
                .find(|symbol| matches!(symbol.kind, SymbolKind::Struct { .. })),
            Some(_) => None,
            None => self
                .lookup_prelude(name)
                .filter(|symbol| matches!(symbol.kind, SymbolKind::Struct { .. })),
        }
    }

    /// Looks up an identifier starting from the innermost scope outwards without emitting an error.
    ///
    /// Falls back to the imports when no scope declares the name; an ambiguous import gives
    /// `None` (use [`Self::resolve`] to report it).
    pub fn lookup(&self, name: &str) -> Option<&Symbol> {
        if let Some((_, symbol)) = self.visible(name).next() {
            return Some(symbol);
        }
        match self.lookup_import(name, |_| true) {
            ImportLookup::One(symbol) => Some(symbol),
            ImportLookup::None => self.lookup_prelude(name),
            ImportLookup::Ambiguous(_) => None,
        }
    }

    /// Looks up a nominal type name (struct or enum declaration) from the innermost scope outwards,
    /// skipping value symbols (variables, functions) that share the name, then among the imports.
    /// An ambiguous imported type name gives `None` (see [`Self::lookup_import`]).
    pub fn lookup_type(&self, name: &str) -> Option<&Symbol> {
        let in_scope = self
            .visible(name)
            .map(|(_, symbol)| symbol)
            .find(|symbol| is_type_symbol(symbol));
        match in_scope {
            Some(symbol) => Some(symbol),
            None => match self.lookup_import(name, is_type_symbol) {
                ImportLookup::One(symbol) => Some(symbol),
                ImportLookup::None => self
                    .lookup_prelude(name)
                    .filter(|symbol| is_type_symbol(symbol)),
                ImportLookup::Ambiguous(_) => None,
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
        if let Some((_, symbol)) = self.visible(name).next() {
            return Ok(symbol);
        }
        match self.lookup_import(name, |_| true) {
            ImportLookup::One(symbol) => Ok(symbol),
            ImportLookup::Ambiguous(modules) => Err(TypeError::AmbiguousImport {
                name: name.to_string(),
                modules,
                span,
            }),
            ImportLookup::None => match self.lookup_prelude(name) {
                Some(symbol) => Ok(symbol),
                None => Err(TypeError::UnresolvedSymbol {
                    name: name.to_string(),
                    span,
                    import_suggestions: Vec::new(),
                }),
            },
        }
    }

    /// Hides every scope level nested inside the module scope, so only the module scope's names
    /// resolve and the levels entered next start fresh. [`Self::restore_locals`] shows them
    /// again. Used to check a declaration's body on demand while another body is being checked.
    /// It costs nothing per hidden binding.
    pub fn take_locals(&mut self) -> DetachedLocals {
        let detached = DetachedLocals {
            depth: self.levels.len(),
            hidden_through: self.hidden_through,
        };
        self.hidden_through = self.current_level();
        detached
    }

    /// Shows the levels hidden by [`Self::take_locals`] again, leaving any level entered since.
    pub fn restore_locals(&mut self, detached: DetachedLocals) {
        while self.levels.len() > detached.depth {
            self.exit_scope();
        }
        self.hidden_through = detached.hidden_through;
    }

    /// Removes a symbol from the current (innermost) scope level, returning it if it was there.
    pub fn remove(&mut self, name: &str) -> Option<Symbol> {
        let level = self.current_level();
        let bindings = self.bindings.get_mut(name)?;
        if bindings.last().is_some_and(|(at, _)| *at == level) {
            // The level's list keeps the name; leaving the level skips it (`exit_scope`).
            bindings.pop().map(|(_, symbol)| symbol)
        } else {
            None
        }
    }

    /// Whether `name` currently resolves to a module-level symbol (it is declared at module
    /// level and no nested scope shadows it).
    pub fn resolves_at_module_level(&self, name: &str) -> bool {
        matches!(self.visible(name).next(), Some((0, _)))
    }

    /// Whether `name` currently resolves to a local: a symbol of a scope nested inside the module
    /// scope (a parameter, a local binding, a loop variable).
    pub fn resolves_to_local(&self, name: &str) -> bool {
        matches!(self.visible(name).next(), Some((level, _)) if *level > 0)
    }

    /// Updates the type of an existing symbol in the innermost scope that defines it
    /// (useful for inferred private expression bodies).
    pub fn update_type(&mut self, name: &str, new_type: Type) -> bool {
        let hidden_through = self.hidden_through;
        let Some(bindings) = self.bindings.get_mut(name) else {
            return false;
        };
        let visible = bindings
            .iter_mut()
            .rev()
            .find(|(level, _)| *level == 0 || *level > hidden_through);
        let Some((_, symbol)) = visible else {
            return false;
        };
        match &mut symbol.kind {
            SymbolKind::Variable { ty, .. } => *ty = new_type,
            SymbolKind::Function { fn_type, .. } => *fn_type = new_type,
            _ => {}
        }
        true
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
    fn test_taken_locals_are_hidden_until_restored() {
        let mut env = ScopeStack::new();
        env.define(Symbol::variable(
            "x",
            Type::Int,
            Mutability::Val,
            Span::dummy(),
        ))
        .unwrap();
        env.enter_scope();
        env.define(Symbol::variable(
            "x",
            Type::String,
            Mutability::Val,
            Span::dummy(),
        ))
        .unwrap();
        env.define(Symbol::variable(
            "y",
            Type::Int,
            Mutability::Val,
            Span::dummy(),
        ))
        .unwrap();

        let detached = env.take_locals();
        // Only the module scope resolves, and a fresh level may declare the hidden names.
        assert_eq!(env.lookup("x").unwrap().ty(), Type::Int);
        assert!(env.lookup("y").is_none());
        assert!(env.resolves_at_module_level("x"));
        env.enter_scope();
        env.define(Symbol::variable(
            "y",
            Type::String,
            Mutability::Val,
            Span::dummy(),
        ))
        .unwrap();
        assert!(env.resolves_to_local("y"));
        env.exit_scope();
        // Leaving more levels than were entered never pops a hidden one.
        env.exit_scope();
        env.restore_locals(detached);

        assert_eq!(env.lookup("x").unwrap().ty(), Type::String);
        assert_eq!(env.lookup("y").unwrap().ty(), Type::Int);
        env.exit_scope();
        assert_eq!(env.lookup("x").unwrap().ty(), Type::Int);
        assert!(env.lookup("y").is_none());
    }

    fn struct_symbol(name: &str) -> Symbol {
        Symbol::struct_decl(
            NominalId::new("<test>", name),
            Visibility::Public,
            None,
            Span::dummy(),
        )
    }

    fn top_level_value(name: &str, ty: Type) -> Symbol {
        Symbol::top_level_variable(
            name,
            ty,
            Mutability::Val,
            Visibility::Public,
            None,
            Span::dummy(),
        )
        .with_module("<test>")
    }

    /// `ui-toolkit` D4: a struct and a top-level value of one file may share a name.
    #[test]
    fn a_struct_and_its_companion_value_share_a_name() {
        let mut env = ScopeStack::new();
        env.define(struct_symbol("Color")).unwrap();
        env.define_companion(top_level_value("Color", Type::Int))
            .unwrap();
        assert_eq!(env.lookup("Color").unwrap().ty(), Type::Int, "the value");
        assert!(matches!(
            env.lookup_type("Color").unwrap().kind,
            SymbolKind::Struct { .. }
        ));
        assert!(matches!(
            env.lookup_constructor("Color").unwrap().kind,
            SymbolKind::Struct { .. }
        ));
        // A third binding, or a second value, is still a duplicate.
        let err = env
            .define_companion(top_level_value("Color", Type::Int))
            .unwrap_err();
        assert_eq!(err.code(), "E4010");
        // A value without a struct is defined as any value, and constructs nothing.
        env.define_companion(top_level_value("Plain", Type::Int))
            .unwrap();
        assert!(env.lookup_constructor("Plain").is_none());
        // A local shadows the pair: the call names the local.
        env.enter_scope();
        env.define(Symbol::variable(
            "Color",
            Type::Bool,
            Mutability::Val,
            Span::dummy(),
        ))
        .unwrap();
        assert!(env.lookup_constructor("Color").is_none());
        assert_eq!(env.lookup("Color").unwrap().ty(), Type::Bool);
    }

    #[test]
    fn an_enum_or_a_second_struct_is_no_companion() {
        let mut env = ScopeStack::new();
        env.define(Symbol::enum_decl(
            NominalId::new("<test>", "Mode"),
            Visibility::Public,
            None,
            Span::dummy(),
        ))
        .unwrap();
        let err = env
            .define_companion(top_level_value("Mode", Type::Int))
            .unwrap_err();
        assert_eq!(err.code(), "E4010");
        env.define(struct_symbol("User")).unwrap();
        assert_eq!(
            env.define(struct_symbol("User")).unwrap_err().code(),
            "E4010"
        );
    }

    #[test]
    fn an_import_brings_a_struct_and_its_companion_value() {
        let mut env = ScopeStack::new();
        env.define_import("lib/shape", struct_symbol("Size"));
        env.define_import("lib/shape", top_level_value("Size", Type::Int));
        // A second struct of the module is still dropped.
        env.define_import("lib/shape", struct_symbol("Size"));
        assert_eq!(env.lookup("Size").unwrap().ty(), Type::Int);
        assert_eq!(env.resolve("Size", Span::dummy()).unwrap().ty(), Type::Int);
        assert!(matches!(
            env.lookup_type("Size").unwrap().kind,
            SymbolKind::Struct { .. }
        ));
        assert!(matches!(
            env.lookup_constructor("Size").unwrap().kind,
            SymbolKind::Struct { .. }
        ));
        // Another module exporting the name makes it ambiguous.
        env.define_import("lib/other", top_level_value("Size", Type::Int));
        assert!(env.lookup("Size").is_none());
        assert!(env.lookup_constructor("Size").is_none());
    }

    #[test]
    fn test_unresolved_symbol_e4001() {
        let env = ScopeStack::new();
        let err = env.resolve("undefined_var", Span::dummy()).unwrap_err();
        assert_eq!(err.code(), "E4001");
    }
}

//! Local type argument inference for calls of generic declarations (D8 of `generics`).
//!
//! A call of a generic function, extension, struct constructor or variant gets one unknown per
//! type parameter of its callee. The callee's declared types are patterns, in which
//! [`Type::Parameter`] of index `i` stands for unknown `i`; the types they are matched against
//! come from the caller and may mention the caller's own type parameters, which are ordinary
//! types here. The receiver and the expected type give exact solutions, and arguments give lower
//! bounds, which are joined.

use crate::types::{Type, TypeRelations};
use prism_ast::Span;

/// The state of one unknown: an exact solution, from the receiver, the expected type or an
/// explicit type argument, and the lower bounds the arguments gave, with the span of each.
#[derive(Debug, Clone, Default)]
struct Slot {
    exact: Option<Type>,
    lower: Vec<(Type, Span)>,
    /// A solution the expected type suggests where it cannot decide one exactly: `T?` expected
    /// as `X?` fits both `T = X` and `T = X?`. Used only when no argument bounds the unknown.
    fallback: Option<Type>,
}

/// The unknowns of one generic call.
#[derive(Debug, Clone)]
pub struct Unknowns {
    slots: Vec<Slot>,
}

/// An unknown the call left unsolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unsolved {
    /// The index of its type parameter.
    pub index: usize,
}

/// Lower bounds without a join: the argument at `span` gave a bound that does not join the
/// earlier ones. Its own fit reports the mismatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    pub span: Span,
}

impl Unknowns {
    /// One unknown per type parameter.
    pub fn new(count: usize) -> Self {
        Self {
            slots: vec![Slot::default(); count],
        }
    }

    /// Fixes unknown `index` to `ty`, as an explicit type argument does.
    pub fn fix(&mut self, index: usize, ty: Type) {
        if let Some(slot) = self.slots.get_mut(index) {
            slot.exact = Some(ty);
        }
    }

    /// Whether unknown `index` has an exact solution.
    pub fn is_exact(&self, index: usize) -> bool {
        self.slots
            .get(index)
            .is_some_and(|slot| slot.exact.is_some())
    }

    /// Whether `pattern` mentions an unknown without an exact solution.
    pub fn mentions_inexact(&self, pattern: &Type) -> bool {
        self.mentions(pattern, |unknowns, index| !unknowns.is_exact(index))
    }

    /// Whether `pattern` mentions an unknown with no solution at all yet, exact or bounded.
    pub fn mentions_unsolved(&self, pattern: &Type) -> bool {
        self.mentions(pattern, |unknowns, index| unknowns.current(index).is_none())
    }

    fn mentions(&self, pattern: &Type, unsolved: impl Fn(&Self, usize) -> bool + Copy) -> bool {
        match pattern {
            Type::Parameter(parameter) => unsolved(self, parameter.index as usize),
            Type::Reference { inner, .. }
            | Type::Nullable(inner)
            | Type::Heap(inner)
            | Type::List(inner) => self.mentions(inner, unsolved),
            Type::Struct { arguments, .. } | Type::Enum { arguments, .. } => {
                arguments.iter().any(|ty| self.mentions(ty, unsolved))
            }
            Type::Fn { params, ret, .. } => {
                params.iter().any(|ty| self.mentions(ty, unsolved)) || self.mentions(ret, unsolved)
            }
            _ => false,
        }
    }

    /// Matches `pattern` against `actual`, which a value of the pattern's type must be exactly:
    /// the receiver of an extension or the type the call is checked against. Each unknown met
    /// without an exact solution takes the corresponding part of `actual`. Parts that do not
    /// line up solve nothing; the call's checks report them.
    pub fn match_exact(&mut self, pattern: &Type, actual: &Type) {
        if actual == &Type::Error {
            return;
        }
        match (pattern, actual) {
            (Type::Parameter(parameter), _) => {
                if let Some(slot) = self.slots.get_mut(parameter.index as usize) {
                    if slot.exact.is_none() {
                        slot.exact = Some(actual.clone());
                    }
                }
            }
            // `T?` as `X?`: `T` is `X` or `X?`, which nullables flattening makes the same type
            // (D4 of `generics`), so the arguments decide.
            (Type::Nullable(pattern), Type::Nullable(actual))
                if matches!(**pattern, Type::Parameter(_)) =>
            {
                if let Type::Parameter(parameter) = pattern.as_ref() {
                    if let Some(slot) = self.slots.get_mut(parameter.index as usize) {
                        slot.fallback.get_or_insert_with(|| actual.as_ref().clone());
                    }
                }
            }
            (Type::Nullable(pattern), Type::Nullable(actual)) => self.match_exact(pattern, actual),
            // A non-null value fits a nullable expectation: match against its payload.
            (pattern, Type::Nullable(actual)) => self.match_exact(pattern, actual),
            (
                Type::Reference {
                    mutable: pattern_mutable,
                    inner: pattern,
                },
                Type::Reference { mutable, inner },
            ) if pattern_mutable == mutable => self.match_exact(pattern, inner),
            (Type::Heap(pattern), Type::Heap(actual))
            | (Type::List(pattern), Type::List(actual)) => self.match_exact(pattern, actual),
            (
                Type::Struct {
                    id: pattern_id,
                    arguments: patterns,
                },
                Type::Struct { id, arguments },
            )
            | (
                Type::Enum {
                    id: pattern_id,
                    arguments: patterns,
                },
                Type::Enum { id, arguments },
            ) if pattern_id == id && patterns.len() == arguments.len() => {
                for (pattern, actual) in patterns.iter().zip(arguments) {
                    self.match_exact(pattern, actual);
                }
            }
            (
                Type::Fn {
                    params: patterns,
                    ret: pattern_ret,
                    ..
                },
                Type::Fn { params, ret, .. },
            ) if patterns.len() == params.len() => {
                for (pattern, actual) in patterns.iter().zip(params) {
                    self.match_exact(pattern, actual);
                }
                self.match_exact(pattern_ret, ret);
            }
            _ => {}
        }
    }

    /// Matches `pattern`, an argument's declared parameter type, against `found`, the type of
    /// the argument, giving lower bounds (D8 of `generics`, step 4). It looks through an
    /// implicit borrow (declared `&T`, found `Int`), strips the references of a found type
    /// matched against a by-value unknown (declared `T`, found `&Int` gives `T >= Int`, since
    /// type arguments are borrow-free), matches `T?` against `U?` or `U`, ignores `null`'s
    /// `Nothing?` at a nullable position, and descends into nominal arguments and function
    /// types.
    pub fn match_lower(&mut self, pattern: &Type, found: &Type, span: Span) {
        // Only the argument itself is a value that is copied or moved out of its borrows; a
        // borrow inside it stays, and makes the solution hold one (`E4022`).
        let mut found = found;
        if matches!(pattern, Type::Parameter(_)) {
            while let Type::Reference { inner, .. } = found {
                found = inner;
            }
        }
        self.match_lower_inside(pattern, found, span);
    }

    fn match_lower_inside(&mut self, pattern: &Type, found: &Type, span: Span) {
        if found == &Type::Error {
            return;
        }
        match (pattern, found) {
            (Type::Parameter(parameter), _) => {
                if let Some(slot) = self.slots.get_mut(parameter.index as usize) {
                    slot.lower.push((found.clone(), span));
                }
            }
            (Type::Nullable(_), Type::Nullable(inner)) if **inner == Type::Nothing => {}
            (Type::Nullable(pattern), Type::Nullable(found)) => {
                self.match_lower_inside(pattern, found, span)
            }
            (Type::Nullable(pattern), found) => self.match_lower_inside(pattern, found, span),
            (Type::Reference { inner: pattern, .. }, Type::Reference { inner, .. }) => {
                self.match_lower_inside(pattern, inner, span)
            }
            // An implicit borrow: the argument is the referent.
            (Type::Reference { inner: pattern, .. }, found) => {
                self.match_lower_inside(pattern, found, span)
            }
            (Type::Heap(pattern), Type::Heap(found)) | (Type::List(pattern), Type::List(found)) => {
                self.match_lower_inside(pattern, found, span)
            }
            (
                Type::Struct {
                    id: pattern_id,
                    arguments: patterns,
                },
                Type::Struct { id, arguments },
            )
            | (
                Type::Enum {
                    id: pattern_id,
                    arguments: patterns,
                },
                Type::Enum { id, arguments },
            ) if pattern_id == id && patterns.len() == arguments.len() => {
                for (pattern, found) in patterns.iter().zip(arguments) {
                    self.match_lower_inside(pattern, found, span);
                }
            }
            (
                Type::Fn {
                    params: patterns,
                    ret: pattern_ret,
                    ..
                },
                Type::Fn { params, ret, .. },
            ) if patterns.len() == params.len() => {
                for (pattern, found) in patterns.iter().zip(params) {
                    self.match_lower_inside(pattern, found, span);
                }
                self.match_lower_inside(pattern_ret, ret, span);
            }
            _ => {}
        }
    }

    /// The current solution of unknown `index`: its exact solution, else the join of its lower
    /// bounds so far, else `None`.
    pub fn current(&self, index: usize) -> Option<Type> {
        let slot = self.slots.get(index)?;
        if let Some(exact) = &slot.exact {
            return Some(exact.clone());
        }
        Self::join_bounds(&slot.lower)
            .map(|(joined, _)| joined)
            .or_else(|| slot.fallback.clone())
    }

    /// The current solutions, with `Type::Error` for each unknown not solved yet, for
    /// substituting into a declared type whose solved parts are needed early.
    pub fn partial(&self) -> Vec<Type> {
        (0..self.slots.len())
            .map(|index| self.current(index).unwrap_or(Type::Error))
            .collect()
    }

    /// Solves every unknown (D8 of `generics`, step 5): an exact solution wins, else the join of
    /// the lower bounds. Returns the solutions, the first unknown without any, and the lower
    /// bounds that do not join (whose unknown takes the join of the bounds before it).
    pub fn solve(&self) -> (Vec<Type>, Option<Unsolved>, Option<Conflict>) {
        let mut solutions = Vec::new();
        let mut unsolved = None;
        let mut conflict = None;
        for (index, slot) in self.slots.iter().enumerate() {
            if let Some(exact) = &slot.exact {
                solutions.push(exact.clone());
                continue;
            }
            match Self::join_bounds(&slot.lower) {
                Some((joined, breaking)) => {
                    if conflict.is_none() {
                        conflict = breaking.map(|span| Conflict { span });
                    }
                    solutions.push(joined);
                }
                None => match &slot.fallback {
                    Some(fallback) => solutions.push(fallback.clone()),
                    None => {
                        if unsolved.is_none() {
                            unsolved = Some(Unsolved { index });
                        }
                        solutions.push(Type::Error);
                    }
                },
            }
        }
        (solutions, unsolved, conflict)
    }

    /// The join of `bounds`, and the span of the first bound that does not join the ones before
    /// it, which is left out.
    fn join_bounds(bounds: &[(Type, Span)]) -> Option<(Type, Option<Span>)> {
        let mut bounds = bounds.iter();
        let (first, _) = bounds.next()?;
        let mut joined = first.clone();
        let mut breaking = None;
        for (bound, span) in bounds {
            match joined.join(bound) {
                Some(next) => joined = next,
                None => {
                    breaking.get_or_insert(*span);
                }
            }
        }
        Some((joined, breaking))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::NominalId;
    use prism_typed_tree::TypeParameter;

    fn parameter(index: u32) -> Type {
        Type::Parameter(TypeParameter {
            index,
            name: format!("T{}", index),
            bound: None,
        })
    }

    fn span(n: u32) -> Span {
        Span::new(n as usize, n as usize + 1, 1, n)
    }

    fn reference(inner: Type) -> Type {
        Type::Reference {
            mutable: false,
            inner: Box::new(inner),
        }
    }

    #[test]
    fn arguments_join_into_the_solution() {
        // `pick(1, null)` for `fun <T> pick(a: T, b: T): T` gives `T = Int?`.
        let mut unknowns = Unknowns::new(1);
        unknowns.match_lower(&parameter(0), &Type::Int, span(1));
        unknowns.match_lower(&parameter(0), &Type::Nothing.nullable(), span(2));
        let (solutions, unsolved, conflict) = unknowns.solve();
        assert_eq!(solutions, vec![Type::Int.nullable()]);
        assert_eq!((unsolved, conflict), (None, None));
    }

    #[test]
    fn conflicting_bounds_name_the_breaking_argument() {
        let mut unknowns = Unknowns::new(1);
        unknowns.match_lower(&parameter(0), &Type::Int, span(1));
        unknowns.match_lower(&parameter(0), &Type::String, span(2));
        let (solutions, _, conflict) = unknowns.solve();
        assert_eq!(solutions, vec![Type::Int]);
        assert_eq!(conflict, Some(Conflict { span: span(2) }));
    }

    #[test]
    fn exact_solutions_win_over_bounds() {
        let boxed = NominalId::new("main", "Box");
        let pattern = Type::Struct {
            id: boxed.clone(),
            arguments: vec![parameter(0)],
        };
        let expected = Type::Struct {
            id: boxed,
            arguments: vec![Type::Int.nullable()],
        };
        let mut unknowns = Unknowns::new(1);
        unknowns.match_exact(&pattern, &expected);
        unknowns.match_lower(&parameter(0), &Type::Nothing.nullable(), span(1));
        assert_eq!(unknowns.solve().0, vec![Type::Int.nullable()]);
        assert!(unknowns.is_exact(0));
    }

    #[test]
    fn a_nullable_expectation_matches_a_non_null_pattern_inside() {
        let option = NominalId::new("main", "Option");
        let pattern = Type::Enum {
            id: option.clone(),
            arguments: vec![parameter(0)],
        };
        let expected = Type::Enum {
            id: option,
            arguments: vec![Type::Int],
        }
        .nullable();
        let mut unknowns = Unknowns::new(1);
        unknowns.match_exact(&pattern, &expected);
        assert_eq!(unknowns.solve().0, vec![Type::Int]);
    }

    #[test]
    fn borrows_are_looked_through_and_stripped() {
        // Declared `&T`, found `Int`: an implicit borrow.
        let mut unknowns = Unknowns::new(2);
        unknowns.match_lower(&reference(parameter(0)), &Type::Int, span(1));
        // Declared `T`, found `&String`: the referent, since type arguments are borrow-free.
        unknowns.match_lower(&parameter(1), &reference(Type::String), span(2));
        assert_eq!(unknowns.solve().0, vec![Type::Int, Type::String]);
    }

    #[test]
    fn nullable_patterns_match_values_and_ignore_null() {
        let mut unknowns = Unknowns::new(1);
        let pattern = parameter(0).nullable();
        unknowns.match_lower(&pattern, &Type::Nothing.nullable(), span(1));
        assert!(unknowns.mentions_unsolved(&pattern));
        unknowns.match_lower(&pattern, &Type::String, span(2));
        assert_eq!(unknowns.solve().0, vec![Type::String]);
    }

    #[test]
    fn function_types_and_nominal_arguments_are_descended() {
        let boxed = NominalId::new("main", "Box");
        let pattern = Type::function(
            false,
            vec![parameter(0)],
            Type::Struct {
                id: boxed.clone(),
                arguments: vec![parameter(1)],
            },
        );
        let found = Type::function(
            false,
            vec![Type::Int],
            Type::Struct {
                id: boxed,
                arguments: vec![Type::Bool],
            },
        );
        let mut unknowns = Unknowns::new(2);
        unknowns.match_lower(&pattern, &found, span(1));
        assert_eq!(unknowns.solve().0, vec![Type::Int, Type::Bool]);
    }

    #[test]
    fn a_nullable_expectation_of_a_nullable_parameter_lets_the_arguments_decide() {
        // `orNull(s, true)` for `fun <T> orNull(x: T, keep: Bool): T?`, expected `String?`
        // with `s: String?`: `T = String?`, and `T?` flattens to `String?`.
        let mut unknowns = Unknowns::new(1);
        unknowns.match_exact(&parameter(0).nullable(), &Type::String.nullable());
        assert!(!unknowns.is_exact(0));
        unknowns.match_lower(&parameter(0), &Type::String.nullable(), span(1));
        assert_eq!(unknowns.solve().0, vec![Type::String.nullable()]);
        // Without an argument, the expectation decides.
        let mut unknowns = Unknowns::new(1);
        unknowns.match_exact(&parameter(0).nullable(), &Type::String.nullable());
        assert_eq!(unknowns.solve(), (vec![Type::String], None, None));
    }

    #[test]
    fn a_borrow_inside_an_argument_is_kept() {
        // `apply(s)` with `s: (&Int) -> Unit` for `f: (T) -> Unit` gives `T = &Int`.
        let pattern = Type::function(false, vec![parameter(0)], Type::Unit);
        let found = Type::function(false, vec![reference(Type::Int)], Type::Unit);
        let mut unknowns = Unknowns::new(1);
        unknowns.match_lower(&pattern, &found, span(1));
        assert_eq!(unknowns.solve().0, vec![reference(Type::Int)]);
    }

    #[test]
    fn an_unknown_without_any_solution_is_unsolved() {
        let mut unknowns = Unknowns::new(2);
        unknowns.fix(0, Type::Int);
        let (_, unsolved, _) = unknowns.solve();
        assert_eq!(unsolved, Some(Unsolved { index: 1 }));
        assert!(unknowns.mentions_inexact(&parameter(1)));
        assert!(!unknowns.mentions_inexact(&parameter(0)));
    }
}

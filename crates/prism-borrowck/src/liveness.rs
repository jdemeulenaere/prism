//! Liveness of loan-carrying locals (D5 of `borrowck-exclusivity`): a backward analysis whose
//! bits are the body's loan-carrying locals ([`BorrowSet::carriers`]).
//!
//! A local is used by any effect of [`crate::uses`] on a place rooted at it (an operand, a
//! borrow, a read, a discriminant, a function-value callee, a closure capture, or a write
//! through it), and defined, which kills it, by a whole-local assignment, a call destination
//! or `StorageDead`. Drops are not uses: only closure values among loan carriers are dropped,
//! and dropping one reads nothing it borrowed. That holds for a closure that owns some captures
//! and borrows others too (D14 of `escaping-closures`): its environment's drop function drops
//! the owned captures by structural drop glue, which frees memory and never reads through a
//! reference or runs user code. User-defined destructors would have to revisit this. The
//! return place is not used by `Return`, since
//! what a body returns is checked against the elision rules instead (`returns`); but a closure's
//! capture locals and the arguments of type `&mut R` (or `(&mut R)?`) whose `R` carries loans
//! are used at every return, because the caller sees them afterwards.

use crate::borrows::BorrowSet;
use crate::dataflow::{Analysis, BitSet, Direction};
use crate::location::Location;
use crate::places;
use crate::uses::{statement_effects, terminator_effects, PlaceEffect};
use prism_mir::{Body, LocalId, LocalKind, Statement, Terminator, TerminatorKind};
use prism_typed_tree::ArenaId;

/// The locals every `Return` of `body` uses: its loan-carrying capture locals and its
/// arguments of type `&mut R` (or `(&mut R)?`) whose `R` carries loans, which the caller sees
/// afterwards.
pub fn returned_locals(body: &Body, borrows: &BorrowSet) -> Vec<LocalId> {
    body.locals
        .iter()
        .filter(|(local, declaration)| {
            borrows.carrier(*local).is_some()
                && match declaration.kind {
                    LocalKind::Capture => true,
                    LocalKind::Argument => places::writes_loans_through(&declaration.ty),
                    _ => false,
                }
        })
        .map(|(local, _)| local)
        .collect()
}

/// The loan-carrying locals that may be used later.
pub struct Liveness<'a> {
    pub borrows: &'a BorrowSet,
    /// What each location does to loan-carrying locals, in evaluation order: each carrier it
    /// uses (`true`) or defines (`false`), found once rather than on every pass of the solver.
    /// A `Return` uses the locals the caller sees afterwards.
    effects: Vec<Vec<(u32, bool)>>,
    /// The position in `effects` of each block's first location.
    block_starts: Vec<usize>,
}

impl<'a> Liveness<'a> {
    pub fn new(body: &Body, borrows: &'a BorrowSet) -> Self {
        let returned = returned_locals(body, borrows);
        let carrier = |local: LocalId| borrows.carrier(local).map(|bit| bit as u32);
        let mut effects = Vec::new();
        let mut block_starts = Vec::with_capacity(body.blocks.len());
        for (_, data) in body.blocks.iter() {
            block_starts.push(effects.len());
            for statement in &data.statements {
                effects.push(carrier_effects(statement_effects(statement), &carrier));
            }
            effects.push(match data.terminator.kind {
                TerminatorKind::Return => returned
                    .iter()
                    .filter_map(|local| carrier(*local))
                    .map(|bit| (bit, true))
                    .collect(),
                _ => carrier_effects(terminator_effects(&data.terminator), &carrier),
            });
        }
        Self {
            borrows,
            effects,
            block_starts,
        }
    }

    /// The carriers `location` uses (`true`) or defines (`false`), in evaluation order.
    pub fn carrier_effects(&self, location: Location) -> &[(u32, bool)] {
        &self.effects[self.block_starts[location.block.index()] + location.statement]
    }

    /// Applies the effects of `location`, backward.
    fn apply_location(&self, state: &mut BitSet, location: Location) {
        for (bit, used) in self.carrier_effects(location).iter().rev() {
            if *used {
                state.insert(*bit as usize);
            } else {
                state.remove(*bit as usize);
            }
        }
    }

    /// Whether `local` is in `state`.
    pub fn is_live(&self, state: &BitSet, local: LocalId) -> bool {
        self.borrows
            .carrier(local)
            .is_some_and(|bit| state.contains(bit))
    }
}

/// The uses and definitions of carriers among `effects`.
fn carrier_effects(
    effects: Vec<PlaceEffect>,
    carrier: &impl Fn(LocalId) -> Option<u32>,
) -> Vec<(u32, bool)> {
    effects
        .into_iter()
        .filter_map(|effect| match effect {
            PlaceEffect::Use(place, _, _) | PlaceEffect::Move(place, _) => {
                carrier(place.local).map(|bit| (bit, true))
            }
            PlaceEffect::Init(local) | PlaceEffect::StorageDead(local) => {
                carrier(local).map(|bit| (bit, false))
            }
            PlaceEffect::StorageLive(_) => None,
        })
        .collect()
}

impl Analysis for Liveness<'_> {
    type Domain = BitSet;

    fn bottom(&self) -> BitSet {
        BitSet::new(self.borrows.carriers.len())
    }

    fn direction(&self) -> Direction {
        Direction::Backward
    }

    fn initialize_boundary(&self, _: &mut BitSet) {}

    fn apply_statement(&self, state: &mut BitSet, _: &Statement, location: Location) {
        self.apply_location(state, location);
    }

    fn apply_terminator(&self, state: &mut BitSet, _: &Terminator, location: Location) {
        self.apply_location(state, location);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataflow::solve;
    use crate::location::locations;
    use crate::test_bodies::{body, elaborate};
    use prism_mir::{BodyOwner, MirFile, StatementKind};

    fn elaborated(source: &str) -> MirFile {
        elaborate(source)
    }

    fn local_named(body: &Body, name: &str) -> LocalId {
        body.locals
            .iter()
            .find(|(_, l)| l.name.as_deref() == Some(name))
            .map(|(id, _)| id)
            .unwrap_or_else(|| panic!("no local {}", name))
    }

    /// The statements of `body` assigning the whole `local`, in visiting order.
    fn assignments(body: &Body, local: LocalId) -> Vec<Location> {
        locations(body)
            .into_iter()
            .filter(|location| {
                matches!(
                    body.blocks[location.block].statements.get(location.statement),
                    Some(Statement { kind: StatementKind::Assign(place, _), .. })
                        if *place == prism_mir::Place::local(local)
                )
            })
            .collect()
    }

    const PRINT: &str = "fun print(n: &Int) {}\nfun run(f: () -> Int): Int = f()\n";

    #[test]
    fn a_reference_is_dead_after_its_last_use() {
        let file = elaborated(&format!(
            "{}fun f() {{ var x = 1; val r = &x; print(r); x = 2 }}\n",
            PRINT
        ));
        let f = body(&file, "f");
        let borrows = BorrowSet::gather(&file, f);
        let liveness = solve(f, Liveness::new(f, &borrows));
        let r = local_named(f, "r");
        // `x = 2`, after `var x = 1`.
        let write = assignments(f, local_named(f, "x"))[1];
        let mut cursor = liveness.cursor(f);
        assert!(!liveness.analysis.is_live(cursor.seek(write), r));
    }

    #[test]
    fn a_reference_used_in_the_next_iteration_is_live_around_the_loop() {
        let file = elaborated(&format!(
            "{}fun f(c: Bool) {{ var x = 1; val r = &x; while (c) {{ print(r); x = 2 }} }}\n",
            PRINT
        ));
        let f = body(&file, "f");
        let borrows = BorrowSet::gather(&file, f);
        let liveness = solve(f, Liveness::new(f, &borrows));
        let write = assignments(f, local_named(f, "x"))[1];
        let mut cursor = liveness.cursor(f);
        assert!(liveness
            .analysis
            .is_live(cursor.seek(write), local_named(f, "r")));
    }

    #[test]
    fn a_capture_is_live_at_the_return() {
        let file = elaborated(&format!(
            "{}fun f() {{ var a = 1; var r: &Int = &a; run {{ val n = 2; r = &a; 1 }} }}\n",
            PRINT
        ));
        let (_, closure) = file
            .bodies
            .iter()
            .find(|(_, body)| matches!(body.owner, BodyOwner::Closure { .. }))
            .unwrap();
        let borrows = BorrowSet::gather(&file, closure);
        let liveness = solve(closure, Liveness::new(closure, &borrows));
        let r = local_named(closure, "r");
        let n = local_named(closure, "n");
        // Where the closure's local `n` ends, the capture `r` is still live: the caller sees it.
        let end = locations(closure)
            .into_iter()
            .find(|location| {
                matches!(
                    closure.blocks[location.block].statements.get(location.statement),
                    Some(Statement { kind: StatementKind::StorageDead(local), .. }) if *local == n
                )
            })
            .expect("the end of n");
        let mut cursor = liveness.cursor(closure);
        assert!(liveness.analysis.is_live(cursor.seek(end), r));
    }

    #[test]
    fn a_dropped_closure_is_dead_before_its_drop() {
        let file = elaborated(&format!(
            "{}fun f(x: Int): Int {{ val g = {{ x }}; return g() }}\n",
            PRINT
        ));
        let f = body(&file, "f");
        let borrows = BorrowSet::gather(&file, f);
        let liveness = solve(f, Liveness::new(f, &borrows));
        let g = local_named(f, "g");
        let drop = locations(f)
            .into_iter()
            .find(|location| {
                location.is_terminator(f)
                    && matches!(
                        &f.blocks[location.block].terminator.kind,
                        TerminatorKind::Drop { place, .. } if place.local == g
                    )
            })
            .expect("the drop of g");
        let call = locations(f)
            .into_iter()
            .find(|location| {
                location.is_terminator(f)
                    && matches!(
                        &f.blocks[location.block].terminator.kind,
                        TerminatorKind::Call { .. }
                    )
            })
            .expect("the call of g");
        let mut cursor = liveness.cursor(f);
        assert!(!liveness.analysis.is_live(cursor.seek(call), g));
        assert!(!liveness.analysis.is_live(cursor.seek(drop), g));
    }
}

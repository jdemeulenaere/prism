//! `prism-borrowck`: the borrow checker, which checks the MIR of one file and elaborates its
//! drops (`borrowck-moves`, `borrowck-exclusivity`, `borrowck-elision`).
//!
//! [`check_file`] reads nothing but the file's [`MirFile`]: the type table travels in it, and
//! every operand already says whether it moves and every call its parameter types, so no other
//! file's bodies, MIR or ABI is read (the single-file compilation units and deterministic
//! hermetic builds invariants). It runs in two stages (D16 of `borrowck-exclusivity`):
//!
//! 1. **Moves.** For each body as lowered it builds move paths over places, solves
//!    initialization dataflows over the body's control-flow graph, and checks every use of a
//!    place against them: a use of a moved value is `E5001`, a read of an uninitialized one
//!    `E5002`. If any body has an error, those errors alone are returned, since conflicts on
//!    moved values would be noise. Otherwise it elaborates the drops of every body, so that a
//!    moved value is never dropped and a conditionally moved one is dropped under a drop flag,
//!    and infers how each closure borrows its captures ([`captures`]).
//! 2. **Exclusivity.** For each elaborated body it gathers the loans ([`borrows`]), solves the
//!    liveness of loan-carrying locals ([`liveness`], backward) and the loans each may hold
//!    ([`loan_flow`], forward, dropping what a local holds once it is dead, so that its states
//!    hold the loans in scope rather than every loan made so far), then checks every access against the live loans of overlapping
//!    places (`E5003` to `E5006`) and every write of a function value that may hold a capture
//!    or a function-typed parameter to a place it escapes through (`E5007`), then each write of
//!    its return place against the lifetime elision rules ([`elision`]): a returned borrow of a
//!    local, a temporary or a by-value parameter is `E5008`, one derived from an input the
//!    rule does not tie the result to `E5009` (D14 of `borrowck-elision`). Without errors it
//!    returns the elaborated MIR, whose closure bodies record their capture modes.
//!
//! The elision rules decide, from a signature alone, which inputs a returned borrow is tied to,
//! and both halves use the same function: a caller keeps those inputs borrowed while the result
//! is live, and a callee may only return borrows derived from them, each argument and capture
//! that carries loans holding an input that stands for what the caller passed. Since an imported
//! callee's signature comes from its ABI, caller and callee agree across files with no lifetime
//! annotation (the zero-cost memory-safety invariant).

pub mod analyses;
pub mod borrows;
pub mod captures;
mod check;
mod conflicts;
pub mod dataflow;
pub mod dominators;
mod elaborate;
pub mod elision;
mod error;
mod escapes;
pub mod liveness;
pub mod loan_flow;
pub mod location;
pub mod move_paths;
pub mod places;
mod returns;
#[cfg(test)]
mod test_bodies;
pub mod uses;

pub use error::{
    AccessKind, BorrowError, BorrowErrorKind, Borrowed, EscapingHolder, Label, UntiedReason,
};
pub use location::Location;

use analyses::{MaybeInitialized, ReachingUninitializations};
use dataflow::solve;
use move_paths::MoveData;
use prism_mir::{MirFile, MirPhase};
use prism_typed_tree::Arena;

/// Checks every body of `file`, as lowered, for uses of moved and uninitialized values, then
/// its elaborated MIR for borrow conflicts and escaping closures, and returns the file with its
/// drops elaborated and its capture modes recorded ([`MirPhase::Elaborated`]) when no body has
/// an error, or the errors sorted by primary location, then code.
pub fn check_file(file: &MirFile) -> Result<MirFile, Vec<BorrowError>> {
    let (elaborated, captures) = check_moves_and_elaborate(file)?;
    let mut errors = Vec::new();
    for (_, body) in elaborated.bodies.iter() {
        errors.extend(check_exclusivity(&elaborated, body, &captures));
    }
    if !errors.is_empty() {
        sort_errors(&mut errors);
        return Err(errors);
    }
    Ok(elaborated)
}

/// The conflicts, escapes and returned borrows of the elaborated `body` of `file`, whose
/// closures borrow their captures by `captures`.
fn check_exclusivity(
    file: &MirFile,
    body: &prism_mir::Body,
    captures: &captures::CaptureModes,
) -> Vec<BorrowError> {
    let borrows = borrows::BorrowSet::gather(file, body);
    // A body that makes no loan and has no local that could hold one, as most bodies of plain
    // values, has nothing in scope to conflict with, escape or return.
    if borrows.carriers.is_empty() && borrows.loans.is_empty() {
        return Vec::new();
    }
    let liveness = solve(body, liveness::Liveness::new(body, &borrows));
    let flow = solve(
        body,
        loan_flow::LoanFlow::pruned(file, body, &borrows, &liveness),
    );
    let (mut escapes, escaped) = escapes::check_escapes(file, body, &borrows, &flow);
    // An escape's diagnostic may name what an owned capture's operand holds after the operand
    // is dead, which only the full flow keeps.
    if !escapes.is_empty() {
        let full = solve(body, loan_flow::LoanFlow::new(file, body, &borrows));
        escapes = escapes::check_escapes(file, body, &borrows, &full).0;
    }
    let mut errors =
        conflicts::check_conflicts(file, body, captures, &borrows, &liveness, &flow, &escaped);
    errors.extend(escapes);
    errors.extend(returns::check_returns(file, body, &borrows, &flow));
    errors
}

/// The move check of every body of `file`, as lowered; when no body has an error, the file
/// with its drops elaborated and its closure bodies' capture modes recorded, with the capture
/// modes and the uses that decided them.
pub(crate) fn check_moves_and_elaborate(
    file: &MirFile,
) -> Result<(MirFile, captures::CaptureModes), Vec<BorrowError>> {
    assert_eq!(
        file.phase,
        MirPhase::Built,
        "internal error: borrow-checking MIR that is already elaborated"
    );
    let mut errors = Vec::new();
    let mut elaborated = Arena::new();
    for (_, body) in file.bodies.iter() {
        let data = MoveData::gather(body, &file.types);
        let maybe = solve(body, MaybeInitialized { data: &data });
        let reaching = solve(body, ReachingUninitializations { data: &data });
        errors.extend(check::check_body(file, body, &data, &maybe, &reaching));
        if errors.is_empty() {
            elaborated.alloc(elaborate::elaborate_body(body, &data, &maybe, &reaching));
        }
    }
    if !errors.is_empty() {
        sort_errors(&mut errors);
        return Err(errors);
    }
    let mut elaborated = MirFile {
        module: file.module.clone(),
        phase: MirPhase::Elaborated,
        types: file.types.clone(),
        bodies: elaborated,
    };
    let captures = captures::infer_capture_modes(&elaborated);
    for (id, modes) in &captures.modes {
        elaborated.bodies[*id].capture_modes = modes.clone();
    }
    Ok((elaborated, captures))
}

/// Checks the drops of an elaborated file (D15 of `borrowck-moves`): every drop of a whole local
/// and every use finds it holding a value on every path reaching it (so no value is dropped
/// twice or used after its drop), no value that needs a drop is lost to a new assignment, the end
/// of its storage or the return, and every drop flag that guards a drop is true exactly when its
/// local holds a value. Returns one line per violation, empty when the elaboration is sound. Only the tests of the CLI run it, over every passing fixture and
/// example: it is the only check of elaboration before the backend executes it.
pub fn verify_elaborated(file: &MirFile) -> Vec<String> {
    let mut violations = Vec::new();
    for (id, body) in file.bodies.iter() {
        for violation in elaborate::verify_body(body, &file.types) {
            violations.push(format!("{}: {}", file.body_name(id), violation));
        }
    }
    violations
}

/// Sorts errors by primary location, then code, so the output is deterministic.
fn sort_errors(errors: &mut [BorrowError]) {
    errors.sort_by(|a, b| {
        let key = |error: &BorrowError| {
            (
                error.primary.span.line,
                error.primary.span.column,
                error.primary.span.start,
            )
        };
        key(a).cmp(&key(b)).then_with(|| a.code().cmp(b.code()))
    });
}

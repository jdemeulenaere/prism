//! `prism-borrowck`: the borrow checker, which checks the MIR of one file and elaborates its
//! drops (`borrowck-moves`).
//!
//! [`check_file`] reads nothing but the file's [`MirFile`]: the type table travels in it, and
//! every operand already says whether it moves, so no other file's bodies, MIR or ABI is read
//! (the single-file compilation units and deterministic hermetic builds invariants). For each
//! body it builds move paths over places, solves initialization dataflows over the body's
//! control-flow graph, and checks every use of a place against them: a use of a moved value is
//! `E5001`, a read of an uninitialized one `E5002`. When no body has an error, it elaborates the
//! drops of every body, so that a moved value is never dropped and a conditionally moved one is
//! dropped under a drop flag, and returns the elaborated MIR.
//!
//! Later stages extend the crate (D16 of `borrowck-moves`): `borrowck-exclusivity` adds borrow
//! conflicts on the elaborated MIR, with liveness as a backward analysis of [`dataflow`] and
//! disjoint fields from the move paths' field projections; `borrowck-elision` adds returned
//! borrows. Their errors join [`BorrowErrorKind`] with the next `E5xxx` codes.

pub mod analyses;
mod check;
pub mod dataflow;
pub mod dominators;
mod error;
pub mod location;
pub mod move_paths;
#[cfg(test)]
mod test_bodies;
pub mod uses;

pub use error::{BorrowError, BorrowErrorKind, Label};
pub use location::Location;

use move_paths::MoveData;
use prism_mir::MirFile;

/// Checks every body of `file`, as lowered, for uses of moved and uninitialized values, and
/// returns the file with its drops elaborated when no body has an error, or the errors sorted by
/// primary location, then code.
pub fn check_file(file: &MirFile) -> Result<MirFile, Vec<BorrowError>> {
    let mut errors = Vec::new();
    for (_, body) in file.bodies.iter() {
        let data = MoveData::gather(body, &file.types);
        errors.extend(check::check_body(file, body, &data));
    }
    if !errors.is_empty() {
        sort_errors(&mut errors);
        return Err(errors);
    }
    Ok(file.clone())
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

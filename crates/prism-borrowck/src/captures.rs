//! Capture-mode inference (D10 of `borrowck-exclusivity`): how each closure borrows each binding
//! it captures.
//!
//! A capture is mutable when the closure's body assigns its capture local or a place rooted at
//! it (a field, or through it when it is a reference), is a call's destination rooted at it,
//! borrows a place rooted at it mutably (written, as an argument or as a receiver), or builds a
//! nested closure that captures a place rooted at it mutably; otherwise it is shared. Closures
//! are visited innermost first, so a nested closure's mode is known when its parent is decided.
//! A closure cannot move a capture (D11), so no capture is taken by value.

use prism_ast::Span;
use prism_mir::{
    BodyId, BodyOwner, CaptureMode, LocalId, MirFile, Rvalue, StatementKind, TerminatorKind,
};
use std::collections::HashMap;

/// The inferred capture modes of the closure bodies of a file.
#[derive(Debug, Clone, Default)]
pub struct CaptureModes {
    /// The mode of each capture local of each closure body, in capture order.
    pub modes: HashMap<BodyId, Vec<CaptureMode>>,
    /// For each capture of each closure body, the use that decided its mode: the first mutable
    /// use in source order for a mutable capture, the first use for a shared one.
    pub deciding_uses: HashMap<BodyId, Vec<Span>>,
}

impl CaptureModes {
    /// The mode of capture `index` of the closure `body`.
    pub fn mode(&self, body: BodyId, index: usize) -> CaptureMode {
        self.modes
            .get(&body)
            .and_then(|modes| modes.get(index))
            .copied()
            .unwrap_or(CaptureMode::Shared)
    }

    /// The use that decided the mode of capture `index` of the closure `body`.
    pub fn deciding_use(&self, body: BodyId, index: usize) -> Option<Span> {
        self.deciding_uses
            .get(&body)
            .and_then(|spans| spans.get(index))
            .copied()
    }
}

/// Infers the capture modes of every closure body of `file`.
pub fn infer_capture_modes(file: &MirFile) -> CaptureModes {
    let mut result = CaptureModes::default();
    // A closure's body follows its parent's, so the reverse order visits nested closures first.
    let mut closures: Vec<BodyId> = file
        .bodies
        .iter()
        .filter(|(_, body)| matches!(body.owner, BodyOwner::Closure { .. }))
        .map(|(id, _)| id)
        .collect();
    closures.reverse();
    for id in closures {
        let body = &file.bodies[id];
        let mut modes = Vec::with_capacity(body.capture_count);
        let mut deciding_uses = Vec::with_capacity(body.capture_count);
        for index in 0..body.capture_count {
            let capture = LocalId((body.argument_count + 1 + index) as u32);
            match first_mutable_use(file, &result, id, capture) {
                Some(span) => {
                    modes.push(CaptureMode::Mutable);
                    deciding_uses.push(span);
                }
                None => {
                    modes.push(CaptureMode::Shared);
                    // The capture local's span is the binding's first use in the closure.
                    deciding_uses.push(body.locals[capture].span);
                }
            }
        }
        result.modes.insert(id, modes);
        result.deciding_uses.insert(id, deciding_uses);
    }
    result
}

/// The span of the first use, in source order, of `capture` in the closure `id` that needs a
/// mutable borrow of it.
fn first_mutable_use(
    file: &MirFile,
    known: &CaptureModes,
    id: BodyId,
    capture: LocalId,
) -> Option<Span> {
    let body = &file.bodies[id];
    let mut uses: Vec<Span> = Vec::new();
    for (_, block) in body.blocks.iter() {
        for statement in &block.statements {
            let StatementKind::Assign(place, rvalue) = &statement.kind else {
                continue;
            };
            if place.local == capture {
                uses.push(statement.span);
            }
            match rvalue.as_ref() {
                Rvalue::Borrow {
                    mutable: true,
                    place,
                    ..
                } if place.local == capture => uses.push(statement.span),
                Rvalue::Closure {
                    body: nested,
                    captures,
                } => {
                    let mutable = captures.iter().enumerate().any(|(index, place)| {
                        place.local == capture && known.mode(*nested, index) == CaptureMode::Mutable
                    });
                    if mutable {
                        uses.push(statement.span);
                    }
                }
                _ => {}
            }
        }
        if let TerminatorKind::Call { destination, .. } = &block.terminator.kind {
            if destination.local == capture {
                uses.push(block.terminator.span);
            }
        }
    }
    uses.into_iter()
        .min_by_key(|span| (span.line, span.column, span.start))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_bodies::lower;

    /// The modes of the captures of the only closure of `source`'s function `f`.
    fn modes(source: &str) -> Vec<CaptureMode> {
        let file = lower(source);
        let inferred = infer_capture_modes(&file);
        let (id, _) = file
            .bodies
            .iter()
            .find(|(_, body)| {
                matches!(body.owner, BodyOwner::Closure { parent, .. }
                    if file.body_name(parent) == "main::f")
            })
            .expect("a closure of f");
        inferred.modes[&id].clone()
    }

    const DECLARATIONS: &str = "\
struct User { var name: String }
fun greet(s: &String): Int = 1
fun bump(r: &mut Int) {}
fun run(f: () -> Int): Int = f()
";

    fn modes_in(body: &str) -> Vec<CaptureMode> {
        modes(&format!("{}{}", DECLARATIONS, body))
    }

    use CaptureMode::{Mutable, Shared};

    #[test]
    fn reads_are_shared() {
        assert_eq!(
            modes_in("fun f(u: User): Int = run { greet(u.name) }\n"),
            vec![Shared]
        );
        assert_eq!(
            modes_in("fun f(n: Int): Int = run { n + 1 }\n"),
            vec![Shared]
        );
    }

    #[test]
    fn writes_and_mutable_borrows_are_mutable() {
        assert_eq!(
            modes_in("fun f() { var u = User(\"a\"); run { u.name = \"z\"; 1 } }\n"),
            vec![Mutable]
        );
        assert_eq!(
            modes_in("fun f(r: &mut Int): Int = run { *r = 2; 1 }\n"),
            vec![Mutable]
        );
        assert_eq!(
            modes_in("fun f(r: &mut Int): Int = run { bump(r); 1 }\n"),
            vec![Mutable]
        );
    }

    #[test]
    fn a_nested_mutable_capture_makes_the_capture_mutable() {
        assert_eq!(
            modes_in("fun f() { var x = 1; run { run { x = 2; 1 } } }\n"),
            vec![Mutable]
        );
    }

    #[test]
    fn modes_follow_capture_order() {
        assert_eq!(
            modes_in("fun f(n: Int) { var u = User(\"a\"); run { u.name = \"z\"; n } }\n"),
            vec![Mutable, Shared]
        );
    }
}

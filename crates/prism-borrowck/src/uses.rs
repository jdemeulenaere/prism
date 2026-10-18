//! What each statement and terminator does to places, in evaluation order (D5 of
//! `borrowck-moves`): which places it uses, moves and initializes, and which locals' storage it
//! starts or ends. A use or a move by an operand is at the operand's span, any other use at its
//! statement's or terminator's. [`crate::move_paths::MoveData`] resolves these effects to move paths once, and
//! the analyses and the use check share them.

use prism_ast::Span;
use prism_mir::{
    Callee, Capture, LocalId, Operand, Place, Projection, Rvalue, Statement, StatementKind,
    Terminator, TerminatorKind,
};

/// How a use reads its place, which words the diagnostic of a use after move.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UseKind {
    /// Read, copied, moved, compared or called.
    Used,
    /// Borrowed, written or implied.
    Borrowed,
    /// Captured by a closure being built.
    Captured,
    /// Written through: one of its fields assigned, or its referent.
    Assigned,
}

/// One effect of a statement or terminator on a place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaceEffect {
    /// The place must hold a value, at the span of the source that uses it.
    Use(Place, UseKind, Span),
    /// The place's value moves out (after its use), at the span of the source that moves it.
    Move(Place, Span),
    /// The whole local is assigned a value.
    Init(LocalId),
    StorageLive(LocalId),
    StorageDead(LocalId),
}

/// The effects of `statement`, in evaluation order: an assignment's rvalue, then its place.
pub fn statement_effects(statement: &Statement) -> Vec<PlaceEffect> {
    let mut effects = Vec::new();
    let span = statement.span;
    match &statement.kind {
        StatementKind::Assign(place, rvalue) => {
            rvalue_effects(rvalue, span, &mut effects);
            destination_effects(place, span, &mut effects);
        }
        StatementKind::StorageLive(local) => effects.push(PlaceEffect::StorageLive(*local)),
        StatementKind::StorageDead(local) => effects.push(PlaceEffect::StorageDead(*local)),
        StatementKind::Read(place) => use_place(place, UseKind::Used, span, &mut effects),
        // A whole-value move of the source, then an initialization of each part (D16 of
        // `payload-patterns`).
        StatementKind::Destructure { source, parts, .. } => {
            let source = Place::local(*source);
            use_place(&source, UseKind::Used, span, &mut effects);
            effects.push(PlaceEffect::Move(source, span));
            for (_, target) in parts {
                effects.push(PlaceEffect::Init(*target));
            }
        }
        // Region markers read, write, move and drop nothing (D6 of `compose-transform`).
        StatementKind::GroupStart(_) | StatementKind::GroupEnd(_) => {}
    }
    effects
}

/// The effects of `terminator`, in evaluation order: a call's callee, its operands, then its
/// destination. A drop is not a use: drop elaboration decides whether it runs (D5).
pub fn terminator_effects(terminator: &Terminator) -> Vec<PlaceEffect> {
    let mut effects = Vec::new();
    let span = terminator.span;
    match &terminator.kind {
        TerminatorKind::SwitchInt { value, .. } => operand_effects(value, &mut effects),
        TerminatorKind::Call {
            callee,
            arguments,
            destination,
            ..
        } => {
            if let Callee::Value(place, callee_span) = callee {
                use_place(place, UseKind::Used, *callee_span, &mut effects);
            }
            for argument in arguments {
                operand_effects(argument, &mut effects);
            }
            destination_effects(destination, span, &mut effects);
        }
        TerminatorKind::Goto(_)
        | TerminatorKind::Drop { .. }
        | TerminatorKind::Return
        | TerminatorKind::Unreachable => {}
    }
    effects
}

/// A write to `place` at `span`: the initialization of a whole local, or a use of what holds
/// the written part (the struct of a field, the reference of a referent).
fn destination_effects(place: &Place, span: Span, effects: &mut Vec<PlaceEffect>) {
    match place.as_local() {
        Some(local) => effects.push(PlaceEffect::Init(local)),
        None => use_place(place, UseKind::Assigned, span, effects),
    }
}

/// A use of `place` at `span`, after the uses of the locals its projections index by.
fn use_place(place: &Place, kind: UseKind, span: Span, effects: &mut Vec<PlaceEffect>) {
    for projection in &place.projections {
        if let Projection::Index(index) = projection {
            effects.push(PlaceEffect::Use(Place::local(*index), UseKind::Used, span));
        }
    }
    effects.push(PlaceEffect::Use(place.clone(), kind, span));
}

/// The effects of `operand`, at its own span.
fn operand_effects(operand: &Operand, effects: &mut Vec<PlaceEffect>) {
    match operand {
        Operand::Copy(place, span) => use_place(place, UseKind::Used, *span, effects),
        Operand::Move(place, span) => {
            use_place(place, UseKind::Used, *span, effects);
            effects.push(PlaceEffect::Move(place.clone(), *span));
        }
        Operand::Constant(_) => {}
    }
}

/// The effects of `rvalue`, whose statement is at `span`.
fn rvalue_effects(rvalue: &Rvalue, span: Span, effects: &mut Vec<PlaceEffect>) {
    match rvalue {
        Rvalue::Use(operand)
        | Rvalue::Unary(_, operand)
        | Rvalue::WrapNullable(operand)
        | Rvalue::Subtype(operand) => operand_effects(operand, effects),
        Rvalue::Binary(_, left, right) => {
            operand_effects(left, effects);
            operand_effects(right, effects);
        }
        Rvalue::Borrow { place, .. } => use_place(place, UseKind::Borrowed, span, effects),
        Rvalue::Discriminant(place) => use_place(place, UseKind::Used, span, effects),
        Rvalue::Aggregate(_, operands) => {
            for operand in operands {
                operand_effects(operand, effects);
            }
        }
        Rvalue::Template(_) => {
            for operand in rvalue.template_operands() {
                operand_effects(operand, effects);
            }
        }
        // An owned capture is a use and, for a move, a move of its operand, at the capture's
        // span (D2 of `escaping-closures`); a borrowed one is a use where the closure is built.
        Rvalue::Closure { captures, .. } => {
            for capture in captures {
                match capture {
                    Capture::Borrowed(place) => use_place(place, UseKind::Captured, span, effects),
                    Capture::Owned(Operand::Copy(place, at)) => {
                        use_place(place, UseKind::Captured, *at, effects)
                    }
                    Capture::Owned(Operand::Move(place, at)) => {
                        use_place(place, UseKind::Captured, *at, effects);
                        effects.push(PlaceEffect::Move(place.clone(), *at));
                    }
                    Capture::Owned(Operand::Constant(_)) => {}
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::location::locations;
    use crate::move_paths::{Effect, MoveData};
    use crate::test_bodies::{body, lower};
    use crate::uses::UseKind;
    use prism_mir::{LocalId, Place};

    /// The uses of the path of the local named `name` (or of its fields) in function `f` of
    /// `source`, in visiting order, as `(place, kind)`.
    fn uses_of(source: &str, name: &str) -> Vec<(Place, UseKind)> {
        let file = lower(source);
        let body = body(&file, "f");
        let data = MoveData::gather(body, &file.types);
        let local = body
            .locals
            .iter()
            .find(|(_, l)| l.name.as_deref() == Some(name))
            .map(|(id, _)| id)
            .unwrap();
        let mut uses = Vec::new();
        for location in locations(body) {
            for effect in data.effects(location) {
                if let Effect::Use { path, kind, .. } = effect {
                    let place = &data.paths[*path].place;
                    if place.local == local {
                        uses.push((place.clone(), *kind));
                    }
                }
            }
        }
        uses
    }

    const DECLARATIONS: &str = "\
fun greet(s: &String): Int = 1
fun run(f: () -> Int): Int = f()
struct User { var name: String }
";

    fn single_use(body: &str) -> Vec<(Place, UseKind)> {
        uses_of(&format!("{}{}", DECLARATIONS, body), "s")
    }

    fn s() -> Place {
        Place::local(LocalId(1))
    }

    #[test]
    fn reads_borrows_and_captures() {
        assert_eq!(
            single_use("fun f(s: String): Bool = s == \"a\"\n"),
            vec![(s(), UseKind::Borrowed)]
        );
        assert_eq!(
            single_use("fun f(s: String): Int = greet(s)\n"),
            vec![(s(), UseKind::Borrowed)]
        );
        assert_eq!(
            single_use("fun f(s: String) { val r = &s }\n"),
            vec![(s(), UseKind::Borrowed)]
        );
        assert_eq!(
            single_use("fun f(s: String): String = s.clone()\n"),
            vec![(s(), UseKind::Borrowed)]
        );
        assert_eq!(
            single_use("fun f(s: String) { s }\n"),
            vec![(s(), UseKind::Used)]
        );
        assert_eq!(
            single_use("fun f(s: String): Int = run { greet(s) }\n"),
            vec![(s(), UseKind::Captured)]
        );
    }

    #[test]
    fn writes_through_a_field_or_a_reference() {
        let source = format!(
            "{}fun f(v: String) {{ var s = User(\"a\"); s.name = v }}\n",
            DECLARATIONS
        );
        let uses = uses_of(&source, "s");
        assert_eq!(uses.len(), 1, "{:?}", uses);
        assert_eq!(uses[0].1, UseKind::Assigned);
        let uses = uses_of("fun f(s: &mut Int) { *s = 2 }\n", "s");
        assert_eq!(uses, vec![(s(), UseKind::Assigned)]);
    }
}

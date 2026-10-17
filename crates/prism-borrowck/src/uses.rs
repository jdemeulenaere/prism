//! What each statement and terminator does to places, in evaluation order (D5 of
//! `borrowck-moves`): which places it uses, moves and initializes, and which locals' storage it
//! starts or ends. [`crate::move_paths::MoveData`] resolves these effects to move paths once, and
//! the analyses and the use check share them.

use prism_mir::{
    Callee, LocalId, Operand, Place, Projection, Rvalue, Statement, StatementKind, Terminator,
    TerminatorKind,
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
    /// The place must hold a value.
    Use(Place, UseKind),
    /// The place's value moves out (after its use).
    Move(Place),
    /// The whole local is assigned a value.
    Init(LocalId),
    StorageLive(LocalId),
    StorageDead(LocalId),
}

/// The effects of `statement`, in evaluation order: an assignment's rvalue, then its place.
pub fn statement_effects(statement: &Statement) -> Vec<PlaceEffect> {
    let mut effects = Vec::new();
    match &statement.kind {
        StatementKind::Assign(place, rvalue) => {
            rvalue_effects(rvalue, &mut effects);
            destination_effects(place, &mut effects);
        }
        StatementKind::StorageLive(local) => effects.push(PlaceEffect::StorageLive(*local)),
        StatementKind::StorageDead(local) => effects.push(PlaceEffect::StorageDead(*local)),
        StatementKind::Read(place) => use_place(place, UseKind::Used, &mut effects),
    }
    effects
}

/// The effects of `terminator`, in evaluation order: a call's callee, its operands, then its
/// destination. A drop is not a use: drop elaboration decides whether it runs (D5).
pub fn terminator_effects(terminator: &Terminator) -> Vec<PlaceEffect> {
    let mut effects = Vec::new();
    match &terminator.kind {
        TerminatorKind::SwitchInt { value, .. } => operand_effects(value, &mut effects),
        TerminatorKind::Call {
            callee,
            arguments,
            destination,
            ..
        } => {
            if let Callee::Value(place) = callee {
                use_place(place, UseKind::Used, &mut effects);
            }
            for argument in arguments {
                operand_effects(argument, &mut effects);
            }
            destination_effects(destination, &mut effects);
        }
        TerminatorKind::Goto(_)
        | TerminatorKind::Drop { .. }
        | TerminatorKind::Return
        | TerminatorKind::Unreachable => {}
    }
    effects
}

/// A write to `place`: the initialization of a whole local, or a use of what holds the written
/// part (the struct of a field, the reference of a referent).
fn destination_effects(place: &Place, effects: &mut Vec<PlaceEffect>) {
    match place.as_local() {
        Some(local) => effects.push(PlaceEffect::Init(local)),
        None => use_place(place, UseKind::Assigned, effects),
    }
}

/// A use of `place`, after the uses of the locals its projections index by.
fn use_place(place: &Place, kind: UseKind, effects: &mut Vec<PlaceEffect>) {
    for projection in &place.projections {
        if let Projection::Index(index) = projection {
            effects.push(PlaceEffect::Use(Place::local(*index), UseKind::Used));
        }
    }
    effects.push(PlaceEffect::Use(place.clone(), kind));
}

fn operand_effects(operand: &Operand, effects: &mut Vec<PlaceEffect>) {
    match operand {
        Operand::Copy(place) => use_place(place, UseKind::Used, effects),
        Operand::Move(place) => {
            use_place(place, UseKind::Used, effects);
            effects.push(PlaceEffect::Move(place.clone()));
        }
        Operand::Constant(_) => {}
    }
}

fn rvalue_effects(rvalue: &Rvalue, effects: &mut Vec<PlaceEffect>) {
    match rvalue {
        Rvalue::Use(operand)
        | Rvalue::Unary(_, operand)
        | Rvalue::WrapNullable(operand)
        | Rvalue::Subtype(operand) => operand_effects(operand, effects),
        Rvalue::Binary(_, left, right) => {
            operand_effects(left, effects);
            operand_effects(right, effects);
        }
        Rvalue::Borrow { place, .. } => use_place(place, UseKind::Borrowed, effects),
        Rvalue::Discriminant(place) => use_place(place, UseKind::Used, effects),
        Rvalue::Aggregate(_, operands) => {
            for operand in operands {
                operand_effects(operand, effects);
            }
        }
        Rvalue::Closure { captures, .. } => {
            for capture in captures {
                use_place(capture, UseKind::Captured, effects);
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

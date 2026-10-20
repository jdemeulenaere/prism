//! Move paths and the move, initialization and storage events of a body (D3 of
//! `borrowck-moves`).
//!
//! A move path is a place that can be initialized or moved on its own: a local followed by field
//! projections. Every local has a root path; child paths are created for the field places that a
//! move, an initialization or a use names. A nullable's payload is folded into the nullable (a
//! move of `(u as payload)` moves `u`), and a dereference or an index ends the path, since the
//! body owns nothing behind a reference and an index reads a `Copy` character. A `Move` operand of
//! a `Copy` place is a copy.

use crate::location::Location;
use crate::uses::{statement_effects, terminator_effects, PlaceEffect, UseKind};
use prism_ast::Span;
use prism_mir::{Body, LocalId, LocalKind, Place, Projection};
use prism_typed_tree::{define_id, Arena, ArenaId, TypeTable};
use std::collections::HashMap;

define_id!(
    /// A move path of a body.
    MovePathId
);
define_id!(
    /// A move out of a move path.
    MoveOutId
);
define_id!(
    /// An initialization of a move path.
    InitId
);
define_id!(
    /// A `StorageLive` of a local, after which it holds no value until assigned.
    StorageStartId
);

/// A place that can be initialized or moved on its own.
#[derive(Debug, Clone, PartialEq)]
pub struct MovePath {
    /// The local followed by field projections.
    pub place: Place,
    pub parent: Option<MovePathId>,
    pub children: Vec<MovePathId>,
}

/// A move of a path's value.
#[derive(Debug, Clone, PartialEq)]
pub struct MoveOut {
    pub path: MovePathId,
    pub location: Location,
    pub span: Span,
}

/// How a path is initialized.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitKind {
    /// An argument or a capture, which holds a value when the body starts.
    Entry,
    /// An assignment of a whole local.
    Assign,
    /// The destination of a call, written when the call returns.
    CallDestination,
}

/// An initialization of a path.
#[derive(Debug, Clone, PartialEq)]
pub struct Init {
    pub path: MovePathId,
    /// The location of the assignment or call; `bb0[0]` for an entry initialization.
    pub location: Location,
    pub kind: InitKind,
    pub span: Span,
}

/// The start of a local's storage.
#[derive(Debug, Clone, PartialEq)]
pub struct StorageStart {
    pub local: LocalId,
    pub location: Location,
    pub span: Span,
}

/// An effect of a location on move paths, resolved from [`PlaceEffect`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Effect {
    /// The path must hold a value.
    Use {
        path: MovePathId,
        kind: UseKind,
        span: Span,
    },
    Move(MoveOutId),
    Init(InitId),
    StorageLive(StorageStartId),
    StorageDead(LocalId),
}

/// An event after which a path holds no value: a move, or a storage start.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Event {
    Move(MoveOutId),
    StorageStart(StorageStartId),
}

/// The move paths of one body, with every move, initialization and storage start, and the
/// effects of each location in evaluation order.
pub struct MoveData {
    pub paths: Arena<MovePathId, MovePath>,
    pub moves: Arena<MoveOutId, MoveOut>,
    pub inits: Arena<InitId, Init>,
    pub storage_starts: Arena<StorageStartId, StorageStart>,
    /// The root path of each local.
    roots: Vec<MovePathId>,
    /// The child path of each path for each field index.
    fields: HashMap<(MovePathId, usize), MovePathId>,
    /// The effects of every location, block by block, each block's statements and then its
    /// terminator, in one array: one allocation for the body rather than one per location.
    effects: Vec<Effect>,
    /// Where the effects of each location start in `effects`, with the end of the last.
    effect_starts: Vec<u32>,
    /// The position in `effect_starts` of each block's first location.
    block_starts: Vec<u32>,
    /// The events of each path: its moves, and the storage starts of a root path's local.
    path_events: Vec<Vec<Event>>,
}

impl MoveData {
    /// Gathers the move data of `body`, whose types are in `types`.
    ///
    /// Panics on a move through a dereference, which the type checker rejects and the MIR
    /// validator checks (D3 and D11 of `borrowck-moves`).
    pub fn gather(body: &Body, types: &TypeTable) -> Self {
        let mut data = MoveData {
            paths: Arena::new(),
            moves: Arena::new(),
            inits: Arena::new(),
            storage_starts: Arena::new(),
            roots: Vec::new(),
            fields: HashMap::new(),
            effects: Vec::new(),
            effect_starts: Vec::new(),
            block_starts: Vec::new(),
            path_events: Vec::new(),
        };
        for local in body.locals.ids() {
            let path = data.new_path(Place::local(local), None);
            data.roots.push(path);
        }
        for (local, declaration) in body.locals.iter() {
            if matches!(declaration.kind, LocalKind::Argument | LocalKind::Capture) {
                data.inits.alloc(Init {
                    path: data.roots[local.index()],
                    location: Location::start(prism_mir::BlockId(0)),
                    kind: InitKind::Entry,
                    span: declaration.span,
                });
            }
        }
        for (block, block_data) in body.blocks.iter() {
            data.block_starts.push(data.effect_starts.len() as u32);
            let count = block_data.statements.len();
            for index in 0..=count {
                let location = Location {
                    block,
                    statement: index,
                };
                let (raw, span, is_call) = match block_data.statements.get(index) {
                    Some(statement) => (statement_effects(statement), statement.span, false),
                    None => {
                        let terminator = &block_data.terminator;
                        let is_call =
                            matches!(terminator.kind, prism_mir::TerminatorKind::Call { .. });
                        (terminator_effects(terminator), terminator.span, is_call)
                    }
                };
                // Uses and moves carry their own spans; `span` is the location's.
                data.effect_starts.push(data.effects.len() as u32);
                for effect in raw {
                    if let Some(effect) = data.resolve(body, types, effect, location, span, is_call)
                    {
                        data.effects.push(effect);
                    }
                }
            }
        }
        data.effect_starts.push(data.effects.len() as u32);
        data.path_events = vec![Vec::new(); data.paths.len()];
        for (id, out) in data.moves.iter() {
            data.path_events[out.path.index()].push(Event::Move(id));
        }
        for (id, start) in data.storage_starts.iter() {
            let root = data.roots[start.local.index()];
            data.path_events[root.index()].push(Event::StorageStart(id));
        }
        data
    }

    fn new_path(&mut self, place: Place, parent: Option<MovePathId>) -> MovePathId {
        let id = self.paths.alloc(MovePath {
            place,
            parent,
            children: Vec::new(),
        });
        if let Some(parent) = parent {
            self.paths[parent].children.push(id);
        }
        id
    }

    /// The path of `place`, creating its field paths: the local's root, then a child per field
    /// projection, stopping at a payload (folded into its nullable or enum), a dereference or an
    /// index.
    /// The second value is whether the place is behind a dereference.
    fn path_of(&mut self, place: &Place) -> (MovePathId, bool) {
        let mut path = self.roots[place.local.index()];
        for (index, projection) in place.projections.iter().enumerate() {
            match projection {
                Projection::Field(field) => {
                    path = match self.fields.get(&(path, *field)) {
                        Some(child) => *child,
                        None => {
                            let child_place = Place {
                                local: place.local,
                                projections: place.projections[..=index].to_vec(),
                            };
                            let child = self.new_path(child_place, Some(path));
                            self.fields.insert((path, *field), child);
                            child
                        }
                    }
                }
                // Nothing moves out of a payload except by a destructuring of its whole value
                // (D16 of `payload-patterns`), so a variant's payload folds into its enum.
                Projection::Payload | Projection::VariantField { .. } | Projection::Index(_) => {
                    return (path, false)
                }
                // A weak reference viewed as its state object is never moved out of, like the
                // referent of a shared reference (D10 of `owned-state`).
                Projection::Deref | Projection::View => return (path, true),
            }
        }
        (path, false)
    }

    fn resolve(
        &mut self,
        body: &Body,
        types: &TypeTable,
        effect: PlaceEffect,
        location: Location,
        span: Span,
        is_call: bool,
    ) -> Option<Effect> {
        Some(match effect {
            PlaceEffect::Use(place, kind, span) => Effect::Use {
                path: self.path_of(&place).0,
                kind,
                span,
            },
            PlaceEffect::Move(place, span) => {
                if types.is_copy(&place.ty(body, types)) {
                    return None;
                }
                let (path, behind_reference) = self.path_of(&place);
                if behind_reference {
                    panic!(
                        "internal error: a move out of a dereference at {:?}: {:?}",
                        location, place
                    );
                }
                Effect::Move(self.moves.alloc(MoveOut {
                    path,
                    location,
                    span,
                }))
            }
            PlaceEffect::Init(local) => Effect::Init(self.inits.alloc(Init {
                path: self.roots[local.index()],
                location,
                kind: if is_call {
                    InitKind::CallDestination
                } else {
                    InitKind::Assign
                },
                span,
            })),
            PlaceEffect::StorageLive(local) => {
                Effect::StorageLive(self.storage_starts.alloc(StorageStart {
                    local,
                    location,
                    span,
                }))
            }
            PlaceEffect::StorageDead(local) => Effect::StorageDead(local),
        })
    }

    /// The root path of `local`.
    pub fn root(&self, local: LocalId) -> MovePathId {
        self.roots[local.index()]
    }

    /// The effects of `location`, in evaluation order.
    pub fn effects(&self, location: Location) -> &[Effect] {
        let index = self.block_starts[location.block.index()] as usize + location.statement;
        &self.effects[self.effect_starts[index] as usize..self.effect_starts[index + 1] as usize]
    }

    /// The initializations at the body's entry: its arguments and captures.
    pub fn entry_inits(&self) -> impl Iterator<Item = &Init> {
        self.inits
            .iter()
            .map(|(_, init)| init)
            .filter(|init| init.kind == InitKind::Entry)
    }

    /// Whether a path lies below `path`.
    pub fn has_children(&self, path: MovePathId) -> bool {
        !self.paths[path].children.is_empty()
    }

    /// `path` and every path below it, `path` first.
    pub fn descendants(&self, path: MovePathId) -> Vec<MovePathId> {
        let mut out = vec![path];
        let mut index = 0;
        while index < out.len() {
            out.extend(self.paths[out[index]].children.iter().copied());
            index += 1;
        }
        out
    }

    /// The paths above `path`, nearest first.
    pub fn ancestors(&self, path: MovePathId) -> Vec<MovePathId> {
        let mut out = Vec::new();
        let mut current = self.paths[path].parent;
        while let Some(parent) = current {
            out.push(parent);
            current = self.paths[parent].parent;
        }
        out
    }

    /// The events of `path` itself.
    pub fn events_of(&self, path: MovePathId) -> &[Event] {
        &self.path_events[path.index()]
    }

    /// The number of events, the domain of the analyses over events.
    pub fn event_count(&self) -> usize {
        self.moves.len() + self.storage_starts.len()
    }

    /// The bit of `event` in a set of events: moves first, then storage starts.
    pub fn event_index(&self, event: Event) -> usize {
        match event {
            Event::Move(id) => id.index(),
            Event::StorageStart(id) => self.moves.len() + id.index(),
        }
    }

    /// The event of bit `index`.
    pub fn event(&self, index: usize) -> Event {
        if index < self.moves.len() {
            Event::Move(MoveOutId::from_index(index))
        } else {
            Event::StorageStart(StorageStartId::from_index(index - self.moves.len()))
        }
    }

    /// The path an event leaves without a value.
    pub fn event_path(&self, event: Event) -> MovePathId {
        match event {
            Event::Move(id) => self.moves[id].path,
            Event::StorageStart(id) => self.root(self.storage_starts[id].local),
        }
    }

    /// The source location of an event.
    pub fn event_span(&self, event: Event) -> Span {
        match event {
            Event::Move(id) => self.moves[id].span,
            Event::StorageStart(id) => self.storage_starts[id].span,
        }
    }

    /// The location of an event.
    pub fn event_location(&self, event: Event) -> Location {
        match event {
            Event::Move(id) => self.moves[id].location,
            Event::StorageStart(id) => self.storage_starts[id].location,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_bodies::{body, lower};

    /// The move data of function `name` of `source`.
    fn gather(source: &str, name: &str) -> (Body, MoveData) {
        let file = lower(source);
        let body = body(&file, name).clone();
        let data = MoveData::gather(&body, &file.types);
        (body, data)
    }

    fn moved_places(data: &MoveData) -> Vec<Place> {
        data.moves
            .iter()
            .map(|(_, out)| data.paths[out.path].place.clone())
            .collect()
    }

    #[test]
    fn a_move_into_a_call() {
        let (_, data) = gather(
            "fun take(s: String) {}\nfun f(s: String) { take(s) }\n",
            "f",
        );
        // `s` moves into the argument temporary, which moves into the call.
        let moved = moved_places(&data);
        assert_eq!(moved[0], Place::local(LocalId(1)));
        assert_eq!(moved.len(), 2);
        assert_eq!(data.entry_inits().count(), 1);
    }

    #[test]
    fn a_payload_move_moves_the_nullable() {
        let source = "\
struct User { val name: String }
struct Account { val owner: User }
consuming fun User.toAccount(): Account = Account(this)
fun f(u: User?) { val a = u?.toAccount() }
";
        let (_, data) = gather(source, "f");
        assert!(
            moved_places(&data).contains(&Place::local(LocalId(1))),
            "{:?}",
            moved_places(&data)
        );
        assert!(data
            .paths
            .iter()
            .all(|(_, path)| !path.place.projections.contains(&Projection::Payload)));
    }

    #[test]
    fn copy_values_do_not_move() {
        let source = "\
struct Point : Copy { val x: Int }
struct Pair { val a: Point, val b: Point }
fun f(p: Point): Pair = Pair(p, p)
";
        let (_, data) = gather(source, "f");
        assert!(
            !moved_places(&data).contains(&Place::local(LocalId(1))),
            "{:?}",
            moved_places(&data)
        );
    }

    #[test]
    fn a_binding_initializes_its_local_and_moves_its_value() {
        let (body, data) = gather("fun f(a: String) { val b = a }\n", "f");
        let b = body
            .locals
            .iter()
            .find(|(_, l)| l.name.as_deref() == Some("b"))
            .map(|(id, _)| id)
            .unwrap();
        assert!(data
            .inits
            .iter()
            .any(|(_, init)| init.path == data.root(b) && init.kind == InitKind::Assign));
        assert_eq!(moved_places(&data), vec![Place::local(LocalId(1))]);
        assert_eq!(data.event_count(), 2, "a move and the storage start of b");
    }

    #[test]
    fn field_places_get_child_paths() {
        let source = "\
struct User { val id: Int, var name: String }
fun f(u: User): Int { val n = &u.name; return u.id }
";
        let (_, data) = gather(source, "f");
        let root = data.root(LocalId(1));
        let children: Vec<_> = data.paths[root]
            .children
            .iter()
            .map(|c| data.paths[*c].place.clone())
            .collect();
        assert_eq!(
            children,
            vec![
                Place::local(LocalId(1)).project(Projection::Field(1)),
                Place::local(LocalId(1)).project(Projection::Field(0)),
            ]
        );
        assert_eq!(data.descendants(root).len(), 3);
        assert_eq!(data.ancestors(data.paths[root].children[0]), vec![root]);
    }
}

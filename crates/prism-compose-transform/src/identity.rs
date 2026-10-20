//! The identities of borrowed arguments (D2, D5 and D6 of `strong-skipping`).
//!
//! A borrowed argument of a composable call is identified by its address, the reference
//! itself, and a version that says whether the data at that address may have changed. The
//! version comes from where the reference was derived, which this module reads syntactically
//! from the caller's MIR:
//!
//! - a state's contents, the result of `state read` and any field, index or `Heap`
//!   dereference below it: the state's version, read by a `state version` call on the same
//!   state object right before the read;
//! - the value a `remember` lends, a top-level `val` or `null`, and parts of them: permanent
//!   (`1`);
//! - a borrowed parameter of the body, or a capture local standing for a reference or for a
//!   place reached through one, and parts of their referents: the version the body was given
//!   (its `$version` argument or hidden capture);
//! - anything else, a local, a temporary, a call's result: none (`0`).
//!
//! Each reference local whose value reaches a borrowed argument of a composable call or a
//! hidden capture of a composable closure gets a version local, assigned beside every
//! assignment of the reference, so the two agree on every path. Finding those locals is one
//! scan of the body's assignments and a backward worklist that visits each local once: linear
//! in the body.

use crate::stability::has_version_word;
use prism_mir::{
    Body, BodyId, Callee, Capture, CaptureMode, Constant, LocalId, LocalKind, MirFile, Operand,
    Place, Projection, Rvalue, StatementKind, TerminatorKind,
};
use prism_typed_tree::{ArenaId, Type, TypeTable};
use std::collections::{HashMap, HashSet};

/// The version of the identity a reference has, by where it was derived from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VersionSource {
    /// No identity: the referent lives in a frame, or comes from code the transform does not
    /// see.
    None,
    /// A remembered value, a top-level value or `null`: its storage never changes while a
    /// kept identity can name it.
    Permanent,
    /// The version of the reference held by this local: an argument, a capture, or a local
    /// with a version local.
    Reference(LocalId),
    /// The version of a borrow of this capture local, which stands for a place reached
    /// through a reference of the closure's author: its hidden capture.
    CapturedPlace(LocalId),
}

/// How a composable closure's hidden version capture describes its capture (D6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HiddenKind {
    /// The capture local holds a reference: the hidden capture is that reference's version.
    Reference,
    /// The capture local stands for a place of the author reached through a reference: the
    /// hidden capture is the version of a borrow of that place.
    Place,
}

/// One hidden version capture of a composable closure: the index of the capture it versions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HiddenCapture {
    pub capture: usize,
    pub kind: HiddenKind,
}

/// The hidden version captures of each composable closure of a file, settled before any body
/// is composed, since a closure's author and the closure itself both need them (D6).
#[derive(Debug, Default)]
pub(crate) struct HiddenCaptures {
    bodies: HashMap<BodyId, Vec<HiddenCapture>>,
}

impl HiddenCaptures {
    /// The hidden captures of every composable closure of `file` whose author is composable:
    /// one per capture that holds a reference, read-only, and one per borrowed capture that
    /// stands for a place reached through a reference, or for a part of such a capture of the
    /// author. One scan of the file's closure values, and constant work per capture.
    pub(crate) fn of(file: &MirFile) -> Self {
        let mut constructions: HashMap<BodyId, (BodyId, Vec<Capture>)> = HashMap::new();
        for (parent, body) in file.bodies.iter() {
            for (_, block) in body.blocks.iter() {
                for statement in &block.statements {
                    if let StatementKind::Assign(_, rvalue) = &statement.kind {
                        if let Rvalue::Closure {
                            body: closure,
                            captures,
                        } = rvalue.as_ref()
                        {
                            constructions.insert(*closure, (parent, captures.clone()));
                        }
                    }
                }
            }
        }
        let mut hidden = HiddenCaptures::default();
        let mut ids: Vec<BodyId> = constructions.keys().copied().collect();
        ids.sort_by_key(|id| id.index());
        for id in ids {
            hidden.settle(file, id, &constructions);
        }
        hidden
    }

    /// Settles the hidden captures of `id` and, first, of its author when that is a closure.
    fn settle(
        &mut self,
        file: &MirFile,
        id: BodyId,
        constructions: &HashMap<BodyId, (BodyId, Vec<Capture>)>,
    ) {
        if self.bodies.contains_key(&id) {
            return;
        }
        let body = &file.bodies[id];
        let Some((parent, captures)) = constructions.get(&id) else {
            self.bodies.insert(id, Vec::new());
            return;
        };
        let parent_body = &file.bodies[*parent];
        if !body.is_compose || !parent_body.is_compose {
            self.bodies.insert(id, Vec::new());
            return;
        }
        if constructions.contains_key(parent) {
            self.settle(file, *parent, constructions);
        }
        let assigned = assigned_locals(body);
        let mut list = Vec::new();
        for (index, capture) in captures.iter().enumerate() {
            let local = LocalId((body.argument_count + 1 + index) as u32);
            let Some(declaration) = body.locals.get(local) else {
                continue;
            };
            if assigned.contains(&local) {
                continue;
            }
            let mode = body.capture_modes.get(index).copied();
            let kind = match (capture, mode) {
                (_, Some(CaptureMode::Weak) | Some(CaptureMode::Mutable)) => None,
                (Capture::Weak(_), _) => None,
                _ if is_reference(&declaration.ty) => Some(HiddenKind::Reference),
                (Capture::Borrowed(place), _) => {
                    match place_root(parent_body, &file.types, place) {
                        PlaceRoot::Referent => Some(HiddenKind::Place),
                        PlaceRoot::Own
                            if self.is_hidden_place(*parent, parent_body, place.local) =>
                        {
                            Some(HiddenKind::Place)
                        }
                        _ => None,
                    }
                }
                _ => None,
            };
            if let Some(kind) = kind {
                list.push(HiddenCapture {
                    capture: index,
                    kind,
                });
            }
        }
        self.bodies.insert(id, list);
    }

    /// Whether `local` of the closure `id` is a capture with a hidden version of a place.
    fn is_hidden_place(&self, id: BodyId, body: &Body, local: LocalId) -> bool {
        let Some(index) = capture_index(body, local) else {
            return false;
        };
        self.of_body(id)
            .iter()
            .any(|hidden| hidden.capture == index && hidden.kind == HiddenKind::Place)
    }

    /// The hidden captures of the closure `id`, in capture order.
    /// Whether the closure `id` owns its captures (it escapes or outlives its author), so its
    /// hidden captures hold copies of the versions; a borrowing closure borrows the locals that
    /// hold them, which its author assigns beside the references they describe, so the two
    /// always agree and the environment stays in the creating frame.
    pub(crate) fn owns_captures(file: &MirFile, id: BodyId) -> bool {
        let body = &file.bodies[id];
        body.ownership_reason.is_some() || body.capture_modes.contains(&CaptureMode::Owned)
    }

    pub(crate) fn of_body(&self, id: BodyId) -> &[HiddenCapture] {
        self.bodies.get(&id).map(Vec::as_slice).unwrap_or_default()
    }
}

/// The capture index of `local` in `body`, when it is a capture local.
pub(crate) fn capture_index(body: &Body, local: LocalId) -> Option<usize> {
    let first = body.argument_count + 1;
    let index = local.index();
    (index >= first && index < first + body.capture_count).then(|| index - first)
}

/// Whether `ty` is a reference or a nullable of one: a value whose identity is an address.
pub(crate) fn is_reference(ty: &Type) -> bool {
    match ty {
        Type::Reference { .. } => true,
        Type::Nullable(inner) => matches!(**inner, Type::Reference { .. }),
        _ => false,
    }
}

/// The locals a body assigns as a whole: a capture or an argument assigned is no longer the
/// value its version describes.
fn assigned_locals(body: &Body) -> HashSet<LocalId> {
    let mut assigned = HashSet::new();
    for (_, block) in body.blocks.iter() {
        for statement in &block.statements {
            if let StatementKind::Assign(place, _) = &statement.kind {
                if let Some(local) = place.as_local() {
                    assigned.insert(local);
                }
            }
        }
    }
    assigned
}

/// Where a place's storage is.
enum PlaceRoot {
    /// In the referent of the reference its local holds: the place starts with a dereference
    /// of the local, and every later dereference is of a box.
    Referent,
    /// In its local's own storage, or in boxes the local owns.
    Own,
    /// Behind a reference held inside a value: nothing is known of it.
    Unknown,
}

fn place_root(body: &Body, types: &TypeTable, place: &Place) -> PlaceRoot {
    let Some(mut ty) = body.locals.get(place.local).map(|local| local.ty.clone()) else {
        return PlaceRoot::Unknown;
    };
    let mut root = PlaceRoot::Own;
    for (index, projection) in place.projections.iter().enumerate() {
        if *projection == Projection::Deref && matches!(ty, Type::Reference { .. }) {
            if index > 0 {
                return PlaceRoot::Unknown;
            }
            root = PlaceRoot::Referent;
        }
        if *projection == Projection::View {
            return PlaceRoot::Unknown;
        }
        match prism_mir::projection_type(&ty, *projection, types) {
            Some(next) => ty = next,
            None => return PlaceRoot::Unknown,
        }
    }
    root
}

/// The version source of a borrow of `place` in `body`.
pub(crate) fn borrow_source(
    body: &Body,
    types: &TypeTable,
    place: &Place,
    hidden_place: impl Fn(LocalId) -> bool,
) -> VersionSource {
    match place_root(body, types, place) {
        PlaceRoot::Referent => VersionSource::Reference(place.local),
        PlaceRoot::Own if hidden_place(place.local) => VersionSource::CapturedPlace(place.local),
        _ => VersionSource::None,
    }
}

/// The version source of a reference read by `operand`.
pub(crate) fn operand_source(operand: &Operand) -> VersionSource {
    match operand {
        Operand::Constant(Constant::TopLevelValue(_) | Constant::Null(_)) => {
            VersionSource::Permanent
        }
        Operand::Constant(_) => VersionSource::None,
        Operand::Copy(place, _) | Operand::Move(place, _) => match place.as_local() {
            Some(local) => VersionSource::Reference(local),
            None => VersionSource::None,
        },
    }
}

/// The version source of a reference assigned `rvalue`.
pub(crate) fn rvalue_source(
    body: &Body,
    types: &TypeTable,
    rvalue: &Rvalue,
    hidden_place: impl Fn(LocalId) -> bool,
) -> VersionSource {
    match rvalue {
        Rvalue::Use(operand) | Rvalue::WrapNullable(operand) | Rvalue::Subtype(operand) => {
            operand_source(operand)
        }
        Rvalue::Borrow {
            mutable: false,
            place,
            ..
        } => borrow_source(body, types, place, hidden_place),
        _ => VersionSource::None,
    }
}

/// A version that stays the same for a whole run of a body: a constant, or a version the body
/// was given (an argument or a hidden capture).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Fixed {
    Constant(i64),
    /// The local holding it: a `$version` argument or a hidden capture.
    Given(LocalId),
}

/// How a reference local of a composable body is assigned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Assigned {
    Source(VersionSource),
    /// The result of `state read`, whose version is read beside it.
    StateRead,
}

/// What the transform knows of the versions of a composable body's reference locals.
#[derive(Debug, Default)]
pub(crate) struct Analysis {
    /// The locals whose version is the same however they are reached: every assignment gives
    /// one fixed version (D5 of `strong-skipping`). Their version needs no local.
    pub fixed: HashMap<LocalId, Fixed>,
    /// The locals that need a version local, assigned beside each of their assignments.
    pub needed: HashSet<LocalId>,
}

/// Finds the versions of the reference locals of `body` whose value reaches a borrowed
/// argument of a composable call or a hidden capture of a composable closure, through copies,
/// reborrows and projections (D5 of `strong-skipping`): a fixed version when every assignment
/// gives the same constant or given version, else a version local. `given` names the version
/// a body was given for an argument or a capture local, `None` when it has none. One scan, then
/// a worklist visiting each local once.
pub(crate) fn analyze(
    file: &MirFile,
    body: &Body,
    hidden: &HiddenCaptures,
    own_hidden: &[HiddenCapture],
    given: impl Fn(LocalId) -> Option<Fixed>,
) -> Analysis {
    let types = &file.types;
    let is_hidden_place = |local: LocalId| {
        capture_index(body, local).is_some_and(|index| {
            own_hidden
                .iter()
                .any(|hidden| hidden.capture == index && hidden.kind == HiddenKind::Place)
        })
    };
    let mut assignments: HashMap<LocalId, Vec<Assigned>> = HashMap::new();
    let mut roots: Vec<LocalId> = Vec::new();
    let mut poisoned: HashSet<LocalId> = HashSet::new();
    let is_reference_local =
        |local: LocalId| body.locals.get(local).is_some_and(|l| is_reference(&l.ty));
    for (_, block) in body.blocks.iter() {
        for statement in &block.statements {
            match &statement.kind {
                StatementKind::Assign(place, rvalue) => {
                    match rvalue.as_ref() {
                        // A reference borrowed mutably as a whole may be re-pointed through the
                        // borrow.
                        Rvalue::Borrow {
                            mutable: true,
                            place: borrowed,
                            ..
                        } if borrowed.projections.is_empty()
                            && is_reference_local(borrowed.local) =>
                        {
                            poisoned.insert(borrowed.local);
                        }
                        Rvalue::Closure {
                            body: closure,
                            captures,
                        } => {
                            let closure_body = &file.bodies[*closure];
                            for (index, capture) in captures.iter().enumerate() {
                                if let Capture::Borrowed(borrowed) = capture {
                                    if closure_body.capture_modes.get(index)
                                        == Some(&CaptureMode::Mutable)
                                        && borrowed.projections.is_empty()
                                        && is_reference_local(borrowed.local)
                                    {
                                        poisoned.insert(borrowed.local);
                                    }
                                }
                            }
                            for hidden_capture in hidden.of_body(*closure) {
                                let source = hidden_source(
                                    body,
                                    types,
                                    &captures[hidden_capture.capture],
                                    hidden_capture.kind,
                                    is_hidden_place,
                                );
                                if let VersionSource::Reference(local) = source {
                                    roots.push(local);
                                }
                            }
                        }
                        _ => {}
                    }
                    if let Some(local) = place.as_local().filter(|local| is_reference_local(*local))
                    {
                        assignments.entry(local).or_default().push(Assigned::Source(
                            rvalue_source(body, types, rvalue, is_hidden_place),
                        ));
                    }
                }
                StatementKind::Destructure { parts, .. } => {
                    for (_, target) in parts {
                        if is_reference_local(*target) {
                            assignments
                                .entry(*target)
                                .or_default()
                                .push(Assigned::Source(VersionSource::None));
                        }
                    }
                }
                _ => {}
            }
        }
        if let TerminatorKind::Call {
            arguments,
            parameters,
            is_compose,
            callee,
            destination,
            ..
        } = &block.terminator.kind
        {
            if let Some(local) = destination
                .as_local()
                .filter(|local| is_reference_local(*local))
            {
                let assigned = match callee {
                    Callee::State(prism_mir::StateOperation::Read) => Assigned::StateRead,
                    Callee::Builtin(prism_typed_tree::BuiltinFunction::Remember) => {
                        Assigned::Source(VersionSource::Permanent)
                    }
                    _ => Assigned::Source(VersionSource::None),
                };
                assignments.entry(local).or_default().push(assigned);
            }
            if *is_compose && !matches!(callee, Callee::Builtin(_)) {
                for (argument, parameter) in arguments.iter().zip(parameters) {
                    if !has_version_word(parameter) {
                        continue;
                    }
                    if let VersionSource::Reference(local) = operand_source(argument) {
                        roots.push(local);
                    }
                }
            }
        }
    }
    let mut resolver = Resolver {
        body,
        assignments: &assignments,
        poisoned: &poisoned,
        given: &given,
        resolved: HashMap::new(),
    };
    let mut analysis = Analysis::default();
    let mut visited = HashSet::new();
    while let Some(local) = roots.pop() {
        if !visited.insert(local) {
            continue;
        }
        match resolver.resolve(local) {
            Some(fixed) => {
                analysis.fixed.insert(local, fixed);
            }
            None => {
                analysis.needed.insert(local);
                for assigned in assignments.get(&local).into_iter().flatten() {
                    if let Assigned::Source(VersionSource::Reference(from)) = assigned {
                        roots.push(*from);
                    }
                }
            }
        }
    }
    // Every local a needed one is derived from has either a fixed version or a local.
    for (local, fixed) in resolver.resolved {
        if let Some(fixed) = fixed {
            analysis.fixed.entry(local).or_insert(fixed);
        }
    }
    analysis
}

/// The version source of a hidden capture's value where its closure is built.
pub(crate) fn hidden_source(
    body: &Body,
    types: &TypeTable,
    capture: &Capture,
    kind: HiddenKind,
    hidden_place: impl Fn(LocalId) -> bool,
) -> VersionSource {
    match (capture, kind) {
        (Capture::Borrowed(place), HiddenKind::Reference)
        | (
            Capture::Owned(Operand::Copy(place, _) | Operand::Move(place, _)),
            HiddenKind::Reference,
        ) => match place.as_local() {
            Some(local) => VersionSource::Reference(local),
            None => VersionSource::None,
        },
        (Capture::Borrowed(place), HiddenKind::Place) => {
            borrow_source(body, types, place, hidden_place)
        }
        _ => VersionSource::None,
    }
}

/// Resolves the fixed versions of reference locals, memoized; a cycle of assignments is not
/// fixed.
struct Resolver<'a, G: Fn(LocalId) -> Option<Fixed>> {
    body: &'a Body,
    assignments: &'a HashMap<LocalId, Vec<Assigned>>,
    poisoned: &'a HashSet<LocalId>,
    given: &'a G,
    /// `None` while a local is being resolved, and for a local that is not fixed.
    resolved: HashMap<LocalId, Option<Fixed>>,
}

impl<G: Fn(LocalId) -> Option<Fixed>> Resolver<'_, G> {
    /// The fixed version of `local`, or `None` when it needs a version local.
    fn resolve(&mut self, local: LocalId) -> Option<Fixed> {
        if let Some(resolved) = self.resolved.get(&local) {
            return *resolved;
        }
        if self.poisoned.contains(&local) {
            self.resolved.insert(local, Some(Fixed::Constant(0)));
            return Some(Fixed::Constant(0));
        }
        let kind = self.body.locals.get(local).map(|l| l.kind);
        if matches!(kind, Some(LocalKind::Argument | LocalKind::Capture) | None) {
            let fixed = (self.given)(local).unwrap_or(Fixed::Constant(0));
            self.resolved.insert(local, Some(fixed));
            return Some(fixed);
        }
        self.resolved.insert(local, None);
        let mut result: Option<Fixed> = None;
        let mut fixed = true;
        for assigned in self.assignments.get(&local).into_iter().flatten() {
            let version = match assigned {
                Assigned::StateRead => None,
                Assigned::Source(VersionSource::None) => Some(Fixed::Constant(0)),
                Assigned::Source(VersionSource::Permanent) => Some(Fixed::Constant(1)),
                Assigned::Source(VersionSource::CapturedPlace(captured)) => {
                    Some((self.given)(*captured).unwrap_or(Fixed::Constant(0)))
                }
                Assigned::Source(VersionSource::Reference(from)) => self.resolve(*from),
            };
            match (version, result) {
                (None, _) => fixed = false,
                (Some(version), None) => result = Some(version),
                (Some(version), Some(previous)) if version == previous => {}
                (Some(_), Some(_)) => fixed = false,
            }
        }
        // A local never assigned in the body is never read with a value.
        let resolved = if fixed {
            Some(result.unwrap_or(Fixed::Constant(0)))
        } else {
            None
        };
        self.resolved.insert(local, resolved);
        resolved
    }
}

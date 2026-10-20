//! `prism-compose-transform`: the compose transform (`compose-transform`), the compiler half of
//! the first-class compose and hot reload invariant.
//!
//! [`compose_file`] runs once per file on its elaborated MIR, after borrow checking and before
//! generic records, instances and code generation, so a generic composable is composed once and
//! its instances substitute the composed MIR (D2). It turns [`prism_mir::MirPhase::Elaborated`]
//! MIR into [`prism_mir::MirPhase::Composed`] MIR:
//!
//! - **The calling convention** (D3, D4). Every composable body takes, after its declared
//!   parameters, the composer (the identifier of the pass in progress) and its change words:
//!   two bits per parameter, 31 parameters per word, and a force bit. Every composable call
//!   passes the caller's composer and change words saying what the caller knows of each
//!   argument: static, the same, different, or uncertain ([`rewrite`]).
//! - **Groups and keys** (D5). Each composable body runs in one group keyed by its declaration
//!   path, or for a composable closure by its parent region and the step that reaches it; each
//!   region lowering marked runs in a replaceable group keyed by its construct and ordinal.
//!   Keys are BLAKE3 hashes chained from declaration paths, never from positions ([`keys`]).
//! - **Skipping and restarting** (D7 to D11). A composable returning `Unit` whose parameters
//!   can all be kept ([`stability`]) keeps its arguments in its restart group's slots, compares
//!   them, skips its body when nothing changed and its scope is valid, and ends its group with
//!   its restart function, a static closure reading the kept slots ([`restart`]).
//! - **Borrowed identities** (`strong-skipping`). A borrowed argument is kept as its address
//!   and a version that the caller derives syntactically from where its reference comes from:
//!   a state's version, permanent, the caller's own, or none, passed as one `Int` per borrowed
//!   parameter after the change words; a composable closure owns the versions of the
//!   references it borrows from as hidden captures; a call whose borrow has none runs in a
//!   replaceable group ([`identity`]).
//! - **The compose runtime module** (D12 to D15). `remember` and `key` become composer
//!   operations, and `currentComposer` a copy of the composer (D3 of `compose-library-in-prism`).
//!   Everything else of the compose library is ordinary Prism, called like any function.
//! - **Explicit groups** (D5 of `compose-library-in-prism`). A composable annotated
//!   `@ExplicitGroupsComposable` gets the calling convention but no group of its own.
//!
//! Only composable bodies are rewritten; each is walked once, with constant work per statement,
//! argument and group (D24).

mod identity;
pub mod keys;
mod restart;
mod rewrite;
pub mod stability;

use prism_mir::{Body, BodyId, BodyOwner, MirFile, MirPhase};
use prism_typed_tree::ArenaId;

/// The composed MIR of `file`, elaborated MIR without errors.
pub fn compose_file(file: &MirFile) -> MirFile {
    let keys = BodyKeys::of(file);
    // Each composable closure's hidden version captures, which its author's composition and
    // its own both need (D6 of `strong-skipping`).
    let hidden = identity::HiddenCaptures::of(file);
    let mut composed = file.clone();
    composed.phase = MirPhase::Composed;
    let mut appended = Appended {
        next: file.bodies.len(),
        bodies: Vec::new(),
    };
    for (id, body) in file.bodies.iter() {
        if body.is_compose {
            composed.bodies[id] = rewrite::compose_body(file, id, &keys, &mut appended, &hidden);
        }
        // The body key a reload names the body by (D5 of `compose-hot-reload`).
        if body.is_compose
            && matches!(
                body.owner,
                BodyOwner::Function(_) | BodyOwner::Closure { .. }
            )
        {
            composed.bodies[id].body_key = Some(keys.body(id).0);
        }
    }
    appended.bodies.sort_by_key(|(id, _)| id.index());
    for (id, body) in appended.bodies {
        let allocated = composed.bodies.alloc(body);
        debug_assert_eq!(allocated, id, "appended bodies keep their reserved ids");
    }
    composed
}

/// The bodies the transform adds after the file's: restart functions, each reserved before it
/// is built so that the bodies building its value can name it.
pub(crate) struct Appended {
    next: usize,
    bodies: Vec<(BodyId, Body)>,
}

impl Appended {
    /// The id of the next appended body.
    pub(crate) fn reserve(&mut self) -> BodyId {
        let id = BodyId::from_index(self.next);
        self.next += 1;
        id
    }

    /// Fills the reserved body `id`.
    pub(crate) fn fill(&mut self, id: BodyId, body: Body) {
        self.bodies.push((id, body));
    }
}

/// The key of every body's group and of every region of it (D5): computed in body order, since
/// a closure's key extends its parent's region's.
pub(crate) struct BodyKeys {
    /// Per body: the key its own group has, or for a body that is not composable the root its
    /// closures' keys extend, with its label.
    bodies: Vec<(u64, String)>,
    /// Per body, per region: its key.
    regions: Vec<Vec<u64>>,
}

impl BodyKeys {
    fn of(file: &MirFile) -> Self {
        let mut keys = BodyKeys {
            bodies: Vec::with_capacity(file.bodies.len()),
            regions: Vec::with_capacity(file.bodies.len()),
        };
        for (_, body) in file.bodies.iter() {
            let (key, label) = match &body.owner {
                BodyOwner::Function(path) | BodyOwner::Value(path) => {
                    let path = path.to_string();
                    (keys::body_key(&path), format!("body {}", path))
                }
                BodyOwner::Default { function, .. } => {
                    let path = function.to_string();
                    (keys::body_key(&path), format!("body {}", path))
                }
                BodyOwner::Closure { parent, .. } => match &body.origin {
                    Some(origin) => {
                        let parent_key = keys.region_or_body(*parent, origin.region);
                        (
                            keys::closure_key(parent_key, &origin.step),
                            format!("closure {}", origin.step.describe()),
                        )
                    }
                    None => keys.bodies[parent.index()].clone(),
                },
                BodyOwner::Restart { function: parent } => keys.bodies[parent.index()].clone(),
            };
            let mut regions = Vec::with_capacity(body.groups.len());
            for (_, region) in body.groups.iter() {
                let parent = region.parent.map_or(key, |parent| regions[parent.index()]);
                regions.push(keys::region_key(parent, &region.step));
            }
            keys.bodies.push((key, label));
            keys.regions.push(regions);
        }
        keys
    }

    /// The key of region `region` of body `body`, or of the body when `None`.
    pub(crate) fn region_or_body(&self, body: BodyId, region: Option<prism_mir::GroupId>) -> u64 {
        match region {
            Some(region) => self.regions[body.index()][region.index()],
            None => self.bodies[body.index()].0,
        }
    }

    /// The key and label of body `body`'s own group.
    pub(crate) fn body(&self, body: BodyId) -> (u64, &str) {
        let (key, label) = &self.bodies[body.index()];
        (*key, label)
    }
}

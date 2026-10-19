//! `prism-compose`: the composition runtime, which composable code runs on (`compose-runtime`).
//!
//! It implements the runtime side of the first-class compose and hot reload invariant: a slot
//! table of groups with opaque keys, typed slots that remember values, restart scopes that
//! recompose alone when invalidated, and node groups whose nodes an [`Applier`] owns. It knows
//! nothing about UI and depends on `prism-runtime` only (D2): `prism-ui` supplies an applier,
//! and generated code reaches the runtime only through the `extern "C"` entry points listed in
//! [`COMPOSE_FUNCTIONS`]. It calls program code only through function values and the drop and
//! equality entries of slot types, never through a code address of its own.
//!
//! - **The boundary** (D1). The runtime treats keys as opaque words and never interprets the
//!   change words composables pass each other; `compose-transform` computes both, and emits the
//!   slot type records and restart functions.
//! - **Handles** (D3). Compositions, scopes and nodes are generational 64-bit handles
//!   ([`Handle`]), and a composer is the identifier of the pass in progress, so every entry
//!   point validates its arguments in O(1) and a misuse is a runtime error, never undefined
//!   behavior.
//! - **One pass at a time per thread** (D4). The state is per thread ([`thread`]); a scope
//!   invalidated during a pass is recomposed by the next one.
//! - **The slot table** (D5 to D7): a generational arena tree of [`group::Group`]s with keyed
//!   matching, typed and word [`Slot`]s, and removals released at the end of the pass.
//! - **Restart scopes** (D8) and **node groups** (D9), in [`composer`] and [`composition`].

pub mod applier;
pub mod composer;
pub mod composition;
mod dump;
mod entry;
mod functions;
pub mod group;
mod handle;
pub mod slot;
pub mod state;
pub mod test_support;
pub mod thread;

#[cfg(test)]
mod state_tests;
#[cfg(test)]
mod tests;

pub use applier::{Applier, NodeHandle, NodeTree};
pub use composer::{invalidate, ComposeResult, Composer, Misuse};
pub use composition::{active_composer, Composition};
pub use functions::{compose_function, COMPOSE_FUNCTIONS};
pub use group::GroupKind;
pub use handle::{Arena, Handle};
pub use slot::{Slot, SlotType};
pub use state::{finish_report, CreationOwner, StateError, States};
pub use thread::{active_scope, live_compositions_report};

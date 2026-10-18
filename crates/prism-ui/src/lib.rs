//! `prism-ui`: the UI toolkit's runtime (`ui-toolkit`), which turns the compositions of
//! `runApp` into user interfaces.
//!
//! It implements the UI half of the first-class compose and hot reload invariant: the toolkit's
//! composables are ordinary Prism code over two node intrinsics, and this crate holds what they
//! drive, the node tree that the compose runtime's applier builds, with its layout, drawing,
//! input and frame loop. It depends on `prism-runtime` and `prism-compose` only, and on no
//! window system (D2): generated code reaches it through its C entry points, called by symbol,
//! and a window host plugs in through the [`host`] interface.
//!
//! - **The boundary** (D1). Node groups, placement and deferred removal are `prism-compose`'s;
//!   this crate supplies the [`applier::UiApplier`] they call, the handler table, layout, the
//!   display list, input, the frame loop and the headless host.
//! - **Per-thread state** (D2). The tree, the handler table and at most one running app live in
//!   a `thread_local!` ([`thread`]), and nothing in it is borrowed while program code runs: a
//!   handler's function value is copied out of its entry before it is called, and released
//!   handlers are dropped after the borrow, and after the compose runtime's, ends.
//! - **Handlers in a table** (D23). A `Copy` modifier element names a handler by a
//!   [`HandlerId`] into the [`handlers`] table, whose entries are owned by the nodes that claim
//!   them, duplicated for a second node when keepable, and dropped exactly once without
//!   counting, as the zero-cost memory safety invariant requires.
//! - **Modifiers are plain data** (D25). A chain is decoded into its node, compared element by
//!   element on every pass, and the state a stateful modifier needs (bounds, focus, the
//!   pressed target, handler entries) lives in the node or the dispatcher, never in the
//!   composition.

pub mod applier;
pub mod draw;
pub mod dump;
pub mod error;
pub mod geometry;
pub mod handle;
pub mod handlers;
pub mod layout;
pub mod modifier;
pub mod node;
pub mod properties;
pub mod runtime;
pub mod test_support;
pub mod text;
pub mod thread;
pub mod tree;

pub use applier::UiApplier;
pub use error::{UiError, UiResult};
pub use handle::{HandlerId, NodeId};
pub use tree::UiTree;

#[cfg(test)]
mod tests {
    /// The scenario "Headless programs link no window code": the crate depends on the base and
    /// composition runtimes only.
    #[test]
    fn the_crate_depends_on_the_runtimes_only() {
        let manifest = include_str!("../Cargo.toml");
        let dependencies: Vec<&str> = manifest
            .split("[dependencies]")
            .nth(1)
            .unwrap()
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#') && !line.starts_with('['))
            .map(|line| line.split('=').next().unwrap().trim())
            .collect();
        assert_eq!(dependencies, ["prism-runtime", "prism-compose"]);
        assert!(!manifest.contains("[dev-dependencies]"));
        assert!(!manifest.contains("[build-dependencies]"));
    }
}

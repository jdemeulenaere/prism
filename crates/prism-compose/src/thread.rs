//! The per-thread state of the composition runtime (D4 of `compose-runtime`).
//!
//! Each thread has its own compositions, groups, pass counter and active pass, in a
//! `thread_local!`. Prism runs compositions on one program thread, and Rust tests run on many
//! threads, each with its own state. A thread runs at most one pass at a time.
//!
//! **The re-entrancy rule.** Every operation borrows the state through [`with_state`], does its
//! work and lets go of it before it calls program code: content and restart functions, and the
//! drop and equality entries of slot types and closures. No reference into the state is ever
//! live while program code runs, even when that code calls back into the runtime, so the pass
//! driver keeps handles and indexes, never references, across those calls. A nested borrow is
//! a bug of this crate, which the `RefCell` turns into a Rust panic rather than aliasing.

use std::cell::RefCell;

/// The pass a thread is running: its identifier, which is the composer generated code passes,
/// and the composition it composes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActivePass {
    pub identifier: u64,
    pub composition: crate::Handle,
}

/// What one thread's runtime holds.
#[derive(Debug)]
pub struct State {
    /// The identifier of the last pass started, counting from 1.
    pub pass_counter: u64,
    /// The pass running now, if any.
    pub active: Option<ActivePass>,
    /// Compositions started and not disposed.
    pub live_compositions: usize,
}

impl State {
    const fn new() -> State {
        State {
            pass_counter: 0,
            active: None,
            live_compositions: 0,
        }
    }
}

thread_local! {
    static STATE: RefCell<State> = const { RefCell::new(State::new()) };
}

/// Runs `f` with this thread's state. `f` must not call program code (the re-entrancy rule).
pub fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> R {
    STATE.with(|state| f(&mut state.borrow_mut()))
}

/// The line the allocation check writes when compositions of this thread were started and
/// never disposed (D13): `allocation check: compositions still live at exit: <n>`. Installed
/// with `prism_runtime::set_finish_report`, which calls it on the program's thread.
pub fn live_compositions_report() -> Option<String> {
    let live = with_state(|state| state.live_compositions);
    (live > 0).then(|| {
        format!(
            "allocation check: compositions still live at exit: {}",
            live
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_thread_has_its_own_state() {
        with_state(|state| state.live_compositions = 2);
        assert_eq!(
            live_compositions_report().as_deref(),
            Some("allocation check: compositions still live at exit: 2")
        );
        let other = std::thread::spawn(|| {
            assert_eq!(live_compositions_report(), None);
            with_state(|state| {
                state.pass_counter += 1;
                state.pass_counter
            })
        })
        .join()
        .unwrap();
        assert_eq!(other, 1);
        with_state(|state| {
            assert_eq!(state.pass_counter, 0, "the other thread's pass is its own");
            state.live_compositions = 0;
        });
        assert_eq!(live_compositions_report(), None);
    }
}

//! Recomposition counts (D14 of `strong-skipping`): how many times each composable body ran and
//! was skipped, for a headless script's `dump recompositions`, like Compose's recomposition and
//! skip counts.
//!
//! Counting is per thread and off by default. [`enable`] switches it on for a set of body keys,
//! the keys of the program's own composable functions; while it is on, a restart or replaceable
//! group started with one of them counts a run, and skipping that group's body turns the run
//! into a skip. While it is off, a group start pays one branch.

use crate::thread::with_state;
use std::collections::HashMap;

/// The counts of one body: runs and skips.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    pub ran: u64,
    pub skipped: u64,
}

/// Switches counting on, on this thread, for the bodies whose group keys are `keys`, from zero.
pub fn enable(keys: &[u64]) {
    with_state(|state| {
        state.counting = Some(Box::new(
            keys.iter()
                .map(|key| (*key as i64, Counts::default()))
                .collect(),
        ));
    })
}

/// Switches counting off on this thread.
pub fn disable() {
    with_state(|state| state.counting = None)
}

/// The counts since counting was enabled or last taken, of the bodies that ran or were skipped,
/// as `(key, counts)` in no particular order; every count starts again from zero.
pub fn take() -> Vec<(u64, Counts)> {
    with_state(|state| {
        let Some(counting) = &mut state.counting else {
            return Vec::new();
        };
        counting
            .iter_mut()
            .filter(|(_, counts)| counts.ran + counts.skipped > 0)
            .map(|(key, counts)| (*key as u64, std::mem::take(counts)))
            .collect()
    })
}

/// Counts a run of the body whose group starts with `key`, when it is counted.
pub(crate) fn count_start(counting: &mut HashMap<i64, Counts>, key: i64) {
    if let Some(counts) = counting.get_mut(&key) {
        counts.ran += 1;
    }
}

/// Turns the run counted for the group started with `key` into a skip.
pub(crate) fn count_skip(counting: &mut HashMap<i64, Counts>, key: i64) {
    if let Some(counts) = counting.get_mut(&key) {
        counts.ran = counts.ran.saturating_sub(1);
        counts.skipped += 1;
    }
}

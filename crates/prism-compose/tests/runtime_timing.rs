//! The run-time benchmark of the composition runtime (D16 of `compose-runtime`): compositions
//! of 1,000, 10,000 and 100,000 restart groups, each holding a word slot, in a tree of fan-out
//! 10, whose content and restart functions allocate nothing. It reports the first composition
//! (per group), a steady-state pass that recomposes the root with every child skipping, the
//! recomposition of one invalid leaf scope, and a keyed reversal of 1,000 movable groups.
//!
//! It is ignored by default, since timings of a debug build or a busy machine vary. Run it with
//!
//! ```text
//! PRISM_ASSERT_TIMING=1 cargo test --release -p prism-compose --test runtime_timing -- --ignored --nocapture
//! ```
//!
//! Under `PRISM_ASSERT_TIMING=1` it fails when the median leaf recomposition, or the first
//! composition's time per group, grows by a factor of 2 or more from the smallest composition
//! to the largest (compose, "Recomposing one scope does not depend on the composition's
//! size").

use prism_compose::test_support::{descriptor, inline_function, inline_word};
use prism_compose::{invalidate, Composer, Composition, GroupKind, Handle, NodeTree};
use prism_runtime::Descriptor;
use std::cell::{Cell, RefCell};
use std::time::{Duration, Instant};

const FAN_OUT: u64 = 10;

thread_local! {
    static SIZE: Cell<u64> = const { Cell::new(0) };
    static GROUP: Cell<Option<&'static Descriptor>> = const { Cell::new(None) };
    static SCOPES: RefCell<Vec<u64>> = const { RefCell::new(Vec::new()) };
    static REVERSED: Cell<bool> = const { Cell::new(false) };
}

/// The group `id` and its subtree: a restart group with a word slot, skipping when it may,
/// whose children are `10 id + 1` to `10 id + 10`.
fn group(k: Composer, id: u64) {
    k.start_group(GroupKind::Restart, 1, 0).unwrap();
    SCOPES.with(|scopes| scopes.borrow_mut()[id as usize] = k.current_scope().unwrap().word());
    if k.skipping().unwrap() {
        k.skip_to_group_end().unwrap();
    } else {
        if k.has_word(0).unwrap() {
            k.word().unwrap();
        } else {
            k.set_word(0, id as i64).unwrap();
        }
        let size = SIZE.with(Cell::get);
        for child in id * FAN_OUT + 1..=(id * FAN_OUT + FAN_OUT).min(size - 1) {
            group(k, child);
        }
    }
    k.end_restart_group(inline_function(GROUP.with(Cell::get).unwrap(), id))
        .unwrap();
}

unsafe extern "C" fn group_code(environment: *mut u8, composer: u64) {
    group(Composer(composer), inline_word(environment));
}

unsafe extern "C" fn root_code(_: *mut u8, composer: u64) {
    group(Composer(composer), 0);
}

unsafe extern "C" fn movable_code(_: *mut u8, composer: u64) {
    let k = Composer(composer);
    let reversed = REVERSED.with(Cell::get);
    for index in 0..1000i64 {
        let id = if reversed { 999 - index } else { index };
        k.start_group(GroupKind::Movable, 1, id).unwrap();
        if !k.has_word(0).unwrap() {
            k.set_word(0, id).unwrap();
        } else {
            k.word().unwrap();
        }
        k.end_group().unwrap();
    }
}

fn median(mut samples: Vec<Duration>) -> Duration {
    samples.sort();
    samples[samples.len() / 2]
}

struct Measured {
    size: u64,
    first_per_group: Duration,
    steady: Duration,
    leaf: Duration,
}

fn measure(size: u64) -> Measured {
    SIZE.with(|s| s.set(size));
    SCOPES.with(|scopes| *scopes.borrow_mut() = vec![0; size as usize]);
    GROUP.with(|g| g.set(Some(descriptor(group_code))));
    let root = descriptor(root_code);
    let mut firsts = Vec::new();
    for _ in 0..5 {
        let started = Instant::now();
        let composition =
            Composition::start(Box::new(NodeTree::new()), inline_function(root, 0)).unwrap();
        firsts.push(started.elapsed());
        Composition::dispose(composition).unwrap();
    }
    let composition =
        Composition::start(Box::new(NodeTree::new()), inline_function(root, 0)).unwrap();
    let root_scope = Composition::root_scope(composition).unwrap();
    let leaf = Handle(SCOPES.with(|scopes| scopes.borrow()[size as usize - 1]));
    let mut steady = Vec::new();
    let mut leaves = Vec::new();
    for _ in 0..101 {
        invalidate(root_scope);
        let started = Instant::now();
        assert!(Composition::recompose(composition).unwrap());
        steady.push(started.elapsed());
        invalidate(leaf);
        let started = Instant::now();
        assert!(Composition::recompose(composition).unwrap());
        leaves.push(started.elapsed());
    }
    Composition::dispose(composition).unwrap();
    Measured {
        size,
        first_per_group: median(firsts) / size as u32,
        steady: median(steady),
        leaf: median(leaves),
    }
}

#[test]
#[ignore]
fn runtime_timing() {
    let results: Vec<Measured> = [1_000, 10_000, 100_000].into_iter().map(measure).collect();
    println!(
        "{:>8} {:>16} {:>14} {:>14}",
        "groups", "first per group", "steady pass", "leaf pass"
    );
    for result in &results {
        println!(
            "{:>8} {:>16?} {:>14?} {:>14?}",
            result.size, result.first_per_group, result.steady, result.leaf
        );
    }
    let movable = descriptor(movable_code);
    let composition =
        Composition::start(Box::new(NodeTree::new()), inline_function(movable, 0)).unwrap();
    let root_scope = Composition::root_scope(composition).unwrap();
    let mut reversals = Vec::new();
    for _ in 0..101 {
        REVERSED.with(|r| r.set(!r.get()));
        invalidate(root_scope);
        let started = Instant::now();
        Composition::recompose(composition).unwrap();
        reversals.push(started.elapsed());
    }
    Composition::dispose(composition).unwrap();
    println!(
        "keyed reversal of 1,000 movable groups: {:?}",
        median(reversals)
    );
    if std::env::var("PRISM_ASSERT_TIMING").as_deref() == Ok("1") {
        let (small, large) = (&results[0], &results[2]);
        let leaf_growth = large.leaf.as_secs_f64() / small.leaf.as_secs_f64();
        let first_growth =
            large.first_per_group.as_secs_f64() / small.first_per_group.as_secs_f64();
        println!(
            "leaf pass growth x{:.2}, first composition per group growth x{:.2}",
            leaf_growth, first_growth
        );
        assert!(leaf_growth < 2.0, "the leaf pass grows x{:.2}", leaf_growth);
        assert!(
            first_growth < 2.0,
            "the first composition per group grows x{:.2}",
            first_growth
        );
    }
}

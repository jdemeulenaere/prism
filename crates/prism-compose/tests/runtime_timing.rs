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
//!
//! `many_readers_of_one_state` (`state-handles`, compose: "Many readers of one state") composes
//! 1,000, 10,000 and 100,000 restart scopes that each read one state ten times per pass, then
//! writes the state and recomposes, ten times: it reports the median time per read of those
//! passes and checks that each scope is registered once. Under `PRISM_ASSERT_TIMING=1` it fails
//! when that time grows by a factor of 2 or more from the smallest composition to the largest.

use prism_compose::test_support::{descriptor, inline_function, inline_word, slot_type};
use prism_compose::{
    invalidate, Composer, Composition, GroupKind, Handle, NodeTree, SlotType, States,
};
use prism_runtime::Descriptor;
use std::cell::{Cell, RefCell};
use std::time::{Duration, Instant};

const FAN_OUT: u64 = 10;

thread_local! {
    static SIZE: Cell<u64> = const { Cell::new(0) };
    static GROUP: Cell<Option<&'static Descriptor>> = const { Cell::new(None) };
    static SCOPES: RefCell<Vec<u64>> = const { RefCell::new(Vec::new()) };
    static REVERSED: Cell<bool> = const { Cell::new(false) };
    static READER: Cell<Option<&'static Descriptor>> = const { Cell::new(None) };
    static STATE: Cell<(u64, usize)> = const { Cell::new((0, 0)) };
}

/// How many times each scope of `many_readers_of_one_state` reads the state per pass.
const READS: u64 = 10;

/// The state of `many_readers_of_one_state` and its record.
fn state() -> (Handle, *const SlotType) {
    let (handle, record) = STATE.with(Cell::get);
    (Handle(handle), record as *const SlotType)
}

/// The reader `id` and its subtree: a restart group reading the state ten times, skipping
/// when it may, whose children are `10 id + 1` to `10 id + 10`.
fn reader(k: Composer, id: u64) {
    k.start_group(GroupKind::Restart, 2, 0).unwrap();
    if k.skipping().unwrap() {
        k.skip_to_group_end().unwrap();
    } else {
        let (handle, record) = state();
        for _ in 0..READS {
            let value = States::read(handle, record).unwrap();
            std::hint::black_box(unsafe { *(value as *const i64) });
        }
        let size = SIZE.with(Cell::get);
        for child in id * FAN_OUT + 1..=(id * FAN_OUT + FAN_OUT).min(size - 1) {
            reader(k, child);
        }
    }
    k.end_restart_group(inline_function(READER.with(Cell::get).unwrap(), id))
        .unwrap();
}

unsafe extern "C" fn reader_code(environment: *mut u8, composer: u64) {
    reader(Composer(composer), inline_word(environment));
}

unsafe extern "C" fn reader_root_code(_: *mut u8, composer: u64) {
    reader(Composer(composer), 0);
}

/// The median time per read of passes that recompose `size` readers of one state after a
/// write, with the number of readers the state registered.
fn measure_readers(size: u64) -> (Duration, usize) {
    SIZE.with(|s| s.set(size));
    READER.with(|r| r.set(Some(descriptor(reader_code))));
    let record = slot_type::<i64>("Int");
    let mut value = 0i64;
    let handle = unsafe { States::create(record, (&mut value as *mut i64).cast()) }.unwrap();
    STATE.with(|s| s.set((handle.word(), record as *const SlotType as usize)));
    let root = descriptor(reader_root_code);
    let composition =
        Composition::start(Box::new(NodeTree::new()), inline_function(root, 0)).unwrap();
    let mut samples = Vec::new();
    for pass in 1..=10i64 {
        let mut next = pass;
        unsafe { States::write(handle, record, (&mut next as *mut i64).cast()) }.unwrap();
        let started = Instant::now();
        assert!(Composition::recompose(composition).unwrap());
        samples.push(started.elapsed() / (size * READS) as u32);
    }
    let readers = prism_compose::thread::with_state(|state| {
        state.cells.get(handle).unwrap().readers.scopes().len()
    });
    Composition::dispose(composition).unwrap();
    States::release(&[handle]);
    (median(samples), readers)
}

#[test]
#[ignore]
fn many_readers_of_one_state() {
    let results: Vec<(u64, Duration, usize)> = [1_000, 10_000, 100_000]
        .into_iter()
        .map(|size| {
            let (read, readers) = measure_readers(size);
            (size, read, readers)
        })
        .collect();
    println!(
        "{:>8} {:>14} {:>10}",
        "readers", "time per read", "registered"
    );
    for (size, read, readers) in &results {
        println!("{:>8} {:>14?} {:>10}", size, read, readers);
        assert_eq!(*readers as u64, *size, "each scope is registered once");
    }
    if std::env::var("PRISM_ASSERT_TIMING").as_deref() == Ok("1") {
        let growth = results[2].1.as_secs_f64() / results[0].1.as_secs_f64();
        println!("time per read growth x{:.2}", growth);
        assert!(growth < 2.0, "the time per read grows x{:.2}", growth);
    }
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

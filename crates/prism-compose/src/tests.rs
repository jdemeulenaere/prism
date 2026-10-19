//! Rust tests of the composition runtime: what a Prism program cannot observe (release order,
//! slot addresses outliving removals, typed slot records, node placement), and the protocol's
//! behavior driven from Rust content.

use crate::applier::{Applier, NodeHandle, NodeTree};
use crate::composer::*;
use crate::composition::Composition;
use crate::group::GroupKind;
use crate::handle::Handle;
use crate::test_support::*;
use crate::thread::with_state;
use prism_runtime::FunctionValue;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};

type Log = Rc<RefCell<Vec<String>>>;

thread_local! {
    static DROPS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

fn drops() -> Vec<String> {
    DROPS.with(|drops| drops.borrow().clone())
}

fn record_drop(what: String) {
    DROPS.with(|drops| drops.borrow_mut().push(what));
}

/// A value whose drop is recorded.
#[derive(Debug, PartialEq)]
struct Tracked(&'static str);

impl Drop for Tracked {
    fn drop(&mut self) {
        record_drop(format!("drop {}", self.0));
    }
}

/// A 16-byte value whose drop is recorded.
#[derive(Debug, PartialEq)]
struct Wide(&'static str);

impl Drop for Wide {
    fn drop(&mut self) {
        record_drop(format!("drop wide {}", self.0));
    }
}

fn log() -> Log {
    Rc::new(RefCell::new(Vec::new()))
}

fn push(log: &Log, line: impl Into<String>) {
    log.borrow_mut().push(line.into());
}

fn tree() -> Box<dyn Applier> {
    Box::new(NodeTree::new())
}

/// A restart function whose drop is recorded as `drop <name>`.
fn named_restart(name: &'static str, body: impl Fn(Composer) + 'static) -> FunctionValue {
    let marker = Tracked(name);
    content(move |k| {
        let _ = &marker;
        body(k)
    })
}

/// Stores `value` in a typed slot of `T` at the cursor.
fn set<T>(k: Composer, slot_type: &'static crate::SlotType, value: T) {
    let mut value = std::mem::ManuallyDrop::new(value);
    unsafe {
        k.set_slot(slot_type, (&mut *value as *mut T).cast())
            .unwrap()
    };
}

/// Reads a typed slot of `T` at the cursor, cloning it out.
fn get<T: Clone>(k: Composer, slot_type: &'static crate::SlotType) -> Option<T> {
    let address = unsafe { k.slot(slot_type).unwrap() };
    (!address.is_null()).then(|| unsafe { (*(address as *const T)).clone() })
}

/// Remembers a word of `tag`, logging when the calculation runs.
fn remember(k: Composer, tag: i64, log: &Log, value: i64) -> i64 {
    if k.has_word(tag).unwrap() {
        return k.word().unwrap();
    }
    push(log, format!("computed {}", value));
    k.set_word(tag, value).unwrap();
    value
}

#[test]
fn starting_a_composition_composes_its_content_once() {
    let log = log();
    let seen = log.clone();
    let c = Composition::start(tree(), content(move |_| push(&seen, "composed"))).unwrap();
    assert_eq!(*log.borrow(), ["composed"]);
    assert!(!Composition::recompose(c).unwrap(), "nothing is invalid");
    assert_eq!(log.borrow().len(), 1);
    invalidate(Composition::root_scope(c).unwrap());
    assert!(Composition::recompose(c).unwrap());
    assert_eq!(log.borrow().len(), 2);
    Composition::dispose(c).unwrap();
    assert_eq!(Composition::recompose(c), Err(STALE_COMPOSITION));
    assert_eq!(Composition::dispose(c), Err(STALE_COMPOSITION));
    assert_eq!(crate::live_compositions_report(), None);
}

#[test]
fn setting_the_content_recomposes_the_root_and_releases_the_old_content_after_the_pass() {
    let log = log();
    let first = log.clone();
    let c = Composition::start(tree(), named_restart("first", move |_| push(&first, "1"))).unwrap();
    let second = log.clone();
    Composition::set_content(
        c,
        content(move |_| {
            push(&second, "2");
            assert!(
                drops().is_empty(),
                "the old content lives until the pass ends"
            );
        }),
    )
    .unwrap();
    assert_eq!(*log.borrow(), ["1", "2"]);
    assert_eq!(drops(), ["drop first"]);
    assert!(!Composition::recompose(c).unwrap());
    Composition::dispose(c).unwrap();
}

#[test]
fn groups_are_reused_in_order_and_unvisited_ones_removed() {
    let log = log();
    let handles: Rc<RefCell<Vec<Handle>>> = Rc::default();
    let count = Rc::new(Cell::new(3));
    let (l, h, n) = (log.clone(), handles.clone(), count.clone());
    let c = Composition::start(
        tree(),
        content(move |k| {
            h.borrow_mut().clear();
            for i in 0..n.get() {
                k.start_group(GroupKind::Replaceable, 7, 0).unwrap();
                remember(k, 0, &l, i);
                h.borrow_mut()
                    .push(with_state(|s| s.frames.last().unwrap().group));
                k.end_group().unwrap();
            }
        }),
    )
    .unwrap();
    let first = handles.borrow().clone();
    // Equal keys match in order: two of three groups are kept, the third removed.
    count.set(2);
    invalidate(Composition::root_scope(c).unwrap());
    Composition::recompose(c).unwrap();
    assert_eq!(*handles.borrow(), first[..2]);
    assert_eq!(*log.borrow(), ["computed 0", "computed 1", "computed 2"]);
    assert!(with_state(|s| s.groups.get(first[2]).is_none()), "released");
    // A group is created again for the third.
    count.set(3);
    invalidate(Composition::root_scope(c).unwrap());
    Composition::recompose(c).unwrap();
    assert_eq!(handles.borrow()[..2], first[..2]);
    assert_ne!(handles.borrow()[2], first[2]);
    assert_eq!(log.borrow().last().unwrap(), "computed 2");
    assert_eq!(log.borrow().len(), 4);
    Composition::dispose(c).unwrap();
}

#[test]
fn a_reversal_of_a_thousand_movable_groups_keeps_every_group() {
    let reversed = Rc::new(Cell::new(false));
    let handles: Rc<RefCell<Vec<(i64, Handle)>>> = Rc::default();
    let computed = Rc::new(Cell::new(0));
    let (r, h, n) = (reversed.clone(), handles.clone(), computed.clone());
    let c = Composition::start(
        tree(),
        content(move |k| {
            h.borrow_mut().clear();
            for i in 0..1000i64 {
                let id = if r.get() { 999 - i } else { i };
                k.start_group(GroupKind::Movable, 1, id).unwrap();
                if !k.has_word(0).unwrap() {
                    n.set(n.get() + 1);
                    k.set_word(0, id * 10).unwrap();
                } else {
                    assert_eq!(k.word().unwrap(), id * 10);
                }
                h.borrow_mut()
                    .push((id, with_state(|s| s.frames.last().unwrap().group)));
                k.end_group().unwrap();
            }
        }),
    )
    .unwrap();
    let mut before = handles.borrow().clone();
    reversed.set(true);
    invalidate(Composition::root_scope(c).unwrap());
    Composition::recompose(c).unwrap();
    let mut after = handles.borrow().clone();
    assert_eq!(computed.get(), 1000, "no group was created again");
    before.sort();
    after.sort();
    assert_eq!(before, after);
    let dump = Composition::dump(c).unwrap();
    assert!(
        dump.starts_with("root\n  movable 1 (999)\n    word 0 = 9990\n"),
        "{}",
        dump
    );
    Composition::dispose(c).unwrap();
}

#[test]
fn unbalanced_groups_are_misuses() {
    let result: Rc<Cell<Option<Misuse>>> = Rc::default();
    let r = result.clone();
    let c = Composition::start(tree(), content(move |k| r.set(k.end_group().err()))).unwrap();
    assert_eq!(result.get(), Some(NOT_STARTED));
    Composition::dispose(c).unwrap();

    let r = result.clone();
    let c = Composition::start(
        tree(),
        content(move |k| {
            k.start_group(GroupKind::Restart, 1, 0).unwrap();
            r.set(k.end_group().err());
            k.end_restart_group(content(|_| {})).unwrap();
            k.start_group(GroupKind::Replaceable, 2, 0).unwrap();
            assert_eq!(k.end_restart_group(content(|_| {})), Err(WRONG_KIND));
            k.end_group().unwrap();
        }),
    )
    .unwrap();
    assert_eq!(result.get(), Some(WRONG_KIND));
    Composition::dispose(c).unwrap();

    let started = Composition::start(
        tree(),
        content(|k| k.start_group(GroupKind::Replaceable, 1, 0).unwrap()),
    );
    assert_eq!(started, Err(LEFT_OPEN));
    // The pass ended: other operations work.
    let c = Composition::start(tree(), content(|_| {})).unwrap();
    Composition::dispose(c).unwrap();
}

#[test]
fn a_composer_is_valid_only_during_its_pass() {
    let kept: Rc<Cell<Option<Composer>>> = Rc::default();
    let k2 = kept.clone();
    let c = Composition::start(tree(), content(move |k| k2.set(Some(k)))).unwrap();
    let old = kept.get().unwrap();
    assert_eq!(
        old.start_group(GroupKind::Replaceable, 1, 0),
        Err(OUTSIDE_PASS)
    );
    assert_eq!(old.word(), Err(OUTSIDE_PASS));
    let k3 = kept.clone();
    Composition::set_content(
        c,
        content(move |k| {
            assert_ne!(Some(k), k3.get());
            assert_eq!(k3.get().unwrap().skipping(), Err(OUTSIDE_PASS));
            // A composition operation during a pass.
            assert_eq!(Composition::recompose(Handle(1)), Err(DURING_PASS));
        }),
    )
    .unwrap();
    Composition::dispose(c).unwrap();
}

/// compose: "A typed slot read with another slot type".
#[test]
fn a_typed_slot_read_with_another_slot_type() {
    let wide = slot_type::<Wide>("Wide");
    let narrow = slot_type::<i64>("Int");
    assert_eq!(wide.size, 16);
    let phase = Rc::new(Cell::new(0));
    let read: Rc<Cell<Option<i64>>> = Rc::default();
    let (p, r) = (phase.clone(), read.clone());
    let c = Composition::start(
        tree(),
        content(move |k| match p.get() {
            0 => set(k, wide, Wide("old")),
            1 => {
                assert!(unsafe { k.slot(narrow).unwrap() }.is_null(), "no slot");
                set(k, narrow, 42i64);
                assert!(drops().is_empty(), "dropped at the end of the pass");
            }
            _ => r.set(get::<i64>(k, narrow)),
        }),
    )
    .unwrap();
    phase.set(1);
    invalidate(Composition::root_scope(c).unwrap());
    Composition::recompose(c).unwrap();
    assert_eq!(drops(), ["drop wide old"]);
    phase.set(2);
    invalidate(Composition::root_scope(c).unwrap());
    Composition::recompose(c).unwrap();
    assert_eq!(read.get(), Some(42));
    assert_eq!(drops().len(), 1);
    Composition::dispose(c).unwrap();
}

#[test]
fn comparing_values_with_slots() {
    let with_equality = slot_type::<Tracked>("Tracked");
    let without = slot_type_with_fingerprint::<i64>("Int", [99, 1, 2, 3], false);
    let results: Rc<RefCell<Vec<bool>>> = Rc::default();
    let value = Rc::new(Cell::new("a"));
    let (r, v) = (results.clone(), value.clone());
    let c = Composition::start(
        tree(),
        content(move |k| unsafe {
            let mut tracked = std::mem::ManuallyDrop::new(Tracked(v.get()));
            r.borrow_mut().push(
                k.changed(with_equality, (&mut *tracked as *mut Tracked).cast())
                    .unwrap(),
            );
            let mut number = 5i64;
            r.borrow_mut().push(
                k.changed(without, (&mut number as *mut i64).cast())
                    .unwrap(),
            );
        }),
    )
    .unwrap();
    invalidate(Composition::root_scope(c).unwrap());
    Composition::recompose(c).unwrap();
    // Equal: the new value is released at once, and the old one kept.
    assert_eq!(drops(), ["drop a"]);
    value.set("b");
    invalidate(Composition::root_scope(c).unwrap());
    Composition::recompose(c).unwrap();
    // Changed: the old value is released at the end of the pass.
    assert_eq!(drops(), ["drop a", "drop a"]);
    // A type without an equality entry always compares as changed.
    assert_eq!(*results.borrow(), [true, true, false, true, true, true]);
    Composition::dispose(c).unwrap();
    assert_eq!(drops(), ["drop a", "drop a", "drop b"]);
}

/// `keep` (D8 of `compose-transform`): uncertain compares, different stores, same and static
/// read past the kept slot and store only where there is none; a value not stored is released
/// at once.
#[test]
fn keeping_arguments_by_their_change_bits() {
    let tracked = slot_type::<Tracked>("Tracked");
    let results: Rc<RefCell<Vec<i64>>> = Rc::default();
    let phase = Rc::new(Cell::new(0));
    let (r, p) = (results.clone(), phase.clone());
    let c = Composition::start(
        tree(),
        content(move |k| unsafe {
            let keep = |name: &'static str, bits: i64| {
                let mut value = std::mem::ManuallyDrop::new(Tracked(name));
                k.keep(tracked, (&mut *value as *mut Tracked).cast(), bits)
                    .unwrap()
            };
            let (first, bits) = match p.get() {
                0 => ("a", UNCERTAIN),
                1 => ("b", SAME),
                2 => ("c", 3),
                3 => ("a", UNCERTAIN),
                _ => ("d", DIFFERENT),
            };
            r.borrow_mut().push(keep(first, bits));
            if p.get() == 1 {
                // Same, where no slot is kept yet: stored.
                r.borrow_mut().push(keep("x", SAME));
            }
        }),
    )
    .unwrap();
    for next in 1..=4 {
        phase.set(next);
        invalidate(Composition::root_scope(c).unwrap());
        Composition::recompose(c).unwrap();
    }
    assert_eq!(
        *results.borrow(),
        [
            DIFFERENT | KEEP_CHANGED,
            SAME,
            SAME,
            3,
            SAME,
            DIFFERENT | KEEP_CHANGED
        ]
    );
    assert_eq!(drops(), ["drop b", "drop c", "drop x", "drop a", "drop a"]);
    Composition::dispose(c).unwrap();
    assert_eq!(drops().last().map(String::as_str), Some("drop d"));
}

#[test]
fn reading_a_word_where_there_is_none_is_a_misuse() {
    let result: Rc<Cell<Option<Misuse>>> = Rc::default();
    let r = result.clone();
    let c = Composition::start(tree(), content(move |k| r.set(k.word().err()))).unwrap();
    assert_eq!(result.get(), Some(NO_SLOT));
    Composition::dispose(c).unwrap();
}

/// compose: "A removed group's slots are released after the pass".
#[test]
fn a_removed_groups_slots_are_released_after_the_pass() {
    let tracked = slot_type::<Tracked>("Tracked");
    let show = Rc::new(Cell::new(true));
    let s = show.clone();
    let c = Composition::start(
        tree(),
        content(move |k| {
            k.start_group(GroupKind::Replaceable, 1, 0).unwrap();
            if s.get() {
                k.start_group(GroupKind::Replaceable, 2, 0).unwrap();
                if get::<()>(k, tracked).is_none() {
                    set(k, tracked, Tracked("slot"));
                }
                k.end_group().unwrap();
            }
            k.end_group().unwrap();
            record_drop("mark".to_string());
        }),
    )
    .unwrap();
    show.set(false);
    invalidate(Composition::root_scope(c).unwrap());
    Composition::recompose(c).unwrap();
    assert_eq!(drops(), ["mark", "mark", "drop slot"]);
    Composition::dispose(c).unwrap();
}

/// compose: "Release order of a removed subtree".
#[test]
fn release_order_of_a_removed_subtree() {
    let tracked = slot_type::<Tracked>("Tracked");
    let show = Rc::new(Cell::new(true));
    let s = show.clone();
    let c = Composition::start(
        tree(),
        content(move |k| {
            if !s.get() {
                return;
            }
            k.start_group(GroupKind::Restart, 1, 0).unwrap();
            set(k, tracked, Tracked("first slot"));
            set(k, tracked, Tracked("second slot"));
            k.start_group(GroupKind::Restart, 2, 0).unwrap();
            set(k, tracked, Tracked("child's slot"));
            k.end_restart_group(named_restart("child's restart function", |_| {}))
                .unwrap();
            k.end_restart_group(named_restart("restart function", |_| {}))
                .unwrap();
        }),
    )
    .unwrap();
    show.set(false);
    invalidate(Composition::root_scope(c).unwrap());
    Composition::recompose(c).unwrap();
    assert_eq!(
        drops(),
        [
            "drop child's slot",
            "drop child's restart function",
            "drop second slot",
            "drop first slot",
            "drop restart function"
        ]
    );
    Composition::dispose(c).unwrap();
}

#[test]
fn removing_a_deep_chain_does_not_overflow_the_stack() {
    let depth = Rc::new(Cell::new(100_000));
    let d = depth.clone();
    let c = Composition::start(
        tree(),
        content(move |k| {
            for i in 0..d.get() {
                k.start_group(GroupKind::Replaceable, i, 0).unwrap();
            }
            for _ in 0..d.get() {
                k.end_group().unwrap();
            }
        }),
    )
    .unwrap();
    assert_eq!(with_state(|s| s.groups.len()), 100_001);
    depth.set(0);
    invalidate(Composition::root_scope(c).unwrap());
    Composition::recompose(c).unwrap();
    assert_eq!(with_state(|s| s.groups.len()), 1);
    Composition::dispose(c).unwrap();
    assert_eq!(with_state(|s| s.groups.len()), 0);
}

/// A restart scope over `body`: a restart group of `key` whose restart function runs it again.
fn scope(k: Composer, key: i64, body: Rc<dyn Fn(Composer)>) {
    k.start_group(GroupKind::Restart, key, 0).unwrap();
    body(k);
    let again = body.clone();
    k.end_restart_group(content(move |c| scope(c, key, again.clone())))
        .unwrap();
}

#[test]
fn invalid_scopes_recompose_in_tree_order_parents_first_and_once() {
    let log = log();
    let scopes: Rc<RefCell<Vec<Handle>>> = Rc::default();
    let (l, s) = (log.clone(), scopes.clone());
    let leaf =
        |name: &'static str, log: Log, scopes: Rc<RefCell<Vec<Handle>>>| -> Rc<dyn Fn(Composer)> {
            Rc::new(move |k: Composer| {
                push(&log, name);
                scopes.borrow_mut().push(k.current_scope().unwrap());
            })
        };
    let a = leaf("a", l.clone(), s.clone());
    let b = leaf("b", l.clone(), s.clone());
    let (l2, s2) = (l.clone(), s.clone());
    let parent: Rc<dyn Fn(Composer)> = Rc::new(move |k: Composer| {
        push(&l2, "parent");
        s2.borrow_mut().push(k.current_scope().unwrap());
        scope(k, 11, a.clone());
        scope(k, 12, b.clone());
    });
    let c = Composition::start(tree(), content(move |k| scope(k, 1, parent.clone()))).unwrap();
    assert_eq!(*log.borrow(), ["parent", "a", "b"]);
    let (parent_scope, a_scope, b_scope) = {
        let s = scopes.borrow();
        (s[0], s[1], s[2])
    };
    // Unrelated scopes run in tree order, whatever the order of invalidation.
    log.borrow_mut().clear();
    invalidate(b_scope);
    invalidate(a_scope);
    assert!(Composition::recompose(c).unwrap());
    assert_eq!(*log.borrow(), ["a", "b"]);
    // A parent and its child both invalid: each composes once, the parent first.
    log.borrow_mut().clear();
    invalidate(b_scope);
    invalidate(parent_scope);
    assert!(Composition::recompose(c).unwrap());
    assert_eq!(*log.borrow(), ["parent", "a", "b"]);
    assert!(!Composition::recompose(c).unwrap());
    Composition::dispose(c).unwrap();
}

/// compose: "A restart reads its kept slots" (`compose-transform`): during a restart and before
/// the restart function starts a group, it reads the slots its scope's group kept.
#[test]
fn a_restart_reads_its_kept_slots() {
    let word = slot_type::<i64>("Int");
    let other = slot_type::<u8>("Byte");
    let captured: Rc<Cell<Option<Handle>>> = Rc::default();
    let read: Rc<RefCell<Vec<Result<i64, Misuse>>>> = Rc::default();
    let cap = captured.clone();
    let seen = read.clone();
    let c = Composition::start(
        tree(),
        content(move |k| {
            k.start_group(GroupKind::Restart, 7, 0).unwrap();
            set(k, word, 10i64);
            set(k, word, 20i64);
            cap.set(Some(k.current_scope().unwrap()));
            let seen = seen.clone();
            k.end_restart_group(content(move |k| {
                let reading = |index, slot_type| {
                    let result = unsafe { k.kept_slot(index, slot_type) };
                    seen.borrow_mut()
                        .push(result.map(|address| unsafe { *(address as *const i64) }));
                };
                reading(1, word);
                reading(0, word);
                reading(2, word);
                reading(0, other);
                k.start_group(GroupKind::Restart, 7, 0).unwrap();
                k.skip_to_group_end().unwrap();
                k.end_restart_group(content(|_| {})).unwrap();
            }))
            .unwrap();
        }),
    )
    .unwrap();
    // Outside a restart there is no slot to read.
    let outside = Rc::new(Cell::new(None));
    let out = outside.clone();
    let d = Composition::start(
        tree(),
        content(move |k| out.set(Some(unsafe { k.kept_slot(0, word) }.err()))),
    )
    .unwrap();
    assert_eq!(outside.get(), Some(Some(NO_SLOT)));
    invalidate(captured.get().unwrap());
    assert!(Composition::recompose(c).unwrap());
    assert_eq!(*read.borrow(), [Ok(20), Ok(10), Err(NO_SLOT), Err(NO_SLOT)]);
    Composition::dispose(c).unwrap();
    Composition::dispose(d).unwrap();
}

/// compose: "A restart function that starts another key".
#[test]
fn a_restart_function_that_starts_another_key() {
    let tracked = slot_type::<Tracked>("Tracked");
    let captured: Rc<Cell<Option<Handle>>> = Rc::default();
    let cap = captured.clone();
    let c = Composition::start(
        tree(),
        content(move |k| {
            k.start_group(GroupKind::Replaceable, 1, 0).unwrap();
            k.end_group().unwrap();
            k.start_group(GroupKind::Restart, 2, 0).unwrap();
            set(k, tracked, Tracked("old group's slot"));
            cap.set(Some(k.current_scope().unwrap()));
            k.end_restart_group(content(|k| {
                // The restart starts another key in the old group's place.
                k.start_group(GroupKind::Restart, 3, 0).unwrap();
                k.end_restart_group(content(|_| {})).unwrap();
            }))
            .unwrap();
            k.start_group(GroupKind::Replaceable, 4, 0).unwrap();
            k.end_group().unwrap();
        }),
    )
    .unwrap();
    invalidate(captured.get().unwrap());
    assert!(Composition::recompose(c).unwrap());
    assert_eq!(drops(), ["drop old group's slot"]);
    assert_eq!(
        Composition::dump(c).unwrap(),
        "root\n  replaceable 1\n  restart 3\n  replaceable 4\nnodes\n"
    );
    Composition::dispose(c).unwrap();
}

thread_local! {
    static CODE_RUNS: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
}

unsafe extern "C" fn first_code(_: *mut u8, composer: u64) {
    CODE_RUNS.with(|runs| runs.borrow_mut().push("first"));
    let k = Composer(composer);
    k.start_group(GroupKind::Restart, 5, 0).unwrap();
    k.end_restart_group(inline_function(RESTART.with(|r| r.get().unwrap()), 0))
        .unwrap();
}

unsafe extern "C" fn other_code(_: *mut u8, composer: u64) {
    CODE_RUNS.with(|runs| runs.borrow_mut().push("other"));
    let k = Composer(composer);
    k.start_group(GroupKind::Restart, 5, 0).unwrap();
    k.end_restart_group(inline_function(RESTART.with(|r| r.get().unwrap()), 0))
        .unwrap();
}

thread_local! {
    static RESTART: Cell<Option<&'static prism_runtime::Descriptor>> = const { Cell::new(None) };
}

/// compose: "A restart function reaches its closure's current code".
#[test]
fn a_restart_function_reaches_its_closures_current_code() {
    let restart = descriptor(first_code);
    RESTART.with(|r| r.set(Some(restart)));
    let scope_handle: Rc<Cell<Option<Handle>>> = Rc::default();
    let s = scope_handle.clone();
    let c = Composition::start(
        tree(),
        content(move |k| {
            unsafe { first_code(std::ptr::null_mut(), k.0) };
            let _ = &s;
        }),
    )
    .unwrap();
    let scope_handle = with_state(|s| {
        let root = s.compositions.get(c).unwrap().root;
        s.groups.get(root).unwrap().children[0]
    });
    // Repoint the restart function's slot between two passes.
    unsafe {
        (*restart.slot).store(other_code as *const () as usize, Ordering::Release);
    }
    invalidate(scope_handle);
    assert!(Composition::recompose(c).unwrap());
    assert_eq!(
        CODE_RUNS.with(|runs| runs.borrow().clone()),
        ["first", "other"]
    );
    // A reload naming the scope's key invalidates it (D4 of `compose-hot-reload`).
    let count = crate::reload::invalidate_reloaded(
        &std::collections::HashSet::from([5]),
        &std::collections::HashSet::new(),
    );
    assert_eq!(count, 1);
    assert!(Composition::recompose(c).unwrap());
    assert_eq!(CODE_RUNS.with(|runs| runs.borrow().len()), 3);
    Composition::dispose(c).unwrap();
}

#[test]
fn a_scope_removed_by_its_parent_is_not_recomposed() {
    let log = log();
    let show = Rc::new(Cell::new(true));
    let child_scope: Rc<Cell<Option<Handle>>> = Rc::default();
    let (l, sh, cs) = (log.clone(), show.clone(), child_scope.clone());
    let c = Composition::start(
        tree(),
        content(move |k| {
            push(&l, "screen");
            if sh.get() {
                let (l, cs) = (l.clone(), cs.clone());
                scope(
                    k,
                    2,
                    Rc::new(move |k: Composer| {
                        push(&l, "child");
                        cs.set(Some(k.current_scope().unwrap()));
                    }),
                );
            }
        }),
    )
    .unwrap();
    invalidate(child_scope.get().unwrap());
    show.set(false);
    invalidate(Composition::root_scope(c).unwrap());
    assert!(Composition::recompose(c).unwrap());
    assert_eq!(*log.borrow(), ["screen", "child", "screen"]);
    assert!(!Composition::recompose(c).unwrap());
    // Invalidating the removed scope does nothing.
    invalidate(child_scope.get().unwrap());
    assert!(!Composition::recompose(c).unwrap());
    Composition::dispose(c).unwrap();
}

#[test]
fn skipping_and_skipping_to_the_group_end() {
    let log = log();
    let l = log.clone();
    let c = Composition::start(
        tree(),
        content(move |k| {
            k.start_group(GroupKind::Restart, 1, 0).unwrap();
            let skipping = k.skipping().unwrap();
            push(&l, format!("skipping {}", skipping));
            if skipping {
                k.skip_to_group_end().unwrap();
            } else {
                k.set_word(0, 1).unwrap();
                k.start_group(GroupKind::Replaceable, 2, 0).unwrap();
                k.end_group().unwrap();
            }
            k.end_restart_group(content(|_| {})).unwrap();
        }),
    )
    .unwrap();
    invalidate(Composition::root_scope(c).unwrap());
    Composition::recompose(c).unwrap();
    assert_eq!(*log.borrow(), ["skipping false", "skipping true"]);
    // The skipped group kept its slot and its child.
    assert_eq!(
        Composition::dump(c).unwrap(),
        "root\n  restart 1\n    word 0 = 1\n    replaceable 2\nnodes\n"
    );
    Composition::dispose(c).unwrap();
}

#[test]
fn invalidations_during_a_pass_go_to_the_next_pass() {
    let log = log();
    let runs = Rc::new(Cell::new(0));
    let (l, r) = (log.clone(), runs.clone());
    let c = Composition::start(
        tree(),
        content(move |k| {
            r.set(r.get() + 1);
            push(&l, "root");
            if r.get() == 1 {
                invalidate(k.current_scope().unwrap());
            }
        }),
    )
    .unwrap();
    assert_eq!(*log.borrow(), ["root"]);
    assert!(Composition::dump(c).unwrap().starts_with("root invalid\n"));
    assert!(Composition::recompose(c).unwrap());
    assert!(!Composition::recompose(c).unwrap());
    assert_eq!(log.borrow().len(), 2);
    Composition::dispose(c).unwrap();
}

/// An applier that records what it is asked to do, over a node tree.
struct Recording {
    tree: NodeTree,
    events: Log,
}

impl Applier for Recording {
    fn root(&self) -> NodeHandle {
        self.tree.root()
    }
    fn create(&mut self, kind: i64) -> NodeHandle {
        let node = self.tree.create(kind);
        push(&self.events, format!("create #{}", self.tree.number(node)));
        node
    }
    fn place(&mut self, parent: NodeHandle, previous: Option<NodeHandle>, node: NodeHandle) {
        let before = self.tree.children(parent);
        self.tree.place(parent, previous, node);
        if self.tree.children(parent) != before {
            push(&self.events, format!("place #{}", self.tree.number(node)));
        }
    }
    fn remove(&mut self, parent: NodeHandle, node: NodeHandle) {
        push(&self.events, format!("remove #{}", self.tree.number(node)));
        self.tree.remove(parent, node);
    }
    fn release(&mut self, node: NodeHandle) {
        push(&self.events, format!("release #{}", self.tree.number(node)));
        self.tree.release(node);
    }
    fn number(&self, node: NodeHandle) -> u64 {
        self.tree.number(node)
    }
    fn dump(&self, out: &mut String) {
        self.tree.dump(out)
    }
}

fn recording(events: &Log) -> Box<dyn Applier> {
    Box::new(Recording {
        tree: NodeTree::new(),
        events: events.clone(),
    })
}

/// compose: "Removed node groups remove their nodes after the pass".
#[test]
fn removed_node_groups_remove_their_nodes_after_the_pass() {
    let events = log();
    let show = Rc::new(Cell::new(true));
    let (e, s) = (events.clone(), show.clone());
    let c = Composition::start(
        recording(&events),
        content(move |k| {
            k.start_node_group(1, 1).unwrap();
            k.end_group().unwrap();
            if s.get() {
                k.start_node_group(2, 2).unwrap();
                k.start_node_group(3, 3).unwrap();
                k.end_group().unwrap();
                k.end_group().unwrap();
            }
            k.start_node_group(4, 4).unwrap();
            k.end_group().unwrap();
            push(&e, "last body returned");
        }),
    )
    .unwrap();
    events.borrow_mut().clear();
    show.set(false);
    invalidate(Composition::root_scope(c).unwrap());
    Composition::recompose(c).unwrap();
    assert_eq!(
        *events.borrow(),
        [
            "last body returned",
            "release #3",
            "remove #2",
            "release #2"
        ],
        "no other node is placed again"
    );
    Composition::dispose(c).unwrap();
}

#[test]
fn keyed_reorders_place_each_moved_node_once() {
    let events = log();
    let order = Rc::new(RefCell::new(vec![1i64, 2, 3, 4]));
    let o = order.clone();
    let c = Composition::start(
        recording(&events),
        content(move |k| {
            for id in o.borrow().iter() {
                k.start_group(GroupKind::Movable, 1, *id).unwrap();
                k.start_group(GroupKind::Restart, 2, 0).unwrap();
                if k.skipping().unwrap() {
                    k.skip_to_group_end().unwrap();
                } else {
                    k.start_node_group(3, *id).unwrap();
                    k.end_group().unwrap();
                }
                k.end_restart_group(content(|_| {})).unwrap();
                k.end_group().unwrap();
            }
        }),
    )
    .unwrap();
    events.borrow_mut().clear();
    *order.borrow_mut() = vec![4, 1, 2, 3];
    invalidate(Composition::root_scope(c).unwrap());
    Composition::recompose(c).unwrap();
    // The groups matched behind #4 are placed again after it, each once.
    assert_eq!(*events.borrow(), ["place #1", "place #2", "place #3"]);
    let dump = Composition::dump(c).unwrap();
    assert!(
        dump.ends_with("nodes\n  #4 kind 4\n  #1 kind 1\n  #2 kind 2\n  #3 kind 3\n"),
        "{}",
        dump
    );
    *order.borrow_mut() = vec![3, 1, 4, 2];
    invalidate(Composition::root_scope(c).unwrap());
    Composition::recompose(c).unwrap();
    let dump = Composition::dump(c).unwrap();
    assert!(
        dump.ends_with("nodes\n  #3 kind 3\n  #1 kind 1\n  #4 kind 4\n  #2 kind 2\n"),
        "{}",
        dump
    );
    Composition::dispose(c).unwrap();
}

#[test]
fn a_kind_change_creates_a_new_node() {
    let events = log();
    let kind = Rc::new(Cell::new(1));
    let nodes: Rc<RefCell<Vec<NodeHandle>>> = Rc::default();
    let (kd, n) = (kind.clone(), nodes.clone());
    let c = Composition::start(
        recording(&events),
        content(move |k| {
            n.borrow_mut()
                .push(k.start_node_group(1, kd.get()).unwrap());
            k.start_node_group(2, 9).unwrap();
            k.end_group().unwrap();
            k.end_group().unwrap();
        }),
    )
    .unwrap();
    kind.set(2);
    invalidate(Composition::root_scope(c).unwrap());
    Composition::recompose(c).unwrap();
    assert_ne!(nodes.borrow()[0], nodes.borrow()[1]);
    assert!(events.borrow().contains(&"release #1".to_string()));
    let dump = Composition::dump(c).unwrap();
    assert!(
        dump.ends_with("nodes\n  #3 kind 2\n    #2 kind 9\n"),
        "{}",
        dump
    );
    Composition::dispose(c).unwrap();
}

#[test]
fn a_restarted_scope_places_its_node_after_the_previous_scopes_last_node() {
    let show = Rc::new(Cell::new(false));
    let second_scope: Rc<Cell<Option<Handle>>> = Rc::default();
    let (s, sc) = (show.clone(), second_scope.clone());
    let c = Composition::start(
        tree(),
        content(move |k| {
            k.start_node_group(1, 1).unwrap();
            k.start_group(GroupKind::Replaceable, 2, 0).unwrap();
            k.start_node_group(3, 2).unwrap();
            k.end_group().unwrap();
            k.end_group().unwrap();
            let (s, sc) = (s.clone(), sc.clone());
            scope(
                k,
                4,
                Rc::new(move |k: Composer| {
                    sc.set(Some(k.current_scope().unwrap()));
                    if s.get() {
                        k.start_node_group(5, 3).unwrap();
                        k.end_group().unwrap();
                    }
                }),
            );
            k.start_node_group(6, 4).unwrap();
            k.end_group().unwrap();
            k.end_group().unwrap();
        }),
    )
    .unwrap();
    show.set(true);
    invalidate(second_scope.get().unwrap());
    Composition::recompose(c).unwrap();
    let dump = Composition::dump(c).unwrap();
    assert!(
        dump.ends_with("nodes\n  #1 kind 1\n    #2 kind 2\n    #4 kind 3\n    #3 kind 4\n"),
        "{}",
        dump
    );
    // A later restart of the same scope keeps the order, and the parent's last node is right.
    invalidate(second_scope.get().unwrap());
    Composition::recompose(c).unwrap();
    assert!(Composition::dump(c)
        .unwrap()
        .ends_with("    #4 kind 3\n    #3 kind 4\n"));
    Composition::dispose(c).unwrap();
}

/// compose: "Dumping a Composition" and "Dumping the Node Tree".
#[test]
fn the_dump_formats() {
    let tracked = slot_type::<Tracked>("Tracked");
    let scope_handle: Rc<Cell<Option<Handle>>> = Rc::default();
    let s = scope_handle.clone();
    let c = Composition::start(
        tree(),
        content(move |k| {
            k.start_group(GroupKind::Restart, 10, 0).unwrap();
            k.set_word(0, 3).unwrap();
            set(k, tracked, Tracked("value"));
            s.set(Some(k.current_scope().unwrap()));
            k.start_group(GroupKind::Movable, 20, 7).unwrap();
            k.start_node_group(30, 5).unwrap();
            k.start_node_group(31, 6).unwrap();
            k.end_group().unwrap();
            k.end_group().unwrap();
            k.end_group().unwrap();
            k.end_restart_group(content(|_| {})).unwrap();
        }),
    )
    .unwrap();
    invalidate(scope_handle.get().unwrap());
    assert_eq!(
        Composition::dump(c).unwrap(),
        "root\n  restart 10 invalid\n    word 0 = 3\n    slot Tracked\n    movable 20 (7)\n      \
         node 30 #1 kind 5\n        node 31 #2 kind 6\nnodes\n  #1 kind 5\n    #2 kind 6\n"
    );
    Composition::dispose(c).unwrap();
}

#[test]
fn a_drop_entry_may_dispose_another_composition() {
    let other = Composition::start(tree(), named_restart("other content", |_| {})).unwrap();
    let disposer = slot_type::<Disposer>("Disposer");
    let c =
        Composition::start(tree(), content(move |k| set(k, disposer, Disposer(other)))).unwrap();
    Composition::dispose(c).unwrap();
    assert_eq!(drops(), ["drop other content"]);
    assert_eq!(Composition::dispose(other), Err(STALE_COMPOSITION));
    assert_eq!(crate::live_compositions_report(), None);
}

#[derive(PartialEq)]
struct Disposer(Handle);

impl Drop for Disposer {
    fn drop(&mut self) {
        Composition::dispose(self.0).unwrap();
    }
}

#[test]
fn undisposed_compositions_are_reported() {
    let c = Composition::start(tree(), content(|_| {})).unwrap();
    assert_eq!(
        crate::live_compositions_report().as_deref(),
        Some("allocation check: compositions still live at exit: 1")
    );
    Composition::dispose(c).unwrap();
    assert_eq!(crate::live_compositions_report(), None);
    let _ = AtomicUsize::new(0);
}

/// An applier over a node tree the test can read.
struct Shared(Rc<RefCell<NodeTree>>);

impl Applier for Shared {
    fn root(&self) -> NodeHandle {
        self.0.borrow().root()
    }
    fn create(&mut self, kind: i64) -> NodeHandle {
        self.0.borrow_mut().create(kind)
    }
    fn place(&mut self, parent: NodeHandle, previous: Option<NodeHandle>, node: NodeHandle) {
        self.0.borrow_mut().place(parent, previous, node)
    }
    fn remove(&mut self, parent: NodeHandle, node: NodeHandle) {
        self.0.borrow_mut().remove(parent, node)
    }
    fn release(&mut self, node: NodeHandle) {
        self.0.borrow_mut().release(node)
    }
    fn number(&self, node: NodeHandle) -> u64 {
        self.0.borrow().number(node)
    }
    fn dump(&self, out: &mut String) {
        self.0.borrow().dump(out)
    }
}

/// Checks that every node's children are the top-level nodes of its group's subtree, in group
/// order, and that the tree holds no other node.
fn check_nodes(composition: Handle, tree: &NodeTree) {
    with_state(|s| {
        fn top_nodes(s: &crate::thread::State, group: Handle, out: &mut Vec<NodeHandle>) {
            for child in &s.groups.get(group).unwrap().children {
                let record = s.groups.get(*child).unwrap();
                match (record.kind, record.node) {
                    (GroupKind::Node, Some(node)) => out.push(node),
                    _ => top_nodes(s, *child, out),
                }
            }
        }
        let root = s.compositions.get(composition).unwrap().root;
        let mut contexts = vec![(root, tree.root())];
        let mut count = 0;
        while let Some((group, node)) = contexts.pop() {
            let mut expected = Vec::new();
            top_nodes(s, group, &mut expected);
            assert_eq!(tree.children(node), expected, "children of a node");
            count += expected.len();
            let mut walk = vec![group];
            while let Some(g) = walk.pop() {
                for child in &s.groups.get(g).unwrap().children {
                    let record = s.groups.get(*child).unwrap();
                    match (record.kind, record.node) {
                        (GroupKind::Node, Some(n)) => contexts.push((*child, n)),
                        _ => walk.push(*child),
                    }
                }
            }
        }
        assert_eq!(count, tree.len(), "no other node is live");
    });
}

struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

#[derive(Clone, Debug)]
struct Item {
    id: i64,
    wrap: bool,
    children: Vec<i64>,
}

fn random_items(random: &mut Random) -> Vec<Item> {
    let mut ids: Vec<i64> = (0..8).collect();
    for i in (1..ids.len()).rev() {
        ids.swap(i, random.below(i as u64 + 1) as usize);
    }
    ids.truncate(random.below(8) as usize + 1);
    ids.into_iter()
        .map(|id| {
            let mut children: Vec<i64> = (0..4).filter(|_| random.below(2) == 0).collect();
            if random.below(2) == 0 {
                children.reverse();
            }
            Item {
                id,
                wrap: random.below(3) == 0,
                children,
            }
        })
        .collect()
}

type Items = Rc<RefCell<Vec<Item>>>;

/// The body of one item's restart scope: its node with keyed child nodes.
fn item_body(k: Composer, id: i64, items: &Items, skip: &Rc<Cell<bool>>) {
    if k.skipping().unwrap() && skip.get() {
        k.skip_to_group_end().unwrap();
        return;
    }
    let children = items
        .borrow()
        .iter()
        .find(|item| item.id == id)
        .map(|item| item.children.clone())
        .unwrap_or_default();
    k.start_node_group(4, id).unwrap();
    for child in children {
        k.start_group(GroupKind::Movable, 5, child).unwrap();
        k.start_node_group(6, 100 + child).unwrap();
        k.end_group().unwrap();
        k.end_group().unwrap();
    }
    k.end_group().unwrap();
}

fn item_scope(k: Composer, id: i64, items: Items, skip: Rc<Cell<bool>>) {
    k.start_group(GroupKind::Restart, 3, 0).unwrap();
    item_body(k, id, &items, &skip);
    k.end_restart_group(content(move |c| {
        item_scope(c, id, items.clone(), skip.clone())
    }))
    .unwrap();
}

/// compose: "Nodes Follow Their Groups", over random keyed edits, skips and restarts.
#[test]
fn nodes_follow_their_groups_over_random_edits() {
    for seed in 1..40u64 {
        let mut random = Random(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let tree = Rc::new(RefCell::new(NodeTree::new()));
        let items: Items = Rc::new(RefCell::new(random_items(&mut random)));
        let skip = Rc::new(Cell::new(true));
        let scopes: Rc<RefCell<Vec<Handle>>> = Rc::default();
        let (i, sk, sc) = (items.clone(), skip.clone(), scopes.clone());
        let c = Composition::start(
            Box::new(Shared(tree.clone())),
            content(move |k| {
                sc.borrow_mut().clear();
                let list = i.borrow().clone();
                for item in list {
                    k.start_group(GroupKind::Movable, 1, item.id).unwrap();
                    if item.wrap {
                        k.start_group(GroupKind::Replaceable, 2, 0).unwrap();
                    }
                    k.start_group(GroupKind::Restart, 3, 0).unwrap();
                    sc.borrow_mut().push(k.current_scope().unwrap());
                    item_body(k, item.id, &i, &sk);
                    let (i, sk, id) = (i.clone(), sk.clone(), item.id);
                    k.end_restart_group(content(move |c| item_scope(c, id, i.clone(), sk.clone())))
                        .unwrap();
                    if item.wrap {
                        k.end_group().unwrap();
                    }
                    k.end_group().unwrap();
                }
            }),
        )
        .unwrap();
        check_nodes(c, &tree.borrow());
        for _ in 0..30 {
            match random.below(3) {
                0 => {
                    // A keyed edit of the list, the items' bodies skipping or not.
                    *items.borrow_mut() = random_items(&mut random);
                    skip.set(random.below(2) == 0);
                    invalidate(Composition::root_scope(c).unwrap());
                }
                _ => {
                    // New children for some items, recomposed through their own scopes.
                    let mut list = items.borrow().clone();
                    for item in list.iter_mut() {
                        if random.below(2) == 0 {
                            let mut children: Vec<i64> =
                                (0..4).filter(|_| random.below(2) == 0).collect();
                            if random.below(2) == 0 {
                                children.reverse();
                            }
                            item.children = children;
                        }
                    }
                    *items.borrow_mut() = list;
                    skip.set(true);
                    let all = scopes.borrow().clone();
                    for scope in all {
                        if random.below(2) == 0 {
                            invalidate(scope);
                        }
                    }
                }
            }
            Composition::recompose(c).unwrap();
            check_nodes(c, &tree.borrow());
        }
        Composition::dispose(c).unwrap();
        assert!(tree.borrow().is_empty(), "every node is released");
    }
}

/// compose: "Composition Misuse Is a Runtime Error": a restart function that reads a slot
/// before it starts its group has no group to read in.
#[test]
fn a_slot_outside_a_group_is_a_misuse() {
    let result: Rc<Cell<Option<Misuse>>> = Rc::default();
    let scope_handle: Rc<Cell<Option<Handle>>> = Rc::default();
    let (r, s) = (result.clone(), scope_handle.clone());
    let c = Composition::start(
        tree(),
        content(move |k| {
            k.start_group(GroupKind::Restart, 1, 0).unwrap();
            s.set(Some(k.current_scope().unwrap()));
            let r = r.clone();
            k.end_restart_group(content(move |k| {
                r.set(k.has_word(0).err());
                k.start_group(GroupKind::Restart, 1, 0).unwrap();
                k.end_restart_group(content(|_| {})).unwrap();
            }))
            .unwrap();
        }),
    )
    .unwrap();
    invalidate(scope_handle.get().unwrap());
    Composition::recompose(c).unwrap();
    assert_eq!(result.get(), Some(SLOT_OUTSIDE_GROUP));
    Composition::dispose(c).unwrap();
}

// A model-checked random test of whole trees: node groups nested in replaceable, movable and
// restart groups, edited at the front, middle and end, moved by keys, wrapped and unwrapped,
// with subtrees that hold scopes leaving while those scopes are invalid. After every pass the
// node tree must be the model's, every node group must hold exactly one live typed value, and
// after disposal every value, restart function and node must be released once.

thread_local! {
    static LIVE_VALUES: Cell<i64> = const { Cell::new(0) };
}

/// A slot value that counts how many are live, so a double release or a leak shows.
#[derive(PartialEq)]
struct Counted;

impl Counted {
    fn new() -> Counted {
        LIVE_VALUES.with(|live| live.set(live.get() + 1));
        Counted
    }
}

impl Drop for Counted {
    fn drop(&mut self) {
        LIVE_VALUES.with(|live| live.set(live.get() - 1));
    }
}

fn live_values() -> i64 {
    LIVE_VALUES.with(|live| live.get())
}

#[derive(Clone, Debug)]
enum Element {
    Node {
        key: i64,
        kind: i64,
        children: Vec<Element>,
    },
    Movable {
        data: i64,
        children: Vec<Element>,
    },
    Replaceable {
        key: i64,
        children: Vec<Element>,
    },
    /// A restart scope whose body is `Model::bodies[id]`.
    Scope(u32),
}

#[derive(Default)]
struct Model {
    /// The body of each scope; 0 is the root content's.
    bodies: std::collections::HashMap<u32, Vec<Element>>,
    next_scope: u32,
    /// The handle of each scope as of the last time its group was composed.
    handles: std::collections::HashMap<u32, Handle>,
    /// The scopes whose bodies ran in the last pass.
    rendered: Vec<u32>,
}

struct Context {
    model: RefCell<Model>,
    counted: &'static crate::SlotType,
}

type Shared2 = Rc<Context>;

/// How deep random elements nest, and how many children each has at most, plus one.
const MAX_DEPTH: u32 = 3;
const MAX_CHILDREN: u64 = 4;

fn random_element(random: &mut Random, model: &mut Model, depth: u32) -> Element {
    let children = |random: &mut Random, model: &mut Model| -> Vec<Element> {
        if depth >= MAX_DEPTH {
            return Vec::new();
        }
        (0..random.below(MAX_CHILDREN))
            .map(|_| random_element(random, model, depth + 1))
            .collect()
    };
    match random.below(if depth >= MAX_DEPTH { 2 } else { 5 }) {
        0 | 1 => Element::Node {
            key: random.below(3) as i64,
            kind: random.below(4) as i64,
            children: children(random, model),
        },
        2 => Element::Movable {
            data: random.below(6) as i64,
            children: children(random, model),
        },
        3 => Element::Replaceable {
            key: random.below(2) as i64,
            children: children(random, model),
        },
        _ => {
            model.next_scope += 1;
            let id = model.next_scope;
            let body = children(random, model);
            model.bodies.insert(id, body);
            Element::Scope(id)
        }
    }
}

/// The children lists of `elements` and every element below them, outside scope bodies.
fn lists(elements: &mut Vec<Element>, out: &mut Vec<*mut Vec<Element>>) {
    out.push(elements as *mut _);
    for element in elements.iter_mut() {
        match element {
            Element::Node { children, .. }
            | Element::Movable { children, .. }
            | Element::Replaceable { children, .. } => lists(children, out),
            Element::Scope(_) => {}
        }
    }
}

/// The scopes reachable from the root.
fn reachable_scopes(model: &Model) -> Vec<u32> {
    fn walk(model: &Model, elements: &[Element], out: &mut Vec<u32>) {
        for element in elements {
            match element {
                Element::Node { children, .. }
                | Element::Movable { children, .. }
                | Element::Replaceable { children, .. } => walk(model, children, out),
                Element::Scope(id) => {
                    out.push(*id);
                    walk(model, &model.bodies[id], out);
                }
            }
        }
    }
    let mut out = vec![0];
    walk(model, &model.bodies[&0], &mut out);
    out
}

/// One random edit of one list of a reachable body; gives the scope whose body changed.
fn edit(random: &mut Random, model: &mut Model) -> u32 {
    let scopes = reachable_scopes(model);
    let scope = scopes[random.below(scopes.len() as u64) as usize];
    let mut body = model.bodies.remove(&scope).unwrap();
    let mut targets = Vec::new();
    lists(&mut body, &mut targets);
    let target = targets[random.below(targets.len() as u64) as usize];
    // SAFETY: the pointers name distinct lists inside `body`, which is not touched otherwise
    // while `list` is used.
    let list = unsafe { &mut *target };
    let length = list.len() as u64;
    let position = match random.below(3) {
        0 => 0,
        1 => length / 2,
        _ => length,
    } as usize;
    match random.below(6) {
        // Insert at the front, the middle or the end.
        0 => {
            let element = random_element(random, model, 1);
            list.insert(position, element);
        }
        // Remove at the front, the middle or the end, with whatever is below.
        1 if length > 0 => {
            list.remove(position.min(list.len() - 1));
        }
        // Move one element elsewhere, as a keyed move.
        2 if length > 1 => {
            let from = random.below(length) as usize;
            let element = list.remove(from);
            let to = random.below(length) as usize;
            list.insert(to, element);
        }
        // Wrap an element in a new group, or unwrap one.
        3 if length > 0 => {
            let at = random.below(length) as usize;
            let element = list.remove(at);
            let replacement = match element {
                Element::Movable { children, .. } | Element::Replaceable { children, .. }
                    if random.below(2) == 0 =>
                {
                    children
                }
                other => vec![Element::Replaceable {
                    key: random.below(2) as i64,
                    children: vec![other],
                }],
            };
            for (offset, element) in replacement.into_iter().enumerate() {
                list.insert(at + offset, element);
            }
        }
        // Change a node's kind.
        4 if length > 0 => {
            if let Element::Node { kind, .. } = &mut list[random.below(length) as usize] {
                *kind = (*kind + 1) % 4;
            }
        }
        // Reverse the list.
        _ => list.reverse(),
    }
    model.bodies.insert(scope, body);
    scope
}

fn render(k: Composer, context: &Shared2, elements: &[Element]) {
    for element in elements {
        match element {
            Element::Node {
                key,
                kind,
                children,
            } => {
                k.start_node_group(*key, *kind).unwrap();
                if unsafe { k.slot(context.counted).unwrap() }.is_null() {
                    set(k, context.counted, Counted::new());
                }
                render(k, context, children);
                k.end_group().unwrap();
            }
            Element::Movable { data, children } => {
                k.start_group(GroupKind::Movable, 1, *data).unwrap();
                render(k, context, children);
                k.end_group().unwrap();
            }
            Element::Replaceable { key, children } => {
                k.start_group(GroupKind::Replaceable, 10 + key, 0).unwrap();
                render(k, context, children);
                k.end_group().unwrap();
            }
            Element::Scope(id) => render_scope(k, context, *id),
        }
    }
}

fn render_scope(k: Composer, context: &Shared2, id: u32) {
    k.start_group(GroupKind::Restart, 1000 + id as i64, 0)
        .unwrap();
    let handle = k.current_scope().unwrap();
    context.model.borrow_mut().handles.insert(id, handle);
    if k.skipping().unwrap() {
        k.skip_to_group_end().unwrap();
    } else {
        context.model.borrow_mut().rendered.push(id);
        let body = context.model.borrow().bodies[&id].clone();
        render(k, context, &body);
    }
    let context = context.clone();
    k.end_restart_group(content(move |c| render_scope(c, &context, id)))
        .unwrap();
}

/// The node tree the model describes: each node's kind and children.
#[derive(Debug, PartialEq)]
struct Expected(i64, Vec<Expected>);

fn expected(model: &Model, elements: &[Element], out: &mut Vec<Expected>) {
    for element in elements {
        match element {
            Element::Node { kind, children, .. } => {
                let mut below = Vec::new();
                expected(model, children, &mut below);
                out.push(Expected(*kind, below));
            }
            Element::Movable { children, .. } | Element::Replaceable { children, .. } => {
                expected(model, children, out)
            }
            Element::Scope(id) => expected(model, &model.bodies[id], out),
        }
    }
}

fn actual(tree: &NodeTree, node: NodeHandle) -> Vec<Expected> {
    tree.children(node)
        .into_iter()
        .map(|child| Expected(tree.kind(child).unwrap(), actual(tree, child)))
        .collect()
}

fn count_nodes(nodes: &[Expected]) -> usize {
    nodes.iter().map(|node| 1 + count_nodes(&node.1)).sum()
}

/// compose: "Nodes Follow Their Groups" and "Removals Are Deferred to the End of the Pass",
/// over random whole-tree edits checked against a model.
#[test]
fn random_tree_edits_match_the_model() {
    for seed in 1..300u64 {
        let mut random = Random(seed.wrapping_mul(0x2545_F491_4F6C_DD1D) | 1);
        let mut model = Model::default();
        let root: Vec<Element> = (0..random.below(MAX_CHILDREN + 1))
            .map(|_| random_element(&mut random, &mut model, 0))
            .collect();
        model.bodies.insert(0, root);
        let context: Shared2 = Rc::new(Context {
            model: RefCell::new(model),
            counted: slot_type::<Counted>("Counted"),
        });
        let tree = Rc::new(RefCell::new(NodeTree::new()));
        let root_context = context.clone();
        let c = Composition::start(
            Box::new(Shared(tree.clone())),
            content(move |k| {
                let body = root_context.model.borrow().bodies[&0].clone();
                render(k, &root_context, &body);
            }),
        )
        .unwrap();
        let check = |step: usize| {
            let model = context.model.borrow();
            let mut want = Vec::new();
            expected(&model, &model.bodies[&0], &mut want);
            let tree = tree.borrow();
            check_nodes(c, &tree);
            let got = actual(&tree, tree.root());
            assert_eq!(got, want, "seed {} step {}", seed, step);
            assert_eq!(
                tree.len(),
                count_nodes(&want),
                "seed {} step {}",
                seed,
                step
            );
            assert_eq!(
                live_values(),
                count_nodes(&want) as i64,
                "one value per node group: seed {} step {}",
                seed,
                step
            );
            // No scope outside the tree ran, and none ran twice.
            let reachable = reachable_scopes(&model);
            let mut rendered = model.rendered.clone();
            assert!(
                rendered.iter().all(|id| reachable.contains(id)),
                "a removed scope was recomposed: seed {} step {}",
                seed,
                step
            );
            rendered.sort_unstable();
            let count = rendered.len();
            rendered.dedup();
            assert_eq!(rendered.len(), count, "a scope ran twice in one pass");
        };
        check(0);
        for step in 1..40 {
            // A few edits, each invalidating the scope whose body it changed, plus scopes
            // invalidated for nothing, which recompose to the same tree.
            let mut invalid = Vec::new();
            for _ in 0..=random.below(3) {
                let scope = edit(&mut random, &mut context.model.borrow_mut());
                invalid.push(scope);
            }
            let reachable = reachable_scopes(&context.model.borrow());
            for scope in reachable {
                if random.below(4) == 0 {
                    invalid.push(scope);
                }
            }
            context.model.borrow_mut().rendered.clear();
            for scope in invalid {
                let handle = if scope == 0 {
                    Some(Composition::root_scope(c).unwrap())
                } else {
                    context.model.borrow().handles.get(&scope).copied()
                };
                if let Some(handle) = handle {
                    invalidate(handle);
                }
            }
            if random.below(10) == 0 {
                // New content, equal to the old, replaces the root's restart function.
                let root_context = context.clone();
                Composition::set_content(
                    c,
                    content(move |k| {
                        let body = root_context.model.borrow().bodies[&0].clone();
                        render(k, &root_context, &body);
                    }),
                )
                .unwrap();
            } else {
                Composition::recompose(c).unwrap();
            }
            check(step);
        }
        Composition::dispose(c).unwrap();
        assert!(tree.borrow().is_empty(), "every node is released");
        assert_eq!(live_values(), 0, "every value is released once");
        assert_eq!(
            Rc::strong_count(&context),
            1,
            "every restart function is released"
        );
    }
}

/// compose: "Handles Are Validated": the state is per thread, so a composition's handle used on
/// another thread names nothing there, and that thread's misuse leaves the first one intact.
#[test]
fn a_handle_from_another_thread_is_stale() {
    let c = Composition::start(tree(), content(|_| {})).unwrap();
    let scope = Composition::root_scope(c).unwrap();
    std::thread::spawn(move || {
        assert_eq!(Composition::recompose(c), Err(STALE_COMPOSITION));
        assert_eq!(Composition::dispose(c), Err(STALE_COMPOSITION));
        assert_eq!(Composition::dump(c), Err(STALE_COMPOSITION));
        invalidate(scope);
        assert_eq!(crate::live_compositions_report(), None);
    })
    .join()
    .unwrap();
    assert!(
        !Composition::recompose(c).unwrap(),
        "the other thread invalidated nothing here"
    );
    Composition::dispose(c).unwrap();
}

/// `ui-toolkit` task 3.4: the composition of the thread's active pass, for `prism-ui`.
#[test]
fn the_active_composition_is_the_one_whose_pass_runs() {
    assert_eq!(
        crate::active_composition(),
        None,
        "no pass outside composition"
    );
    let seen: Rc<Cell<Option<Handle>>> = Rc::new(Cell::new(None));
    let inside = Rc::clone(&seen);
    let composition = Composition::start(
        tree(),
        content(move |_| inside.set(crate::active_composition())),
    )
    .unwrap();
    assert_eq!(seen.get(), Some(composition));
    assert_eq!(crate::active_composition(), None, "none after the pass");
    Composition::dispose(composition).unwrap();
}

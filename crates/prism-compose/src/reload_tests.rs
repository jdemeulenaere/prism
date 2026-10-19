//! Rust tests of what a hot reload recomposes (D3 to D8 of `compose-hot-reload`), over
//! compositions built from Rust content: which scope a reloaded body's group invalidates, retired
//! scopes, and reloads applied during a pass.

use crate::composer::Composer;
use crate::composition::Composition;
use crate::group::GroupKind;
use crate::reload::{invalidate_reloaded, reload_applied};
use crate::test_support::*;
use crate::thread::with_state;
use crate::{Applier, NodeTree};
use prism_runtime::reload::AppliedReload;
use prism_runtime::Descriptor;
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;

type Log = Rc<RefCell<Vec<String>>>;

fn log() -> Log {
    Rc::new(RefCell::new(Vec::new()))
}

fn push(log: &Log, line: &str) {
    log.borrow_mut().push(line.to_string());
}

fn tree() -> Box<dyn Applier> {
    Box::new(NodeTree::new())
}

fn keys(keys: &[u64]) -> HashSet<u64> {
    keys.iter().copied().collect()
}

/// A restart scope keyed `key` whose body is `body`, restarted by a `content` function value.
fn scope(k: Composer, key: i64, body: Rc<dyn Fn(Composer)>) {
    k.start_group(GroupKind::Restart, key, 0).unwrap();
    body(k);
    let again = body.clone();
    k.end_restart_group(content(move |c| scope(c, key, again.clone())))
        .unwrap();
}

/// A replaceable group keyed `key` around `body`.
fn group(k: Composer, key: i64, body: impl FnOnce(Composer)) {
    k.start_group(GroupKind::Replaceable, key, 0).unwrap();
    body(k);
    k.end_group().unwrap();
}

/// compose-hot-reload, D4: a replaceable body, a non-skippable composable's, invalidates the
/// restart scope above it, and that scope alone recomposes.
#[test]
fn a_replaceable_body_invalidates_its_restart_ancestor() {
    let log = log();
    let (root_log, scope_log) = (log.clone(), log.clone());
    let body: Rc<dyn Fn(Composer)> = Rc::new(move |k| {
        push(&scope_log, "screen");
        group(k, 100, |_| {});
    });
    let c = Composition::start(
        tree(),
        content(move |k| {
            push(&root_log, "root");
            scope(k, 1, body.clone());
        }),
    )
    .unwrap();
    assert_eq!(invalidate_reloaded(&keys(&[100]), &HashSet::new()), 1);
    assert!(Composition::recompose(c).unwrap());
    assert_eq!(*log.borrow(), ["root", "screen", "screen"]);
    Composition::dispose(c).unwrap();
}

/// compose-hot-reload, D4: a content lambda's group, inside a composable of another file,
/// invalidates the restart scope of the composable that wrote the lambda.
#[test]
fn a_content_lambdas_group_invalidates_its_author() {
    let log = log();
    let author_log = log.clone();
    let author: Rc<dyn Fn(Composer)> = Rc::new(move |k| {
        push(&author_log, "author");
        // `Column`, of another file, runs the author's content lambda in its own group.
        group(k, 50, |k| group(k, 200, |_| {}));
    });
    let c = Composition::start(tree(), content(move |k| scope(k, 1, author.clone()))).unwrap();
    assert_eq!(invalidate_reloaded(&keys(&[200]), &HashSet::new()), 1);
    assert!(Composition::recompose(c).unwrap());
    assert_eq!(*log.borrow(), ["author", "author"]);
    // A key no live group has invalidates nothing.
    assert_eq!(invalidate_reloaded(&keys(&[999]), &HashSet::new()), 0);
    assert!(!Composition::recompose(c).unwrap());
    Composition::dispose(c).unwrap();
}

/// compose-hot-reload, D4: a group with no restart scope above it invalidates the root.
#[test]
fn a_group_without_a_restart_ancestor_invalidates_the_root() {
    let log = log();
    let root_log = log.clone();
    let c = Composition::start(
        tree(),
        content(move |k| {
            push(&root_log, "root");
            group(k, 300, |_| {});
        }),
    )
    .unwrap();
    assert_eq!(invalidate_reloaded(&keys(&[300]), &HashSet::new()), 1);
    assert!(Composition::recompose(c).unwrap());
    assert_eq!(*log.borrow(), ["root", "root"]);
    Composition::dispose(c).unwrap();
}

thread_local! {
    static CHILD: Cell<Option<&'static Descriptor>> = const { Cell::new(None) };
    static CHILD_LOG: RefCell<Option<Log>> = const { RefCell::new(None) };
    static RECORD: Cell<Option<&'static crate::SlotType>> = const { Cell::new(None) };
    static DROPS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// A value whose drop is recorded.
#[derive(Debug, PartialEq, Clone)]
struct Tracked(&'static str);

impl Drop for Tracked {
    fn drop(&mut self) {
        DROPS.with(|drops| drops.borrow_mut().push(format!("drop {}", self.0)));
    }
}

/// Remembers a `Tracked` of `name` in a typed slot at the cursor, when the slot holds none.
fn remember_tracked(k: Composer, name: &'static str) {
    let record = RECORD.with(Cell::get).unwrap();
    // SAFETY: the record is of `Tracked`, and the value moved into the slot is given up.
    unsafe {
        if k.slot(record).unwrap().is_null() {
            let mut value = std::mem::ManuallyDrop::new(Tracked(name));
            k.set_slot(record, (&mut *value as *mut Tracked).cast())
                .unwrap();
        }
    }
}

/// The child scope: a restart group keyed 2 that remembers `kept`, with a grandchild
/// replaceable group keyed 3 that remembers `grandchild`, ended with a restart function of the
/// child's own descriptor, whose slot is distinct from every `content` value's.
fn child(k: Composer) {
    k.start_group(GroupKind::Restart, 2, 0).unwrap();
    CHILD_LOG.with(|log| push(log.borrow().as_ref().unwrap(), "child"));
    remember_tracked(k, "kept");
    group(k, 3, |k| remember_tracked(k, "grandchild"));
    k.end_restart_group(inline_function(CHILD.with(Cell::get).unwrap(), 0))
        .unwrap();
}

unsafe extern "C" fn child_code(_: *mut u8, composer: u64) {
    child(Composer(composer));
}

/// compose-hot-reload, D8: a scope whose restart function a reload retired is recomposed by
/// its parent; starting it again forgets its own slots, released at the end of the pass, while
/// its child group keeps its slots.
#[test]
fn a_retired_scope_recomposes_from_its_parent_and_resets_its_slots() {
    let descriptor = descriptor(child_code);
    CHILD.with(|cell| cell.set(Some(descriptor)));
    RECORD.with(|cell| cell.set(Some(slot_type::<Tracked>("Tracked"))));
    DROPS.with(|drops| drops.borrow_mut().clear());
    let log = log();
    CHILD_LOG.with(|cell| *cell.borrow_mut() = Some(log.clone()));
    let parent_log = log.clone();
    let parent: Rc<dyn Fn(Composer)> = Rc::new(move |k| {
        push(&parent_log, "parent");
        child(k);
    });
    let c = Composition::start(tree(), content(move |k| scope(k, 1, parent.clone()))).unwrap();
    let retired: HashSet<usize> = [descriptor.slot as *const _ as usize].into();
    // The child's own key is affected too, as a reloaded composable's always is; the
    // invalidation goes to the parent.
    assert_eq!(invalidate_reloaded(&keys(&[2]), &retired), 1);
    let child_group = with_state(|state| {
        let root = state.compositions.iter().next().unwrap().1.root;
        let parent = state.groups.get(root).unwrap().children[0];
        let child = state.groups.get(parent).unwrap().children[0];
        let record = state.groups.get(child).unwrap();
        assert!(record.reset_slots);
        assert!(
            !record.invalid,
            "the retired scope itself is not invalidated"
        );
        assert!(state.groups.get(parent).unwrap().invalid);
        child
    });
    assert!(Composition::recompose(c).unwrap());
    assert_eq!(*log.borrow(), ["parent", "child", "parent", "child"]);
    // The old `kept` was forgotten and released at the end of the pass, and remembered again;
    // the grandchild kept its value.
    assert_eq!(DROPS.with(|drops| drops.borrow().clone()), ["drop kept"]);
    with_state(|state| {
        let record = state.groups.get(child_group).unwrap();
        assert!(!record.reset_slots);
        assert_eq!(record.slots.len(), 1);
    });
    Composition::dispose(c).unwrap();
    CHILD_LOG.with(|cell| *cell.borrow_mut() = None);
}

/// compose-hot-reload, D3: a reload applied during a pass, as at a function entry inside a
/// plain composition's body, invalidates nothing during the pass, and its scope recomposes in
/// the next pass.
#[test]
fn a_reload_during_a_pass_is_handled_when_the_pass_ends() {
    let log = log();
    let body_log = log.clone();
    let reloaded = Rc::new(Cell::new(false));
    let once = reloaded.clone();
    let body: Rc<dyn Fn(Composer)> = Rc::new(move |k| {
        push(&body_log, "show");
        group(k, 400, |_| {});
        if !once.replace(true) {
            reload_applied(&AppliedReload {
                composable_bodies: vec![400],
                ..AppliedReload::default()
            });
            let scope = k.current_scope().unwrap();
            let invalid = with_state(|state| state.groups.get(scope).unwrap().invalid);
            assert!(!invalid, "nothing is invalidated during the pass");
        }
    });
    let c = Composition::start(tree(), content(move |k| scope(k, 1, body.clone()))).unwrap();
    assert_eq!(*log.borrow(), ["show"]);
    assert!(
        Composition::recompose(c).unwrap(),
        "the next pass recomposes"
    );
    assert_eq!(*log.borrow(), ["show", "show"]);
    assert!(!Composition::recompose(c).unwrap());
    Composition::dispose(c).unwrap();
}

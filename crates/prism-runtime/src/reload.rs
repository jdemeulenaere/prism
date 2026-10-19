//! Handing a reload to the program thread (D3 and D4 of `hot-reload`).
//!
//! The watch session compiles and finalizes a reload on its own thread while the program keeps
//! running, and publishes the result here as a [`PendingReload`]: a closure that writes the
//! reload's slots and commits its table updates. The program thread runs it at its next safe
//! point, so all of a reload's slot changes take effect between two operations of the program,
//! and only the thread that loads slots ever stores them.
//!
//! Under the default policy, [`ReloadPolicy::AtNextCall`], [`request`] stores the interrupt
//! sentinel into the stack limit, so the program applies the reload at its next function entry
//! (`crate::stack`). A frame loop can instead defer reloads to reload points it calls itself
//! between frames ([`defer_to_reload_points`], [`reload_point`]), which `compose-hot-reload` uses
//! to apply reloads between frames.
//!
//! A reload is applied while the pending slot's lock is held, so [`withdraw`] returning nothing
//! means that the last request has been applied completely, or is being applied and will be
//! before the lock is released.
//!
//! `compose-hot-reload` adds an applied-reload hook ([`set_applied_hook`], its D3), through which
//! the composition runtime invalidates the scopes a reload affects, and a scripted-edit hook
//! ([`set_edit_hook`], its D12), through which an event script's `edit` commands reach the watch
//! session.

use crate::output::flush;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

/// What applying a reload did: the units it defined, what compositions must recompose, and the
/// text the program thread prints to stderr once it is applied (`reload: applied ...` and its
/// notes).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AppliedReload {
    /// The names of the file units and instance units whose code the reload defined.
    pub units: Vec<String>,
    /// The body keys of the composable bodies the reload affects (D4 and D5 of
    /// `compose-hot-reload`): those whose code is in a unit it defines, and those of the files
    /// importing a reloaded file.
    pub composable_bodies: Vec<u64>,
    /// The addresses of the slots the reload's splits retired: the slot each split symbol was
    /// mapped to before it (D8 of `compose-hot-reload`).
    pub retired_slots: Vec<usize>,
    /// Printed to stderr after the program's output is flushed.
    pub message: String,
}

/// What the program thread calls once a reload's slots are written, before its message is
/// printed: `prism_compose::reload::reload_applied` (D3 of `compose-hot-reload`).
pub type AppliedHook = fn(&AppliedReload);

/// One edit of an event script's `edit` command: `target` reads as `content` (D12 of
/// `compose-hot-reload`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptedEdit {
    pub target: PathBuf,
    pub content: String,
}

/// What hands a group of scripted edits to the watch session and waits until the session has
/// requested their reload, or found that they do not compile.
pub type EditHook = Arc<dyn Fn(Vec<ScriptedEdit>) + Send + Sync>;

/// A reload compiled and finalized, waiting for the program thread.
pub struct PendingReload {
    apply: Box<dyn FnOnce() -> AppliedReload + Send>,
}

impl PendingReload {
    /// A reload that `apply` applies on the program thread.
    pub fn new(apply: impl FnOnce() -> AppliedReload + Send + 'static) -> Self {
        Self {
            apply: Box::new(apply),
        }
    }
}

impl std::fmt::Debug for PendingReload {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("PendingReload")
    }
}

/// When the program thread applies a pending reload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReloadPolicy {
    /// At its next entry to a function that calls another, or to glue (the default).
    AtNextCall,
    /// Only when it calls [`reload_point`].
    AtReloadPoints,
}

struct Policy {
    policy: ReloadPolicy,
    /// Called after a request under [`ReloadPolicy::AtReloadPoints`], to wake the loop.
    wake: Option<fn()>,
}

static PENDING: Mutex<Option<PendingReload>> = Mutex::new(None);
/// Whether a reload is pending, read by [`reload_point`] without the pending reload's lock, so
/// that a frame-loop iteration without a reload costs one load (D2 of `compose-hot-reload`).
static REQUESTED: AtomicBool = AtomicBool::new(false);
static APPLIED_HOOK: Mutex<Option<AppliedHook>> = Mutex::new(None);
static EDIT_HOOK: Mutex<Option<EditHook>> = Mutex::new(None);
static POLICY: Mutex<Policy> = Mutex::new(Policy {
    policy: ReloadPolicy::AtNextCall,
    wake: None,
});
/// Whether a reload is pending under [`ReloadPolicy::AtNextCall`], read when a run sets its stack
/// limit so that an interrupt raised before is raised again.
static INTERRUPT_PENDING: AtomicBool = AtomicBool::new(false);

fn pending() -> MutexGuard<'static, Option<PendingReload>> {
    PENDING
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn policy() -> MutexGuard<'static, Policy> {
    POLICY
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Publishes `reload` for the program thread, replacing any reload that is still pending, which
/// is returned. Under [`ReloadPolicy::AtNextCall`], interrupts the program's next function entry;
/// under [`ReloadPolicy::AtReloadPoints`], calls the loop's wake-up function instead.
pub fn request(reload: PendingReload) -> Option<PendingReload> {
    let replaced = pending().replace(reload);
    REQUESTED.store(true, Ordering::SeqCst);
    let (policy, wake) = {
        let policy = policy();
        (policy.policy, policy.wake)
    };
    match policy {
        ReloadPolicy::AtNextCall => {
            INTERRUPT_PENDING.store(true, Ordering::SeqCst);
            crate::stack::interrupt();
        }
        ReloadPolicy::AtReloadPoints => {
            if let Some(wake) = wake {
                wake();
            }
        }
    }
    replaced
}

/// Takes back the pending reload if the program thread has not applied it (D12 of
/// `hot-reload`): a newer change supersedes it. Returns nothing when there is none, or when the
/// last request has been applied; it waits for a reload being applied to finish.
pub fn withdraw() -> Option<PendingReload> {
    let taken = pending().take();
    INTERRUPT_PENDING.store(false, Ordering::SeqCst);
    REQUESTED.store(false, Ordering::SeqCst);
    taken
}

/// Whether a reload is pending, applied or withdrawn yet.
pub fn is_requested() -> bool {
    REQUESTED.load(Ordering::SeqCst)
}

/// Sets the hook the program thread calls after applying each reload, or removes it.
pub fn set_applied_hook(hook: Option<AppliedHook>) {
    *APPLIED_HOOK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = hook;
}

/// Sets the hook through which an event script's `edit` commands reach the watch session, or
/// removes it.
pub fn set_edit_hook(hook: Option<EditHook>) {
    *EDIT_HOOK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = hook;
}

/// Hands `edits`, one group of an event script's `edit` commands, to the watch session and
/// returns once it has requested their reload or reported their errors. Called on the program
/// thread; does nothing without a session, since the script parser rejects `edit` outside
/// `--watch` (D12 of `compose-hot-reload`).
pub fn request_scripted_edit(edits: Vec<ScriptedEdit>) {
    let hook = EDIT_HOOK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    if let Some(hook) = hook {
        hook(edits);
    }
}

/// Whether a reload is pending under [`ReloadPolicy::AtNextCall`].
pub(crate) fn has_pending_interrupt() -> bool {
    INTERRUPT_PENDING.load(Ordering::SeqCst)
}

/// Applies the pending reload, if any, on the calling thread, which must be the program's:
/// flushes the program's output, runs the reload, prints its message to stderr, and executes an
/// instruction synchronization barrier, so that this thread fetches the new code afresh (the
/// session's finalization already invalidated the instruction cache for it).
pub fn apply_pending() -> Option<AppliedReload> {
    let mut pending = pending();
    let reload = pending.take()?;
    INTERRUPT_PENDING.store(false, Ordering::SeqCst);
    REQUESTED.store(false, Ordering::SeqCst);
    flush();
    let applied = (reload.apply)();
    instruction_barrier();
    let hook = *APPLIED_HOOK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(hook) = hook {
        hook(&applied);
    }
    {
        let mut stderr = std::io::stderr().lock();
        let _ = stderr.write_all(applied.message.as_bytes());
        let _ = stderr.flush();
    }
    drop(pending);
    Some(applied)
}

/// Executes `isb` on aarch64, which makes this thread's later instruction fetches see code made
/// visible since; elsewhere instruction fetches are coherent and nothing is needed.
#[inline]
fn instruction_barrier() {
    #[cfg(target_arch = "aarch64")]
    // SAFETY: `isb` has no operands and only waits for the pipeline to drain.
    unsafe {
        std::arch::asm!("isb", options(nostack, preserves_flags))
    };
}

/// Applies the pending reload now, at a runtime call that is a safe point (an edit script's
/// `after line` trigger, D16 of `hot-reload`), restoring the stack limit first if a request
/// interrupted it. While reloads are deferred to reload points, the reload stays pending for
/// the loop's next reload point instead, so that a line printed during a pass never applies a
/// reload in it (D12 of `compose-hot-reload`).
pub fn apply_now() -> Option<AppliedReload> {
    crate::stack::take_interrupt();
    if reload_policy() == ReloadPolicy::AtReloadPoints {
        return None;
    }
    apply_pending()
}

/// Defers reloads to the reload points the calling frame loop runs (D4 of `hot-reload`): from
/// now on a request does not interrupt the program, but calls `wake`.
pub fn defer_to_reload_points(wake: fn()) {
    let mut policy = policy();
    policy.policy = ReloadPolicy::AtReloadPoints;
    policy.wake = Some(wake);
}

/// Applies reloads at the program's next call again, the default. A reload still pending is
/// applied at that call, as if it had been requested then (`compose-hot-reload`, "Leaving
/// Reload Points Applies a Pending Reload at the Next Call").
pub fn apply_at_next_call() {
    {
        let mut policy = policy();
        policy.policy = ReloadPolicy::AtNextCall;
        policy.wake = None;
    }
    // Taken after the policy changed: a request between the two interrupts by itself.
    if pending().is_some() {
        INTERRUPT_PENDING.store(true, Ordering::SeqCst);
        crate::stack::interrupt();
    }
}

/// The current policy.
pub fn reload_policy() -> ReloadPolicy {
    policy().policy
}

/// A frame loop's reload point: applies the pending reload, if any, and reports it. Without
/// one it costs a load of the requested flag, not the pending reload's lock.
pub fn reload_point() -> Option<AppliedReload> {
    if !REQUESTED.load(Ordering::SeqCst) {
        return None;
    }
    apply_pending()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    fn applying(units: &[&str]) -> PendingReload {
        let units: Vec<String> = units.iter().map(|unit| unit.to_string()).collect();
        PendingReload::new(move || AppliedReload {
            units,
            ..AppliedReload::default()
        })
    }

    #[test]
    fn a_request_replaces_an_earlier_one() {
        let _lock = crate::test_lock();
        crate::stack::set_stack_limit(0x1000_0000, 8 * 1024 * 1024);
        assert!(request(applying(&["a"])).is_none());
        let replaced = request(applying(&["b"]));
        assert!(replaced.is_some());
        assert!(crate::stack::take_interrupt(), "the request interrupts");
        let applied = apply_pending().expect("a reload is pending");
        assert_eq!(applied.units, ["b"]);
        assert!(apply_pending().is_none());
        assert!(withdraw().is_none());
        crate::stack::clear_stack_limit();
    }

    #[test]
    fn a_withdrawn_reload_is_not_applied() {
        let _lock = crate::test_lock();
        request(applying(&["a"]));
        assert!(withdraw().is_some());
        assert!(apply_pending().is_none());
        crate::stack::clear_stack_limit();
    }

    static WOKEN: AtomicUsize = AtomicUsize::new(0);

    fn wake() {
        WOKEN.fetch_add(1, Ordering::SeqCst);
    }

    #[test]
    fn reload_points_apply_a_deferred_request_without_an_interrupt() {
        let _lock = crate::test_lock();
        crate::stack::clear_stack_limit();
        defer_to_reload_points(wake);
        assert_eq!(reload_policy(), ReloadPolicy::AtReloadPoints);
        let woken = WOKEN.load(Ordering::SeqCst);
        request(applying(&["lib/math"]));
        assert_eq!(WOKEN.load(Ordering::SeqCst), woken + 1);
        assert!(!crate::stack::take_interrupt(), "no interrupt is raised");
        let applied = reload_point().expect("the reload point applies the request");
        assert_eq!(applied.units, ["lib/math"]);
        assert!(reload_point().is_none());
        apply_at_next_call();
        assert_eq!(reload_policy(), ReloadPolicy::AtNextCall);
    }

    static HOOK_SAW: Mutex<Vec<(Vec<u64>, Vec<usize>)>> = Mutex::new(Vec::new());

    fn record_applied(applied: &AppliedReload) {
        HOOK_SAW.lock().unwrap().push((
            applied.composable_bodies.clone(),
            applied.retired_slots.clone(),
        ));
    }

    /// D3 of `compose-hot-reload`: the hook sees the reload's bodies and retired slots, on the
    /// thread that applies it, before `apply_pending` returns.
    #[test]
    fn the_applied_hook_sees_the_reloads_fields() {
        let _lock = crate::test_lock();
        crate::stack::clear_stack_limit();
        HOOK_SAW.lock().unwrap().clear();
        set_applied_hook(Some(record_applied));
        request(PendingReload::new(|| AppliedReload {
            units: vec!["main".to_string()],
            composable_bodies: vec![7, 9],
            retired_slots: vec![0x1000],
            message: String::new(),
        }));
        assert!(apply_pending().is_some());
        set_applied_hook(None);
        assert_eq!(*HOOK_SAW.lock().unwrap(), [(vec![7, 9], vec![0x1000])]);
    }

    #[test]
    fn the_requested_flag_follows_requests_applications_and_withdrawals() {
        let _lock = crate::test_lock();
        crate::stack::clear_stack_limit();
        defer_to_reload_points(wake);
        assert!(!is_requested());
        assert!(reload_point().is_none());
        request(applying(&["a"]));
        assert!(is_requested());
        assert!(withdraw().is_some());
        assert!(!is_requested());
        request(applying(&["b"]));
        assert_eq!(reload_point().unwrap().units, ["b"]);
        assert!(!is_requested());
        apply_at_next_call();
    }

    /// D12 of `compose-hot-reload`: under reload points, an `after line` step's `apply_now`
    /// leaves the reload for the loop's reload point.
    #[test]
    fn apply_now_leaves_the_reload_pending_under_reload_points() {
        let _lock = crate::test_lock();
        crate::stack::clear_stack_limit();
        defer_to_reload_points(wake);
        request(applying(&["a"]));
        assert!(apply_now().is_none());
        assert_eq!(reload_point().unwrap().units, ["a"]);
        apply_at_next_call();
        request(applying(&["b"]));
        assert_eq!(apply_now().unwrap().units, ["b"]);
    }

    /// "Leaving Reload Points Applies a Pending Reload at the Next Call": switching back raises
    /// the interrupt for a reload still pending.
    #[test]
    fn leaving_reload_points_interrupts_for_a_pending_reload() {
        let _lock = crate::test_lock();
        crate::stack::set_stack_limit(0x1000_0000, 8 * 1024 * 1024);
        defer_to_reload_points(wake);
        request(applying(&["a"]));
        assert!(
            !crate::stack::take_interrupt(),
            "no interrupt under reload points"
        );
        apply_at_next_call();
        assert!(
            crate::stack::take_interrupt(),
            "the pending reload interrupts"
        );
        assert_eq!(apply_pending().unwrap().units, ["a"]);
        apply_at_next_call();
        assert!(
            !crate::stack::take_interrupt(),
            "nothing pending, no interrupt"
        );
        crate::stack::clear_stack_limit();
    }

    #[test]
    fn scripted_edits_reach_the_hook_in_order() {
        let _lock = crate::test_lock();
        let seen: Arc<Mutex<Vec<PathBuf>>> = Arc::default();
        let sink = Arc::clone(&seen);
        set_edit_hook(Some(Arc::new(move |edits: Vec<ScriptedEdit>| {
            sink.lock()
                .unwrap()
                .extend(edits.into_iter().map(|edit| edit.target));
        })));
        let edit = |target: &str| ScriptedEdit {
            target: PathBuf::from(target),
            content: String::new(),
        };
        request_scripted_edit(vec![edit("lib/labels.prism"), edit("app/main.prism")]);
        set_edit_hook(None);
        request_scripted_edit(vec![edit("ignored.prism")]);
        assert_eq!(
            *seen.lock().unwrap(),
            [
                PathBuf::from("lib/labels.prism"),
                PathBuf::from("app/main.prism")
            ]
        );
    }
}

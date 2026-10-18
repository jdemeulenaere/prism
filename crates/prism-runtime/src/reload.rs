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

use crate::output::flush;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};

/// What applying a reload did: the units it defined, and the text the program thread prints to
/// stderr once it is applied (`reload: applied ...` and its notes).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AppliedReload {
    /// The names of the file units and instance units whose code the reload defined, which
    /// `compose-hot-reload` recomposes the restart scopes of.
    pub units: Vec<String>,
    /// Printed to stderr after the program's output is flushed.
    pub message: String,
}

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
    taken
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
    flush();
    let applied = (reload.apply)();
    instruction_barrier();
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
/// interrupted it.
pub fn apply_now() -> Option<AppliedReload> {
    crate::stack::take_interrupt();
    apply_pending()
}

/// Defers reloads to the reload points the calling frame loop runs (D4 of `hot-reload`): from
/// now on a request does not interrupt the program, but calls `wake`.
pub fn defer_to_reload_points(wake: fn()) {
    let mut policy = policy();
    policy.policy = ReloadPolicy::AtReloadPoints;
    policy.wake = Some(wake);
}

/// Applies reloads at the program's next call again, the default.
pub fn apply_at_next_call() {
    let mut policy = policy();
    policy.policy = ReloadPolicy::AtNextCall;
    policy.wake = None;
}

/// The current policy.
pub fn reload_policy() -> ReloadPolicy {
    policy().policy
}

/// A frame loop's reload point: applies the pending reload, if any, and reports it.
pub fn reload_point() -> Option<AppliedReload> {
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
            message: String::new(),
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
}

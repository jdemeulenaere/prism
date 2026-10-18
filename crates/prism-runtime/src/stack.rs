//! The stack limit that turns a stack overflow into a runtime error (D27 of `codegen-core`).
//!
//! Every generated function that calls another compares the stack pointer with
//! [`STACK_LIMIT`] when it is entered, and raises the runtime error "stack overflow" when the
//! pointer is below it. The limit sits [`STACK_RESERVE`] above the end of the program thread's
//! stack, which leaves room for the frame that detects the overflow and for reporting it, so the
//! guard page is never reached by a Prism frame and the program's output is flushed as for any
//! other runtime error. It is zero, which disables the check, outside a run.
//!
//! The same check makes every such function entry a reload point (D3 of `hot-reload`): to have
//! the program thread apply a pending reload, the watch session stores the interrupt sentinel
//! [`INTERRUPT`] into the limit, which every stack pointer is below, so the next entry takes the
//! check's cold path. That path calls [`prism_rt_stack_check_failed`], which restores the real
//! limit, applies the reload and lets the function continue, and reports an overflow only when
//! the stack really is below the real limit. This is Go's preemption trick (`stackguard0` set to
//! `stackPreempt`) applied to reloads, and costs nothing on the hot path.

use std::sync::atomic::{AtomicUsize, Ordering};

/// The lowest stack address generated code may use, read by every function that calls another;
/// zero outside a run.
#[export_name = "prism_rt_stack_limit"]
pub static STACK_LIMIT: AtomicUsize = AtomicUsize::new(0);

/// The limit [`set_stack_limit`] computed, which [`STACK_LIMIT`] holds whenever it does not hold
/// the interrupt sentinel.
static REAL_LIMIT: AtomicUsize = AtomicUsize::new(0);

/// The value of [`STACK_LIMIT`] that asks the program thread to apply a pending reload at its
/// next function entry: no stack pointer is at or above it, so every check takes its cold path.
pub const INTERRUPT: usize = usize::MAX;

/// The symbol under which generated code imports [`STACK_LIMIT`].
pub const STACK_LIMIT_SYMBOL: &str = "prism_rt_stack_limit";

/// The bytes left between the limit and the end of the stack, for the frame that detects an
/// overflow and the runtime's report of it.
pub const STACK_RESERVE: usize = 256 * 1024;

/// Sets the limit for a program whose thread has a stack of `stack_size` bytes, of which
/// `stack_top` is an address near the start (the highest address, since the stack grows down).
pub fn set_stack_limit(stack_top: usize, stack_size: usize) {
    let limit = stack_top.saturating_sub(stack_size.saturating_sub(STACK_RESERVE));
    REAL_LIMIT.store(limit, Ordering::SeqCst);
    STACK_LIMIT.store(limit, Ordering::SeqCst);
    // A reload requested before the program thread got here must not lose its interrupt.
    if crate::reload::has_pending_interrupt() {
        interrupt();
    }
}

/// Disables the check, at the end of a run.
pub fn clear_stack_limit() {
    REAL_LIMIT.store(0, Ordering::SeqCst);
    STACK_LIMIT.store(0, Ordering::SeqCst);
}

/// Asks the program thread to take the cold path of its next stack check, where it applies the
/// pending reload (D3 of `hot-reload`). Called by [`crate::reload::request`].
///
/// Only a run's real limit is replaced: outside a run, and once `main` returned while the run
/// ends (its top-level values are dropped by glue, which checks the stack too), the limit stays
/// zero, so a reload requested then is never applied to a run that is over. A run that starts
/// later raises the interrupt itself ([`set_stack_limit`]).
pub fn interrupt() {
    let real = REAL_LIMIT.load(Ordering::SeqCst);
    if real != 0 {
        // Fails when the run ended or the interrupt is raised already, which is as wanted.
        let _ = STACK_LIMIT.compare_exchange(real, INTERRUPT, Ordering::SeqCst, Ordering::SeqCst);
    }
}

/// Restores the real limit if the limit holds the interrupt sentinel, returning whether it did.
/// The limit is restored before the pending reload is taken, so a request that arrives in
/// between either is taken now, leaving a harmless spurious interrupt, or sets the sentinel
/// again after the restore: no request is lost.
pub(crate) fn take_interrupt() -> bool {
    let real = REAL_LIMIT.load(Ordering::SeqCst);
    STACK_LIMIT
        .compare_exchange(INTERRUPT, real, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
}

/// The cold path of every stack check (the `code-generation` requirement "Stack Checks Are
/// Reload Points"), called with the stack pointer of the function being entered. When the limit
/// held the interrupt sentinel, restores the real limit and applies the pending reload. Returns
/// 0 when the function may continue, and 1 when the stack pointer is below the real limit, so
/// that the function raises "stack overflow".
#[no_mangle]
pub extern "C" fn prism_rt_stack_check_failed(stack_pointer: usize) -> u8 {
    if take_interrupt() {
        crate::reload::apply_pending();
    }
    u8::from(stack_pointer < REAL_LIMIT.load(Ordering::SeqCst))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_limit_leaves_the_reserve_above_the_end_of_the_stack() {
        let _lock = crate::test_lock();
        set_stack_limit(0x1000_0000, 8 * 1024 * 1024);
        assert_eq!(
            STACK_LIMIT.load(Ordering::Relaxed),
            0x1000_0000 - 8 * 1024 * 1024 + STACK_RESERVE
        );
        clear_stack_limit();
        assert_eq!(STACK_LIMIT.load(Ordering::Relaxed), 0);
    }

    /// A reload requested outside a run, or after `main` returned, leaves the limit alone; a run
    /// that starts afterwards raises the interrupt itself.
    #[test]
    fn a_request_outside_a_run_waits_for_the_next_run() {
        let _lock = crate::test_lock();
        clear_stack_limit();
        crate::reload::request(crate::reload::PendingReload::new(Default::default));
        assert_eq!(STACK_LIMIT.load(Ordering::SeqCst), 0);
        set_stack_limit(0x1000_0000, 8 * 1024 * 1024);
        assert_eq!(STACK_LIMIT.load(Ordering::SeqCst), INTERRUPT);
        let real = REAL_LIMIT.load(Ordering::SeqCst);
        assert_eq!(prism_rt_stack_check_failed(real + 64), 0);
        assert!(crate::reload::withdraw().is_none(), "the run applied it");
        assert_eq!(STACK_LIMIT.load(Ordering::SeqCst), real);
        clear_stack_limit();
    }

    /// An interrupt with no pending reload lets the function continue and restores the limit;
    /// a real overflow is still one.
    #[test]
    fn an_interrupt_without_a_reload_continues_and_restores_the_limit() {
        let _lock = crate::test_lock();
        set_stack_limit(0x1000_0000, 8 * 1024 * 1024);
        let real = STACK_LIMIT.load(Ordering::SeqCst);
        interrupt();
        assert_eq!(STACK_LIMIT.load(Ordering::SeqCst), INTERRUPT);
        assert_eq!(prism_rt_stack_check_failed(real + 64), 0);
        assert_eq!(STACK_LIMIT.load(Ordering::SeqCst), real);
        assert_eq!(prism_rt_stack_check_failed(real - 64), 1);
        interrupt();
        assert_eq!(prism_rt_stack_check_failed(real - 64), 1);
        assert_eq!(STACK_LIMIT.load(Ordering::SeqCst), real);
        clear_stack_limit();
    }
}

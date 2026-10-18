//! The stack limit that turns a stack overflow into a runtime error (D27 of `codegen-core`).
//!
//! Every generated function that calls another compares the stack pointer with
//! [`STACK_LIMIT`] when it is entered, and raises the runtime error "stack overflow" when the
//! pointer is below it. The limit sits [`STACK_RESERVE`] above the end of the program thread's
//! stack, which leaves room for the frame that detects the overflow and for reporting it, so the
//! guard page is never reached by a Prism frame and the program's output is flushed as for any
//! other runtime error. It is zero, which disables the check, outside a run.

use std::sync::atomic::{AtomicUsize, Ordering};

/// The lowest stack address generated code may use, read by every function that calls another;
/// zero outside a run.
#[export_name = "prism_rt_stack_limit"]
pub static STACK_LIMIT: AtomicUsize = AtomicUsize::new(0);

/// The symbol under which generated code imports [`STACK_LIMIT`].
pub const STACK_LIMIT_SYMBOL: &str = "prism_rt_stack_limit";

/// The bytes left between the limit and the end of the stack, for the frame that detects an
/// overflow and the runtime's report of it.
pub const STACK_RESERVE: usize = 256 * 1024;

/// Sets the limit for a program whose thread has a stack of `stack_size` bytes, of which
/// `stack_top` is an address near the start (the highest address, since the stack grows down).
pub fn set_stack_limit(stack_top: usize, stack_size: usize) {
    let limit = stack_top.saturating_sub(stack_size.saturating_sub(STACK_RESERVE));
    STACK_LIMIT.store(limit, Ordering::Relaxed);
}

/// Disables the check, at the end of a run.
pub fn clear_stack_limit() {
    STACK_LIMIT.store(0, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_limit_leaves_the_reserve_above_the_end_of_the_stack() {
        set_stack_limit(0x1000_0000, 8 * 1024 * 1024);
        assert_eq!(
            STACK_LIMIT.load(Ordering::Relaxed),
            0x1000_0000 - 8 * 1024 * 1024 + STACK_RESERVE
        );
        clear_stack_limit();
        assert_eq!(STACK_LIMIT.load(Ordering::Relaxed), 0);
    }
}

//! Runtime errors (D12 of `codegen-core`) and the arithmetic that needs a runtime call (D11).
//!
//! A runtime error flushes the program's output, writes `panic: <message>` and
//! `  at <file>:<line>:<column>` to stderr and exits the process with status
//! [`PANIC_EXIT_STATUS`]. It does not unwind: no drop runs and nothing can catch it.
//!
//! Under a watch session ([`set_watch_mode`], D13 of `hot-reload`), it stops the program's thread
//! instead of the process: it reports the error the same way, tells the session, and parks the
//! thread forever, so no further code of the program runs and no drop runs.

use crate::output::flush;
use std::io::Write;
use std::sync::{Arc, Mutex};

/// What a watch session is told when the program stops, with the status the process would have
/// exited with.
pub type StopHandler = Arc<dyn Fn(i32) + Send + Sync>;

static WATCH_MODE: Mutex<Option<StopHandler>> = Mutex::new(None);

/// Under `Some(on_stop)`, a runtime error, or a failed allocation check, stops the program's
/// thread instead of ending the process: after reporting, it calls `on_stop` with the exit
/// status and parks the thread forever. `None` restores ending the process.
pub fn set_watch_mode(on_stop: Option<StopHandler>) {
    *WATCH_MODE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = on_stop;
}

/// Ends the program with `status` once its error is reported: the process, or under watch mode
/// the program's thread. No runtime lock is held here: the output and stderr locks are released
/// after each write.
pub(crate) fn stop(status: i32) -> ! {
    let handler = WATCH_MODE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    match handler {
        Some(on_stop) => {
            crate::stack::clear_stack_limit();
            on_stop(status);
            loop {
                std::thread::park();
            }
        }
        None => std::process::exit(status),
    }
}

/// The exit status of a program that ended with a runtime error, distinct from `1` (the program
/// did not compile) and from signal statuses.
pub const PANIC_EXIT_STATUS: i32 = 101;

/// What went wrong, passed by generated code as a `u32`.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanicKind {
    /// An `Int` operation whose exact result does not fit; the detail is the operator.
    IntegerOverflow = 0,
    /// `/` or `%` of an `Int` by zero.
    DivisionByZero = 1,
    /// A top-level value read while its own initializer runs; the detail is its name.
    ValueReadDuringInitialization = 2,
    /// Code the compiler proved unreachable was reached.
    Unreachable = 3,
    /// A call through a slot of the function table that no function fills.
    UnlinkedFunction = 4,
    /// A call entered a function with the stack pointer below the stack limit.
    StackOverflow = 5,
    /// An allocation the system cannot satisfy, or a size no allocation can have. Reported
    /// without a location, since it can happen deep in the runtime (D17 of
    /// `runtime-strings-and-drops`).
    OutOfMemory = 6,
    /// `s[i]` outside `0..<s.length`; the detail is `<i> out of range for length <n>`.
    StringIndexOutOfRange = 7,
    /// An element access, `insert` or `removeAt` of a list with an index out of range; the
    /// detail is `<i> out of range for size <n>` (D8 of `collections`).
    ListIndexOutOfRange = 8,
    /// `removeLast()` of an empty list.
    EmptyList = 9,
}

impl PanicKind {
    /// Every kind, in code order.
    pub const ALL: [PanicKind; 10] = [
        PanicKind::IntegerOverflow,
        PanicKind::DivisionByZero,
        PanicKind::ValueReadDuringInitialization,
        PanicKind::Unreachable,
        PanicKind::UnlinkedFunction,
        PanicKind::StackOverflow,
        PanicKind::OutOfMemory,
        PanicKind::StringIndexOutOfRange,
        PanicKind::ListIndexOutOfRange,
        PanicKind::EmptyList,
    ];

    /// The kind with code `code`.
    pub fn from_code(code: u32) -> Option<PanicKind> {
        PanicKind::ALL.into_iter().find(|kind| *kind as u32 == code)
    }
}

/// The message of a runtime error of `kind` with `detail`, as written after `panic: `.
pub fn panic_message(kind: PanicKind, detail: &str) -> String {
    match kind {
        PanicKind::IntegerOverflow => format!("integer overflow in '{}'", detail),
        PanicKind::DivisionByZero => "division by zero".to_string(),
        PanicKind::ValueReadDuringInitialization => {
            format!(
                "top-level value '{}' read during its own initialization",
                detail
            )
        }
        PanicKind::Unreachable => "internal error: entered unreachable code".to_string(),
        PanicKind::UnlinkedFunction => {
            "internal error: called a function that is not linked".to_string()
        }
        PanicKind::StackOverflow => "stack overflow".to_string(),
        PanicKind::OutOfMemory => "out of memory".to_string(),
        PanicKind::StringIndexOutOfRange => format!("string index {}", detail),
        PanicKind::ListIndexOutOfRange => format!("list index {}", detail),
        PanicKind::EmptyList => "removeLast() on an empty list".to_string(),
    }
}

/// Reads `length` bytes at `pointer` as text.
///
/// # Safety
///
/// `pointer` must point at `length` readable bytes, or `length` must be zero.
pub(crate) unsafe fn text<'a>(pointer: *const u8, length: usize) -> std::borrow::Cow<'a, str> {
    if length == 0 || pointer.is_null() {
        return std::borrow::Cow::Borrowed("");
    }
    String::from_utf8_lossy(std::slice::from_raw_parts(pointer, length))
}

/// Reports a runtime error and ends the process. Called by generated code from cold blocks,
/// with the error's location as immediates and its file and detail as read-only data of the
/// file's code unit.
///
/// # Safety
///
/// `detail` and `file` must point at `detail_length` and `file_length` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn prism_rt_panic(
    kind: u32,
    detail: *const u8,
    detail_length: usize,
    file: *const u8,
    file_length: usize,
    line: u32,
    column: u32,
) -> ! {
    let detail = text(detail, detail_length);
    let file = text(file, file_length);
    let message = match PanicKind::from_code(kind) {
        Some(kind) => panic_message(kind, &detail),
        None => format!("internal error: unknown runtime error {}", kind),
    };
    report_at(&message, &file, line, column)
}

/// Reports the runtime error `kind` with `detail` at `file:line:column` and ends the process:
/// what runtime functions that receive their caller's location raise.
pub(crate) fn panic_at(kind: PanicKind, detail: &str, file: &str, line: u32, column: u32) -> ! {
    report_at(&panic_message(kind, detail), file, line, column)
}

fn report_at(message: &str, file: &str, line: u32, column: u32) -> ! {
    report(&format!(
        "panic: {}\n  at {}:{}:{}\n",
        message, file, line, column
    ))
}

/// Reports the runtime error `kind` with `detail` without a location line and ends the process:
/// "out of memory", which can happen inside the runtime, far from any expression.
pub fn panic_without_location(kind: PanicKind, detail: &str) -> ! {
    report(&format!("panic: {}\n", panic_message(kind, detail)))
}

/// Reports the runtime error of code `kind` without a location line and ends the process.
/// Called by generated code that has no location to report: drop, clone and equality glue,
/// which raise "stack overflow" when they recurse deeper than the stack allows (D14 of
/// `heap-boxes`).
#[no_mangle]
pub extern "C" fn prism_rt_panic_without_location(kind: u32) -> ! {
    match PanicKind::from_code(kind) {
        Some(kind) => panic_without_location(kind, ""),
        None => report(&format!(
            "panic: internal error: unknown runtime error {}\n",
            kind
        )),
    }
}

/// What a slot of the function table holds until a function fills it. In a program that
/// checked, no call reaches it; it ends the program with a runtime error rather than jumping to
/// an invalid address. It is called with whatever arguments the call site passes, which it
/// ignores.
#[no_mangle]
pub extern "C" fn prism_rt_unlinked_function() -> ! {
    report(&format!(
        "panic: {}\n",
        panic_message(PanicKind::UnlinkedFunction, "")
    ))
}

/// Flushes the program's output, writes `report` to stderr and exits with
/// [`PANIC_EXIT_STATUS`].
fn report(report: &str) -> ! {
    flush();
    {
        let mut stderr = std::io::stderr().lock();
        let _ = stderr.write_all(report.as_bytes());
        let _ = stderr.flush();
    }
    stop(PANIC_EXIT_STATUS)
}

/// `%` of two `Double`s: the remainder of truncated division, with the sign of the dividend
/// (C's `fmod`), which Cranelift has no instruction for (D11).
#[no_mangle]
pub extern "C" fn prism_rt_double_remainder(dividend: f64, divisor: f64) -> f64 {
    dividend % divisor
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_of_every_kind() {
        let cases = [
            (PanicKind::IntegerOverflow, "+", "integer overflow in '+'"),
            (PanicKind::DivisionByZero, "", "division by zero"),
            (
                PanicKind::ValueReadDuringInitialization,
                "A",
                "top-level value 'A' read during its own initialization",
            ),
            (
                PanicKind::Unreachable,
                "",
                "internal error: entered unreachable code",
            ),
            (
                PanicKind::UnlinkedFunction,
                "",
                "internal error: called a function that is not linked",
            ),
            (PanicKind::StackOverflow, "", "stack overflow"),
            (PanicKind::OutOfMemory, "", "out of memory"),
            (
                PanicKind::StringIndexOutOfRange,
                "3 out of range for length 3",
                "string index 3 out of range for length 3",
            ),
            (
                PanicKind::ListIndexOutOfRange,
                "-1 out of range for size 1",
                "list index -1 out of range for size 1",
            ),
            (PanicKind::EmptyList, "", "removeLast() on an empty list"),
        ];
        assert_eq!(cases.len(), PanicKind::ALL.len());
        for (kind, detail, message) in cases {
            assert_eq!(panic_message(kind, detail), message);
            assert_eq!(PanicKind::from_code(kind as u32), Some(kind));
        }
        assert_eq!(PanicKind::from_code(99), None);
    }

    /// Under watch mode a runtime error parks the program's thread and tells the session,
    /// instead of ending the process.
    #[test]
    fn a_runtime_error_under_watch_mode_parks_the_thread() {
        let _lock = crate::test_lock();
        let (sender, receiver) = std::sync::mpsc::channel();
        let sender = Mutex::new(sender);
        set_watch_mode(Some(Arc::new(move |status| {
            let _ = sender.lock().unwrap().send(status);
        })));
        let thread = std::thread::spawn(|| {
            panic_without_location(PanicKind::DivisionByZero, "");
        });
        let status = receiver
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("the session is told");
        assert_eq!(status, PANIC_EXIT_STATUS);
        set_watch_mode(None);
        std::thread::sleep(std::time::Duration::from_millis(20));
        assert!(!thread.is_finished(), "the thread stays parked");
    }

    #[test]
    fn double_remainder_truncates() {
        assert_eq!(prism_rt_double_remainder(-7.5, 2.0), -1.5);
        assert_eq!(prism_rt_double_remainder(7.5, -2.0), 1.5);
        assert_eq!(prism_rt_double_remainder(6.0, 3.0), 0.0);
        assert!(prism_rt_double_remainder(1.0, 0.0).is_nan());
        assert!(prism_rt_double_remainder(f64::INFINITY, 2.0).is_nan());
        assert_eq!(prism_rt_double_remainder(2.0, f64::INFINITY), 2.0);
    }
}

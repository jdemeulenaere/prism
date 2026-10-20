//! The program's output and the process state (D16 and D17 of `codegen-core`).
//!
//! Output is appended to a buffer and written to stdout when the buffer reaches
//! [`OUTPUT_BUFFER_SIZE`], when the program ends ([`finish`]), before a runtime error is reported,
//! and after each newline when stdout is a terminal, so an interactive program shows each line as
//! it is written while output to a pipe or file costs one write per buffer.

use crate::allocation::{finish_tracking, start_tracking, ALLOCATION_CHECK_EXIT_STATUS};
use crate::double::format_double;
use std::io::{IsTerminal, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// The size at which buffered output is written out.
pub const OUTPUT_BUFFER_SIZE: usize = 8 * 1024;

/// The state of the running program. Prism programs are single-threaded today; the lock costs
/// nothing next to formatting.
struct State {
    output: Vec<u8>,
    /// Whether stdout is a terminal, which flushes at each newline.
    line_buffered: bool,
    /// The arguments after `--` on the `prism run` command line, which a future built-in will
    /// read once strings and lists exist.
    arguments: Vec<String>,
}

static STATE: Mutex<State> = Mutex::new(State {
    output: Vec::new(),
    line_buffered: false,
    arguments: Vec::new(),
});

/// What an edit script runs at each newline the program writes (D16 of `hot-reload`), with the
/// number of newlines written so far in the current run.
pub type LineHook = Arc<dyn Fn(u64) + Send + Sync>;

static LINE_HOOK: Mutex<Option<LineHook>> = Mutex::new(None);
/// Whether a line hook is installed, so that output without one counts no newlines.
static HOOKED: AtomicBool = AtomicBool::new(false);
/// The newlines written in the current run while a hook is installed.
static LINES: AtomicU64 = AtomicU64::new(0);

/// Installs `hook`, called with the newline count of the current run after each newline the
/// program writes, outside the output lock and after the output is flushed, so that it can stop
/// the program there; `None` removes it.
pub fn set_line_hook(hook: Option<LineHook>) {
    let mut installed = LINE_HOOK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    HOOKED.store(hook.is_some(), Ordering::SeqCst);
    *installed = hook;
}

/// Calls the line hook once per newline of `text`, if one is installed.
fn run_line_hook(text: &[u8]) {
    if !HOOKED.load(Ordering::Relaxed) {
        return;
    }
    let newlines = text.iter().filter(|byte| **byte == b'\n').count();
    if newlines == 0 {
        return;
    }
    flush();
    let hook = LINE_HOOK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    for _ in 0..newlines {
        let count = LINES.fetch_add(1, Ordering::SeqCst) + 1;
        if let Some(hook) = &hook {
            hook(count);
        }
    }
}

fn with_state<T>(f: impl FnOnce(&mut State) -> T) -> T {
    // A poisoned lock only means an earlier panic of the host; the buffer is still usable.
    let mut state = STATE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    f(&mut state)
}

/// Starts a run of a program with `arguments`: clears the output buffer, records whether
/// stdout is a terminal, and turns the allocation check on or off (`prism run
/// --check-allocations`, D13 of `runtime-strings-and-drops`).
pub fn start(arguments: Vec<String>, check_allocations: bool) {
    with_state(|state| {
        state.output.clear();
        state.output.reserve(OUTPUT_BUFFER_SIZE);
        state.line_buffered = std::io::stdout().is_terminal();
        state.arguments = arguments;
    });
    LINES.store(0, Ordering::SeqCst);
    start_tracking(check_allocations);
}

/// Ends a run whose `main` returned: flushes the output and gives the exit status, `0`, or,
/// when the allocation check finds allocations still live once the top-level values are
/// released, reports them on stderr and gives [`ALLOCATION_CHECK_EXIT_STATUS`].
pub fn finish() -> i32 {
    flush();
    // Dropping the top-level values under the check runs their hooks, which may print (D13 of
    // `drop-and-clone-hooks`): their output is flushed before the report.
    let report = finish_tracking();
    flush();
    match report {
        None => 0,
        Some(report) => {
            let mut stderr = std::io::stderr().lock();
            let _ = writeln!(stderr, "{}", report);
            let _ = stderr.flush();
            ALLOCATION_CHECK_EXIT_STATUS
        }
    }
}

/// Writes the buffered output to stdout.
pub(crate) fn flush() {
    with_state(write_out);
}

fn write_out(state: &mut State) {
    if state.output.is_empty() {
        return;
    }
    let mut stdout = std::io::stdout().lock();
    // A closed stdout (`prism run x | head`) must not turn into a runtime error: the output is
    // dropped, as a C program's would be after SIGPIPE.
    let _ = stdout.write_all(&state.output);
    let _ = stdout.flush();
    state.output.clear();
}

/// Appends `text` to the output, writing it out as D16 says.
pub(crate) fn append(text: &[u8]) {
    with_state(|state| {
        state.output.extend_from_slice(text);
        let at_newline = state.line_buffered && text.contains(&b'\n');
        if at_newline || state.output.len() >= OUTPUT_BUFFER_SIZE {
            write_out(state);
        }
    });
    run_line_hook(text);
}

/// Writes the program's buffered output to stdout now, as before a frame loop blocks for a
/// reload (`wait reload`, D12 of `compose-hot-reload`), so that what it printed is seen.
pub fn flush_output() {
    flush();
}

/// Appends `text` to the program's output, in order with what the program prints: what the
/// runtimes that write for the program use, such as `prism-ui`'s dumps (D16 of `ui-toolkit`).
pub fn write_output(text: &str) {
    append(text.as_bytes());
}

/// Takes the buffered output without writing it, for tests of what a function prints.
#[cfg(test)]
pub(crate) fn take_output() -> Vec<u8> {
    with_state(|state| std::mem::take(&mut state.output))
}

/// The text of an `Int`: decimal, with a leading `-` when negative.
pub fn format_int(value: i64) -> String {
    value.to_string()
}

/// The text of a `Bool`: `true` or `false`.
pub fn format_bool(value: bool) -> &'static str {
    if value {
        "true"
    } else {
        "false"
    }
}

/// The text of a `Char`, a Unicode scalar value: its UTF-8 encoding. Generated code only passes
/// scalar values; anything else is written as U+FFFD.
pub fn format_char(value: u32) -> String {
    char::from_u32(value)
        .unwrap_or(char::REPLACEMENT_CHARACTER)
        .to_string()
}

/// `print` of an `Int`.
#[no_mangle]
pub extern "C" fn prism_rt_print_int(value: i64) {
    append(format_int(value).as_bytes());
}

/// `print` of a `Double`, in the text form of the `execution` spec.
#[no_mangle]
pub extern "C" fn prism_rt_print_double(value: f64) {
    append(format_double(value).as_bytes());
}

/// `print` of a `Bool`, passed as a byte holding 0 or 1.
#[no_mangle]
pub extern "C" fn prism_rt_print_bool(value: u8) {
    append(format_bool(value != 0).as_bytes());
}

/// `print` of a `Char`, passed as its scalar value.
#[no_mangle]
pub extern "C" fn prism_rt_print_char(value: u32) {
    append(format_char(value).as_bytes());
}

/// The newline `println` writes after its argument.
#[no_mangle]
pub extern "C" fn prism_rt_print_newline() {
    append(b"\n");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integers_are_decimal() {
        assert_eq!(format_int(0), "0");
        assert_eq!(format_int(-5), "-5");
        assert_eq!(format_int(i64::MAX), "9223372036854775807");
        assert_eq!(format_int(i64::MIN), "-9223372036854775808");
    }

    #[test]
    fn booleans_are_words() {
        assert_eq!(format_bool(true), "true");
        assert_eq!(format_bool(false), "false");
    }

    #[test]
    fn the_line_hook_counts_the_newlines_of_a_run() {
        let _lock = crate::test_lock();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&seen);
        start(Vec::new(), false);
        set_line_hook(Some(Arc::new(move |count| {
            recorded.lock().unwrap().push(count)
        })));
        prism_rt_print_int(1);
        prism_rt_print_newline();
        prism_rt_print_newline();
        append(b"a\nb\n");
        set_line_hook(None);
        prism_rt_print_newline();
        assert_eq!(*seen.lock().unwrap(), [1, 2, 3, 4]);
        start(Vec::new(), false);
        assert_eq!(LINES.load(Ordering::SeqCst), 0);
        take_output();
    }

    #[test]
    fn written_output_joins_printed_output_in_order() {
        let _lock = crate::test_lock();
        start(Vec::new(), false);
        prism_rt_print_int(1);
        write_output("window 4x3\n");
        let string = crate::PrismString::owned("x");
        // SAFETY: `string` is a live string.
        unsafe { crate::prism_rt_print_string(&string) };
        prism_rt_print_newline();
        assert_eq!(take_output(), b"1window 4x3\nx\n");
        // SAFETY: `string` is dropped once.
        unsafe { crate::prism_rt_string_drop(&string as *const _ as *mut _) };
    }

    #[test]
    fn characters_are_utf8() {
        assert_eq!(format_char('x' as u32), "x");
        assert_eq!(format_char('é' as u32), "é");
        assert_eq!(format_char('€' as u32).as_bytes(), [0xE2, 0x82, 0xAC]);
        assert_eq!(format_char('😀' as u32).len(), 4);
        assert_eq!(format_char(0xD800), "\u{FFFD}");
    }
}

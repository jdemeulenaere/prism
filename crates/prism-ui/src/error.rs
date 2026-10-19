//! The UI runtime errors (D17 of `ui-toolkit`): misuse of the toolkit, and event scripts that
//! fail.
//!
//! The operations of this crate return a [`UiResult`], as `prism-compose`'s return a
//! `ComposeResult`, so Rust tests can check every misuse in process. The entry points that
//! generated code calls turn an error into the runtime error it names with [`raise`], which
//! ends the program without a location (an entry point has none).

use std::borrow::Cow;

/// What went wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiError {
    /// `ui misuse: <detail>`.
    Misuse(Cow<'static, str>),
    /// `event script line <line>: <detail>`.
    Script { line: usize, detail: String },
    /// `composition misuse: <detail>`, which the composition runtime reports, as when an app
    /// starts during a pass.
    Composition(&'static str),
}

impl From<prism_compose::Misuse> for UiError {
    fn from(misuse: prism_compose::Misuse) -> UiError {
        UiError::Composition(misuse.0)
    }
}

/// The result of a UI operation.
pub type UiResult<T> = Result<T, UiError>;

impl UiError {
    /// The message written after `panic: `.
    pub fn message(&self) -> String {
        match self {
            UiError::Misuse(detail) => format!("ui misuse: {}", detail),
            UiError::Script { line, detail } => format!("event script line {}: {}", line, detail),
            UiError::Composition(detail) => format!("composition misuse: {}", detail),
        }
    }
}

/// A misuse with a fixed detail.
pub const fn misuse(detail: &'static str) -> UiError {
    UiError::Misuse(Cow::Borrowed(detail))
}

pub const OUTSIDE_RUN_APP: UiError = misuse("toolkit composables run only inside runApp");
pub const OUTSIDE_COMPOSITION: UiError = misuse("node property set outside composition");
pub const STALE_NODE: UiError = misuse("stale node handle");
pub const APP_RUNNING: UiError = misuse("runApp called while an app is running");
pub const NO_WINDOW_SYSTEM: UiError = misuse("no window system is available; run with --headless");
pub const CHAIN_OVERFLOW: UiError = misuse("a modifier chain holds more than 16 elements");
pub const HANDLER_WITHOUT_APP: UiError =
    misuse("modifier handlers are made only while runApp runs");
pub const STALE_HANDLER: UiError = misuse("stale modifier handler");
pub const SHARED_HANDLER: UiError =
    misuse("a handler that owns captures that are not Copy was given to two nodes");
pub const HANDLER_OF_ANOTHER_TYPE: UiError =
    misuse("a handler of another type was given to a modifier");

/// The misuse for a node kind no composable has.
pub fn unknown_kind(kind: i64) -> UiError {
    UiError::Misuse(Cow::Owned(format!("unknown node kind {}", kind)))
}

/// Ends the program with the runtime error `error` names, without a location, and status 101.
///
/// Until `ui-toolkit` task 3.1 gives `prism-runtime` the kinds `UiMisuse` and
/// `EventScriptFailed`, this writes the report itself; that task makes it
/// `panic_without_location` with `UiMisuse`, `EventScriptFailed` or `CompositionMisuse`, which
/// also flushes the program's output first and stops only the program's thread under a watch
/// session.
pub fn raise(error: &UiError) -> ! {
    use std::io::Write;
    let mut stderr = std::io::stderr().lock();
    let _ = writeln!(stderr, "panic: {}", error.message());
    let _ = stderr.flush();
    std::process::exit(prism_runtime::PANIC_EXIT_STATUS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages() {
        assert_eq!(STALE_NODE.message(), "ui misuse: stale node handle");
        assert_eq!(unknown_kind(9).message(), "ui misuse: unknown node kind 9");
        let script = UiError::Script {
            line: 3,
            detail: "no text field is focused".to_string(),
        };
        assert_eq!(
            script.message(),
            "event script line 3: no text field is focused"
        );
    }
}

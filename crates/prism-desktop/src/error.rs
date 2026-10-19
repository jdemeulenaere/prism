//! Display errors (D16 of `desktop-window`): `display error: <detail>`, a runtime error without
//! a location, with status 101.
//!
//! [`raise_display_error`] raises `prism-runtime`'s `PanicKind::DisplayFailed` through
//! `panic_without_location`, as `prism-ui`'s `error::raise` raises its kinds: the program's
//! output is flushed first, and under a watch session only the program's thread stops.

/// The message written after `panic: `.
pub fn display_error_message(detail: &str) -> String {
    format!("display error: {}", detail)
}

/// The detail of a window that could not be opened.
pub fn cannot_open_a_window(reason: &str) -> String {
    format!("cannot open a window ({})", reason)
}

/// The detail of a screenshot that could not be written.
pub fn cannot_write_the_screenshot(path: &str, reason: &str) -> String {
    format!("cannot write the screenshot {} ({})", path, reason)
}

/// Ends the program with `display error: <detail>` and status 101.
pub fn raise_display_error(detail: &str) -> ! {
    prism_runtime::panic_without_location(prism_runtime::PanicKind::DisplayFailed, detail)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The runtime's message of the kind is the one this crate builds.
    #[test]
    fn the_runtime_message_matches() {
        assert_eq!(
            prism_runtime::panic_message(
                prism_runtime::PanicKind::DisplayFailed,
                &cannot_open_a_window("no display")
            ),
            display_error_message(&cannot_open_a_window("no display"))
        );
    }

    #[test]
    fn messages() {
        assert_eq!(
            display_error_message(&cannot_open_a_window("no display")),
            "display error: cannot open a window (no display)"
        );
        assert_eq!(
            display_error_message(&cannot_write_the_screenshot("missing/shot.png", "gone")),
            "display error: cannot write the screenshot missing/shot.png (gone)"
        );
    }
}

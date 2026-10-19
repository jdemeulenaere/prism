//! Display errors (D16 of `desktop-window`): `display error: <detail>`, a runtime error without
//! a location, with status 101.
//!
//! Until task 4.2 gives `prism-runtime` the kind `DisplayFailed`, [`raise_display_error`]
//! writes the report itself, as `prism-ui`'s `error::raise` does until `ui-toolkit` task 3.1;
//! that task makes it `panic_without_location` with `DisplayFailed`, which also flushes the
//! program's output first and stops only the program's thread under a watch session.

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
    use std::io::Write;
    let mut stderr = std::io::stderr().lock();
    let _ = writeln!(stderr, "panic: {}", display_error_message(detail));
    let _ = stderr.flush();
    std::process::exit(prism_runtime::PANIC_EXIT_STATUS)
}

#[cfg(test)]
mod tests {
    use super::*;

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

//! `prism_cli::run_file`, the library's entry for running a program, which opens no window (D3
//! of `desktop-window`): the scenario "No window and no headless mode" of the `ui` spec, on real
//! `.prism` source.
//!
//! It was an executable fixture until `desktop-window` gave the `prism` binary a window host:
//! that binary now opens a window for the same program, and a build without the `window`
//! feature, which the scenario names, is checked by hand (`desktop-window` task 6.4). The
//! library entry keeps the rule, so this test runs the program through it, in a child process
//! since the runtime error ends the process.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const CHILD: &str = "PRISM_RUN_LIBRARY_CHILD";

fn temporary_directory(test_name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("prism_run_library_{}_{}", test_name, nanos));
    std::fs::create_dir_all(&path).unwrap();
    path
}

/// ui: "No window and no headless mode".
#[test]
fn no_window_and_no_headless_mode() {
    if let Some(entry) = std::env::var_os(CHILD) {
        let options = prism_cli::RunOptions {
            check_allocations: true,
            ..prism_cli::RunOptions::default()
        };
        let status = prism_cli::run_file(std::path::Path::new(&entry), &options)
            .unwrap_or_else(|error| panic!("{}", error.render()));
        std::process::exit(status);
    }
    let directory = temporary_directory("no_window");
    let entry = directory.join("main.prism");
    std::fs::write(
        &entry,
        "import /prism/compose/ui
import /prism/compose/material3

fun main() { runApp { Text(\"hi\") } }
",
    )
    .unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "no_window_and_no_headless_mode",
            "--test-threads=1",
        ])
        .env(CHILD, &entry)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(101));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.ends_with("panic: ui misuse: no window system is available; run with --headless\n"),
        "{}",
        stderr
    );
    std::fs::remove_dir_all(&directory).unwrap();
}

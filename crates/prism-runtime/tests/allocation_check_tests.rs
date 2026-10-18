//! End-to-end tests of the allocation check's failures, which end the process: each test runs
//! itself again as a child process that makes the failing call, and checks the child's output
//! and exit status (D13 of `runtime-strings-and-drops`).

use prism_runtime::{
    prism_rt_allocate, prism_rt_free, prism_rt_print_int, prism_rt_print_newline,
    ALLOCATION_CHECK_EXIT_STATUS, INVALID_FREE_MESSAGE,
};
use std::process::Command;

/// The variable telling a child process which failure to make.
const CHILD: &str = "PRISM_ALLOCATION_CHECK_CHILD";

/// Runs the test `name` of this binary as a child with `CHILD` set to `name`, returning its
/// stdout, stderr and exit status.
fn run_child(name: &str) -> (String, String, Option<i32>) {
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--nocapture", "--test-threads", "1"])
        .env(CHILD, name)
        .output()
        .unwrap();
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// Whether this process is the child of the test `name`.
fn is_child(name: &str) -> bool {
    std::env::var(CHILD).as_deref() == Ok(name)
}

#[test]
fn a_double_free_is_reported() {
    // The scenario "A double free is reported" of the `execution` spec: the output printed
    // before is flushed, the message is written and the status is 102.
    if is_child("a_double_free_is_reported") {
        prism_runtime::start(Vec::new(), true);
        prism_rt_print_int(7);
        prism_rt_print_newline();
        let pointer = prism_rt_allocate(5, 1);
        // SAFETY: the first free is valid; the second is caught by the check before it reaches
        // the allocator.
        unsafe {
            prism_rt_free(pointer, 5, 1);
            prism_rt_free(pointer, 5, 1);
        }
        unreachable!("the second free ends the process");
    }
    let (stdout, stderr, status) = run_child("a_double_free_is_reported");
    assert!(stdout.contains("7\n"), "stdout: {}", stdout);
    assert!(
        stderr.contains(&format!("{}\n", INVALID_FREE_MESSAGE)),
        "stderr: {}",
        stderr
    );
    assert_eq!(status, Some(ALLOCATION_CHECK_EXIT_STATUS));
}

#[test]
fn a_free_of_another_size_is_reported() {
    if is_child("a_free_of_another_size_is_reported") {
        prism_runtime::start(Vec::new(), true);
        let pointer = prism_rt_allocate(16, 8);
        // SAFETY: the check catches the wrong size before the allocator sees it.
        unsafe { prism_rt_free(pointer, 8, 8) };
        unreachable!("the free ends the process");
    }
    let (_, stderr, status) = run_child("a_free_of_another_size_is_reported");
    assert!(stderr.contains(INVALID_FREE_MESSAGE), "stderr: {}", stderr);
    assert_eq!(status, Some(ALLOCATION_CHECK_EXIT_STATUS));
}

#[test]
fn a_leak_ends_the_run_with_status_102() {
    // The scenario "A leak is reported", through `finish` as `prism run` calls it.
    if is_child("a_leak_ends_the_run_with_status_102") {
        prism_runtime::start(Vec::new(), true);
        let _leaked = prism_rt_allocate(5, 1);
        std::process::exit(prism_runtime::finish());
    }
    let (_, stderr, status) = run_child("a_leak_ends_the_run_with_status_102");
    assert!(
        stderr.contains("allocation check: allocations still live at exit: 1 (5 bytes)\n"),
        "stderr: {}",
        stderr
    );
    assert_eq!(status, Some(ALLOCATION_CHECK_EXIT_STATUS));
}

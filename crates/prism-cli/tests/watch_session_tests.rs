//! Tests of the watch session in the process (`hot-reload`), for the scenarios whose WHEN holds
//! the program at a point no edit script can name. The session runs the program on a thread of
//! this process and installs process-wide runtime hooks, so these tests run in a process of
//! their own.

use prism_cli::{ReloadOutcome, RunOptions, SessionEvent, WatchSession};
use std::path::PathBuf;
use std::time::Instant;

fn workspace(name: &str, source: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("prism_watch_session_{}_{}", name, nanos));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("main.prism"), source).unwrap();
    root
}

/// hot-reload: "Two saves before the program calls anything": the program is held at its first
/// output line, `value` is changed to return 2 and then 3, and the program then calls `value`.
/// It reads 3 (otherwise it divides by zero), and only the second reload is applied: the first
/// was withdrawn and its module freed.
#[test]
fn two_saves_before_the_program_calls_anything() {
    let program = |value: i64| {
        format!(
            "fun zero(): Int = 0\nfun value(): Int = {}\n\
             fun main() {{ println(0); if (value() != 3) {{ println(1 / zero()) }} }}\n",
            value
        )
    };
    let root = workspace("two_saves", &program(1));
    let entry = prism_resolver::absolute_path(&root.join("main.prism"));
    let mut session = WatchSession::new(&entry, RunOptions::default());
    session.install_stop_handler();
    session.install_line_hook();
    session.stop_at_line(1);
    assert_eq!(session.start(), ReloadOutcome::Started);
    match session.next_event() {
        Some(SessionEvent::LineWritten(1)) => {}
        other => panic!("expected the first line, got {:?}", other),
    }
    for value in [2, 3] {
        session.overlay(entry.clone(), program(value));
        assert_eq!(
            session.reload(std::slice::from_ref(&entry), Instant::now()),
            ReloadOutcome::Requested
        );
    }
    session.resume();
    match session.next_event() {
        Some(SessionEvent::MainReturned(status)) => session.program_ended(status, false),
        other => panic!("expected main to return, got {:?}", other),
    }
    assert_eq!(session.exit_status(), 0, "the program read 3");
    assert_eq!(
        session.live_modules(),
        Some(2),
        "the first load and one applied reload"
    );
    std::fs::remove_dir_all(&root).ok();
}

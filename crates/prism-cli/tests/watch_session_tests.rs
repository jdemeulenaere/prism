//! Tests of the watch session in the process (`hot-reload`), for the scenarios whose WHEN holds
//! the program at a point no edit script can name. The session runs the program on a thread of
//! this process and installs process-wide runtime hooks, so these tests run in a process of
//! their own, one at a time.

use prism_cli::{ReloadOutcome, RunOptions, SessionEvent, WatchSession};
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};
use std::time::Instant;

/// Held by each test: the runtime's hooks, stack limit and pending reload are process-wide, so
/// two sessions must not run at once.
fn one_at_a_time() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

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
    let _lock = one_at_a_time();
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

/// Waits for the running program's `main` to return, and gives its status.
fn wait_for_return(session: &mut WatchSession) -> i32 {
    match session.next_event() {
        Some(SessionEvent::MainReturned(status)) => {
            session.program_ended(status, false);
            status
        }
        other => panic!("expected main to return, got {:?}", other),
    }
}

/// hot-reload: "Changing a file back after a syntax error": after `main` returned, the entry
/// gets a syntax error and then its first content again, which runs `main` again although the
/// session last parsed that same content.
#[test]
fn changing_a_file_back_after_a_syntax_error() {
    let _lock = one_at_a_time();
    let source = "fun main() { println(1) }\n";
    let root = workspace("syntax_error_reverted", source);
    let entry = prism_resolver::absolute_path(&root.join("main.prism"));
    let mut session = WatchSession::new(&entry, RunOptions::default());
    session.install_stop_handler();
    assert_eq!(session.start(), ReloadOutcome::Started);
    assert_eq!(wait_for_return(&mut session), 0);
    session.overlay(entry.clone(), "fun main() { println(1 }\n".to_string());
    assert_eq!(
        session.reload(std::slice::from_ref(&entry), Instant::now()),
        ReloadOutcome::Failed
    );
    session.overlay(entry.clone(), source.to_string());
    assert_eq!(
        session.reload(std::slice::from_ref(&entry), Instant::now()),
        ReloadOutcome::Started
    );
    assert_eq!(wait_for_return(&mut session), 0);
    assert_eq!(session.exit_status(), 0);
    std::fs::remove_dir_all(&root).ok();
}

/// hot-reload: "A file imported again is read again": an edit drops the import of a file, the
/// file changes on disk while the program does not import it, and a later edit imports it
/// again: the program uses its new content.
#[test]
fn a_file_imported_again_is_read_again() {
    let _lock = one_at_a_time();
    let importing = "import helper\nfun main() { println(value()) }\n";
    let root = workspace("imported_again", importing);
    std::fs::write(root.join("helper.prism"), "public fun value(): Int = 1\n").unwrap();
    let entry = prism_resolver::absolute_path(&root.join("main.prism"));
    let helper = prism_resolver::absolute_path(&root.join("helper.prism"));
    let mut session = WatchSession::new(&entry, RunOptions::default());
    session.install_stop_handler();
    assert_eq!(session.start(), ReloadOutcome::Started);
    assert_eq!(wait_for_return(&mut session), 0);
    assert!(session.watched().contains(&helper));
    session.overlay(entry.clone(), "fun main() { println(0) }\n".to_string());
    assert_eq!(
        session.reload(std::slice::from_ref(&entry), Instant::now()),
        ReloadOutcome::Started
    );
    assert_eq!(wait_for_return(&mut session), 0);
    assert!(!session.watched().contains(&helper));
    // Not watched, so the session is not told.
    std::fs::write(
        &helper,
        "public fun value(): Int = 1 / zero()\nfun zero(): Int = 0\n",
    )
    .unwrap();
    session.overlay(entry.clone(), importing.to_string());
    assert_eq!(
        session.reload(std::slice::from_ref(&entry), Instant::now()),
        ReloadOutcome::Started
    );
    match session.next_event() {
        Some(SessionEvent::Stopped(status)) => session.program_ended(status, true),
        other => panic!("expected the new helper to divide by zero, got {:?}", other),
    }
    std::fs::remove_dir_all(&root).ok();
}

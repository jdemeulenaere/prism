//! The main-thread server without a window (D3, D4 and D16 of `desktop-window`): a program
//! that opens no window costs no event loop and no font, and a window system that cannot start
//! ends the program with a display error.

use prism_desktop::server::EventLoopStarter;
use prism_desktop::MainThreadServer;
use prism_ui::test_support::run_in_child;

/// The scenario "A program without an app".
#[test]
fn a_program_without_an_app() {
    let (server, system) = MainThreadServer::new();
    let statistics = server.statistics();
    let program = std::thread::spawn(move || system.program_ended());
    server.serve();
    program.join().unwrap();
    assert_eq!(statistics.event_loops_created(), 0, "no event loop");
    assert_eq!(statistics.windows_opened(), 0);
    assert!(!statistics.font_loaded(), "no font");
    assert!(!prism_desktop::font::is_loaded(), "no font in the process");
}

fn no_display_in_child() -> ! {
    let starter: EventLoopStarter = Box::new(|| Err("no display".to_string()));
    let (server, system) = MainThreadServer::with_starter(starter);
    std::thread::spawn(move || {
        let factory = system.factory();
        let _host = factory("Counter", 320, 200);
        std::process::exit(0)
    });
    server.serve();
    std::process::exit(3)
}

/// The scenario "No display".
#[test]
fn no_display() {
    let output = run_in_child("no_display", no_display_in_child);
    assert_eq!(output.status.code(), Some(101));
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "panic: display error: cannot open a window (no display)\n"
    );
    assert!(output.stdout.is_empty() || !String::from_utf8_lossy(&output.stdout).contains("panic"));
}

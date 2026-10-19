//! `prism run` and `prism run --watch` of programs that may open windows (D3 and D17 of
//! `desktop-window`), with the `window` feature and without `--headless`.
//!
//! The platform's event loop must run on the process's main thread, so the program runs on its
//! own thread, as always, and the main thread serves its windows through the
//! [`MainThreadServer`] until the program, or the watch session, ends. The caller makes the
//! server and its window system, so that tests can keep a [`WindowSystem`] to drive the windows
//! of a real program with [`WindowSystem::deliver`].

use crate::run::{prepare_run, start_run, RunOptions};
use crate::workspace::WorkspaceError;
use prism_desktop::{MainThreadServer, WindowSystem};
use std::path::Path;

/// Runs the program whose entry file is `file`: compiled here, its `main` on the program's
/// thread, and the main thread, the caller's, serving its windows until it ends. A program that
/// opens none costs a channel: the event loop and the font are created at the first window
/// (D4). Returns the exit status.
pub fn run_with_windows(
    file: &Path,
    options: &RunOptions,
    server: MainThreadServer,
    windows: WindowSystem,
) -> i32 {
    let compiled = match prepare_run(file, options) {
        Ok(compiled) => compiled,
        Err(err) => {
            eprint!("{}", err.render());
            return 1;
        }
    };
    prism_ui::configure(prism_ui::HostConfiguration::Window(windows.factory()));
    let running = match start_run(&compiled, options, move |_| windows.program_ended()) {
        Ok(running) => running,
        Err(err) => {
            eprint!("{}", err.render());
            return 1;
        }
    };
    server.serve();
    match running.join() {
        Ok(status) => status,
        Err(err) => {
            eprint!("{}", WorkspaceError::Backend(err).render());
            1
        }
    }
}

/// Runs a watch session on `file`, following the edit script `edits` when one is given: the
/// session runs on a `watch-session` thread, with the main thread's stack size, and the main
/// thread, the caller's, serves the windows of every run of `main`. Each time the program
/// stops, the session closes the window it left open, and when the session ends, the server
/// stops. Returns the session's exit status.
pub fn watch_with_windows(
    file: &Path,
    options: RunOptions,
    edits: Option<String>,
    server: MainThreadServer,
    windows: WindowSystem,
) -> i32 {
    prism_ui::configure(prism_ui::HostConfiguration::Window(windows.factory()));
    let entry = file.to_path_buf();
    let closer = windows.clone();
    let session = std::thread::Builder::new()
        .name("watch-session".to_string())
        .stack_size(prism_codegen::PROGRAM_STACK_SIZE)
        .spawn(move || {
            let status = crate::watch::run_watch_with(
                &entry,
                options,
                edits.as_deref().map(Path::new),
                Some(std::sync::Arc::new(move || closer.close_all())),
            );
            windows.shut_down();
            status
        })
        .expect("the watch session's thread starts");
    server.serve();
    session.join().unwrap_or(1)
}

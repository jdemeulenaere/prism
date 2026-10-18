//! Ending the process from the program (D10 of `standard-library`): `exitProcess` of the
//! bundled `/prism/system`, whose external is [`prism_rt_exit`].

use crate::output::flush;
use crate::panic::stop_program;

/// The exit status of `status`: its low 8 bits, as POSIX `exit` keeps them, so a program exits
/// with the same status on every platform.
pub fn exit_status(status: i64) -> i32 {
    (status & 255) as i32
}

/// Flushes the program's output and ends the process with the low 8 bits of `status`. No drop
/// runs and the allocation check of `--check-allocations` does not run, as after a runtime
/// error. Under a watch session it ends the program's run instead, as `main` returning with
/// that status does.
#[no_mangle]
pub extern "C" fn prism_rt_exit(status: i64) -> ! {
    flush();
    stop_program(exit_status(status), true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_low_bits_of_the_status_are_kept() {
        assert_eq!(exit_status(7), 7);
        assert_eq!(exit_status(258), 2);
        assert_eq!(exit_status(-1), 255);
        assert_eq!(exit_status(256), 0);
    }

    /// Under watch mode an exit parks the program's thread and tells the session it exited.
    #[test]
    fn an_exit_under_watch_mode_parks_the_thread() {
        let _lock = crate::test_lock();
        let (sender, receiver) = std::sync::mpsc::channel();
        let sender = std::sync::Mutex::new(sender);
        crate::set_watch_mode(Some(std::sync::Arc::new(move |status, exited| {
            let _ = sender.lock().unwrap().send((status, exited));
        })));
        let thread = std::thread::spawn(|| prism_rt_exit(258));
        let stopped = receiver
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("the session is told");
        assert_eq!(stopped, (2, true));
        crate::set_watch_mode(None);
        std::thread::sleep(std::time::Duration::from_millis(20));
        assert!(!thread.is_finished(), "the thread stays parked");
    }
}

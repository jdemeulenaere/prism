//! Hosts (D15 and D16 of `ui-toolkit`): what an app gets its events from and presents its
//! frames to, and the process-wide configuration that says which one `runApp` uses.
//!
//! The CLI configures the host before the program runs, on its own thread; the program thread
//! reads the configuration when `runApp` starts. `desktop-window` adds a window host through a
//! [`WindowFactory`]; until then a run without `--headless` has none.
//!
//! A host also says how the app measures text ([`Host::measurer`]) and how many physical
//! pixels a logical one covers ([`Host::scale_factor`]), and the CLI may set a
//! [final-frame sink](set_final_frame_sink) that sees each app's last display list
//! (`desktop-window` D15 and D18).
//!
//! While an app runs, reloads wait for the reload point at the top of its frame loop, and a
//! request wakes the loop through [`wake_for_reload`]: it notifies the condition variable that
//! `wait reload` waits on, and calls the waker a window host registers with
//! [`set_reload_waker`] (D2 and D12 of `compose-hot-reload`).

use crate::draw::DisplayList;
use crate::headless::HeadlessHost;
pub use crate::input::Key;
pub use crate::script::DumpKind;
use crate::script::ScriptLine;
use crate::text::{FixedAdvanceMetrics, TextMeasurer};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// An event a host gives the frame loop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostEvent {
    Press(i64, i64),
    Release(i64, i64),
    Character(char),
    Key(Key),
    Resize(i64, i64),
    /// A frame is due: rebuild the display list if it is stale, and present it.
    Frame,
    Dump(DumpKind),
    /// A line of an event script, whose text targets the loop resolves against the tree.
    Script(ScriptLine),
}

/// What an app runs against: a window, or the headless script runner.
pub trait Host {
    /// The window's size in logical pixels.
    fn size(&self) -> (i64, i64);
    /// The next event, blocking until there is one; `None` ends the app.
    fn next_event(&mut self) -> Option<HostEvent>;
    /// Shows a frame.
    fn present(&mut self, list: &DisplayList);
    /// How the app measures text, taken once when the app starts (D12): the fixed-advance
    /// metric unless the host lays text out with a font of its own, as a window does.
    fn measurer(&self) -> Box<dyn TextMeasurer> {
        Box::new(FixedAdvanceMetrics)
    }
    /// How many physical pixels one logical pixel covers on each axis, which may be
    /// fractional: 1 unless the host draws on a scaled display.
    fn scale_factor(&self) -> f64 {
        1.0
    }
}

/// What sees each app's final display list, with the window's logical size and the host's
/// scale factor: `prism run --screenshot` (`desktop-window` D15).
pub type FinalFrameSink = Box<dyn FnMut(&DisplayList, (i64, i64), f64) + Send>;

static FINAL_FRAME_SINK: Mutex<Option<FinalFrameSink>> = Mutex::new(None);

/// Sets the sink every later app of the process calls when it ends, after its last event and
/// before its composition is dropped, or removes it.
pub fn set_final_frame_sink(sink: Option<FinalFrameSink>) {
    *FINAL_FRAME_SINK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = sink;
}

/// Calls the final-frame sink, if one is set. The sink is taken out while it runs, so a sink
/// that raises a runtime error under a watch session, which parks the thread, leaves no lock
/// held for the next run.
pub(crate) fn call_final_frame_sink(list: &DisplayList, size: (i64, i64), scale_factor: f64) {
    let lock = || {
        FINAL_FRAME_SINK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    };
    let taken = lock().take();
    if let Some(mut sink) = taken {
        sink(list, size, scale_factor);
        let mut slot = lock();
        if slot.is_none() {
            *slot = Some(sink);
        }
    }
}

/// Makes the window host of an app, from its title and size.
pub type WindowFactory = Arc<dyn Fn(&str, i64, i64) -> Box<dyn Host> + Send + Sync>;

/// The host `runApp` uses.
#[derive(Clone, Default)]
pub enum HostConfiguration {
    /// No host: `runApp` is `ui misuse: no window system is available; run with --headless`.
    #[default]
    None,
    /// `prism run --headless[=<w>x<h>] [--events <script>]`: the size overrides `runApp`'s.
    Headless {
        size: Option<(i64, i64)>,
        commands: Vec<ScriptLine>,
    },
    /// A window system (`desktop-window`).
    Window(WindowFactory),
}

impl std::fmt::Debug for HostConfiguration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HostConfiguration::None => f.write_str("None"),
            HostConfiguration::Headless { size, commands } => f
                .debug_struct("Headless")
                .field("size", size)
                .field("commands", commands)
                .finish(),
            HostConfiguration::Window(_) => f.write_str("Window"),
        }
    }
}

static CONFIGURATION: Mutex<HostConfiguration> = Mutex::new(HostConfiguration::None);

/// Sets the host every later `runApp` of the process uses.
pub fn configure(configuration: HostConfiguration) {
    *CONFIGURATION
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = configuration;
}

/// The configuration set last.
pub fn configuration() -> HostConfiguration {
    CONFIGURATION
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

/// The host of an app titled `title`, of `width` × `height` unless the configuration says
/// otherwise, or `None` when no host is configured.
pub fn configured_host(title: &str, width: i64, height: i64) -> Option<Box<dyn Host>> {
    match configuration() {
        HostConfiguration::None => None,
        HostConfiguration::Headless { size, commands } => Some(Box::new(HeadlessHost::new(
            size.unwrap_or((width, height)),
            commands,
        ))),
        HostConfiguration::Window(factory) => Some(factory(title, width, height)),
    }
}

static RELOAD_WAKER: Mutex<Option<fn()>> = Mutex::new(None);
static RELOAD_SIGNAL: (Mutex<()>, Condvar) = (Mutex::new(()), Condvar::new());
static RELOAD_TIMINGS: AtomicBool = AtomicBool::new(false);
/// How long `wait reload` waits, in milliseconds: 10 seconds unless a test shortens it.
static WAIT_FOR_RELOAD_MILLISECONDS: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(10_000);

/// Sets what wakes the running app's host when a reload becomes pending, or removes it: a
/// window's `WindowHost::wake`, which turns into a frame (`desktop-window` D17).
pub fn set_reload_waker(waker: Option<fn()>) {
    *RELOAD_WAKER
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = waker;
}

/// The wake-up `runApp` gives `prism_runtime::reload::defer_to_reload_points`, called on the
/// session's thread after a request: wakes `wait reload`, then the host's waker.
pub fn wake_for_reload() {
    {
        // Notified under the lock, so a waiter that found nothing requested is waiting already.
        let _guard = RELOAD_SIGNAL
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        RELOAD_SIGNAL.1.notify_all();
    }
    let waker = *RELOAD_WAKER
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(waker) = waker {
        waker();
    }
}

/// Waits until a reload is requested, at most the wait of `wait reload`; gives whether one is.
pub fn wait_for_reload() -> bool {
    let limit = Duration::from_millis(WAIT_FOR_RELOAD_MILLISECONDS.load(Ordering::SeqCst));
    let deadline = Instant::now() + limit;
    let mut guard = RELOAD_SIGNAL
        .0
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    loop {
        if prism_runtime::reload::is_requested() {
            return true;
        }
        let now = Instant::now();
        if now >= deadline {
            return false;
        }
        guard = RELOAD_SIGNAL
            .1
            .wait_timeout(guard, deadline - now)
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .0;
    }
}

/// Shortens the wait of `wait reload`, for tests.
pub fn set_wait_for_reload(limit: Duration) {
    WAIT_FOR_RELOAD_MILLISECONDS.store(limit.as_millis() as u64, Ordering::SeqCst);
}

/// Makes the frame loop print `timing: reload pass <ms> (<n> scopes)` after the passes that
/// follow each reload (`--timings` under `--watch`, D13 of `compose-hot-reload`).
pub fn set_reload_timings(on: bool) {
    RELOAD_TIMINGS.store(on, Ordering::SeqCst);
}

/// Whether reload passes are timed.
pub fn reload_timings() -> bool {
    RELOAD_TIMINGS.load(Ordering::SeqCst)
}

#[cfg(test)]
pub(crate) fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script::parse_script;

    #[test]
    fn a_configuration_set_on_one_thread_is_read_on_another() {
        let _lock = test_lock();
        let commands = parse_script("frame").unwrap();
        std::thread::spawn(move || {
            configure(HostConfiguration::Headless {
                size: Some((200, 100)),
                commands,
            })
        })
        .join()
        .unwrap();
        let host = std::thread::spawn(|| {
            let host = configured_host("Prism", 800, 600).unwrap();
            host.size()
        })
        .join()
        .unwrap();
        assert_eq!(host, (200, 100));
        configure(HostConfiguration::Headless {
            size: None,
            commands: Vec::new(),
        });
        assert_eq!(
            configured_host("Prism", 800, 600).unwrap().size(),
            (800, 600)
        );
        let factory: WindowFactory = Arc::new(|_, width, height| {
            Box::new(HeadlessHost::new((width * 2, height * 2), Vec::new()))
        });
        configure(HostConfiguration::Window(factory));
        assert_eq!(configured_host("Prism", 10, 20).unwrap().size(), (20, 40));
        configure(HostConfiguration::None);
        assert!(configured_host("Prism", 800, 600).is_none());
    }
}

//! Hosts (D15 and D16 of `ui-toolkit`): what an app gets its events from and presents its
//! frames to, and the process-wide configuration that says which one `runApp` uses.
//!
//! The CLI configures the host before the program runs, on its own thread; the program thread
//! reads the configuration when `runApp` starts. `desktop-window` adds a window host through a
//! [`WindowFactory`]; until then a run without `--headless` has none.

use crate::draw::DisplayList;
use crate::headless::HeadlessHost;
pub use crate::input::Key;
pub use crate::script::DumpKind;
use crate::script::ScriptLine;
use std::sync::{Arc, Mutex};

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

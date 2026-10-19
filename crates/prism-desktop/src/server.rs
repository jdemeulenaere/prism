//! The main-thread server (D3, D4 and D17 of `desktop-window`): the program keeps its own
//! thread, and the process's main thread serves its windows.
//!
//! ```text
//! let (server, windows) = MainThreadServer::new();      // channels only
//! prism_ui::host::configure(HostConfiguration::Window(windows.factory()));
//! let running = program.start(main, arguments, check, move |_| windows.program_ended())?;
//! server.serve();                                       // returns once the program ended
//! running.join()
//! ```
//!
//! - [`MainThreadServer::serve`] blocks on its request channel. An `OpenWindow` request
//!   creates the event loop if there is none yet (lazily, D4), and runs it with
//!   `run_app_on_demand` until that window closes; `Shutdown` returns. The event loop is kept
//!   for the process, since `winit` cannot create a second one, and every later window reuses
//!   it.
//! - [`WindowSystem`] is the program thread's side: its [`factory`](WindowSystem::factory)
//!   makes the window hosts of `runApp`. Opening sends `OpenWindow`, parses the bundled font
//!   while the main thread creates the window, and waits for the reply; an error reply ends the
//!   program with `display error: cannot open a window (<reason>)`.
//! - [`WindowSystem::close_all`] closes the open window, for a watch session whose program
//!   stopped without dropping its host; [`WindowSystem::program_ended`] also shuts the server
//!   down.
//!
//! A program that never opens a window costs one channel: no event loop, no display
//! connection and no font.

use crate::error::{cannot_open_a_window, raise_display_error};
use crate::font;
use crate::host::{FrameSink, UserEvent, WindowHost};
use crate::input::WindowInput;
use crate::statistics::WindowStatistics;
use crate::window::{WindowApp, WindowRequest};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use winit::event_loop::EventLoop;
use winit::platform::run_on_demand::EventLoopExtRunOnDemand;

/// What the program thread asks the main thread.
enum Request {
    OpenWindow(WindowRequest),
    Shutdown,
}

/// Creates the event loop, or says why it cannot.
pub type EventLoopStarter = Box<dyn FnMut() -> Result<EventLoop<UserEvent>, String>>;

/// What both threads share: the open window's sink, for [`WindowSystem::close_all`].
#[derive(Default)]
pub(crate) struct Shared {
    open_window: Mutex<Option<Arc<dyn FrameSink>>>,
}

impl Shared {
    fn open_window(&self) -> std::sync::MutexGuard<'_, Option<Arc<dyn FrameSink>>> {
        self.open_window
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(crate) fn window_opened(&self, sink: Arc<dyn FrameSink>) {
        *self.open_window() = Some(sink);
    }

    pub(crate) fn window_closed(&self) {
        *self.open_window() = None;
    }
}

/// The event loop `winit` makes, without macOS's default menu, whose Quit item would end the
/// process without flushing the program's output (D14).
pub fn default_starter() -> EventLoopStarter {
    Box::new(|| {
        let mut builder = EventLoop::<UserEvent>::with_user_event();
        #[cfg(target_os = "macos")]
        {
            use winit::platform::macos::EventLoopBuilderExtMacOS;
            builder.with_default_menu(false);
        }
        builder.build().map_err(|error| error.to_string())
    })
}

/// The main thread's side.
pub struct MainThreadServer {
    requests: Receiver<Request>,
    starter: EventLoopStarter,
    event_loop: Option<EventLoop<UserEvent>>,
    windows: u64,
    statistics: Arc<WindowStatistics>,
    shared: Arc<Shared>,
}

/// The program thread's side.
#[derive(Clone)]
pub struct WindowSystem {
    requests: Sender<Request>,
    statistics: Arc<WindowStatistics>,
    shared: Arc<Shared>,
}

impl MainThreadServer {
    /// A server and its window system, with `winit`'s event loop.
    pub fn new() -> (MainThreadServer, WindowSystem) {
        MainThreadServer::with_starter(default_starter())
    }

    /// A server whose event loop `starter` creates, so tests can fake one that fails.
    pub fn with_starter(starter: EventLoopStarter) -> (MainThreadServer, WindowSystem) {
        let (sender, receiver) = channel();
        let statistics = Arc::new(WindowStatistics::new());
        let shared = Arc::new(Shared::default());
        let server = MainThreadServer {
            requests: receiver,
            starter,
            event_loop: None,
            windows: 0,
            statistics: statistics.clone(),
            shared: shared.clone(),
        };
        let system = WindowSystem {
            requests: sender,
            statistics,
            shared,
        };
        (server, system)
    }

    pub fn statistics(&self) -> Arc<WindowStatistics> {
        self.statistics.clone()
    }

    /// Serves windows until the program ends or the window system shuts down.
    pub fn serve(mut self) {
        loop {
            match self.requests.recv() {
                Ok(Request::OpenWindow(request)) => self.open(request),
                Ok(Request::Shutdown) | Err(_) => return,
            }
        }
    }

    /// Runs the event loop for one window until it closes.
    fn open(&mut self, request: WindowRequest) {
        if self.event_loop.is_none() {
            match (self.starter)() {
                Ok(event_loop) => {
                    self.statistics.event_loop_created();
                    self.event_loop = Some(event_loop);
                }
                Err(reason) => {
                    let _ = request.reply.send(Err(reason));
                    return;
                }
            }
        }
        let event_loop = self.event_loop.as_mut().expect("an event loop");
        self.windows += 1;
        let mut app = WindowApp::new(
            request,
            self.windows,
            event_loop.create_proxy(),
            self.statistics.clone(),
            self.shared.clone(),
        );
        if let Err(error) = event_loop.run_app_on_demand(&mut app) {
            app.fail(error.to_string());
        }
        self.shared.window_closed();
    }
}

impl WindowSystem {
    pub fn statistics(&self) -> Arc<WindowStatistics> {
        self.statistics.clone()
    }

    /// The factory of `HostConfiguration::Window`.
    pub fn factory(&self) -> prism_ui::host::WindowFactory {
        let system = self.clone();
        Arc::new(move |title, width, height| Box::new(system.open(title, width, height)))
    }

    /// Opens a window for an app, or ends the program with a display error.
    pub fn open(&self, title: &str, width: i64, height: i64) -> WindowHost {
        match self.try_open(title, width, height) {
            Ok(host) => host,
            Err(reason) => raise_display_error(&cannot_open_a_window(&reason)),
        }
    }

    /// Opens a window for an app, or says why it cannot.
    pub fn try_open(&self, title: &str, width: i64, height: i64) -> Result<WindowHost, String> {
        let (reply, replies) = channel();
        self.requests
            .send(Request::OpenWindow(WindowRequest {
                title: title.to_string(),
                width,
                height,
                reply,
            }))
            .map_err(|_| "the window system has shut down".to_string())?;
        // The main thread creates the window meanwhile (D3, D6).
        font::font();
        self.statistics.font_was_loaded();
        let connection = replies
            .recv()
            .map_err(|_| "the window system stopped".to_string())??;
        Ok(WindowHost::new(connection))
    }

    /// Delivers `input` to the open window, which handles it as it handles the platform's
    /// events, through the same input mapping (D13) and closing (D14); whether a window was
    /// open. Tests drive real windows with it where no person can click or type.
    pub fn deliver(&self, input: WindowInput) -> bool {
        let sink = self.shared.open_window().clone();
        sink.map(|sink| sink.deliver(input)).is_some()
    }

    /// Asks the platform to resize the open window to `width` × `height` logical pixels; the
    /// window then gets the platform's `Resized` event. Whether a window was open.
    pub fn request_inner_size(&self, width: u32, height: u32) -> bool {
        let sink = self.shared.open_window().clone();
        sink.map(|sink| sink.request_inner_size(width, height))
            .is_some()
    }

    /// Closes the open window, if any.
    pub fn close_all(&self) {
        let sink = self.shared.open_window().take();
        if let Some(sink) = sink {
            sink.close_window();
        }
    }

    /// The program has ended: close what it left open and stop serving.
    pub fn program_ended(&self) {
        self.close_all();
        self.shut_down();
    }

    /// Stops serving once the current window, if any, has closed.
    pub fn shut_down(&self) {
        let _ = self.requests.send(Request::Shutdown);
    }
}

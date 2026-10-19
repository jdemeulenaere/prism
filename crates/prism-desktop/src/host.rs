//! The window host (D10, D11 and D17 of `desktop-window`): `prism_ui::Host` for a window, on
//! the program thread, talking to the main thread through channels and a frame mailbox.
//!
//! - **Input** arrives as [`WindowMessage`]s on a channel. [`WindowHost::next_event`] returns
//!   queued input first and notes that a frame is due after each event; when the queue is
//!   empty and a frame is due, it returns `HostEvent::Frame`; otherwise it blocks. A burst of
//!   events therefore gives one pass per event, as `ui-toolkit`'s loop runs them, and one
//!   frame at the end. The first call gives a frame, so the window gets its first picture.
//! - **Frames** cross in a [`FrameMailbox`] that holds the latest display list: `present`
//!   copies the list in with `clone_from`, which reuses the mailbox's buffers, and tells the
//!   main thread through a [`FrameSink`]. The main thread swaps the mailbox's list with its own
//!   when it redraws, so frames presented faster than the display redraws overwrite each
//!   other, and nothing is queued.
//! - **Closing.** A `Close` message ends the app (`None`). Frames presented after the window
//!   closed are dropped. Dropping the host, when `runApp` returns for any reason, asks the
//!   main thread to close the window, so no window outlives its app.
//! - **Reloads.** [`WindowHost::wake`] is a plain `fn()` that turns into a frame, so the frame
//!   loop reaches its top; `compose-hot-reload` registers it with `defer_to_reload_points`.

use crate::metrics::FontMetrics;
use prism_ui::draw::DisplayList;
use prism_ui::host::{Host, HostEvent};
use prism_ui::text::TextMeasurer;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};

/// What the main thread sends the program thread.
#[derive(Debug, Clone, PartialEq)]
pub enum WindowMessage {
    /// A press, release, character, key or resize for the app.
    Input(HostEvent),
    /// The window closed: the app ends.
    Close,
    /// A frame is wanted although nothing was delivered: a reload is pending (D17).
    Wake,
}

/// The events the program thread sends the event loop, tagged with the window they are for,
/// so that one meant for a window that has already closed is ignored by the next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserEvent {
    FrameReady(u64),
    CloseWindow(u64),
}

/// How the program thread reaches the event loop: through its proxy in a window, and a fake in
/// tests.
pub trait FrameSink: Send + Sync {
    /// A new frame is in the mailbox.
    fn frame_ready(&self);
    /// The app has ended: close the window.
    fn close_window(&self);
}

#[derive(Debug, Default)]
struct MailboxState {
    list: DisplayList,
    sequence: u64,
    closed: bool,
}

/// The latest display list the program presented, and the window's scale factor.
#[derive(Debug)]
pub struct FrameMailbox {
    state: Mutex<MailboxState>,
    scale_factor: AtomicU64,
}

impl FrameMailbox {
    pub fn new(scale_factor: f64) -> FrameMailbox {
        FrameMailbox {
            state: Mutex::new(MailboxState::default()),
            scale_factor: AtomicU64::new(scale_factor.to_bits()),
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, MailboxState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Copies `list` in as the latest frame, unless the window has closed; whether it did.
    pub fn put(&self, list: &DisplayList) -> bool {
        let mut state = self.state();
        if state.closed {
            return false;
        }
        state.list.commands.clone_from(&list.commands);
        state.list.text.clone_from(&list.text);
        state.sequence += 1;
        true
    }

    /// Swaps the latest frame into `current` when it is newer than `seen`, the sequence of the
    /// frame taken last, and gives whether it was; otherwise the last pixels can be presented
    /// again.
    pub fn take(&self, current: &mut DisplayList, seen: &mut u64) -> bool {
        let mut state = self.state();
        if state.sequence == *seen {
            return false;
        }
        *seen = state.sequence;
        std::mem::swap(&mut state.list, current);
        true
    }

    /// Marks the window closed: later frames are dropped.
    pub fn close(&self) {
        self.state().closed = true;
    }

    pub fn is_closed(&self) -> bool {
        self.state().closed
    }

    pub fn scale_factor(&self) -> f64 {
        f64::from_bits(self.scale_factor.load(Ordering::Relaxed))
    }

    pub fn set_scale_factor(&self, scale_factor: f64) {
        self.scale_factor
            .store(scale_factor.to_bits(), Ordering::Relaxed);
    }
}

/// The program thread's end of an open window.
pub struct WindowConnection {
    /// The window's inner size in logical pixels when it opened.
    pub size: (i64, i64),
    pub input: Receiver<WindowMessage>,
    /// A sender of the input channel, for [`WindowHost::wake`].
    pub wake: Sender<WindowMessage>,
    pub mailbox: Arc<FrameMailbox>,
    pub sink: Arc<dyn FrameSink>,
}

impl std::fmt::Debug for WindowConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WindowConnection")
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}

/// The main thread's end of an open window.
#[derive(Debug, Clone)]
pub struct WindowEndpoint {
    pub input: Sender<WindowMessage>,
    pub mailbox: Arc<FrameMailbox>,
}

/// The two ends of a window of logical `size` at `scale_factor`, whose frames are announced to
/// `sink`.
pub fn connect(
    size: (i64, i64),
    scale_factor: f64,
    sink: Arc<dyn FrameSink>,
) -> (WindowConnection, WindowEndpoint) {
    let (sender, receiver) = channel();
    let mailbox = Arc::new(FrameMailbox::new(scale_factor));
    let connection = WindowConnection {
        size,
        input: receiver,
        wake: sender.clone(),
        mailbox: mailbox.clone(),
        sink,
    };
    (
        connection,
        WindowEndpoint {
            input: sender,
            mailbox,
        },
    )
}

/// The sender [`WindowHost::wake`] uses: the open window's input channel.
static WAKE: Mutex<Option<Sender<WindowMessage>>> = Mutex::new(None);

fn wake_sender() -> std::sync::MutexGuard<'static, Option<Sender<WindowMessage>>> {
    WAKE.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// `prism_ui::Host` for a window.
pub struct WindowHost {
    size: (i64, i64),
    input: Receiver<WindowMessage>,
    mailbox: Arc<FrameMailbox>,
    sink: Arc<dyn FrameSink>,
    frame_due: bool,
}

impl std::fmt::Debug for WindowHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WindowHost")
            .field("size", &self.size)
            .field("frame_due", &self.frame_due)
            .finish_non_exhaustive()
    }
}

impl WindowHost {
    pub fn new(connection: WindowConnection) -> WindowHost {
        *wake_sender() = Some(connection.wake);
        WindowHost {
            size: connection.size,
            input: connection.input,
            mailbox: connection.mailbox,
            sink: connection.sink,
            frame_due: true,
        }
    }

    /// Asks the open window's app for a frame; nothing when no window is open. A plain `fn()`,
    /// as `prism_runtime::reload::defer_to_reload_points` takes (D17).
    pub fn wake() {
        if let Some(sender) = wake_sender().as_ref() {
            let _ = sender.send(WindowMessage::Wake);
        }
    }

    /// The event a message gives, or `None` for a wake-up, which only makes a frame due.
    fn receive(&mut self, message: WindowMessage) -> Option<Option<HostEvent>> {
        match message {
            WindowMessage::Input(event) => {
                if let HostEvent::Resize(width, height) = event {
                    self.size = (width, height);
                }
                self.frame_due = true;
                Some(Some(event))
            }
            WindowMessage::Close => Some(None),
            WindowMessage::Wake => {
                self.frame_due = true;
                None
            }
        }
    }
}

impl Host for WindowHost {
    fn size(&self) -> (i64, i64) {
        self.size
    }

    fn next_event(&mut self) -> Option<HostEvent> {
        loop {
            let message = match self.input.try_recv() {
                Ok(message) => message,
                Err(TryRecvError::Disconnected) => return None,
                Err(TryRecvError::Empty) if self.frame_due => {
                    self.frame_due = false;
                    return Some(HostEvent::Frame);
                }
                Err(TryRecvError::Empty) => match self.input.recv() {
                    Ok(message) => message,
                    Err(_) => return None,
                },
            };
            if let Some(event) = self.receive(message) {
                return event;
            }
        }
    }

    fn present(&mut self, list: &DisplayList) {
        if self.mailbox.put(list) {
            self.sink.frame_ready();
        }
    }

    fn measurer(&self) -> Box<dyn TextMeasurer> {
        Box::new(FontMetrics::new())
    }

    fn scale_factor(&self) -> f64 {
        self.mailbox.scale_factor()
    }
}

impl Drop for WindowHost {
    fn drop(&mut self) {
        *wake_sender() = None;
        self.sink.close_window();
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::test_support::ListBuilder;
    use prism_ui::host::Key;
    use std::sync::atomic::AtomicUsize;

    /// A sink that counts what it is told.
    #[derive(Debug, Default)]
    pub struct CountingSink {
        pub frames: AtomicUsize,
        pub closes: AtomicUsize,
    }

    impl FrameSink for CountingSink {
        fn frame_ready(&self) {
            self.frames.fetch_add(1, Ordering::SeqCst);
        }

        fn close_window(&self) {
            self.closes.fetch_add(1, Ordering::SeqCst);
        }
    }

    pub fn host(size: (i64, i64)) -> (WindowHost, WindowEndpoint, Arc<CountingSink>) {
        let sink = Arc::new(CountingSink::default());
        let (connection, endpoint) = connect(size, 2.0, sink.clone());
        (WindowHost::new(connection), endpoint, sink)
    }

    fn input(endpoint: &WindowEndpoint, event: HostEvent) {
        endpoint.input.send(WindowMessage::Input(event)).unwrap();
    }

    /// The scenario "Events in a burst give one frame", and the first frame of a window.
    #[test]
    fn events_in_a_burst_give_one_frame() {
        let _lock = crate::test_lock();
        let (mut host, endpoint, _) = host((400, 300));
        assert_eq!(host.next_event(), Some(HostEvent::Frame), "the first frame");
        for event in [
            HostEvent::Press(1, 2),
            HostEvent::Release(1, 2),
            HostEvent::Press(3, 4),
            HostEvent::Release(3, 4),
        ] {
            input(&endpoint, event);
        }
        let events: Vec<HostEvent> = (0..5).map(|_| host.next_event().unwrap()).collect();
        assert_eq!(
            events,
            [
                HostEvent::Press(1, 2),
                HostEvent::Release(1, 2),
                HostEvent::Press(3, 4),
                HostEvent::Release(3, 4),
                HostEvent::Frame
            ]
        );
        input(&endpoint, HostEvent::Key(Key::Enter));
        endpoint.input.send(WindowMessage::Close).unwrap();
        assert_eq!(host.next_event(), Some(HostEvent::Key(Key::Enter)));
        assert_eq!(host.next_event(), None, "a close ends the app");
    }

    #[test]
    fn a_resize_updates_the_size() {
        let _lock = crate::test_lock();
        let (mut host, endpoint, _) = host((400, 300));
        assert_eq!(host.size(), (400, 300));
        assert_eq!(host.scale_factor(), 2.0);
        input(&endpoint, HostEvent::Resize(200, 100));
        assert_eq!(
            host.next_event(),
            Some(HostEvent::Resize(200, 100)),
            "input first"
        );
        assert_eq!(host.next_event(), Some(HostEvent::Frame));
        assert_eq!(host.size(), (200, 100));
        endpoint.mailbox.set_scale_factor(1.0);
        assert_eq!(host.scale_factor(), 1.0);
    }

    #[test]
    fn a_wake_gives_a_frame() {
        let _lock = crate::test_lock();
        let (mut host, endpoint, _) = host((400, 300));
        assert_eq!(host.next_event(), Some(HostEvent::Frame));
        WindowHost::wake();
        assert_eq!(host.next_event(), Some(HostEvent::Frame));
        drop(host);
        WindowHost::wake();
        assert!(endpoint.input.send(WindowMessage::Wake).is_err(), "closed");
    }

    #[test]
    fn presents_reach_the_mailbox_until_the_window_closes() {
        let _lock = crate::test_lock();
        let (mut host, endpoint, sink) = host((40, 30));
        let list = ListBuilder::new().clear(40, 30, 0xFFFF_FFFF).build();
        host.present(&list);
        assert_eq!(sink.frames.load(Ordering::SeqCst), 1);
        endpoint.mailbox.close();
        host.present(&list);
        assert_eq!(
            sink.frames.load(Ordering::SeqCst),
            1,
            "a present after close"
        );
        let (mut current, mut seen) = (DisplayList::new(), 0);
        assert!(endpoint.mailbox.take(&mut current, &mut seen));
        assert!(!endpoint.mailbox.take(&mut current, &mut seen));
        drop(host);
        assert_eq!(sink.closes.load(Ordering::SeqCst), 1, "dropping closes");
    }

    /// The scenario "Frames faster than redraws": three presents before one take give the
    /// third list, once.
    #[test]
    fn frames_faster_than_redraws() {
        let _lock = crate::test_lock();
        let (mut host, endpoint, sink) = host((40, 30));
        for width in [10, 20, 30] {
            let list = ListBuilder::new()
                .clear(40, 30, 0xFFFF_FFFF)
                .rect((0, 0, width, 5), 0xFFFF_0000, 0)
                .build();
            host.present(&list);
        }
        assert_eq!(sink.frames.load(Ordering::SeqCst), 3);
        let (mut current, mut seen) = (DisplayList::new(), 0);
        assert!(endpoint.mailbox.take(&mut current, &mut seen));
        let third = ListBuilder::new()
            .clear(40, 30, 0xFFFF_FFFF)
            .rect((0, 0, 30, 5), 0xFFFF_0000, 0)
            .build();
        assert_eq!(current.commands, third.commands);
        assert!(
            !endpoint.mailbox.take(&mut current, &mut seen),
            "nothing new: present the last pixels again"
        );
        assert_eq!(current.commands, third.commands);
    }
}

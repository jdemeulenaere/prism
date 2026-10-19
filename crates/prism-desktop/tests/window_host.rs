//! The window host's protocol with a running app, without a window (D11 and D20 of
//! `desktop-window`): a `prism-ui` app built from Rust runs against a [`WindowHost`] on its own
//! thread, as on `prism run`'s program thread, while the test plays the main thread through
//! the window's channel and mailbox.

use prism_compose::test_support::content;
use prism_desktop::host::{connect, FrameSink, WindowEndpoint, WindowMessage};
use prism_desktop::WindowHost;
use prism_ui::draw::DisplayList;
use prism_ui::dump::dump_display_list;
use prism_ui::modifier::ModifierMirror;
use prism_ui::properties::Shape;
use prism_ui::test_support::{background, fill_max};
use prism_ui::{setters, Host, HostEvent, NodeId};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Serializes the tests: the wake-up sender and the reload policy are the process's.
fn test_lock() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

struct NoSink;

impl FrameSink for NoSink {
    fn frame_ready(&self) {}
    fn close_window(&self) {}
}

/// Waits for the next frame in the mailbox and dumps it.
fn next_frame(endpoint: &WindowEndpoint, current: &mut DisplayList, seen: &mut u64) -> String {
    let start = Instant::now();
    while !endpoint.mailbox.take(current, seen) {
        assert!(start.elapsed() < Duration::from_secs(5), "no frame");
        std::thread::sleep(Duration::from_millis(1));
    }
    let mut dump = String::new();
    dump_display_list(current, &mut dump);
    dump
}

/// The scenario "A resize relays out the app": `Box(Modifier.fillMaxSize().background(Red))`
/// follows the window's size.
#[test]
fn a_resize_relays_out_the_app() {
    let _lock = test_lock();
    let (connection, endpoint) = connect((400, 300), 1.0, Arc::new(NoSink));
    let app = std::thread::spawn(move || {
        let host = WindowHost::new(connection);
        let measurer = host.measurer();
        let content = content(|k| {
            let node = NodeId(k.start_node_group(1, 3).unwrap().0);
            let chain = ModifierMirror::encode(&[
                fill_max(true, true, 1.0),
                background(0xFFFF_0000, Shape::RECTANGLE),
            ]);
            setters::set_modifier(node, &chain).unwrap();
            k.end_group().unwrap();
        });
        prism_ui::app::run_app_with(Box::new(host), measurer, content)
    });
    let (mut current, mut seen) = (DisplayList::new(), 0);
    assert_eq!(
        next_frame(&endpoint, &mut current, &mut seen),
        "clear 400x300 #FFFFFFFF\nrect 0,0 400x300 #FFFF0000\n",
        "the first frame"
    );
    endpoint
        .input
        .send(WindowMessage::Input(HostEvent::Resize(200, 100)))
        .unwrap();
    assert_eq!(
        next_frame(&endpoint, &mut current, &mut seen),
        "clear 200x100 #FFFFFFFF\nrect 0,0 200x100 #FFFF0000\n"
    );
    endpoint.input.send(WindowMessage::Close).unwrap();
    let counters = app.join().unwrap().unwrap();
    assert_eq!(counters.events, 3, "frame, resize, frame");
    assert_eq!(counters.passes, 1, "a resize needs no pass");
    assert!(endpoint.mailbox.is_closed() || !endpoint.mailbox.take(&mut current, &mut seen));
}

static RELOADED: AtomicBool = AtomicBool::new(false);

/// The scenario "A reload is presented without input": an idle app whose only message after a
/// reload request is the reload's wake-up presents a frame of the new code. The reload's code
/// switches the text the app shows from `old code` to `new code`.
#[test]
fn a_reload_is_presented_without_input() {
    let _lock = test_lock();
    prism_runtime::reload::set_applied_hook(Some(prism_compose::reload::reload_applied));
    let (connection, endpoint) = connect((400, 300), 1.0, Arc::new(NoSink));
    let app = std::thread::spawn(move || {
        let host = WindowHost::new(connection);
        let measurer = host.measurer();
        let content = content(|k| {
            let column = NodeId(k.start_node_group(1, 1).unwrap().0);
            let text = NodeId(k.start_node_group(2, 4).unwrap().0);
            let label = match RELOADED.load(Ordering::SeqCst) {
                true => "new code",
                false => "old code",
            };
            setters::set_text(text, label).unwrap();
            k.end_group().unwrap();
            setters::set_modifier(column, &ModifierMirror::encode(&[])).unwrap();
            k.end_group().unwrap();
        });
        prism_ui::app::run_app_with(Box::new(host), measurer, content)
    });
    let (mut current, mut seen) = (DisplayList::new(), 0);
    let first = next_frame(&endpoint, &mut current, &mut seen);
    assert!(first.contains("\"old code\""), "{first}");
    // The app idles in its host, and the session requests a reload on its own thread, which
    // wakes the app: the window sends nothing else.
    prism_runtime::reload::request(prism_runtime::reload::PendingReload::new(|| {
        RELOADED.store(true, Ordering::SeqCst);
        prism_runtime::reload::AppliedReload {
            // The column's node group, under the root.
            composable_bodies: vec![1],
            ..Default::default()
        }
    }));
    let reloaded = next_frame(&endpoint, &mut current, &mut seen);
    assert!(reloaded.contains("\"new code\""), "{reloaded}");
    endpoint.input.send(WindowMessage::Close).unwrap();
    let counters = app.join().unwrap().unwrap();
    assert_eq!(counters.events, 3, "frame, wake-up, frame");
    assert_eq!(counters.passes, 2, "the first pass and the reload's");
    assert_eq!(counters.builds, 2, "one display list per code");
    prism_runtime::reload::set_applied_hook(None);
}

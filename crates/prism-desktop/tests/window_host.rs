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
use std::sync::Arc;
use std::time::{Duration, Instant};

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

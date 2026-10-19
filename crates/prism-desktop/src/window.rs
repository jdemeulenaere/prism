//! A window on the main thread (D9 to D14 of `desktop-window`): `winit`'s handler for one app,
//! run by [`crate::server::MainThreadServer`] with `run_app_on_demand` until the window closes.
//!
//! - **Creation** (D14). `resumed` creates the window, visible at once, with `runApp`'s title
//!   and logical inner size, then the `softbuffer` surface, and replies to the program thread
//!   with its connection, or with the error. Until the first frame arrives, a redraw draws
//!   white.
//! - **Input** goes through [`crate::input`] and crosses to the program thread on the
//!   window's channel.
//! - **Drawing** (D10, D11). A frame from the program thread, a resize, a scale change or the
//!   platform asks for a redraw; on `RedrawRequested` the window rasterizes only when the frame,
//!   the size or the scale factor changed since it last did, and otherwise presents its last
//!   pixels again. The control flow stays `Wait`: no timer, no polling.
//! - **Closing** (D14). The close button, Cmd+W or Cmd+Q, and the program dropping its host
//!   all close the window the same way: the mailbox is marked closed, the program thread gets
//!   `Close`, the surface and the window are dropped at once, and the event loop exits.

use crate::host::{connect, FrameSink, UserEvent, WindowConnection, WindowEndpoint, WindowMessage};
use crate::input::{typed_characters, InputState, KeyAction, Platform, WindowInput};
use crate::raster::Rasterizer;
use crate::server::Shared;
use crate::statistics::{WindowReport, WindowStatistics};
use prism_ui::draw::DisplayList;
use std::num::NonZeroU32;
use std::rc::Rc;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoopProxy};
use winit::window::{Window, WindowId};

/// A request to open a window, and where to reply.
pub struct WindowRequest {
    pub title: String,
    pub width: i64,
    pub height: i64,
    pub reply: Sender<Result<WindowConnection, String>>,
}

/// The program thread's way to the event loop: its proxy, with the window's number.
pub(crate) struct ProxySink {
    proxy: Mutex<EventLoopProxy<UserEvent>>,
    number: u64,
}

impl FrameSink for ProxySink {
    fn frame_ready(&self) {
        let proxy = self.proxy.lock().unwrap_or_else(|p| p.into_inner());
        let _ = proxy.send_event(UserEvent::FrameReady(self.number));
    }

    fn close_window(&self) {
        let proxy = self.proxy.lock().unwrap_or_else(|p| p.into_inner());
        let _ = proxy.send_event(UserEvent::CloseWindow(self.number));
    }

    fn deliver(&self, input: WindowInput) {
        let proxy = self.proxy.lock().unwrap_or_else(|p| p.into_inner());
        let _ = proxy.send_event(UserEvent::Input(self.number, input));
    }

    fn request_inner_size(&self, width: u32, height: u32) {
        let proxy = self.proxy.lock().unwrap_or_else(|p| p.into_inner());
        let _ = proxy.send_event(UserEvent::RequestInnerSize(self.number, width, height));
    }
}

/// The window's surface and what draws into it.
struct Surface {
    // Field order is drop order: the surface before the context, both before the window.
    surface: softbuffer::Surface<Rc<Window>, Rc<Window>>,
    _context: softbuffer::Context<Rc<Window>>,
    window: Rc<Window>,
    size: Option<(u32, u32)>,
}

/// The handler of one window's run of the event loop.
pub(crate) struct WindowApp {
    request: Option<WindowRequest>,
    number: u64,
    proxy: EventLoopProxy<UserEvent>,
    statistics: Arc<WindowStatistics>,
    shared: Arc<Shared>,
    surface: Option<Surface>,
    endpoint: Option<WindowEndpoint>,
    input: InputState,
    rasterizer: Rasterizer,
    current: DisplayList,
    seen: u64,
    /// The physical size and scale of the last rasterization.
    drawn: Option<(u32, u32, f64)>,
}

impl WindowApp {
    pub(crate) fn new(
        request: WindowRequest,
        number: u64,
        proxy: EventLoopProxy<UserEvent>,
        statistics: Arc<WindowStatistics>,
        shared: Arc<Shared>,
    ) -> WindowApp {
        WindowApp {
            request: Some(request),
            number,
            proxy,
            statistics,
            shared,
            surface: None,
            endpoint: None,
            input: InputState::new(1.0, (0, 0)),
            rasterizer: Rasterizer::new(),
            current: DisplayList::new(),
            seen: 0,
            drawn: None,
        }
    }

    /// Replies with `error` if the window was never opened.
    pub(crate) fn fail(&mut self, error: String) {
        if let Some(request) = self.request.take() {
            let _ = request.reply.send(Err(error));
        }
    }

    fn open(&mut self, event_loop: &ActiveEventLoop) -> Result<(), String> {
        let Some(request) = self.request.take() else {
            return Ok(());
        };
        let attributes = Window::default_attributes()
            .with_title(request.title.clone())
            .with_inner_size(LogicalSize::new(
                request.width.max(1) as f64,
                request.height.max(1) as f64,
            ));
        let created = (|| {
            let window = Rc::new(
                event_loop
                    .create_window(attributes)
                    .map_err(|error| error.to_string())?,
            );
            let context =
                softbuffer::Context::new(window.clone()).map_err(|error| error.to_string())?;
            let surface = softbuffer::Surface::new(&context, window.clone())
                .map_err(|error| error.to_string())?;
            Ok::<_, String>(Surface {
                surface,
                _context: context,
                window,
                size: None,
            })
        })();
        let surface = match created {
            Ok(surface) => surface,
            Err(error) => {
                let _ = request.reply.send(Err(error.clone()));
                return Err(error);
            }
        };
        let window = &surface.window;
        let scale = window.scale_factor();
        let physical = window.inner_size();
        let logical = physical.to_logical::<f64>(scale);
        let size = InputState::new(scale, (0, 0)).logical_size_of(physical.width, physical.height);
        self.input = InputState::new(scale, size);
        self.statistics.window_opened(WindowReport {
            title: window.title(),
            logical_size: (logical.width, logical.height),
            scale_factor: scale,
        });
        let sink: Arc<dyn FrameSink> = Arc::new(ProxySink {
            proxy: Mutex::new(self.proxy.clone()),
            number: self.number,
        });
        let (connection, endpoint) = connect(size, scale, sink.clone());
        self.shared.window_opened(sink);
        self.endpoint = Some(endpoint);
        self.surface = Some(surface);
        if request.reply.send(Ok(connection)).is_err() {
            return Err("the program stopped waiting for its window".to_string());
        }
        Ok(())
    }

    fn send(&self, message: WindowMessage) {
        if let Some(endpoint) = &self.endpoint {
            let _ = endpoint.input.send(message);
        }
    }

    fn request_redraw(&self) {
        if let Some(surface) = &self.surface {
            surface.window.request_redraw();
        }
    }

    fn close(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(endpoint) = self.endpoint.take() {
            endpoint.mailbox.close();
            let _ = endpoint.input.send(WindowMessage::Close);
        }
        self.shared.window_closed();
        if self.surface.take().is_some() {
            self.statistics.window_closed();
        }
        event_loop.exit();
    }

    /// Draws on the platform's redraw callback.
    fn draw(&mut self) {
        let (Some(surface), Some(endpoint)) = (&mut self.surface, &self.endpoint) else {
            return;
        };
        let physical = surface.window.inner_size();
        let (Some(width), Some(height)) = (
            NonZeroU32::new(physical.width),
            NonZeroU32::new(physical.height),
        ) else {
            return;
        };
        let start = Instant::now();
        let scale = self.input.scale();
        let new_frame = endpoint.mailbox.take(&mut self.current, &mut self.seen);
        let target = (width.get(), height.get(), scale);
        if new_frame || self.drawn != Some(target) {
            self.rasterizer
                .rasterize(&self.current, scale, width.get(), height.get());
            self.drawn = Some(target);
            self.statistics.frame_rasterized();
        }
        if surface.size != Some((width.get(), height.get())) {
            if surface.surface.resize(width, height).is_err() {
                return;
            }
            surface.size = Some((width.get(), height.get()));
        }
        let Ok(mut buffer) = surface.surface.buffer_mut() else {
            return;
        };
        self.rasterizer.copy_to_surface(&mut buffer);
        surface.window.pre_present_notify();
        if buffer.present().is_ok() {
            self.statistics.frame_presented(start.elapsed(), new_frame);
        }
    }
}

impl ApplicationHandler<UserEvent> for WindowApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.request.is_some() && self.open(event_loop).is_err() {
            self.close(event_loop);
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::FrameReady(number) if number == self.number => {
                self.statistics.frame_received();
                self.request_redraw();
            }
            UserEvent::CloseWindow(number) if number == self.number => self.close(event_loop),
            UserEvent::Input(number, input) if number == self.number => {
                self.handle(event_loop, input)
            }
            UserEvent::RequestInnerSize(number, width, height) if number == self.number => {
                if let Some(surface) = &self.surface {
                    // The platform reports the new size as a `Resized` event, now or later.
                    if let Some(size) = surface
                        .window
                        .request_inner_size(LogicalSize::new(width as f64, height as f64))
                    {
                        self.handle(
                            event_loop,
                            WindowInput::Resized {
                                width: size.width,
                                height: size.height,
                            },
                        );
                    }
                }
            }
            // An event meant for a window that has already closed.
            _ => {}
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        if let Some(input) = WindowInput::from_window_event(event) {
            self.handle(event_loop, input);
        }
    }
}

impl WindowApp {
    /// Acts on one window event, from the platform or delivered by a test (D13, D14).
    fn handle(&mut self, event_loop: &ActiveEventLoop, input: WindowInput) {
        match input {
            WindowInput::CloseRequested => self.close(event_loop),
            WindowInput::Resized { width, height } => {
                if let Some(resize) = self.input.resized(width, height) {
                    self.send(WindowMessage::Input(resize));
                }
                self.request_redraw();
            }
            WindowInput::ScaleFactorChanged(scale_factor) => {
                self.input.scale_changed(scale_factor);
                if let Some(endpoint) = &self.endpoint {
                    endpoint.mailbox.set_scale_factor(scale_factor);
                }
                self.request_redraw();
            }
            WindowInput::CursorMoved { x, y } => self.input.cursor_moved(x, y),
            WindowInput::MouseButton { button, state } => {
                if let Some(event) = self.input.pointer_button(button, state) {
                    self.send(WindowMessage::Input(event));
                }
            }
            WindowInput::ModifiersChanged(modifiers) => self.input.modifiers_changed(modifiers),
            WindowInput::KeyPressed { key, text } => {
                match self
                    .input
                    .key_pressed(key.as_ref(), text.as_deref(), Platform::CURRENT)
                {
                    KeyAction::Close => self.close(event_loop),
                    KeyAction::Key(key) => {
                        self.send(WindowMessage::Input(prism_ui::HostEvent::Key(key)))
                    }
                    KeyAction::Text(text) => {
                        for character in typed_characters(text) {
                            self.send(WindowMessage::Input(prism_ui::HostEvent::Character(
                                character,
                            )));
                        }
                    }
                    KeyAction::Nothing => {}
                }
            }
            WindowInput::RedrawRequested => self.draw(),
        }
    }
}

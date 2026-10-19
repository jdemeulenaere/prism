//! Input mapping (D9 and D13 of `desktop-window`): `winit`'s window events as the host's
//! events, through pure functions over small values, so tests need no `winit` event (whose
//! `KeyEvent` cannot be built outside `winit`).
//!
//! - **The cursor** is recorded in logical pixels, the physical position divided by the scale
//!   factor and rounded down, and delivers nothing: the toolkit dispatches no moves.
//! - **The left button** delivers a press or a release at the recorded position; a press
//!   before any cursor move is dropped. Other buttons, the wheel and touch deliver nothing.
//! - **Key presses**, repeats included: Cmd+W and Cmd+Q close the window on macOS; with Ctrl
//!   or Cmd (Super) held nothing is typed; Backspace and Enter are keys; otherwise each
//!   character of the press's text from U+0020 up, other than U+007F, is typed. Alt (Option)
//!   is allowed, since it composes characters on macOS.
//! - **Resizes** deliver the new logical size, `floor(physical / scale)` on each axis, when it
//!   changed; a zero width or height delivers nothing. A scale change alone delivers nothing:
//!   the main thread redraws from the last display list.

use prism_ui::host::{HostEvent, Key};
use winit::event::WindowEvent;
// The `winit` types a [`WindowInput`] is made of, for callers that do not depend on `winit`.
pub use winit::event::{ElementState, MouseButton};
pub use winit::keyboard::{Key as KeyValue, ModifiersState, NamedKey, SmolStr};

/// A window event the window routes, in a form that can be built outside `winit`: the window's
/// handler turns each `WindowEvent` into one, and tests deliver them to a real window through
/// [`crate::WindowSystem::deliver`], so both take the same path from here on.
#[derive(Debug, Clone, PartialEq)]
pub enum WindowInput {
    /// The cursor moved to a physical position.
    CursorMoved {
        x: f64,
        y: f64,
    },
    MouseButton {
        button: MouseButton,
        state: ElementState,
    },
    ModifiersChanged(ModifiersState),
    /// A key press or a repeat, with the text it produces.
    KeyPressed {
        key: KeyValue,
        text: Option<SmolStr>,
    },
    /// The physical inner size changed.
    Resized {
        width: u32,
        height: u32,
    },
    ScaleFactorChanged(f64),
    CloseRequested,
    RedrawRequested,
}

impl WindowInput {
    /// The input a `winit` window event is, if the window acts on it. Key releases, focus,
    /// occlusion, IME, file drops and theme changes are not.
    pub fn from_window_event(event: WindowEvent) -> Option<WindowInput> {
        Some(match event {
            WindowEvent::CloseRequested => WindowInput::CloseRequested,
            WindowEvent::Resized(size) => WindowInput::Resized {
                width: size.width,
                height: size.height,
            },
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                WindowInput::ScaleFactorChanged(scale_factor)
            }
            WindowEvent::CursorMoved { position, .. } => WindowInput::CursorMoved {
                x: position.x,
                y: position.y,
            },
            WindowEvent::MouseInput { state, button, .. } => {
                WindowInput::MouseButton { button, state }
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                WindowInput::ModifiersChanged(modifiers.state())
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                WindowInput::KeyPressed {
                    key: event.logical_key,
                    text: event.text,
                }
            }
            WindowEvent::RedrawRequested => WindowInput::RedrawRequested,
            _ => return None,
        })
    }
}

/// The platform's conventions for closing shortcuts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    MacOs,
    Other,
}

impl Platform {
    /// The platform this build runs on.
    pub const CURRENT: Platform = if cfg!(target_os = "macos") {
        Platform::MacOs
    } else {
        Platform::Other
    };
}

/// What a key press does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction<'a> {
    /// Closes the window (Cmd+W or Cmd+Q on macOS).
    Close,
    Key(Key),
    /// Types the characters of [`typed_characters`] of this text.
    Text(&'a str),
    Nothing,
}

/// The characters a press's text types: those from U+0020 up, other than U+007F.
pub fn typed_characters(text: &str) -> impl Iterator<Item = char> + '_ {
    text.chars()
        .filter(|&character| character >= ' ' && character != '\u{7f}')
}

/// A logical coordinate: physical divided by the scale, rounded down.
fn logical(physical: f64, scale: f64) -> i64 {
    (physical / scale).floor() as i64
}

/// The window's input state on the main thread.
#[derive(Debug, Clone)]
pub struct InputState {
    cursor: Option<(i64, i64)>,
    modifiers: ModifiersState,
    scale: f64,
    logical_size: (i64, i64),
}

impl InputState {
    pub fn new(scale: f64, logical_size: (i64, i64)) -> InputState {
        InputState {
            cursor: None,
            modifiers: ModifiersState::empty(),
            scale,
            logical_size,
        }
    }

    pub fn scale(&self) -> f64 {
        self.scale
    }

    /// The logical size the app was last given.
    pub fn logical_size(&self) -> (i64, i64) {
        self.logical_size
    }

    /// The logical size of a physical size at the current scale.
    pub fn logical_size_of(&self, width: u32, height: u32) -> (i64, i64) {
        (
            logical(width as f64, self.scale),
            logical(height as f64, self.scale),
        )
    }

    /// Records the cursor at physical `(x, y)`.
    pub fn cursor_moved(&mut self, x: f64, y: f64) {
        self.cursor = Some((logical(x, self.scale), logical(y, self.scale)));
    }

    pub fn modifiers_changed(&mut self, modifiers: ModifiersState) {
        self.modifiers = modifiers;
    }

    /// The event a mouse button gives.
    pub fn pointer_button(&self, button: MouseButton, state: ElementState) -> Option<HostEvent> {
        let (x, y) = self.cursor?;
        match (button, state) {
            (MouseButton::Left, ElementState::Pressed) => Some(HostEvent::Press(x, y)),
            (MouseButton::Left, ElementState::Released) => Some(HostEvent::Release(x, y)),
            _ => None,
        }
    }

    /// What a key press, or a repeat, does.
    pub fn key_pressed<'a>(
        &self,
        key: KeyValue<&str>,
        text: Option<&'a str>,
        platform: Platform,
    ) -> KeyAction<'a> {
        let command = self.modifiers.super_key();
        if platform == Platform::MacOs && command {
            if let KeyValue::Character(character) = key {
                if character.eq_ignore_ascii_case("w") || character.eq_ignore_ascii_case("q") {
                    return KeyAction::Close;
                }
            }
        }
        if command || self.modifiers.control_key() {
            return KeyAction::Nothing;
        }
        match key {
            KeyValue::Named(NamedKey::Backspace) => KeyAction::Key(Key::Backspace),
            KeyValue::Named(NamedKey::Enter) => KeyAction::Key(Key::Enter),
            _ => match text {
                Some(text) if typed_characters(text).next().is_some() => KeyAction::Text(text),
                _ => KeyAction::Nothing,
            },
        }
    }

    /// The resize a new physical inner size gives, if the logical size changed.
    pub fn resized(&mut self, width: u32, height: u32) -> Option<HostEvent> {
        if width == 0 || height == 0 {
            return None;
        }
        let size = self.logical_size_of(width, height);
        if size == self.logical_size {
            return None;
        }
        self.logical_size = size;
        Some(HostEvent::Resize(size.0, size.1))
    }

    /// Records a new scale factor; the app hears of it only through a resize.
    pub fn scale_changed(&mut self, scale: f64) {
        self.scale = scale;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The scenario "A click at scale 2".
    #[test]
    fn a_click_at_scale_2() {
        let mut state = InputState::new(2.0, (400, 300));
        state.cursor_moved(101.0, 61.0);
        assert_eq!(
            state.pointer_button(MouseButton::Left, ElementState::Pressed),
            Some(HostEvent::Press(50, 30))
        );
        assert_eq!(
            state.pointer_button(MouseButton::Left, ElementState::Released),
            Some(HostEvent::Release(50, 30))
        );
    }

    #[test]
    fn other_buttons_and_a_press_before_any_move_deliver_nothing() {
        let mut state = InputState::new(1.0, (400, 300));
        assert_eq!(
            state.pointer_button(MouseButton::Left, ElementState::Pressed),
            None
        );
        state.cursor_moved(10.0, 10.0);
        for button in [
            MouseButton::Right,
            MouseButton::Middle,
            MouseButton::Other(7),
        ] {
            assert_eq!(state.pointer_button(button, ElementState::Pressed), None);
        }
        // A release outside the window is delivered at its position.
        state.cursor_moved(-3.5, 500.0);
        assert_eq!(
            state.pointer_button(MouseButton::Left, ElementState::Released),
            Some(HostEvent::Release(-4, 500))
        );
    }

    /// The scenario "Typing, deleting and shortcuts".
    #[test]
    fn typing_deleting_and_shortcuts() {
        let mut state = InputState::new(1.0, (400, 300));
        let mut delivered = Vec::new();
        let mut press = |state: &mut InputState,
                         modifiers: ModifiersState,
                         key: KeyValue<&str>,
                         text: Option<&str>| {
            state.modifiers_changed(modifiers);
            match state.key_pressed(key, text, Platform::MacOs) {
                KeyAction::Text(text) => {
                    delivered.extend(typed_characters(text).map(HostEvent::Character))
                }
                KeyAction::Key(key) => delivered.push(HostEvent::Key(key)),
                KeyAction::Close => panic!("no close"),
                KeyAction::Nothing => {}
            }
        };
        let none = ModifiersState::empty();
        press(&mut state, none, KeyValue::Character("a"), Some("a"));
        press(
            &mut state,
            ModifiersState::SHIFT,
            KeyValue::Character("B"),
            Some("B"),
        );
        press(
            &mut state,
            none,
            KeyValue::Named(NamedKey::Backspace),
            Some("\u{8}"),
        );
        press(
            &mut state,
            ModifiersState::CONTROL,
            KeyValue::Character("c"),
            Some("\u{3}"),
        );
        press(
            &mut state,
            ModifiersState::SUPER,
            KeyValue::Character("a"),
            Some("a"),
        );
        press(
            &mut state,
            none,
            KeyValue::Named(NamedKey::Enter),
            Some("\r"),
        );
        assert_eq!(
            delivered,
            [
                HostEvent::Character('a'),
                HostEvent::Character('B'),
                HostEvent::Key(Key::Backspace),
                HostEvent::Key(Key::Enter)
            ]
        );
    }

    #[test]
    fn control_characters_arrows_and_alt() {
        let mut state = InputState::new(1.0, (400, 300));
        let other = Platform::Other;
        assert_eq!(
            state.key_pressed(KeyValue::Named(NamedKey::Tab), Some("\t"), other),
            KeyAction::Nothing
        );
        assert_eq!(
            state.key_pressed(KeyValue::Named(NamedKey::ArrowLeft), None, other),
            KeyAction::Nothing
        );
        assert_eq!(
            state.key_pressed(KeyValue::Named(NamedKey::Delete), Some("\u{7f}"), other),
            KeyAction::Nothing
        );
        assert_eq!(
            state.key_pressed(KeyValue::Named(NamedKey::Space), Some(" "), other),
            KeyAction::Text(" ")
        );
        state.modifiers_changed(ModifiersState::ALT);
        assert_eq!(
            state.key_pressed(KeyValue::Character("é"), Some("é"), Platform::MacOs),
            KeyAction::Text("é"),
            "Option composes characters"
        );
        assert_eq!(typed_characters("a\u{1}b").collect::<String>(), "ab");
    }

    #[test]
    fn cmd_w_and_cmd_q_close_on_macos() {
        let mut state = InputState::new(1.0, (400, 300));
        state.modifiers_changed(ModifiersState::SUPER);
        for key in ["w", "q", "W"] {
            assert_eq!(
                state.key_pressed(KeyValue::Character(key), Some(key), Platform::MacOs),
                KeyAction::Close
            );
            assert_eq!(
                state.key_pressed(KeyValue::Character(key), Some(key), Platform::Other),
                KeyAction::Nothing,
                "Super+{} elsewhere is a shortcut that types nothing",
                key
            );
        }
        state.modifiers_changed(ModifiersState::CONTROL);
        assert_eq!(
            state.key_pressed(KeyValue::Character("w"), Some("w"), Platform::MacOs),
            KeyAction::Nothing
        );
    }

    /// The window's events become the inputs tests deliver, so both take the same path.
    #[test]
    fn window_events_become_window_inputs() {
        use winit::dpi::{PhysicalPosition, PhysicalSize};
        use winit::event::{DeviceId, Modifiers};
        let device_id = DeviceId::dummy();
        let cases = [
            (
                WindowEvent::CloseRequested,
                Some(WindowInput::CloseRequested),
            ),
            (
                WindowEvent::Resized(PhysicalSize::new(601, 451)),
                Some(WindowInput::Resized {
                    width: 601,
                    height: 451,
                }),
            ),
            (
                WindowEvent::CursorMoved {
                    device_id,
                    position: PhysicalPosition::new(101.0, 61.0),
                },
                Some(WindowInput::CursorMoved { x: 101.0, y: 61.0 }),
            ),
            (
                WindowEvent::MouseInput {
                    device_id,
                    state: ElementState::Pressed,
                    button: MouseButton::Left,
                },
                Some(WindowInput::MouseButton {
                    button: MouseButton::Left,
                    state: ElementState::Pressed,
                }),
            ),
            (
                WindowEvent::ModifiersChanged(Modifiers::from(ModifiersState::SUPER)),
                Some(WindowInput::ModifiersChanged(ModifiersState::SUPER)),
            ),
            (
                WindowEvent::RedrawRequested,
                Some(WindowInput::RedrawRequested),
            ),
            (WindowEvent::Focused(true), None),
        ];
        for (event, input) in cases {
            assert_eq!(WindowInput::from_window_event(event), input);
        }
    }

    /// The scenario "A resize at a fractional scale".
    #[test]
    fn a_resize_at_a_fractional_scale() {
        let mut state = InputState::new(1.5, (300, 200));
        assert_eq!(state.resized(601, 451), Some(HostEvent::Resize(400, 300)));
        assert_eq!(state.resized(600, 450), None, "the same logical size");
        assert_eq!(state.resized(0, 451), None, "a zero size");
        assert_eq!(state.logical_size(), (400, 300));
    }

    /// The program half of "Moving to another display": a scale change that keeps the logical
    /// size delivers nothing.
    #[test]
    fn a_scale_change_with_the_same_logical_size_delivers_nothing() {
        let mut state = InputState::new(2.0, (400, 300));
        state.scale_changed(1.0);
        assert_eq!(state.resized(400, 300), None);
        state.cursor_moved(101.0, 61.0);
        assert_eq!(
            state.pointer_button(MouseButton::Left, ElementState::Pressed),
            Some(HostEvent::Press(101, 61))
        );
    }
}

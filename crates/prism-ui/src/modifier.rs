//! Modifier chains (D6, D23 and D25 of `ui-toolkit`): the `#[repr(C)]` mirrors of
//! `/prism/compose/ui`'s `Modifier`, `ModifierElements` and `ModifierElement`, the decoder
//! into a node's chain of [`Element`]s, and the comparison that tells layout, drawing and
//! handler changes apart.
//!
//! A `Modifier` is a `Copy` struct of a count and two blocks of eight elements, laid out as
//! `code-generation` lays out structs and enums ("Structs Are Laid Out Inline", "Enums Are
//! Laid Out as a Tag and Payloads"): an element is an 8-bit tag, its variant's index, followed
//! by the variant's payload laid out like a struct's fields starting after the tag, so a
//! `Bool` sits at the next byte and an `Int` at the next multiple of 8. Every variant fits in
//! 40 bytes. The mirrors are byte arrays read at those offsets rather than Rust enums, because
//! Rust's `#[repr(C, u8)]` starts every payload at the union's alignment instead.
//!
//! The layout this decoder reads, by variant (task 11.1 checks it against `prism dump-layout`):
//!
//! | tag | variant | payload offsets |
//! |---|---|---|
//! | 0 | `None` | |
//! | 1 | `Padding(Int, Int, Int, Int)` | 8, 16, 24, 32 |
//! | 2 | `Size(Int, Int)` | 8, 16 |
//! | 3 | `DefaultMinSize(Int, Int)` | 8, 16 |
//! | 4 | `FillMax(Bool, Bool, Double)` | 1, 2, 8 |
//! | 5 | `Weight(Double, Bool)` | 8, 16 |
//! | 6 | `Align(Alignment)` | 8 (`Alignment` is `{ code: Int }`) |
//! | 7 | `Background(Color, Shape)` | 8, 16 (`Shape` is `{ cornerRadius: Int, cornerPercent: Int }`) |
//! | 8 | `Border(Int, Color, Shape)` | 8, 16, 24 |
//! | 9 | `Clickable(HandlerId, Bool, Int)` | 8, 16, 24 |
//! | 10 | `Toggleable(HandlerId, Bool, Bool, Int)` | 8, 16, 17, 24 |
//! | 11 | `TextFieldFrame(Color, Color)` | 8, 16 |

use crate::handle::HandlerId;
use crate::properties::{Alignment, Color, Role, Shape};

/// How many elements a chain holds at most.
pub const CAPACITY: usize = 16;
/// The size of one element in memory.
pub const ELEMENT_SIZE: usize = 40;

/// One element of a chain, as a node holds it: whole-pixel payloads, decoded codes, and the
/// handler entries the node holds (D23).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Element {
    /// Start, top, end and bottom, never negative.
    Padding {
        start: i64,
        top: i64,
        end: i64,
        bottom: i64,
    },
    /// An exact size on each set axis.
    Size {
        width: Option<i64>,
        height: Option<i64>,
    },
    /// A minimum on each specified axis.
    DefaultMinSize {
        width: Option<i64>,
        height: Option<i64>,
    },
    FillMax {
        width: bool,
        height: bool,
        fraction: f64,
    },
    Weight {
        weight: f64,
        fill: bool,
    },
    Align(Alignment),
    Background {
        color: Color,
        shape: Shape,
    },
    Border {
        width: i64,
        color: Color,
        shape: Shape,
    },
    Clickable {
        handler: HandlerId,
        enabled: bool,
        role: Option<Role>,
    },
    Toggleable {
        handler: HandlerId,
        value: bool,
        enabled: bool,
        role: Option<Role>,
    },
    /// A text field's frame, drawn in its first colour, or its second when focused.
    TextFieldFrame {
        unfocused: Color,
        focused: Color,
    },
}

/// What an element is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElementClass {
    /// Changes the constraints or size of the rest of the chain, or the parent's placement.
    Layout,
    /// Draws in the bounds at its position.
    Drawing,
    /// A pointer target in the bounds at its position.
    Input,
}

impl Element {
    pub fn class(&self) -> ElementClass {
        match self {
            Element::Padding { .. }
            | Element::Size { .. }
            | Element::DefaultMinSize { .. }
            | Element::FillMax { .. }
            | Element::Weight { .. }
            | Element::Align(_) => ElementClass::Layout,
            Element::Background { .. }
            | Element::Border { .. }
            | Element::TextFieldFrame { .. } => ElementClass::Drawing,
            Element::Clickable { .. } | Element::Toggleable { .. } => ElementClass::Input,
        }
    }

    /// Whether the element records the bounds at its position (drawing and input elements).
    pub fn records_bounds(&self) -> bool {
        self.class() != ElementClass::Layout
    }

    /// The handler entry the element names.
    pub fn handler(&self) -> Option<HandlerId> {
        match self {
            Element::Clickable { handler, .. } | Element::Toggleable { handler, .. } => {
                Some(*handler)
            }
            _ => None,
        }
    }

    /// The element naming `handler` instead.
    pub fn with_handler(mut self, new: HandlerId) -> Element {
        match &mut self {
            Element::Clickable { handler, .. } | Element::Toggleable { handler, .. } => {
                *handler = new
            }
            _ => {}
        }
        self
    }

    /// Whether two elements are the same variant.
    pub fn same_variant(&self, other: &Element) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}

/// One element of a `Modifier` in memory.
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ElementMirror {
    pub bytes: [u8; ELEMENT_SIZE],
}

/// A `ModifierElements`: eight elements, `e0` to `e7`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ElementsMirror {
    pub elements: [ElementMirror; 8],
}

/// A `Modifier`: the number of elements, then the two blocks.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModifierMirror {
    pub count: i64,
    pub first: ElementsMirror,
    pub second: ElementsMirror,
}

const TAG_NONE: u8 = 0;
const TAG_PADDING: u8 = 1;
const TAG_SIZE: u8 = 2;
const TAG_DEFAULT_MIN_SIZE: u8 = 3;
const TAG_FILL_MAX: u8 = 4;
const TAG_WEIGHT: u8 = 5;
const TAG_ALIGN: u8 = 6;
const TAG_BACKGROUND: u8 = 7;
const TAG_BORDER: u8 = 8;
const TAG_CLICKABLE: u8 = 9;
const TAG_TOGGLEABLE: u8 = 10;
const TAG_TEXT_FIELD_FRAME: u8 = 11;

impl ElementMirror {
    pub const NONE: ElementMirror = ElementMirror {
        bytes: [0; ELEMENT_SIZE],
    };

    fn int(&self, offset: usize) -> i64 {
        i64::from_ne_bytes(self.bytes[offset..offset + 8].try_into().unwrap())
    }

    fn bool(&self, offset: usize) -> bool {
        self.bytes[offset] != 0
    }

    fn double(&self, offset: usize) -> f64 {
        f64::from_ne_bytes(self.bytes[offset..offset + 8].try_into().unwrap())
    }

    fn set_int(&mut self, offset: usize, value: i64) {
        self.bytes[offset..offset + 8].copy_from_slice(&value.to_ne_bytes());
    }

    fn set_bool(&mut self, offset: usize, value: bool) {
        self.bytes[offset] = value as u8;
    }

    fn set_double(&mut self, offset: usize, value: f64) {
        self.bytes[offset..offset + 8].copy_from_slice(&value.to_ne_bytes());
    }

    /// The element this holds, or `None` for `ModifierElement.None` and unknown tags.
    pub fn decode(&self) -> Option<Element> {
        // `Size` uses -1 for an axis that is not set; other negative sizes lay out as 0.
        let size = |value: i64| (value != -1).then_some(value.max(0));
        // `defaultMinSize` treats every negative value as unspecified.
        let minimum = |value: i64| (value >= 0).then_some(value);
        Some(match self.bytes[0] {
            TAG_PADDING => Element::Padding {
                start: self.int(8).max(0),
                top: self.int(16).max(0),
                end: self.int(24).max(0),
                bottom: self.int(32).max(0),
            },
            TAG_SIZE => Element::Size {
                width: size(self.int(8)),
                height: size(self.int(16)),
            },
            TAG_DEFAULT_MIN_SIZE => Element::DefaultMinSize {
                width: minimum(self.int(8)),
                height: minimum(self.int(16)),
            },
            TAG_FILL_MAX => Element::FillMax {
                width: self.bool(1),
                height: self.bool(2),
                fraction: self.double(8),
            },
            TAG_WEIGHT => Element::Weight {
                weight: self.double(8),
                fill: self.bool(16),
            },
            TAG_ALIGN => Element::Align(Alignment::from_code(self.int(8))?),
            TAG_BACKGROUND => Element::Background {
                color: Color(self.int(8)),
                shape: Shape::new(self.int(16), self.int(24)),
            },
            TAG_BORDER => Element::Border {
                width: self.int(8).max(0),
                color: Color(self.int(16)),
                shape: Shape::new(self.int(24), self.int(32)),
            },
            TAG_CLICKABLE => Element::Clickable {
                handler: HandlerId(self.int(8) as u64),
                enabled: self.bool(16),
                role: Role::from_code(self.int(24)),
            },
            TAG_TOGGLEABLE => Element::Toggleable {
                handler: HandlerId(self.int(8) as u64),
                value: self.bool(16),
                enabled: self.bool(17),
                role: Role::from_code(self.int(24)),
            },
            TAG_TEXT_FIELD_FRAME => Element::TextFieldFrame {
                unfocused: Color(self.int(8)),
                focused: Color(self.int(16)),
            },
            TAG_NONE => return None,
            _ => return None,
        })
    }

    /// The memory of `element`, as Prism code builds it.
    pub fn encode(element: &Element) -> ElementMirror {
        let mut mirror = ElementMirror::NONE;
        let axis = |value: Option<i64>| value.unwrap_or(-1);
        match *element {
            Element::Padding {
                start,
                top,
                end,
                bottom,
            } => {
                mirror.bytes[0] = TAG_PADDING;
                mirror.set_int(8, start);
                mirror.set_int(16, top);
                mirror.set_int(24, end);
                mirror.set_int(32, bottom);
            }
            Element::Size { width, height } => {
                mirror.bytes[0] = TAG_SIZE;
                mirror.set_int(8, axis(width));
                mirror.set_int(16, axis(height));
            }
            Element::DefaultMinSize { width, height } => {
                mirror.bytes[0] = TAG_DEFAULT_MIN_SIZE;
                mirror.set_int(8, axis(width));
                mirror.set_int(16, axis(height));
            }
            Element::FillMax {
                width,
                height,
                fraction,
            } => {
                mirror.bytes[0] = TAG_FILL_MAX;
                mirror.set_bool(1, width);
                mirror.set_bool(2, height);
                mirror.set_double(8, fraction);
            }
            Element::Weight { weight, fill } => {
                mirror.bytes[0] = TAG_WEIGHT;
                mirror.set_double(8, weight);
                mirror.set_bool(16, fill);
            }
            Element::Align(alignment) => {
                mirror.bytes[0] = TAG_ALIGN;
                mirror.set_int(8, alignment.code());
            }
            Element::Background { color, shape } => {
                mirror.bytes[0] = TAG_BACKGROUND;
                mirror.set_int(8, color.0);
                mirror.set_int(16, shape.corner_radius);
                mirror.set_int(24, shape.corner_percent);
            }
            Element::Border {
                width,
                color,
                shape,
            } => {
                mirror.bytes[0] = TAG_BORDER;
                mirror.set_int(8, width);
                mirror.set_int(16, color.0);
                mirror.set_int(24, shape.corner_radius);
                mirror.set_int(32, shape.corner_percent);
            }
            Element::Clickable {
                handler,
                enabled,
                role,
            } => {
                mirror.bytes[0] = TAG_CLICKABLE;
                mirror.set_int(8, handler.0 as i64);
                mirror.set_bool(16, enabled);
                mirror.set_int(24, role.map_or(0, |role| role as i64));
            }
            Element::Toggleable {
                handler,
                value,
                enabled,
                role,
            } => {
                mirror.bytes[0] = TAG_TOGGLEABLE;
                mirror.set_int(8, handler.0 as i64);
                mirror.set_bool(16, value);
                mirror.set_bool(17, enabled);
                mirror.set_int(24, role.map_or(0, |role| role as i64));
            }
            Element::TextFieldFrame { unfocused, focused } => {
                mirror.bytes[0] = TAG_TEXT_FIELD_FRAME;
                mirror.set_int(8, unfocused.0);
                mirror.set_int(16, focused.0);
            }
        }
        mirror
    }
}

/// A decoded chain of at most [`CAPACITY`] elements, on the stack.
#[derive(Debug, Clone, Copy)]
pub struct ChainBuffer {
    elements: [Element; CAPACITY],
    count: usize,
}

impl Default for ChainBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl ChainBuffer {
    const FILLER: Element = Element::Padding {
        start: 0,
        top: 0,
        end: 0,
        bottom: 0,
    };

    pub const fn new() -> ChainBuffer {
        ChainBuffer {
            elements: [Self::FILLER; CAPACITY],
            count: 0,
        }
    }

    /// The chain of `elements`; the elements after the sixteenth are left out.
    pub fn of(elements: &[Element]) -> ChainBuffer {
        let mut buffer = ChainBuffer::new();
        for element in elements.iter().take(CAPACITY) {
            buffer.push(*element);
        }
        buffer
    }

    fn push(&mut self, element: Element) {
        self.elements[self.count] = element;
        self.count += 1;
    }

    pub fn as_slice(&self) -> &[Element] {
        &self.elements[..self.count]
    }
}

impl ModifierMirror {
    /// The empty chain, the companion value `Modifier`.
    pub const EMPTY: ModifierMirror = ModifierMirror {
        count: 0,
        first: ElementsMirror {
            elements: [ElementMirror::NONE; 8],
        },
        second: ElementsMirror {
            elements: [ElementMirror::NONE; 8],
        },
    };

    /// The element in slot `index` of the two blocks.
    fn slot(&self, index: usize) -> &ElementMirror {
        if index < 8 {
            &self.first.elements[index]
        } else {
            &self.second.elements[index - 8]
        }
    }

    /// Decodes the chain: the first `count` slots (at most 16, as only the library's `append`
    /// writes them), skipping `None` elements and elements with unknown codes, which a chain
    /// built by hand can hold.
    pub fn decode(&self) -> ChainBuffer {
        let count = self.count.clamp(0, CAPACITY as i64) as usize;
        let mut buffer = ChainBuffer::new();
        for index in 0..count {
            if let Some(element) = self.slot(index).decode() {
                buffer.push(element);
            }
        }
        buffer
    }

    /// The memory of a chain of `elements`, as `Modifier.append` builds it.
    pub fn encode(elements: &[Element]) -> ModifierMirror {
        assert!(
            elements.len() <= CAPACITY,
            "a chain holds at most 16 elements"
        );
        let mut mirror = ModifierMirror::EMPTY;
        for (index, element) in elements.iter().enumerate() {
            let encoded = ElementMirror::encode(element);
            if index < 8 {
                mirror.first.elements[index] = encoded;
            } else {
                mirror.second.elements[index - 8] = encoded;
            }
        }
        mirror.count = elements.len() as i64;
        mirror
    }
}

/// What a new chain changes on a node.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ChainChange {
    /// A layout element changed, or the chain's shape did: the node needs measuring.
    pub layout: bool,
    /// A drawing element changed: the display list is stale.
    pub drawing: bool,
    /// An input element's handler or flags changed: nothing to invalidate.
    pub input: bool,
}

impl ChainChange {
    pub fn any(self) -> bool {
        self.layout || self.drawing || self.input
    }
}

/// Compares two elements at one position, given whether their handlers are equal (D23),
/// and adds what changed to `change`.
pub fn compare_elements(
    old: &Element,
    new: &Element,
    handlers_equal: bool,
    change: &mut ChainChange,
) {
    if !old.same_variant(new) {
        change.layout = true;
        change.drawing = true;
        return;
    }
    let payload_equal = old.with_handler(HandlerId(0)) == new.with_handler(HandlerId(0));
    match old.class() {
        ElementClass::Layout => change.layout |= !payload_equal,
        ElementClass::Drawing => change.drawing |= !payload_equal,
        ElementClass::Input => change.input |= !payload_equal || !handlers_equal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn every_element() -> Vec<Element> {
        vec![
            Element::Padding {
                start: 1,
                top: 2,
                end: 3,
                bottom: 4,
            },
            Element::Size {
                width: Some(10),
                height: None,
            },
            Element::DefaultMinSize {
                width: None,
                height: Some(40),
            },
            Element::FillMax {
                width: true,
                height: false,
                fraction: 0.5,
            },
            Element::Weight {
                weight: 1.0,
                fill: false,
            },
            Element::Align(Alignment::BOTTOM_END),
            Element::Background {
                color: Color(0xFFFF_0000),
                shape: Shape::new(0, 50),
            },
            Element::Border {
                width: 2,
                color: Color(0xFF00_00FF),
                shape: Shape::new(2, 0),
            },
            Element::Clickable {
                handler: HandlerId(7 | (1 << 32)),
                enabled: false,
                role: Some(Role::Button),
            },
            Element::Toggleable {
                handler: HandlerId(9 | (3 << 32)),
                value: true,
                enabled: true,
                role: Some(Role::Checkbox),
            },
            Element::TextFieldFrame {
                unfocused: Color(0xFF79_747E),
                focused: Color(0xFF67_50A4),
            },
        ]
    }

    #[test]
    fn the_mirrors_have_the_layout_of_the_prism_structs() {
        assert_eq!(std::mem::size_of::<ElementMirror>(), 40);
        assert_eq!(std::mem::align_of::<ElementMirror>(), 8);
        assert_eq!(std::mem::size_of::<ElementsMirror>(), 320);
        assert_eq!(std::mem::size_of::<ModifierMirror>(), 648);
        assert_eq!(std::mem::offset_of!(ModifierMirror, first), 8);
        assert_eq!(std::mem::offset_of!(ModifierMirror, second), 328);
    }

    #[test]
    fn every_element_round_trips_through_memory() {
        let elements = every_element();
        let mirror = ModifierMirror::encode(&elements);
        assert_eq!(mirror.count, 11);
        assert_eq!(mirror.decode().as_slice(), &elements[..]);
    }

    #[test]
    fn payloads_sit_where_prism_puts_them() {
        let mirror = ElementMirror::encode(&Element::FillMax {
            width: true,
            height: true,
            fraction: 0.25,
        });
        assert_eq!(&mirror.bytes[..3], &[4, 1, 1]);
        assert_eq!(mirror.bytes[8..16], 0.25f64.to_ne_bytes());
        let mirror = ElementMirror::encode(&Element::Toggleable {
            handler: HandlerId(5),
            value: false,
            enabled: true,
            role: None,
        });
        assert_eq!(mirror.bytes[0], 10);
        assert_eq!(mirror.bytes[8..16], 5i64.to_ne_bytes());
        assert_eq!((mirror.bytes[16], mirror.bytes[17]), (0, 1));
        assert_eq!(mirror.bytes[24..32], 0i64.to_ne_bytes());
    }

    #[test]
    fn a_chain_fills_the_second_block_after_eight() {
        let element = Element::Padding {
            start: 1,
            top: 1,
            end: 1,
            bottom: 1,
        };
        let mirror = ModifierMirror::encode(&[element; 16]);
        assert_eq!(mirror.second.elements[7].decode(), Some(element));
        assert_eq!(mirror.decode().as_slice().len(), 16);
    }

    #[test]
    fn negative_and_unset_sizes() {
        let mut mirror = ElementMirror::NONE;
        mirror.bytes[0] = TAG_SIZE;
        mirror.set_int(8, -1);
        mirror.set_int(16, -5);
        assert_eq!(
            mirror.decode(),
            Some(Element::Size {
                width: None,
                height: Some(0)
            })
        );
        mirror.bytes[0] = TAG_PADDING;
        mirror.set_int(8, -4);
        assert!(matches!(
            mirror.decode(),
            Some(Element::Padding {
                start: 0,
                top: 0,
                ..
            })
        ));
        mirror.bytes[0] = TAG_DEFAULT_MIN_SIZE;
        mirror.set_int(8, -2);
        mirror.set_int(16, 3);
        assert_eq!(
            mirror.decode(),
            Some(Element::DefaultMinSize {
                width: None,
                height: Some(3)
            })
        );
    }

    #[test]
    fn none_elements_unknown_tags_and_wrong_counts_are_skipped() {
        let element = Element::Size {
            width: Some(1),
            height: Some(1),
        };
        let mut mirror = ModifierMirror::encode(&[element, element]);
        mirror.first.elements[0] = ElementMirror::NONE;
        assert_eq!(mirror.decode().as_slice(), &[element]);
        mirror.first.elements[0].bytes[0] = 200;
        assert_eq!(mirror.decode().as_slice(), &[element]);
        mirror.count = -3;
        assert!(mirror.decode().as_slice().is_empty());
        mirror.count = 99;
        assert_eq!(mirror.decode().as_slice(), &[element]);
        let mut align = ElementMirror::NONE;
        align.bytes[0] = TAG_ALIGN;
        align.set_int(8, 99);
        assert_eq!(align.decode(), None, "an unknown alignment code");
        assert_eq!(ElementMirror::NONE.decode(), None);
    }

    #[test]
    fn comparisons_tell_layout_drawing_and_input_apart() {
        let padding = |all| Element::Padding {
            start: all,
            top: all,
            end: all,
            bottom: all,
        };
        let background = |argb| Element::Background {
            color: Color(argb),
            shape: Shape::RECTANGLE,
        };
        let clickable = |handler, enabled| Element::Clickable {
            handler: HandlerId(handler),
            enabled,
            role: None,
        };
        let compare = |old: Element, new: Element, equal: bool| {
            let mut change = ChainChange::default();
            compare_elements(&old, &new, equal, &mut change);
            change
        };
        assert_eq!(
            compare(padding(1), padding(1), true),
            ChainChange::default()
        );
        assert!(compare(padding(1), padding(2), true).layout);
        let change = compare(background(1), background(2), true);
        assert!(change.drawing && !change.layout);
        let change = compare(padding(1), background(1), true);
        assert!(change.layout && change.drawing);
        let change = compare(clickable(1, true), clickable(2, true), true);
        assert!(!change.any(), "equal handlers in different entries");
        let change = compare(clickable(1, true), clickable(2, true), false);
        assert!(change.input && !change.layout && !change.drawing);
        assert!(compare(clickable(1, true), clickable(1, false), true).input);
    }
}

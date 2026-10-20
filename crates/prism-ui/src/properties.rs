//! Node properties decoded from the words the toolkit's setters pass (D3, D9 and D11 of
//! `ui-toolkit`): colours, arrangements, alignments, text styles, shapes and roles.
//!
//! The words are what `/prism/compose/ui` and `/prism/compose/layout` hold in their structs:
//! a colour is its `argb` field, an alignment and a role their codes, an arrangement its code
//! and spacing, a shape its corner radius and percentage. Negative sizes lay out as 0.

use std::fmt;

/// A colour: an ARGB value in the low 32 bits, or unspecified when outside that range
/// (`Color.Unspecified` is `0x1_0000_0000`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color(pub i64);

impl Color {
    pub const UNSPECIFIED: Color = Color(0x1_0000_0000);
    pub const BLACK: Color = Color(0xFF00_0000);
    pub const WHITE: Color = Color(0xFFFF_FFFF);

    /// Whether the colour is an ARGB value.
    pub fn is_specified(self) -> bool {
        (0..=0xFFFF_FFFF).contains(&self.0)
    }

    /// This colour when it is specified, else `fallback`.
    pub fn or(self, fallback: Color) -> Color {
        if self.is_specified() {
            self
        } else {
            fallback
        }
    }

    /// The ARGB value, black when unspecified.
    pub fn argb(self) -> u32 {
        self.or(Color::BLACK).0 as u32
    }
}

impl fmt::Display for Color {
    /// `#AARRGGBB`, as dumps write colours.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:08X}", self.argb())
    }
}

/// Where children go along a row's or column's main axis (D11). `Top` and `Start` mean the
/// same on either axis, as do `Bottom` and `End`; they keep their names for dumps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arrangement {
    Top,
    Bottom,
    Start,
    End,
    Center,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
    SpacedBy(i64),
}

impl Arrangement {
    /// The arrangement of `code` (its declaration order in `Arrangements`), with `space` for
    /// `spacedBy`, negative spaces laid out as 0.
    pub fn from_words(code: i64, space: i64) -> Option<Arrangement> {
        Some(match code {
            0 => Arrangement::Top,
            1 => Arrangement::Bottom,
            2 => Arrangement::Start,
            3 => Arrangement::End,
            4 => Arrangement::Center,
            5 => Arrangement::SpaceBetween,
            6 => Arrangement::SpaceAround,
            7 => Arrangement::SpaceEvenly,
            8 => Arrangement::SpacedBy(space.max(0)),
            _ => return None,
        })
    }

    /// The code and spacing words of `/prism/compose/layout`'s `Arrangement`, which
    /// [`Arrangement::from_words`] decodes.
    pub fn words(self) -> (i64, i64) {
        match self {
            Arrangement::Top => (0, 0),
            Arrangement::Bottom => (1, 0),
            Arrangement::Start => (2, 0),
            Arrangement::End => (3, 0),
            Arrangement::Center => (4, 0),
            Arrangement::SpaceBetween => (5, 0),
            Arrangement::SpaceAround => (6, 0),
            Arrangement::SpaceEvenly => (7, 0),
            Arrangement::SpacedBy(space) => (8, space),
        }
    }

    /// The fixed space between two children.
    pub fn spacing(self) -> i64 {
        match self {
            Arrangement::SpacedBy(space) => space,
            _ => 0,
        }
    }

    /// The position of the first child and the extra gap between children, given the free
    /// space `free` (never negative) left by `count` children and their fixed spacing.
    /// Remainders are dropped, so positions are exact integers.
    pub fn start_and_gap(self, free: i64, count: i64) -> (i64, i64) {
        let free = free.max(0);
        match self {
            Arrangement::Top | Arrangement::Start | Arrangement::SpacedBy(_) => (0, 0),
            Arrangement::Bottom | Arrangement::End => (free, 0),
            Arrangement::Center => (free / 2, 0),
            Arrangement::SpaceBetween if count <= 1 => (0, 0),
            Arrangement::SpaceBetween => (0, free / (count - 1)),
            Arrangement::SpaceAround if count == 0 => (0, 0),
            Arrangement::SpaceAround => {
                let gap = free / count;
                (gap / 2, gap)
            }
            Arrangement::SpaceEvenly => {
                let gap = free / (count + 1);
                (gap, gap)
            }
        }
    }
}

impl fmt::Display for Arrangement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Arrangement::Top => f.write_str("Top"),
            Arrangement::Bottom => f.write_str("Bottom"),
            Arrangement::Start => f.write_str("Start"),
            Arrangement::End => f.write_str("End"),
            Arrangement::Center => f.write_str("Center"),
            Arrangement::SpaceBetween => f.write_str("SpaceBetween"),
            Arrangement::SpaceAround => f.write_str("SpaceAround"),
            Arrangement::SpaceEvenly => f.write_str("SpaceEvenly"),
            Arrangement::SpacedBy(space) => write!(f, "spacedBy({})", space),
        }
    }
}

/// Where a child goes on one axis of the space left to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AxisAlignment {
    Start,
    Center,
    End,
}

impl AxisAlignment {
    /// The child's offset in `free` pixels of space.
    pub fn offset(self, free: i64) -> i64 {
        match self {
            AxisAlignment::Start => 0,
            AxisAlignment::Center => free / 2,
            AxisAlignment::End => free,
        }
    }
}

/// An alignment, by its code: the nine two-axis values, then the one-axis ones (D11). Each
/// has a horizontal and a vertical part; a one-axis value is the start or top on the other
/// axis, so `CenterHorizontally` is centre and top.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Alignment(u8);

const ALIGNMENT_NAMES: [&str; 15] = [
    "TopStart",
    "TopCenter",
    "TopEnd",
    "CenterStart",
    "Center",
    "CenterEnd",
    "BottomStart",
    "BottomCenter",
    "BottomEnd",
    "Start",
    "CenterHorizontally",
    "End",
    "Top",
    "CenterVertically",
    "Bottom",
];

impl Alignment {
    pub const TOP_START: Alignment = Alignment(0);
    pub const CENTER: Alignment = Alignment(4);
    pub const BOTTOM_END: Alignment = Alignment(8);
    pub const START: Alignment = Alignment(9);
    pub const CENTER_HORIZONTALLY: Alignment = Alignment(10);
    pub const END: Alignment = Alignment(11);
    pub const TOP: Alignment = Alignment(12);
    pub const CENTER_VERTICALLY: Alignment = Alignment(13);
    pub const BOTTOM: Alignment = Alignment(14);

    /// The alignment of `code`, its position in [`ALIGNMENT_NAMES`].
    pub fn from_code(code: i64) -> Option<Alignment> {
        (0..ALIGNMENT_NAMES.len() as i64)
            .contains(&code)
            .then_some(Alignment(code as u8))
    }

    pub fn code(self) -> i64 {
        self.0 as i64
    }

    pub fn name(self) -> &'static str {
        ALIGNMENT_NAMES[self.0 as usize]
    }

    fn part(code: u8) -> AxisAlignment {
        match code {
            0 => AxisAlignment::Start,
            1 => AxisAlignment::Center,
            _ => AxisAlignment::End,
        }
    }

    /// The horizontal part.
    pub fn horizontal(self) -> AxisAlignment {
        match self.0 {
            0..=8 => Self::part(self.0 % 3),
            9..=11 => Self::part(self.0 - 9),
            _ => AxisAlignment::Start,
        }
    }

    /// The vertical part.
    pub fn vertical(self) -> AxisAlignment {
        match self.0 {
            0..=8 => Self::part(self.0 / 3),
            12..=14 => Self::part(self.0 - 12),
            _ => AxisAlignment::Start,
        }
    }
}

impl fmt::Display for Alignment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// The font size of a text whose style leaves it unspecified (D9).
pub const DEFAULT_FONT_SIZE: i64 = 16;

/// A text node's style: its colour and font size, either unspecified (a negative size).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextStyle {
    pub color: Color,
    pub font_size: i64,
}

impl TextStyle {
    pub const DEFAULT: TextStyle = TextStyle {
        color: Color::UNSPECIFIED,
        font_size: -1,
    };

    /// The font size text is measured and drawn in.
    pub fn resolved_font_size(self) -> i64 {
        if self.font_size < 0 {
            DEFAULT_FONT_SIZE
        } else {
            self.font_size
        }
    }
}

/// A shape's corners: a radius, a percentage of the smaller side, or both, the larger
/// winning (D6). The rectangle has neither.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shape {
    pub corner_radius: i64,
    pub corner_percent: i64,
}

impl Shape {
    pub const RECTANGLE: Shape = Shape {
        corner_radius: 0,
        corner_percent: 0,
    };

    /// The shape of the two words of a `Shape`, negative values laid out as 0.
    pub fn new(corner_radius: i64, corner_percent: i64) -> Shape {
        Shape {
            corner_radius: corner_radius.max(0),
            corner_percent: corner_percent.max(0),
        }
    }

    /// The corner radius drawn in bounds of `width` × `height`.
    pub fn radius(self, width: i64, height: i64) -> i64 {
        let smaller = width.min(height).max(0);
        let percent = smaller.saturating_mul(self.corner_percent.min(100)) / 100;
        self.corner_radius.max(percent)
    }

    pub fn is_rectangle(self) -> bool {
        self == Shape::RECTANGLE
    }
}

impl fmt::Display for Shape {
    /// `RoundedCorner(<r>)`, `RoundedCorner(<p>%)` or both, as dumps write a shape other than
    /// the rectangle.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.corner_radius, self.corner_percent) {
            (radius, 0) => write!(f, "RoundedCorner({})", radius),
            (0, percent) => write!(f, "RoundedCorner({}%)", percent),
            (radius, percent) => write!(f, "RoundedCorner({} {}%)", radius, percent),
        }
    }
}

/// Compose's semantics roles, by code; 0 is no role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Button = 1,
    Checkbox = 2,
    Switch = 3,
    RadioButton = 4,
    Tab = 5,
    Image = 6,
    DropdownList = 7,
}

impl Role {
    /// The role of `code`; 0 and unknown codes are none.
    pub fn from_code(code: i64) -> Option<Role> {
        Some(match code {
            1 => Role::Button,
            2 => Role::Checkbox,
            3 => Role::Switch,
            4 => Role::RadioButton,
            5 => Role::Tab,
            6 => Role::Image,
            7 => Role::DropdownList,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Role::Button => "Button",
            Role::Checkbox => "Checkbox",
            Role::Switch => "Switch",
            Role::RadioButton => "RadioButton",
            Role::Tab => "Tab",
            Role::Image => "Image",
            Role::DropdownList => "DropdownList",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours() {
        assert_eq!(Color(0xFF33_6699).to_string(), "#FF336699");
        assert!(!Color::UNSPECIFIED.is_specified());
        assert!(!Color(-1).is_specified());
        assert_eq!(Color::UNSPECIFIED.or(Color::WHITE), Color::WHITE);
        assert_eq!(Color::UNSPECIFIED.to_string(), "#FF000000");
    }

    #[test]
    fn arrangements_decode_and_place() {
        assert_eq!(
            Arrangement::from_words(8, 8),
            Some(Arrangement::SpacedBy(8))
        );
        assert_eq!(
            Arrangement::from_words(8, -3),
            Some(Arrangement::SpacedBy(0))
        );
        assert_eq!(Arrangement::from_words(9, 0), None);
        assert_eq!(Arrangement::SpacedBy(8).to_string(), "spacedBy(8)");
        assert_eq!(Arrangement::Center.start_and_gap(11, 2), (5, 0));
        assert_eq!(Arrangement::End.start_and_gap(11, 2), (11, 0));
        assert_eq!(Arrangement::SpaceBetween.start_and_gap(84, 2), (0, 84));
        assert_eq!(Arrangement::SpaceBetween.start_and_gap(84, 1), (0, 0));
        assert_eq!(Arrangement::SpaceAround.start_and_gap(30, 3), (5, 10));
        assert_eq!(Arrangement::SpaceEvenly.start_and_gap(30, 2), (10, 10));
        assert_eq!(Arrangement::Bottom.start_and_gap(-4, 2), (0, 0));
    }

    #[test]
    fn alignments_have_two_parts() {
        let center_horizontally = Alignment::from_code(10).unwrap();
        assert_eq!(center_horizontally.name(), "CenterHorizontally");
        assert_eq!(center_horizontally.horizontal(), AxisAlignment::Center);
        assert_eq!(center_horizontally.vertical(), AxisAlignment::Start);
        assert_eq!(Alignment::BOTTOM_END.horizontal(), AxisAlignment::End);
        assert_eq!(Alignment::BOTTOM_END.vertical(), AxisAlignment::End);
        assert_eq!(Alignment::from_code(1).unwrap().name(), "TopCenter");
        assert_eq!(
            Alignment::CENTER_VERTICALLY.vertical(),
            AxisAlignment::Center
        );
        assert_eq!(
            Alignment::CENTER_VERTICALLY.horizontal(),
            AxisAlignment::Start
        );
        assert_eq!(Alignment::from_code(15), None);
        assert_eq!(Alignment::from_code(-1), None);
        assert_eq!(AxisAlignment::Center.offset(11), 5);
    }

    #[test]
    fn styles_shapes_and_roles() {
        assert_eq!(TextStyle::DEFAULT.resolved_font_size(), 16);
        assert_eq!(
            TextStyle {
                color: Color::BLACK,
                font_size: 24
            }
            .resolved_font_size(),
            24
        );
        assert_eq!(Shape::new(0, 50).radius(58, 40), 20);
        assert_eq!(Shape::new(4, 0).radius(24, 36), 4);
        assert_eq!(Shape::new(30, 50).radius(10, 10), 30);
        assert_eq!(Shape::new(-3, -1), Shape::RECTANGLE);
        assert_eq!(Shape::new(0, 50).to_string(), "RoundedCorner(50%)");
        assert_eq!(Shape::new(2, 0).to_string(), "RoundedCorner(2)");
        assert_eq!(Role::from_code(1), Some(Role::Button));
        assert_eq!(Role::from_code(0), None);
        assert_eq!(Role::Checkbox.name(), "Checkbox");
    }
}

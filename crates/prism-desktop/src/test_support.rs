//! Display lists built by hand, for the tests and benchmarks of this crate.

use prism_ui::draw::{DisplayList, DrawCommand};
use prism_ui::geometry::Rect;
use prism_ui::properties::Color;

/// Builds a display list command by command.
#[derive(Debug, Default)]
pub struct ListBuilder {
    pub list: DisplayList,
}

impl ListBuilder {
    pub fn new() -> ListBuilder {
        ListBuilder::default()
    }

    pub fn clear(mut self, width: i64, height: i64, argb: i64) -> Self {
        self.list.commands.push(DrawCommand::Clear {
            width,
            height,
            color: Color(argb),
        });
        self
    }

    /// A rectangle at `(x, y, width, height)`.
    pub fn rect(mut self, bounds: (i64, i64, i64, i64), argb: i64, corner_radius: i64) -> Self {
        self.list.commands.push(DrawCommand::Rectangle {
            bounds: Rect::new(bounds.0, bounds.1, bounds.2, bounds.3),
            color: Color(argb),
            corner_radius,
        });
        self
    }

    pub fn border(
        mut self,
        bounds: (i64, i64, i64, i64),
        width: i64,
        argb: i64,
        corner_radius: i64,
    ) -> Self {
        self.list.commands.push(DrawCommand::Border {
            bounds: Rect::new(bounds.0, bounds.1, bounds.2, bounds.3),
            width,
            color: Color(argb),
            corner_radius,
        });
        self
    }

    pub fn text(mut self, x: i64, y: i64, font_size: i64, argb: i64, text: &str) -> Self {
        let start = self.list.text.len();
        self.list.text.push_str(text);
        self.list.commands.push(DrawCommand::Text {
            x,
            y,
            font_size,
            color: Color(argb),
            text: start..self.list.text.len(),
        });
        self
    }

    pub fn build(self) -> DisplayList {
        self.list
    }
}

/// The display list `dump draw` prints for `ui-toolkit`'s counter screen headless at
/// 400 × 300 ("A dump of the counter screen").
pub fn counter_screen_list() -> DisplayList {
    ListBuilder::new()
        .clear(400, 300, 0xFFFF_FFFF)
        .text(16, 16, 16, 0xFF00_0000, "Count: 0")
        .rect((16, 44, 58, 40), 0xFF67_50A4, 20)
        .text(41, 54, 16, 0xFFFF_FFFF, "+")
        .build()
}

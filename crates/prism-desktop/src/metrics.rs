//! The window's text metrics (D7 of `desktop-window`): Inter's, rounded to whole logical
//! pixels, through `ui-toolkit`'s [`TextMeasurer`].
//!
//! - A character advances by its glyph's horizontal advance at the font size, rounded to the
//!   nearest integer. A character Inter lacks advances like the missing glyph (glyph 0), which
//!   is what it draws (D8).
//! - A line is `ascent - descent + line gap` high, rounded, and its baseline lies `ascent`,
//!   rounded, below the top of its line box.
//!
//! The rasterizer places glyphs at the pen positions these rounded advances give, so a run is
//! drawn exactly as wide as it was measured. Each font size gets a table of the printable
//! ASCII advances, filled when the size is first measured, and a map for other characters:
//! once warm, measuring is a lookup and allocates nothing.

use crate::font::font;
use prism_ui::text::TextMeasurer;
use std::cell::RefCell;
use std::collections::HashMap;

/// The metrics of one font size.
#[derive(Debug)]
struct SizeMetrics {
    px: f32,
    ascii: [i64; 128],
    others: HashMap<char, i64>,
    line_height: i64,
    baseline: i64,
}

impl SizeMetrics {
    fn new(font_size: i64) -> SizeMetrics {
        let px = font_size as f32;
        let mut ascii = [0; 128];
        for (code, advance) in ascii.iter_mut().enumerate() {
            *advance = rounded_advance(code as u8 as char, px);
        }
        let line = font()
            .horizontal_line_metrics(px)
            .expect("Inter has horizontal metrics");
        SizeMetrics {
            px,
            ascii,
            others: HashMap::new(),
            line_height: line.new_line_size.round() as i64,
            baseline: line.ascent.round() as i64,
        }
    }

    fn advance(&mut self, character: char) -> i64 {
        if character.is_ascii() {
            self.ascii[character as usize]
        } else {
            let px = self.px;
            *self
                .others
                .entry(character)
                .or_insert_with(|| rounded_advance(character, px))
        }
    }
}

/// A character's advance at `px` pixels, rounded.
fn rounded_advance(character: char, px: f32) -> i64 {
    font().metrics(character, px).advance_width.round() as i64
}

/// Inter's metrics, rounded to whole logical pixels, with a cache per font size.
#[derive(Debug, Default)]
pub struct FontMetrics {
    sizes: RefCell<HashMap<i64, SizeMetrics>>,
}

impl FontMetrics {
    pub fn new() -> FontMetrics {
        FontMetrics::default()
    }

    fn with_size<T>(&self, font_size: i64, read: impl FnOnce(&mut SizeMetrics) -> T) -> T {
        let mut sizes = self.sizes.borrow_mut();
        let metrics = sizes
            .entry(font_size)
            .or_insert_with(|| SizeMetrics::new(font_size));
        read(metrics)
    }

    /// How far below the top of its line box a line's baseline lies.
    pub fn baseline(&self, font_size: i64) -> i64 {
        self.with_size(font_size, |metrics| metrics.baseline)
    }
}

impl TextMeasurer for FontMetrics {
    fn advance(&self, character: char, font_size: i64) -> i64 {
        self.with_size(font_size, |metrics| metrics.advance(character))
    }

    fn line_height(&self, font_size: i64) -> i64 {
        self.with_size(font_size, |metrics| metrics.line_height)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prism_ui::layout::Layout;
    use prism_ui::test_support::TreeBuilder;
    use prism_ui::text::line_width;

    fn uncached(character: char, font_size: i64) -> i64 {
        font()
            .metrics(character, font_size as f32)
            .advance_width
            .round() as i64
    }

    #[test]
    fn ascii_advances_are_the_rounded_glyph_advances() {
        let metrics = FontMetrics::new();
        for character in (' '..='~').chain(['é', '€', '—']) {
            for font_size in [12, 16, 24] {
                assert_eq!(
                    metrics.advance(character, font_size),
                    uncached(character, font_size),
                    "{:?} at {}",
                    character,
                    font_size
                );
            }
        }
        // Inter is proportional: an `i` is narrower than an `m`.
        assert!(metrics.advance('i', 16) < metrics.advance('m', 16));
    }

    /// D7 records these numbers.
    #[test]
    fn line_height_and_baseline_at_16() {
        let metrics = FontMetrics::new();
        assert_eq!(metrics.line_height(16), 19);
        assert_eq!(metrics.baseline(16), 16);
        let line = font().horizontal_line_metrics(16.0).unwrap();
        assert_eq!(metrics.line_height(16), line.new_line_size.round() as i64);
        assert_eq!(metrics.baseline(16), line.ascent.round() as i64);
    }

    #[test]
    fn a_character_missing_from_the_font_measures_as_the_missing_glyph() {
        let metrics = FontMetrics::new();
        let missing = font().metrics_indexed(0, 16.0).advance_width.round() as i64;
        assert!(missing > 0);
        assert_eq!(metrics.advance('中', 16), missing);
        assert_eq!(line_width("中", 16, &metrics), missing);
    }

    /// The measurement half of "A text node's size in a window".
    #[test]
    fn a_text_nodes_size_in_a_window() {
        let metrics = FontMetrics::new();
        let mut builder = TreeBuilder::new(400, 300);
        let root = builder.root();
        let text = builder.text(root, "Count: 0");
        builder.lay_out_with(&mut Layout::new(), &metrics);
        let node = builder.tree.node(text).unwrap();
        let width: i64 = "Count: 0".chars().map(|c| uncached(c, 16)).sum();
        assert_eq!((node.size.width, node.size.height), (width, 19));
        assert_ne!(width, 64, "not the fixed-advance width");
    }
}

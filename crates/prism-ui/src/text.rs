//! Text measurement (D12 of `ui-toolkit`): the [`TextMeasurer`] interface, the fixed-advance
//! metrics of headless runs, and the toolkit's own line breaking, the same for every measurer.
//!
//! Each `\n` starts a line. Lines break greedily at spaces: a word goes on the current line
//! when the line with the spaces before it and the word fits the maximum width, and otherwise
//! starts the next line; the spaces at a break are not drawn. A word wider than the line stays
//! whole and overflows. Trailing spaces stay on their line when they fit, and are dropped as a
//! break otherwise. An empty text, or an empty line between two newlines, is one line of width
//! 0.

use crate::geometry::UNBOUNDED;

/// How wide characters are and how high lines are, in logical pixels.
pub trait TextMeasurer {
    fn advance(&self, character: char, font_size: i64) -> i64;
    fn line_height(&self, font_size: i64) -> i64;
}

/// The metrics of headless runs and every fixture: each character advances half the font size
/// and a line is 5/4 of it high, both rounded down, so 16-pixel text is 8 pixels per character
/// and 20 pixels high.
#[derive(Debug, Clone, Copy, Default)]
pub struct FixedAdvanceMetrics;

impl TextMeasurer for FixedAdvanceMetrics {
    fn advance(&self, _character: char, font_size: i64) -> i64 {
        font_size / 2
    }

    fn line_height(&self, font_size: i64) -> i64 {
        font_size * 5 / 4
    }
}

/// One line of a broken text: the byte range of what it draws, and its width.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Line {
    pub start: usize,
    pub end: usize,
    pub width: i64,
}

/// The width of `text` on one line.
pub fn line_width(text: &str, font_size: i64, measurer: &dyn TextMeasurer) -> i64 {
    text.chars()
        .map(|character| measurer.advance(character, font_size))
        .sum()
}

/// Breaks `text` into lines at most `max_width` wide (or [`UNBOUNDED`]), replacing the
/// contents of `lines`, which is reused between calls.
pub fn break_lines(
    text: &str,
    max_width: i64,
    font_size: i64,
    measurer: &dyn TextMeasurer,
    lines: &mut Vec<Line>,
) {
    lines.clear();
    let mut paragraph_start = 0;
    for paragraph in text.split('\n') {
        break_paragraph(
            paragraph,
            paragraph_start,
            max_width,
            font_size,
            measurer,
            lines,
        );
        paragraph_start += paragraph.len() + 1;
    }
}

fn break_paragraph(
    paragraph: &str,
    offset: usize,
    max_width: i64,
    font_size: i64,
    measurer: &dyn TextMeasurer,
    lines: &mut Vec<Line>,
) {
    let width_of = |range: &str| line_width(range, font_size, measurer);
    let fits = |width: i64| max_width == UNBOUNDED || width <= max_width;
    let bytes = paragraph.as_bytes();
    // The line being built: where it starts, where its last fitted word ends, its width.
    let mut line_start = 0;
    let mut line_end = 0;
    let mut width = 0;
    let mut position = 0;
    let mut first_word = true;
    while position < bytes.len() {
        let spaces_start = position;
        while position < bytes.len() && bytes[position] == b' ' {
            position += 1;
        }
        let word_start = position;
        while position < bytes.len() && bytes[position] != b' ' {
            position += 1;
        }
        let spaces = width_of(&paragraph[spaces_start..word_start]);
        let word = width_of(&paragraph[word_start..position]);
        if first_word {
            // The paragraph's leading spaces are drawn.
            width = spaces + word;
            line_end = position;
            first_word = false;
        } else if fits(width + spaces + word) {
            width += spaces + word;
            line_end = position;
        } else if word_start == position {
            // Trailing spaces that do not fit are a break: dropped.
            break;
        } else {
            lines.push(Line {
                start: offset + line_start,
                end: offset + line_end,
                width,
            });
            line_start = word_start;
            line_end = position;
            width = word;
        }
    }
    lines.push(Line {
        start: offset + line_start,
        end: offset + line_end,
        width,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn broken(text: &str, max_width: i64) -> Vec<(&str, i64)> {
        let mut lines = Vec::new();
        break_lines(text, max_width, 16, &FixedAdvanceMetrics, &mut lines);
        lines
            .iter()
            .map(|line| (&text[line.start..line.end], line.width))
            .collect()
    }

    #[test]
    fn fixed_advance_metrics() {
        let metrics = FixedAdvanceMetrics;
        assert_eq!(metrics.advance('a', 16), 8);
        assert_eq!(metrics.line_height(16), 20);
        assert_eq!(metrics.advance('a', 24), 12);
        assert_eq!(metrics.line_height(24), 30);
        assert_eq!(metrics.advance('a', 15), 7, "rounded down");
        assert_eq!(metrics.line_height(15), 18);
        assert_eq!(line_width("Count: 0", 16, &metrics), 64);
    }

    #[test]
    fn wrapping_at_an_exact_width() {
        assert_eq!(broken("aa bb cc", 40), [("aa bb", 40), ("cc", 16)]);
        assert_eq!(broken("aa bb cc", 39), [("aa", 16), ("bb", 16), ("cc", 16)]);
        assert_eq!(broken("aa bb cc", UNBOUNDED), [("aa bb cc", 64)]);
    }

    #[test]
    fn a_trailing_space() {
        assert_eq!(broken("ab ", 16), [("ab", 16)]);
        assert_eq!(broken("ab ", UNBOUNDED), [("ab ", 24)]);
        assert_eq!(
            broken(" ab", UNBOUNDED),
            [(" ab", 24)],
            "leading spaces are drawn"
        );
    }

    #[test]
    fn an_overlong_word_stays_whole() {
        assert_eq!(broken("aaaaaa b", 40), [("aaaaaa", 48), ("b", 8)]);
        assert_eq!(broken("b aaaaaa", 40), [("b", 8), ("aaaaaa", 48)]);
    }

    #[test]
    fn an_empty_text_is_one_line() {
        assert_eq!(broken("", 40), [("", 0)]);
        assert_eq!(broken("", 0), [("", 0)]);
    }

    #[test]
    fn newlines_start_lines() {
        assert_eq!(
            broken("a\n\nbb", UNBOUNDED),
            [("a", 8), ("", 0), ("bb", 16)]
        );
        assert_eq!(broken("a b\nc", 8), [("a", 8), ("b", 8), ("c", 8)]);
    }
}

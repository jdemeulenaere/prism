//! The bundled font (D6 of `desktop-window`): Inter 4.1 Regular, embedded in the executable
//! beside its SIL Open Font License 1.1 text (`fonts/OFL.txt`), and parsed at most once per
//! process, the first time a window needs it. No system font is ever read.
//!
//! `fontdue` builds the outline of every glyph the character map reaches when it parses a
//! font, so parsing is the font's one real cost. The substitution table is skipped: without
//! shaping (D8) its alternates are never drawn. One `&'static Font` serves both threads.

use fontdue::{Font, FontSettings};
use std::sync::OnceLock;

/// The font file: `extras/ttf/Inter-Regular.ttf` of Inter 4.1's release archive.
pub const FONT_FILE: &[u8] = include_bytes!("../fonts/Inter-Regular.ttf");

/// The font's licence: Inter's `LICENSE.txt`.
pub const LICENCE: &str = include_str!("../fonts/OFL.txt");

static FONT: OnceLock<Font> = OnceLock::new();

/// The bundled font, parsed on first use.
pub fn font() -> &'static Font {
    FONT.get_or_init(|| {
        let settings = FontSettings {
            scale: 40.0,
            load_substitutions: false,
            collection_index: 0,
        };
        Font::from_bytes(FONT_FILE, settings).expect("the bundled font parses")
    })
}

/// Whether the font has been parsed in this process.
pub fn is_loaded() -> bool {
    FONT.get().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The scenario "The font and its licence".
    #[test]
    fn the_font_and_its_licence() {
        let font = font();
        assert!(is_loaded());
        assert_eq!(font.name(), Some("Inter Regular"), "the full name");
        assert_eq!(family_name(FONT_FILE), "Inter");
        assert!(font.has_glyph('A'));
        assert!(!font.has_glyph('中'), "Inter has no CJK glyphs (D8)");
        assert!(LICENCE.contains("SIL Open Font License, Version 1.1"));
        assert!(LICENCE.contains("The Inter Project Authors"));
        assert!(std::ptr::eq(font, super::font()), "parsed once");
    }

    /// The family name (name 1) of the font's `name` table, in its Windows Unicode record.
    fn family_name(file: &[u8]) -> String {
        let u16_at = |offset: usize| u16::from_be_bytes([file[offset], file[offset + 1]]) as usize;
        let u32_at = |offset: usize| {
            u32::from_be_bytes(file[offset..offset + 4].try_into().unwrap()) as usize
        };
        let tables = u16_at(4);
        let name = (0..tables)
            .map(|index| 12 + index * 16)
            .find(|&record| &file[record..record + 4] == b"name")
            .map(|record| u32_at(record + 8))
            .expect("a name table");
        let (count, strings) = (u16_at(name + 2), name + u16_at(name + 4));
        (0..count)
            .map(|index| name + 6 + index * 12)
            .find(|&record| u16_at(record) == 3 && u16_at(record + 6) == 1)
            .map(|record| {
                let (length, offset) = (u16_at(record + 8), u16_at(record + 10));
                let units: Vec<u16> = file[strings + offset..strings + offset + length]
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|pair| u16::from_be_bytes(*pair))
                    .collect();
                String::from_utf16(&units).unwrap()
            })
            .expect("a Windows family name")
    }

    /// The file is the one D6 records, so a changed font is a reviewed change.
    #[test]
    fn the_font_file_is_inter_4_1_regular() {
        assert_eq!(FONT_FILE.len(), 411_640);
        assert_eq!(
            blake3::hash(FONT_FILE).to_hex().as_str(),
            "0c6305cd0a7a9f94daac842019537c6727fc0fadd4d44c336ec530317b42a42c"
        );
    }
}

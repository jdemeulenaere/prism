//! The rasterizer (D9 and D12 of `desktop-window`): a display list drawn into a premultiplied
//! RGBA pixmap at physical pixels.
//!
//! - **Coordinates.** A logical coordinate `v` is `round(v × scale)` physical pixels, and a
//!   rectangle's physical edges are its snapped corners, so adjacent rectangles neither
//!   overlap nor leave a gap at any scale.
//! - **Rectangles** without a radius are filled at their snapped edges without
//!   anti-aliasing. **Rounded rectangles** keep those edges and anti-alias their corners: a
//!   corner pixel's coverage is how far its centre lies inside the corner's circle, clamped to
//!   one pixel. **Borders** are the outer rounded rectangle minus the inner one, inset by the
//!   scaled width, so they lie inside their bounds.
//! - **Text** is one glyph per character (D8), each at the pen position that the window's
//!   rounded advances give ([`FontMetrics`]), snapped, plus the glyph's bitmap offset, on the
//!   baseline `round(ascent)` below the line box's top; glyphs are rasterized at the font
//!   size times the scale and blended source-over with the text colour.
//! - **Culling.** A command whose physical bounds miss the pixmap is skipped, and the others
//!   are clipped to it.
//!
//! Coverage and blending are computed here, in integers except for the corner distances,
//! rather than with `tiny-skia`'s path filler: that filler allocates its edge list and alpha
//! runs on every fill, and a steady-state frame must allocate nothing (D19). `tiny-skia`
//! holds the pixels and encodes PNGs. The pixmap is reallocated only when the size changes.

use crate::glyphs::GlyphCache;
use crate::metrics::FontMetrics;
use prism_ui::draw::{DisplayList, DrawCommand};
use prism_ui::geometry::Rect;
use prism_ui::properties::Color;
use prism_ui::text::TextMeasurer;
use tiny_skia::Pixmap;

/// A premultiplied RGBA colour.
type Premultiplied = [u8; 4];

/// The colour of a window before its first frame, and of what no clear covers (D14).
const WHITE: Premultiplied = [255, 255, 255, 255];

/// `x / 255`, rounded to the nearest integer, for `x` up to `255 × 255`.
fn div255(x: u32) -> u32 {
    let x = x + 128;
    (x + (x >> 8)) >> 8
}

/// An ARGB colour, premultiplied.
fn premultiply(color: Color) -> Premultiplied {
    let argb = color.argb();
    let alpha = argb >> 24;
    let channel = |shift: u32| div255(((argb >> shift) & 0xFF) * alpha) as u8;
    [channel(16), channel(8), channel(0), alpha as u8]
}

/// `source`, at `coverage` out of 255, over `destination`.
fn blend(destination: &mut Premultiplied, source: Premultiplied, coverage: u8) {
    let coverage = coverage as u32;
    if coverage == 255 && source[3] == 255 {
        *destination = source;
        return;
    }
    let alpha = div255(source[3] as u32 * coverage);
    let keep = 255 - alpha;
    for channel in 0..4 {
        let scaled = div255(source[channel] as u32 * coverage);
        destination[channel] = (scaled + div255(destination[channel] as u32 * keep)) as u8;
    }
}

/// Logical `v` in physical pixels.
pub fn snap(v: i64, scale: f64) -> i64 {
    (v as f64 * scale).round() as i64
}

/// A rectangle in physical pixels, from `left` and `top` inclusive to `right` and `bottom`
/// exclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhysicalRect {
    pub left: i64,
    pub top: i64,
    pub right: i64,
    pub bottom: i64,
}

impl PhysicalRect {
    /// Logical `bounds` snapped at `scale`.
    pub fn snapped(bounds: Rect, scale: f64) -> PhysicalRect {
        PhysicalRect {
            left: snap(bounds.x, scale),
            top: snap(bounds.y, scale),
            right: snap(bounds.x + bounds.width.max(0), scale),
            bottom: snap(bounds.y + bounds.height.max(0), scale),
        }
    }

    fn width(self) -> i64 {
        self.right - self.left
    }

    fn height(self) -> i64 {
        self.bottom - self.top
    }

    fn is_empty(self) -> bool {
        self.right <= self.left || self.bottom <= self.top
    }

    fn inset(self, by: i64) -> PhysicalRect {
        PhysicalRect {
            left: self.left + by,
            top: self.top + by,
            right: self.right - by,
            bottom: self.bottom - by,
        }
    }

    /// The part of the rectangle inside a `width` × `height` pixmap.
    fn clipped(self, width: i64, height: i64) -> PhysicalRect {
        PhysicalRect {
            left: self.left.clamp(0, width),
            top: self.top.clamp(0, height),
            right: self.right.clamp(0, width),
            bottom: self.bottom.clamp(0, height),
        }
    }
}

/// A rounded rectangle in physical pixels, its radius clamped to half its smaller side.
#[derive(Debug, Clone, Copy)]
struct RoundedRect {
    rect: PhysicalRect,
    radius: f64,
}

impl RoundedRect {
    fn new(rect: PhysicalRect, radius: f64) -> RoundedRect {
        let limit = rect.width().min(rect.height()).max(0) as f64 / 2.0;
        RoundedRect {
            rect,
            radius: radius.clamp(0.0, limit),
        }
    }

    /// Whether every pixel of row `y` between the corners is covered: the row misses both
    /// corner bands.
    fn row_is_straight(&self, y: i64) -> bool {
        let centre = y as f64 + 0.5;
        centre >= self.rect.top as f64 + self.radius
            && centre <= self.rect.bottom as f64 - self.radius
    }

    /// The coverage of pixel `(x, y)`, from 0 to 1.
    fn coverage(&self, x: i64, y: i64) -> f64 {
        let rect = self.rect;
        if x < rect.left || x >= rect.right || y < rect.top || y >= rect.bottom {
            return 0.0;
        }
        let radius = self.radius;
        if radius <= 0.0 {
            return 1.0;
        }
        let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
        let corner_x = if px < rect.left as f64 + radius {
            rect.left as f64 + radius
        } else if px > rect.right as f64 - radius {
            rect.right as f64 - radius
        } else {
            return 1.0;
        };
        let corner_y = if py < rect.top as f64 + radius {
            rect.top as f64 + radius
        } else if py > rect.bottom as f64 - radius {
            rect.bottom as f64 - radius
        } else {
            return 1.0;
        };
        let distance = ((px - corner_x).powi(2) + (py - corner_y).powi(2)).sqrt();
        (radius - distance + 0.5).clamp(0.0, 1.0)
    }
}

/// Coverage from 0 to 1 as a byte.
fn coverage_byte(coverage: f64) -> u8 {
    (coverage * 255.0).round() as u8
}

/// What one glyph of a text run was drawn as, recorded for tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GlyphPlacement {
    pub character: char,
    pub glyph: u16,
    /// The logical pen position, from the run's start.
    pub pen: i64,
    /// The bitmap's top-left corner and size, in physical pixels.
    pub left: i64,
    pub top: i64,
    pub width: usize,
    pub height: usize,
}

/// What the last rasterization did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RasterStatistics {
    pub drawn: usize,
    pub culled: usize,
    pub glyphs: usize,
}

/// Draws display lists into a pixmap it keeps between frames.
#[derive(Debug)]
pub struct Rasterizer {
    pixmap: Option<Pixmap>,
    glyphs: GlyphCache,
    metrics: FontMetrics,
    scale: f64,
    pub statistics: RasterStatistics,
    placements: Option<Vec<GlyphPlacement>>,
}

impl Default for Rasterizer {
    fn default() -> Rasterizer {
        Rasterizer::new()
    }
}

impl Rasterizer {
    pub fn new() -> Rasterizer {
        Rasterizer {
            pixmap: None,
            glyphs: GlyphCache::new(),
            metrics: FontMetrics::new(),
            scale: 1.0,
            statistics: RasterStatistics::default(),
            placements: None,
        }
    }

    /// The pixels of the last rasterization, or `None` for a zero size.
    pub fn pixmap(&self) -> Option<&Pixmap> {
        self.pixmap.as_ref()
    }

    /// Takes the pixmap, leaving none.
    pub fn take_pixmap(&mut self) -> Option<Pixmap> {
        self.pixmap.take()
    }

    /// The glyph cache, for tests.
    pub fn glyphs(&self) -> &GlyphCache {
        &self.glyphs
    }

    /// Records where each glyph is drawn, for tests ([`Rasterizer::placements`]).
    pub fn record_placements(&mut self) {
        self.placements = Some(Vec::new());
    }

    /// The glyphs the last rasterization drew, when recording.
    pub fn placements(&self) -> &[GlyphPlacement] {
        self.placements.as_deref().unwrap_or(&[])
    }

    /// Draws `list` at `scale` into a pixmap of `width` × `height` physical pixels. A zero
    /// width or height draws nothing.
    pub fn rasterize(&mut self, list: &DisplayList, scale: f64, width: u32, height: u32) {
        self.statistics = RasterStatistics::default();
        if let Some(placements) = &mut self.placements {
            placements.clear();
        }
        if scale != self.scale {
            self.glyphs.clear();
            self.scale = scale;
        }
        if width == 0 || height == 0 {
            self.pixmap = None;
            return;
        }
        let resized = self
            .pixmap
            .as_ref()
            .is_none_or(|pixmap| pixmap.width() != width || pixmap.height() != height);
        if resized {
            self.pixmap = Pixmap::new(width, height);
        }
        if !matches!(list.commands.first(), Some(DrawCommand::Clear { .. })) {
            self.fill(WHITE);
        }
        for command in &list.commands {
            self.draw(list, command);
        }
    }

    fn size(&self) -> (i64, i64) {
        let pixmap = self.pixmap.as_ref().expect("a pixmap");
        (pixmap.width() as i64, pixmap.height() as i64)
    }

    /// The pixels, row by row.
    fn pixels(&mut self) -> &mut [Premultiplied] {
        self.pixmap
            .as_mut()
            .expect("a pixmap")
            .data_mut()
            .as_chunks_mut::<4>()
            .0
    }

    fn fill(&mut self, color: Premultiplied) {
        self.pixels().fill(color);
    }

    fn draw(&mut self, list: &DisplayList, command: &DrawCommand) {
        let scale = self.scale;
        let (width, height) = self.size();
        let window = PhysicalRect {
            left: 0,
            top: 0,
            right: width,
            bottom: height,
        };
        match command {
            DrawCommand::Clear { color, .. } => {
                self.fill(premultiply(*color));
                self.statistics.drawn += 1;
            }
            DrawCommand::Rectangle {
                bounds,
                color,
                corner_radius,
            } => {
                let rect = PhysicalRect::snapped(*bounds, scale);
                if !self.visible(rect, window) {
                    return;
                }
                let color = premultiply(*color);
                let shape = RoundedRect::new(rect, *corner_radius as f64 * scale);
                if shape.radius <= 0.0 {
                    self.fill_rect(rect.clipped(width, height), color);
                } else {
                    self.fill_rounded(shape, None, color);
                }
            }
            DrawCommand::Border {
                bounds,
                width: border_width,
                color,
                corner_radius,
            } => {
                let rect = PhysicalRect::snapped(*bounds, scale);
                if *border_width <= 0 || !self.visible(rect, window) {
                    return;
                }
                let color = premultiply(*color);
                let outer = RoundedRect::new(rect, *corner_radius as f64 * scale);
                let inset = snap(*border_width, scale).max(1);
                let inner_rect = rect.inset(inset);
                let inner = (!inner_rect.is_empty()).then(|| {
                    let radius = (*corner_radius - *border_width).max(0) as f64 * scale;
                    RoundedRect::new(inner_rect, radius)
                });
                self.fill_rounded(outer, inner, color);
            }
            DrawCommand::Text {
                x,
                y,
                font_size,
                color,
                text,
            } => self.draw_text(list.text_of(text), *x, *y, *font_size, *color, window),
        }
    }

    /// Counts a command as drawn or culled.
    fn visible(&mut self, rect: PhysicalRect, window: PhysicalRect) -> bool {
        let visible = !rect.is_empty()
            && rect.right > window.left
            && rect.bottom > window.top
            && rect.left < window.right
            && rect.top < window.bottom;
        if visible {
            self.statistics.drawn += 1;
        } else {
            self.statistics.culled += 1;
        }
        visible
    }

    fn fill_rect(&mut self, rect: PhysicalRect, color: Premultiplied) {
        let (width, _) = self.size();
        let pixels = self.pixels();
        for y in rect.top..rect.bottom {
            let row = (y * width) as usize;
            let span = &mut pixels[row + rect.left as usize..row + rect.right as usize];
            for pixel in span {
                blend(pixel, color, 255);
            }
        }
    }

    /// Fills `outer` minus `inner`, anti-aliasing the corners.
    fn fill_rounded(
        &mut self,
        outer: RoundedRect,
        inner: Option<RoundedRect>,
        color: Premultiplied,
    ) {
        let (width, height) = self.size();
        let clip = outer.rect.clipped(width, height);
        let pixels = self.pixels();
        for y in clip.top..clip.bottom {
            let row = (y * width) as usize;
            let straight = outer.row_is_straight(y)
                && inner.is_none_or(|inner| {
                    y < inner.rect.top || y >= inner.rect.bottom || inner.row_is_straight(y)
                });
            if straight {
                // Every pixel's coverage is 1 or 0: spans outside the inner rectangle.
                let (hole_left, hole_right) = match inner {
                    Some(inner) if y >= inner.rect.top && y < inner.rect.bottom => (
                        inner.rect.left.clamp(clip.left, clip.right),
                        inner.rect.right.clamp(clip.left, clip.right),
                    ),
                    _ => (clip.right, clip.right),
                };
                for (left, right) in [(clip.left, hole_left), (hole_right, clip.right)] {
                    for x in left..right {
                        blend(&mut pixels[row + x as usize], color, 255);
                    }
                }
                continue;
            }
            for x in clip.left..clip.right {
                let mut coverage = outer.coverage(x, y);
                if let Some(inner) = inner {
                    coverage = (coverage - inner.coverage(x, y)).max(0.0);
                }
                let coverage = coverage_byte(coverage);
                if coverage > 0 {
                    blend(&mut pixels[row + x as usize], color, coverage);
                }
            }
        }
    }

    fn draw_text(
        &mut self,
        text: &str,
        x: i64,
        y: i64,
        font_size: i64,
        color: Color,
        window: PhysicalRect,
    ) {
        let scale = self.scale;
        let metrics = &self.metrics;
        // The run's line box, widened by a font size on each side for glyphs that overhang it.
        let run_width: i64 = text
            .chars()
            .map(|character| metrics.advance(character, font_size))
            .sum();
        let margin = font_size.max(0);
        let line_height = metrics.line_height(font_size);
        let bounds = Rect::new(
            x - margin,
            y - margin,
            run_width + 2 * margin,
            line_height + 2 * margin,
        );
        if !self.visible(PhysicalRect::snapped(bounds, scale), window) {
            return;
        }
        let color = premultiply(color);
        let baseline = snap(y + self.metrics.baseline(font_size), scale);
        let px = (font_size as f64 * scale) as f32;
        let font = crate::font::font();
        let mut pen = 0;
        for character in text.chars() {
            let index = font.lookup_glyph_index(character);
            let left = snap(x + pen, scale);
            let advance = self.metrics.advance(character, font_size);
            let glyph = self.glyphs.get(index, px);
            let metrics = glyph.metrics;
            let glyph_left = left + metrics.xmin as i64;
            let glyph_top = baseline - (metrics.ymin as i64 + metrics.height as i64);
            if let Some(placements) = &mut self.placements {
                placements.push(GlyphPlacement {
                    character,
                    glyph: index,
                    pen,
                    left: glyph_left,
                    top: glyph_top,
                    width: metrics.width,
                    height: metrics.height,
                });
            }
            let pixmap = self.pixmap.as_mut().expect("a pixmap");
            let (width, height) = (pixmap.width() as i64, pixmap.height() as i64);
            let pixels = pixmap.data_mut().as_chunks_mut::<4>().0;
            for row in 0..metrics.height as i64 {
                let py = glyph_top + row;
                if py < 0 || py >= height {
                    continue;
                }
                let coverage_row = &glyph.coverage[row as usize * metrics.width..];
                for column in 0..metrics.width as i64 {
                    let px = glyph_left + column;
                    let coverage = coverage_row[column as usize];
                    if px < 0 || px >= width || coverage == 0 {
                        continue;
                    }
                    blend(&mut pixels[(py * width + px) as usize], color, coverage);
                }
            }
            self.statistics.glyphs += 1;
            pen += advance;
        }
    }

    /// Copies the pixels into a `softbuffer` buffer of the same size, as `0x00RRGGBB` words.
    /// The pixels are opaque after a clear, so their premultiplied channels are the colour's.
    pub fn copy_to_surface(&self, buffer: &mut [u32]) {
        let Some(pixmap) = &self.pixmap else {
            return;
        };
        for (word, pixel) in buffer.iter_mut().zip(pixmap.data().as_chunks::<4>().0) {
            *word = (pixel[0] as u32) << 16 | (pixel[1] as u32) << 8 | pixel[2] as u32;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font::font;
    use crate::test_support::ListBuilder;

    fn list() -> ListBuilder {
        ListBuilder::new()
    }

    fn pixel(rasterizer: &Rasterizer, x: u32, y: u32) -> [u8; 4] {
        let pixel = rasterizer.pixmap().unwrap().pixel(x, y).unwrap();
        [pixel.red(), pixel.green(), pixel.blue(), pixel.alpha()]
    }

    const RED: [u8; 4] = [255, 0, 0, 255];
    const BLUE: [u8; 4] = [0, 0, 255, 255];
    const WHITE: [u8; 4] = [255, 255, 255, 255];

    /// The scenario "A rectangle at scale 2".
    #[test]
    fn a_rectangle_at_scale_2() {
        let built = list()
            .clear(20, 10, 0xFFFF_FFFF)
            .rect((2, 2, 4, 3), 0xFFFF_0000, 0);
        let mut rasterizer = Rasterizer::new();
        rasterizer.rasterize(&built.list, 2.0, 40, 20);
        let pixmap = rasterizer.pixmap().unwrap();
        assert_eq!((pixmap.width(), pixmap.height()), (40, 20));
        for y in 0..20 {
            for x in 0..40 {
                let inside = (4..12).contains(&x) && (4..10).contains(&y);
                let expected = if inside { RED } else { WHITE };
                assert_eq!(pixel(&rasterizer, x, y), expected, "{},{}", x, y);
            }
        }
    }

    #[test]
    fn adjacent_rectangles_at_scale_1_5_neither_overlap_nor_gap() {
        // Three rectangles of 3 logical pixels side by side: 4.5 physical pixels each.
        let built = list()
            .clear(9, 2, 0xFFFF_FFFF)
            .rect((0, 0, 3, 2), 0x80FF_0000, 0)
            .rect((3, 0, 3, 2), 0x80FF_0000, 0)
            .rect((6, 0, 3, 2), 0x80FF_0000, 0);
        let mut rasterizer = Rasterizer::new();
        rasterizer.rasterize(&built.list, 1.5, 14, 4);
        // Translucent red over white: an overlap would be darker, a gap white.
        let once = pixel(&rasterizer, 0, 0);
        assert_ne!(once, WHITE);
        for x in 0..14 {
            assert_eq!(pixel(&rasterizer, x, 0), once, "column {}", x);
        }
        assert_eq!(
            pixel(&rasterizer, 13, 2),
            once,
            "2 logical rows are 3 physical ones"
        );
        assert_eq!(pixel(&rasterizer, 0, 3), WHITE);
    }

    #[test]
    fn a_border_lies_inside_its_bounds() {
        let built = list()
            .clear(10, 10, 0xFFFF_FFFF)
            .border((2, 2, 6, 6), 1, 0xFF00_00FF, 0);
        let mut rasterizer = Rasterizer::new();
        rasterizer.rasterize(&built.list, 2.0, 20, 20);
        for y in 0..20 {
            for x in 0..20 {
                let outer = (4..16).contains(&x) && (4..16).contains(&y);
                let inner = (6..14).contains(&x) && (6..14).contains(&y);
                let expected = if outer && !inner { BLUE } else { WHITE };
                assert_eq!(pixel(&rasterizer, x, y), expected, "{},{}", x, y);
            }
        }
    }

    #[test]
    fn rounded_corners_are_anti_aliased_and_edges_are_not() {
        let rounded = |radius| {
            let built = list()
                .clear(60, 40, 0xFFFF_FFFF)
                .rect((0, 0, 60, 40), 0xFFFF_0000, radius);
            let mut rasterizer = Rasterizer::new();
            rasterizer.rasterize(&built.list, 1.0, 60, 40);
            rasterizer
        };
        let rasterizer = rounded(10);
        assert_eq!(pixel(&rasterizer, 0, 0), WHITE, "outside the circle");
        assert_eq!(pixel(&rasterizer, 30, 0), RED, "the top edge is crisp");
        assert_eq!(pixel(&rasterizer, 0, 20), RED, "the left edge is crisp");
        assert_eq!(pixel(&rasterizer, 10, 10), RED);
        let edge = pixel(&rasterizer, 3, 2);
        assert!(
            edge != RED && edge != WHITE,
            "a partly covered corner pixel"
        );
        // A radius larger than half the smaller side is clamped to it.
        assert_eq!(
            rounded(100).pixmap().unwrap().data(),
            rounded(20).pixmap().unwrap().data()
        );
    }

    #[test]
    fn commands_outside_the_pixmap_are_clipped_or_culled() {
        let built = list()
            .clear(10, 10, 0xFFFF_FFFF)
            .rect((-5, -5, 7, 7), 0xFFFF_0000, 0)
            .rect((20, 0, 5, 5), 0xFFFF_0000, 0)
            .rect((0, -10, 5, 5), 0xFFFF_0000, 3)
            .text(0, 100, 16, 0xFF00_0000, "far below");
        let mut rasterizer = Rasterizer::new();
        rasterizer.rasterize(&built.list, 1.0, 10, 10);
        assert_eq!(pixel(&rasterizer, 0, 0), RED);
        assert_eq!(pixel(&rasterizer, 1, 1), RED);
        assert_eq!(pixel(&rasterizer, 2, 2), WHITE);
        assert_eq!(
            rasterizer.statistics,
            RasterStatistics {
                drawn: 2,
                culled: 3,
                glyphs: 0
            }
        );
    }

    #[test]
    fn the_surface_words_are_0x00rrggbb() {
        let built = list()
            .clear(2, 1, 0xFF12_3456)
            .rect((1, 0, 1, 1), 0xFFAB_CDEF, 0);
        let mut rasterizer = Rasterizer::new();
        rasterizer.rasterize(&built.list, 1.0, 2, 1);
        let mut buffer = [0u32; 2];
        rasterizer.copy_to_surface(&mut buffer);
        assert_eq!(buffer, [0x0012_3456, 0x00AB_CDEF]);
    }

    #[test]
    fn a_list_without_a_clear_draws_on_white_and_a_zero_size_draws_nothing() {
        let mut rasterizer = Rasterizer::new();
        rasterizer.rasterize(&DisplayList::new(), 1.0, 3, 3);
        assert_eq!(pixel(&rasterizer, 1, 1), WHITE);
        rasterizer.rasterize(&DisplayList::new(), 1.0, 0, 3);
        assert!(rasterizer.pixmap().is_none());
    }

    fn placements(text: &str, scale: f64) -> Vec<GlyphPlacement> {
        let built = list()
            .clear(200, 40, 0xFFFF_FFFF)
            .text(0, 0, 16, 0xFF00_0000, text);
        let mut rasterizer = Rasterizer::new();
        rasterizer.record_placements();
        let size = (200.0 * scale) as u32;
        rasterizer.rasterize(&built.list, scale, size, size / 5);
        rasterizer.placements().to_vec()
    }

    /// The scenario "No kerning".
    #[test]
    fn no_kerning() {
        let av = placements("AV", 1.0);
        let ax = placements("AX", 1.0);
        let metrics = FontMetrics::new();
        assert_eq!(av[1].pen, metrics.advance('A', 16));
        assert_eq!(av[1].pen, ax[1].pen);
        assert!(
            font().horizontal_kern('A', 'V', 16.0).unwrap_or(0.0) <= 0.0,
            "fontdue reads no GPOS kerning, and draws none"
        );
    }

    /// The scenario "A character missing from the font".
    #[test]
    fn a_character_missing_from_the_font() {
        let drawn = placements("中", 1.0);
        assert_eq!(drawn.len(), 1);
        assert_eq!(drawn[0].glyph, 0, "the missing glyph");
        assert!(drawn[0].width > 0, "which has ink");
    }

    /// The scenario "Glyph positions follow the measured advances".
    #[test]
    fn glyph_positions_follow_the_measured_advances() {
        let metrics = FontMetrics::new();
        let drawn = placements("Count: 0", 1.0);
        let mut pen = 0;
        for (placement, character) in drawn.iter().zip("Count: 0".chars()) {
            assert_eq!(placement.character, character);
            assert_eq!(placement.pen, pen);
            let glyph = font().metrics(character, 16.0);
            assert_eq!(placement.left, pen + glyph.xmin as i64);
            assert_eq!(
                placement.top,
                metrics.baseline(16) - (glyph.ymin as i64 + glyph.height as i64)
            );
            pen += metrics.advance(character, 16);
        }
        let last = drawn.last().unwrap();
        assert_eq!(
            last.pen + metrics.advance('0', 16),
            prism_ui::text::line_width("Count: 0", 16, &metrics)
        );
        // At scale 2, pens are the logical ones doubled.
        let doubled = placements("Count: 0", 2.0);
        for (one, two) in drawn.iter().zip(&doubled) {
            assert_eq!(one.pen, two.pen);
            assert!((two.width as i64 - 2 * one.width as i64).abs() <= 2);
        }
    }

    #[test]
    fn the_second_rasterization_hits_the_glyph_cache() {
        let built = list()
            .clear(100, 20, 0xFFFF_FFFF)
            .text(0, 0, 16, 0xFF00_0000, "aab");
        let mut rasterizer = Rasterizer::new();
        rasterizer.rasterize(&built.list, 1.0, 100, 20);
        assert_eq!(
            (rasterizer.glyphs().hits, rasterizer.glyphs().misses),
            (1, 2)
        );
        let first = rasterizer.pixmap().unwrap().data().to_vec();
        rasterizer.rasterize(&built.list, 1.0, 100, 20);
        assert_eq!(
            (rasterizer.glyphs().hits, rasterizer.glyphs().misses),
            (4, 2)
        );
        assert_eq!(
            rasterizer.pixmap().unwrap().data(),
            &first[..],
            "deterministic"
        );
        assert_eq!(rasterizer.statistics.glyphs, 3);
        // A scale change clears it.
        rasterizer.rasterize(&built.list, 2.0, 200, 40);
        assert_eq!(rasterizer.glyphs().len(), 2);
        assert_eq!(rasterizer.glyphs().misses, 4);
    }

    #[test]
    fn text_is_drawn_in_its_colour() {
        let built = list()
            .clear(40, 20, 0xFFFF_FFFF)
            .text(0, 0, 16, 0xFF00_0000, "H");
        let mut rasterizer = Rasterizer::new();
        rasterizer.rasterize(&built.list, 1.0, 40, 20);
        let darkest = rasterizer
            .pixmap()
            .unwrap()
            .data()
            .as_chunks::<4>()
            .0
            .iter()
            .map(|pixel| pixel[0])
            .min()
            .unwrap();
        assert_eq!(darkest, 0, "the stems of an H are fully covered");
    }
}

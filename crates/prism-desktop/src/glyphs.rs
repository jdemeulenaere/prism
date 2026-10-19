//! The glyph cache (D12 of `desktop-window`): each glyph's `fontdue` metrics and coverage
//! bitmap, keyed by glyph index and pixel size, so a glyph is rasterized once per size.
//!
//! The pixel size is the font size times the scale factor, so a scale change gives new entries
//! rather than blurry scaled ones; the rasterizer also clears the cache then, since the old
//! sizes are unlikely to come back. The cache is cleared when it passes 4,096 entries, which
//! bounds its memory (a 24-pixel glyph at scale 2 is about 2 KB) without an eviction order.

use crate::font::font;
use std::collections::HashMap;

/// The most entries the cache holds before it is cleared.
pub const CAPACITY: usize = 4096;

/// A rasterized glyph.
#[derive(Debug)]
pub struct Glyph {
    pub metrics: fontdue::Metrics,
    /// One coverage byte per pixel, row by row from the top.
    pub coverage: Vec<u8>,
}

/// Rasterized glyphs by glyph index and pixel size.
#[derive(Debug, Default)]
pub struct GlyphCache {
    entries: HashMap<(u16, u32), Glyph>,
    pub hits: u64,
    pub misses: u64,
}

impl GlyphCache {
    pub fn new() -> GlyphCache {
        GlyphCache::default()
    }

    /// The glyph `index` at `px` pixels, rasterized on a miss.
    pub fn get(&mut self, index: u16, px: f32) -> &Glyph {
        let key = (index, px.to_bits());
        if self.entries.contains_key(&key) {
            self.hits += 1;
        } else {
            self.misses += 1;
            if self.entries.len() >= CAPACITY {
                self.entries.clear();
            }
            let (metrics, coverage) = font().rasterize_indexed(index, px);
            self.entries.insert(key, Glyph { metrics, coverage });
        }
        &self.entries[&key]
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_glyph_is_rasterized_once_per_size() {
        let mut cache = GlyphCache::new();
        let a = font().lookup_glyph_index('a');
        let width = cache.get(a, 16.0).metrics.width;
        assert!(width > 0);
        assert_eq!(cache.get(a, 16.0).metrics.width, width);
        assert_eq!((cache.hits, cache.misses), (1, 1));
        assert!(cache.get(a, 32.0).metrics.width > width, "another size");
        assert_eq!((cache.len(), cache.misses), (2, 2));
    }

    #[test]
    fn the_cache_is_cleared_beyond_its_capacity() {
        let mut cache = GlyphCache::new();
        for size in 0..CAPACITY {
            cache.get(0, 1.0 + size as f32 / 64.0);
        }
        assert_eq!(cache.len(), CAPACITY);
        cache.get(0, 1000.0);
        assert_eq!(cache.len(), 1);
    }
}

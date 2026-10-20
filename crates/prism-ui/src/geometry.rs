//! Sizes, rectangles and constraints in whole logical pixels (D11 of `ui-toolkit`).

/// A maximum without a bound.
pub const UNBOUNDED: i64 = i64::MAX;

/// The largest size a node takes and the farthest a child is placed from its parent's content:
/// Kotlin's `Int.MAX_VALUE`, the range of Compose's sizes and offsets. Larger values a program
/// asks for (a modifier's size or padding, a size under huge constraints, a measure policy's
/// size or position) are clamped to it, so that no sum of sizes and positions along the tree
/// overflows (D23 of `layout-primitive`).
pub const MAX_EXTENT: i64 = i32::MAX as i64;

/// `value` clamped between `-MAX_EXTENT` and `MAX_EXTENT`.
pub fn clamp_extent(value: i64) -> i64 {
    value.clamp(-MAX_EXTENT, MAX_EXTENT)
}

/// A width and a height.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Size {
    pub width: i64,
    pub height: i64,
}

impl Size {
    pub const ZERO: Size = Size {
        width: 0,
        height: 0,
    };

    pub fn new(width: i64, height: i64) -> Size {
        Size { width, height }
    }

    /// This size with each side at most [`MAX_EXTENT`].
    pub fn bounded(self) -> Size {
        Size::new(self.width.min(MAX_EXTENT), self.height.min(MAX_EXTENT))
    }
}

/// A rectangle: its top-left corner and size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    pub x: i64,
    pub y: i64,
    pub width: i64,
    pub height: i64,
}

impl Rect {
    pub const ZERO: Rect = Rect {
        x: 0,
        y: 0,
        width: 0,
        height: 0,
    };

    pub fn new(x: i64, y: i64, width: i64, height: i64) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    /// The rectangle moved by `(dx, dy)`.
    pub fn offset(self, dx: i64, dy: i64) -> Rect {
        Rect {
            x: self.x + dx,
            y: self.y + dy,
            ..self
        }
    }

    /// Whether `(x, y)` is inside, the right and bottom edges excluded.
    pub fn contains(self, x: i64, y: i64) -> bool {
        x >= self.x && y >= self.y && x - self.x < self.width && y - self.y < self.height
    }

    /// The centre, rounded down.
    pub fn centre(self) -> (i64, i64) {
        (
            self.x + self.width.div_euclid(2),
            self.y + self.height.div_euclid(2),
        )
    }
}

/// Minimum and maximum widths and heights; a maximum may be [`UNBOUNDED`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Constraints {
    pub min_width: i64,
    pub max_width: i64,
    pub min_height: i64,
    pub max_height: i64,
}

impl Constraints {
    /// From 0 to `width` × `height`.
    pub fn up_to(width: i64, height: i64) -> Constraints {
        Constraints {
            min_width: 0,
            max_width: width,
            min_height: 0,
            max_height: height,
        }
    }

    /// Exactly `width` × `height`.
    pub fn exact(width: i64, height: i64) -> Constraints {
        Constraints {
            min_width: width,
            max_width: width,
            min_height: height,
            max_height: height,
        }
    }

    /// The constraints with both minimums 0.
    pub fn loosened(self) -> Constraints {
        Constraints {
            min_width: 0,
            min_height: 0,
            ..self
        }
    }

    /// The constraints less `horizontal` and `vertical` pixels on each bound, floored at 0;
    /// an unbounded maximum stays unbounded.
    pub fn deflated(self, horizontal: i64, vertical: i64) -> Constraints {
        let shrink = |value: i64, by: i64| {
            if value == UNBOUNDED {
                UNBOUNDED
            } else {
                value.saturating_sub(by).max(0)
            }
        };
        Constraints {
            min_width: shrink(self.min_width, horizontal),
            max_width: shrink(self.max_width, horizontal),
            min_height: shrink(self.min_height, vertical),
            max_height: shrink(self.max_height, vertical),
        }
    }

    /// `width` clamped into the width bounds.
    pub fn clamp_width(self, width: i64) -> i64 {
        width.clamp(self.min_width, self.max_width.max(self.min_width))
    }

    /// `height` clamped into the height bounds.
    pub fn clamp_height(self, height: i64) -> i64 {
        height.clamp(self.min_height, self.max_height.max(self.min_height))
    }

    /// `size` clamped into the constraints.
    pub fn constrain(self, size: Size) -> Size {
        Size::new(self.clamp_width(size.width), self.clamp_height(size.height))
    }

    /// The constraints with an exact width, clamped into these.
    pub fn with_exact_width(self, width: i64) -> Constraints {
        let width = self.clamp_width(width);
        Constraints {
            min_width: width,
            max_width: width,
            ..self
        }
    }

    /// The constraints with an exact height, clamped into these.
    pub fn with_exact_height(self, height: i64) -> Constraints {
        let height = self.clamp_height(height);
        Constraints {
            min_height: height,
            max_height: height,
            ..self
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deflating_floors_at_zero_and_keeps_unbounded() {
        let c = Constraints {
            min_width: 10,
            max_width: 100,
            min_height: 0,
            max_height: UNBOUNDED,
        };
        let d = c.deflated(16, 8);
        assert_eq!(
            d,
            Constraints {
                min_width: 0,
                max_width: 84,
                min_height: 0,
                max_height: UNBOUNDED
            }
        );
        assert_eq!(c.constrain(Size::new(200, 5)), Size::new(100, 5));
        assert_eq!(c.with_exact_width(500).min_width, 100);
    }

    #[test]
    fn rectangles() {
        let r = Rect::new(16, 44, 58, 40);
        assert!(r.contains(16, 44));
        assert!(!r.contains(74, 44));
        assert_eq!(r.centre(), (45, 64));
        assert_eq!(Rect::new(0, 0, 9, 9).centre(), (4, 4));
    }
}

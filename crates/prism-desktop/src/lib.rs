//! `prism-desktop`: runs `runApp`'s apps in a native window (`desktop-window`).
//!
//! It implements the window half of the first-class compose and hot reload invariant's UI
//! layer: `prism-ui` composes, lays out and builds a display list on the program thread, and
//! this crate draws that list on the CPU and shows it in a window.
//!
//! - **The boundary** (D1, D2). Only `prism-cli` depends on this crate, through its default
//!   `window` feature; no compiler crate, `prism-runtime`, `prism-compose` or `prism-ui`
//!   builds a window, rasterization or font crate.
//! - **The bundled font** (D6, D7, D8). Inter 4.1 Regular is embedded ([`font`]) and windows
//!   lay text out with its advances rounded to logical pixels ([`FontMetrics`]), one glyph per
//!   character, without shaping.
//! - **The rasterizer** (D9, D12). A display list is drawn into a pixmap at physical pixels,
//!   edges snapped, corners and glyphs anti-aliased ([`Rasterizer`], [`glyphs`]).

pub mod font;
pub mod glyphs;
pub mod metrics;
pub mod raster;
pub mod test_support;

pub use metrics::FontMetrics;
pub use raster::Rasterizer;

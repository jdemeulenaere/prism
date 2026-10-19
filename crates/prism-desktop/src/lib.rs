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
//! - **Two threads** (D3, D4). The program keeps its thread, where the [`WindowHost`] gives
//!   `ui-toolkit`'s frame loop its events and takes its frames; the process's main thread runs
//!   the [`MainThreadServer`], which creates the event loop at the first window and owns the
//!   window, its surface and the rasterizer ([`window`]). Input crosses on a channel, frames
//!   through a latest-frame mailbox, and generated code never runs on the main thread.
//! - **Redraws on change only** (D10, D11), input mapping ([`input`], D13), closing (D14),
//!   display errors ([`error`], D16) and the window's counters ([`statistics`], D19).

pub mod error;
pub mod font;
pub mod glyphs;
pub mod host;
pub mod input;
pub mod metrics;
pub mod raster;
pub mod screenshot;
pub mod server;
pub mod statistics;
pub mod test_support;
pub mod window;

pub use host::WindowHost;
pub use metrics::FontMetrics;
pub use raster::Rasterizer;
pub use server::{MainThreadServer, WindowSystem};

/// Serializes the tests that use the process-wide wake-up sender.
#[cfg(test)]
pub(crate) fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

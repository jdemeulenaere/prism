//! Screenshots (D15 of `desktop-window`): an app's final display list rasterized as a window
//! draws it, at the host's scale factor (1 headless), and written as an 8-bit RGBA PNG without
//! premultiplied alpha.
//!
//! The CLI's `--screenshot <file.png>` sets [`writer`] as `prism-ui`'s final-frame sink, so the
//! image is made on the program thread, with its own rasterizer, each time `runApp` returns;
//! the last app wins. A file that cannot be written ends the program with
//! `display error: cannot write the screenshot <path> (<reason>)`.

use crate::error::{cannot_write_the_screenshot, raise_display_error};
use crate::raster::Rasterizer;
use prism_ui::draw::DisplayList;
use prism_ui::host::FinalFrameSink;
use std::path::{Path, PathBuf};
pub use tiny_skia::Pixmap;

/// The physical size of a logical size at `scale`.
fn physical(size: (i64, i64), scale: f64) -> (u32, u32) {
    let side = |v: i64| (v.max(0) as f64 * scale).round() as u32;
    (side(size.0), side(size.1))
}

/// `list`, laid out at logical `size`, rasterized at `scale` and encoded as a PNG.
pub fn encode_png(list: &DisplayList, size: (i64, i64), scale: f64) -> Result<Vec<u8>, String> {
    let (width, height) = physical(size, scale);
    let mut rasterizer = Rasterizer::new();
    rasterizer.rasterize(list, scale, width.max(1), height.max(1));
    let pixmap = rasterizer
        .take_pixmap()
        .expect("a pixmap of at least 1 x 1");
    pixmap.encode_png().map_err(|error| error.to_string())
}

/// Writes the screenshot of `list` to `path`.
pub fn write(path: &Path, list: &DisplayList, size: (i64, i64), scale: f64) -> Result<(), String> {
    let png = encode_png(list, size, scale)?;
    std::fs::write(path, png).map_err(|error| error.to_string())
}

/// The final-frame sink of `--screenshot <path>`.
pub fn writer(path: PathBuf) -> FinalFrameSink {
    Box::new(move |list, size, scale| {
        if let Err(reason) = write(&path, list, size, scale) {
            raise_display_error(&cannot_write_the_screenshot(
                &path.display().to_string(),
                &reason,
            ))
        }
    })
}

/// Reads a screenshot back, for tests.
pub fn read(path: &Path) -> Result<Pixmap, String> {
    Pixmap::load_png(path).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::ListBuilder;

    /// The pixels of the scenario "A headless screenshot", through the writer.
    #[test]
    fn a_screenshot_of_a_red_box() {
        let directory = std::env::temp_dir().join(format!("prism-desktop-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("shot.png");
        let list = ListBuilder::new()
            .clear(40, 30, 0xFFFF_FFFF)
            .rect((0, 0, 10, 10), 0xFFFF_0000, 0)
            .build();
        let mut sink = writer(path.clone());
        sink(&list, (40, 30), 1.0);
        let image = read(&path).unwrap();
        assert_eq!((image.width(), image.height()), (40, 30));
        for y in 0..30 {
            for x in 0..40 {
                let pixel = image.pixel(x, y).unwrap();
                let red = x < 10 && y < 10;
                let expected = if red { (255, 0, 0) } else { (255, 255, 255) };
                assert_eq!(
                    (pixel.red(), pixel.green(), pixel.blue(), pixel.alpha()),
                    (expected.0, expected.1, expected.2, 255),
                    "{},{}",
                    x,
                    y
                );
            }
        }
        // At the window's scale.
        let png = encode_png(&list, (40, 30), 2.0).unwrap();
        let doubled = Pixmap::decode_png(&png).unwrap();
        assert_eq!((doubled.width(), doubled.height()), (80, 60));
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn a_screenshot_that_cannot_be_written() {
        let list = ListBuilder::new().clear(4, 3, 0xFFFF_FFFF).build();
        let error = write(
            Path::new("/nonexistent-prism-directory/shot.png"),
            &list,
            (4, 3),
            1.0,
        )
        .unwrap_err();
        assert!(!error.is_empty());
    }
}

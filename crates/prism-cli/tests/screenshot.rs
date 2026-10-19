//! `prism run --screenshot <file.png>` on the binary (the `execution` spec, "Screenshots of an
//! App" and "Screenshot Errors"; D15 of `desktop-window`): each app's final frame is rasterized
//! as a window draws it and written as a PNG, headless at scale 1.
//!
//! The option needs the `window` feature, which holds the rasterizer, so this test needs it too.

#![cfg(feature = "window")]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

fn temporary_directory(test_name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("prism_screenshot_{}_{}", test_name, nanos));
    fs::create_dir_all(&path).unwrap();
    path
}

fn prism(arguments: &[&str], directory: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_prism"))
        .env("PRISM_CACHE", "off")
        .args(arguments)
        .current_dir(directory)
        .output()
        .expect("failed to run prism")
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

const RED_BOX: &str = "import /prism/compose/ui
import /prism/compose/foundation
import /prism/compose/layout

fun main() { runApp { Box(modifier = Modifier.size(10.dp).background(Color.Red)) { } } }
";

/// execution: "A headless screenshot".
#[test]
fn a_headless_screenshot() {
    let directory = temporary_directory("headless");
    fs::write(directory.join("main.prism"), RED_BOX).unwrap();
    let output = prism(
        &[
            "run",
            "--headless=40x30",
            "--screenshot",
            "shot.png",
            "main.prism",
        ],
        &directory,
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let image = prism_desktop::screenshot::read(&directory.join("shot.png")).unwrap();
    assert_eq!((image.width(), image.height()), (40, 30));
    for y in 0..30 {
        for x in 0..40 {
            let pixel = image.pixel(x, y).unwrap();
            let expected = if x < 10 && y < 10 {
                (0xFF, 0xFF, 0x00, 0x00)
            } else {
                (0xFF, 0xFF, 0xFF, 0xFF)
            };
            assert_eq!(
                (pixel.alpha(), pixel.red(), pixel.green(), pixel.blue()),
                expected,
                "the pixel at {},{}",
                x,
                y
            );
        }
    }
    fs::remove_dir_all(&directory).unwrap();
}

/// execution: "A screenshot without a file name".
#[test]
fn a_screenshot_without_a_file_name() {
    let directory = temporary_directory("no_file_name");
    let output = prism(&["run", "--screenshot"], &directory);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("usage: prism run") && stderr(&output).contains("--screenshot"),
        "{}",
        stderr(&output)
    );
    assert!(output.stdout.is_empty(), "nothing runs");
    fs::remove_dir_all(&directory).unwrap();
}

/// execution: "A screenshot that cannot be written".
#[test]
fn a_screenshot_that_cannot_be_written() {
    let directory = temporary_directory("cannot_be_written");
    fs::write(directory.join("main.prism"), RED_BOX).unwrap();
    let output = prism(
        &[
            "run",
            "--headless",
            "--screenshot",
            "missing/shot.png",
            "main.prism",
        ],
        &directory,
    );
    assert_eq!(output.status.code(), Some(101));
    let stderr = stderr(&output);
    let reason = stderr
        .strip_prefix("panic: display error: cannot write the screenshot missing/shot.png (")
        .and_then(|rest| rest.strip_suffix(")\n"))
        .unwrap_or_else(|| panic!("unexpected stderr: {:?}", stderr));
    assert!(!reason.is_empty(), "the platform's reason is given");
    assert!(!directory.join("missing").exists());
    fs::remove_dir_all(&directory).unwrap();
}

/// Every `runApp` overwrites the file, so the last app's final frame wins (D15).
#[test]
fn the_last_app_wins() {
    let directory = temporary_directory("last_app");
    fs::write(
        directory.join("main.prism"),
        "import /prism/compose/ui
import /prism/compose/foundation
import /prism/compose/layout

fun main() {
    runApp { Box(modifier = Modifier.size(10.dp).background(Color.Red)) { } }
    runApp { Box(modifier = Modifier.size(10.dp).background(Color.Blue)) { } }
}
",
    )
    .unwrap();
    let output = prism(
        &[
            "run",
            "--headless=20x20",
            "--screenshot",
            "shot.png",
            "main.prism",
        ],
        &directory,
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let image = prism_desktop::screenshot::read(&directory.join("shot.png")).unwrap();
    let pixel = image.pixel(0, 0).unwrap();
    assert_eq!(
        (pixel.red(), pixel.green(), pixel.blue()),
        (0x00, 0x00, 0xFF)
    );
    fs::remove_dir_all(&directory).unwrap();
}

//! `prism fmt`: the command scenarios of `openspec/specs/formatting` ("Format Command",
//! "Formatting Directories", "Formatting Standard Input", "Check Mode" and "Files With Syntax
//! Errors Are Not Formatted", and "Line endings normalized" of "File Structure"), run on the
//! binary.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

const UNFORMATTED: &str = "fun main( ):Int{return 1+2}";
const FORMATTED: &str = "fun main(): Int {\n    return 1 + 2\n}\n";

fn temporary_directory(test_name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("prism_fmt_{}_{}", test_name, nanos));
    fs::create_dir_all(&path).unwrap();
    path
}

fn write(path: &Path, contents: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

fn prism(arguments: &[&str], directory: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_prism"))
        .args(arguments)
        .current_dir(directory)
        .output()
        .expect("failed to run prism")
}

fn prism_with_input(arguments: &[&str], input: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_prism"))
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to run prism");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap()
}

#[test]
fn formatting_a_file_in_place() {
    let directory = temporary_directory("in_place");
    write(&directory.join("main.prism"), UNFORMATTED);
    let output = prism(&["fmt", "main.prism"], &directory);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(read(&directory.join("main.prism")), FORMATTED);
    assert_eq!(stdout(&output), "main.prism\n");
    fs::remove_dir_all(&directory).ok();
}

#[test]
fn formatted_file_left_alone() {
    let directory = temporary_directory("left_alone");
    let file = directory.join("main.prism");
    write(&file, FORMATTED);
    let modified = fs::metadata(&file).unwrap().modified().unwrap();
    let output = prism(&["fmt", "main.prism"], &directory);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "");
    assert_eq!(read(&file), FORMATTED);
    assert_eq!(fs::metadata(&file).unwrap().modified().unwrap(), modified);
    fs::remove_dir_all(&directory).ok();
}

#[test]
fn formatting_a_directory() {
    let directory = temporary_directory("directory");
    let src = directory.join("src");
    write(&src.join("a.prism"), UNFORMATTED);
    write(&src.join("b/c.prism"), UNFORMATTED);
    write(&src.join("d.prism"), FORMATTED);
    write(&src.join(".hidden/e.prism"), UNFORMATTED);
    write(&src.join("notes.txt"), UNFORMATTED);
    let output = prism(&["fmt", "src"], &directory);
    assert!(output.status.success(), "{}", stderr(&output));
    let listed: Vec<String> = stdout(&output).lines().map(str::to_string).collect();
    let expected: Vec<String> = [src.join("a.prism"), src.join("b").join("c.prism")]
        .iter()
        .map(|path| path.strip_prefix(&directory).unwrap().display().to_string())
        .collect();
    assert_eq!(listed, expected);
    assert_eq!(read(&src.join("a.prism")), FORMATTED);
    assert_eq!(read(&src.join("b/c.prism")), FORMATTED);
    assert_eq!(read(&src.join("d.prism")), FORMATTED);
    assert_eq!(read(&src.join(".hidden/e.prism")), UNFORMATTED);
    assert_eq!(read(&src.join("notes.txt")), UNFORMATTED);

    // A file named explicitly is formatted whatever its extension.
    let explicit = prism(&["fmt", "src/notes.txt"], &directory);
    assert!(explicit.status.success(), "{}", stderr(&explicit));
    assert_eq!(read(&src.join("notes.txt")), FORMATTED);
    fs::remove_dir_all(&directory).ok();
}

#[test]
fn line_endings_normalized() {
    let directory = temporary_directory("line_endings");
    let file = directory.join("main.prism");
    write(&file, "val x = 1 // one\r\nval y = 2\r\n");
    let output = prism(&["fmt", "main.prism"], &directory);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "main.prism\n");
    assert_eq!(read(&file), "val x = 1 // one\nval y = 2\n");
}

#[test]
fn formatting_standard_input() {
    let output = prism_with_input(&["fmt", "-"], "val x=1");
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "val x = 1\n");
}

#[test]
fn check_mode_with_an_unformatted_file() {
    let directory = temporary_directory("check_unformatted");
    let src = directory.join("src");
    write(&src.join("a.prism"), UNFORMATTED);
    write(&src.join("b.prism"), FORMATTED);
    let output = prism(&["fmt", "--check", "src"], &directory);
    assert!(!output.status.success());
    assert_eq!(
        stdout(&output),
        format!("{}\n", Path::new("src").join("a.prism").display())
    );
    assert_eq!(read(&src.join("a.prism")), UNFORMATTED);
    assert_eq!(read(&src.join("b.prism")), FORMATTED);

    let unformatted_input = prism_with_input(&["fmt", "--check", "-"], UNFORMATTED);
    assert!(!unformatted_input.status.success());
    assert_eq!(stdout(&unformatted_input), "");
    fs::remove_dir_all(&directory).ok();
}

#[test]
fn check_mode_with_formatted_files() {
    let directory = temporary_directory("check_formatted");
    write(&directory.join("src/a.prism"), FORMATTED);
    write(&directory.join("src/b/c.prism"), FORMATTED);
    let output = prism(&["fmt", "--check", "src"], &directory);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "");

    let formatted_input = prism_with_input(&["fmt", "--check", "-"], FORMATTED);
    assert!(formatted_input.status.success());
    assert_eq!(stdout(&formatted_input), "");
    fs::remove_dir_all(&directory).ok();
}

#[test]
fn syntax_error_refused() {
    let directory = temporary_directory("syntax_error");
    let src = directory.join("src");
    write(&src.join("bad.prism"), "fun f( {");
    write(&src.join("good.prism"), UNFORMATTED);
    let output = prism(&["fmt", "src"], &directory);
    assert!(!output.status.success());
    let errors = stderr(&output);
    assert!(errors.contains("error[E2001]"), "{}", errors);
    assert!(errors.contains("bad.prism"), "{}", errors);
    assert_eq!(read(&src.join("bad.prism")), "fun f( {");
    assert_eq!(read(&src.join("good.prism")), FORMATTED);

    // In check mode the file is reported the same way, and not listed as one that would change.
    write(&src.join("good.prism"), UNFORMATTED);
    let check = prism(&["fmt", "--check", "src"], &directory);
    assert!(!check.status.success());
    assert!(stderr(&check).contains("error[E2001]"));
    assert_eq!(
        stdout(&check),
        format!("{}\n", Path::new("src").join("good.prism").display())
    );
    fs::remove_dir_all(&directory).ok();
}

#[test]
fn syntax_error_on_standard_input() {
    let output = prism_with_input(&["fmt", "-"], "val = 1");
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("error[E2001]"),
        "{}",
        stderr(&output)
    );
    assert_eq!(stdout(&output), "");
}

#[test]
fn missing_path_and_unknown_option_are_errors() {
    let directory = temporary_directory("usage");
    let missing = prism(&["fmt", "missing.prism"], &directory);
    assert!(!missing.status.success());
    assert!(
        stderr(&missing).contains("error[E1000]"),
        "{}",
        stderr(&missing)
    );
    let no_path = prism(&["fmt"], &directory);
    assert!(!no_path.status.success());
    let unknown = prism(&["fmt", "--write", "x.prism"], &directory);
    assert!(!unknown.status.success());
    assert!(stderr(&unknown).contains("unknown option"));
    fs::remove_dir_all(&directory).ok();
}

#[test]
fn leading_byte_order_mark_dropped() {
    let directory = temporary_directory("byte_order_mark");
    let file = directory.join("main.prism");
    write(&file, "\u{FEFF}val x = 1\n");
    let output = prism(&["fmt", "main.prism"], &directory);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "main.prism\n");
    assert_eq!(fs::read(&file).unwrap(), b"val x = 1\n");

    // Without the mark the file is already formatted.
    let again = prism(&["fmt", "--check", "main.prism"], &directory);
    assert!(again.status.success(), "{}", stderr(&again));
    fs::remove_dir_all(&directory).ok();
}

/// formatting: "Library declarations formatted".
#[test]
fn library_declarations_formatted() {
    let output = prism_with_input(
        &["fmt", "-"],
        "public   intrinsic  mut fun <T> List<T>.add( element :T )\npublic intrinsic val <T> List<T>.size :Int\n",
    );
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "public intrinsic mut fun <T> List<T>.add(element: T)\n\npublic intrinsic val <T> List<T>.size: Int\n"
    );
}

/// formatting: "Writable intrinsic properties formatted".
#[test]
fn writable_intrinsic_properties_formatted() {
    let output = prism_with_input(
        &["fmt", "-"],
        "public  intrinsic var<T>MutableState<T>.value:T\n",
    );
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "public intrinsic var <T> MutableState<T>.value: T\n"
    );
}

/// formatting: "Intrinsic structs formatted".
#[test]
fn intrinsic_structs_formatted() {
    let output = prism_with_input(
        &["fmt", "-"],
        "public  intrinsic struct RecomposeScope:Copy\n",
    );
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "public intrinsic struct RecomposeScope : Copy\n"
    );
}

/// formatting: "Getters formatted".
#[test]
fn getters_formatted() {
    let output = prism_with_input(
        &["fmt", "-"],
        "public  val   Int.dp :Dp\n  get( )=Dp( this )\nval <T> List<T>.someVeryLongPropertyName: Range get() = computeTheRangeOfThisListFromItsElements(this, 0)\n",
    );
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "public val Int.dp: Dp get() = Dp(this)\n\nval <T> List<T>.someVeryLongPropertyName: Range\n    get() = computeTheRangeOfThisListFromItsElements(this, 0)\n"
    );
    // Formatting the result again changes nothing.
    let again = prism_with_input(&["fmt", "-"], &stdout(&output));
    assert_eq!(stdout(&again), stdout(&output));
}

/// formatting: "A getter with a block formatted".
#[test]
fn a_getter_with_a_block_formatted() {
    let output = prism_with_input(&["fmt", "-"], "val Box.area:Int get(){ return w*h }\n");
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "val Box.area: Int get() {\n    return w * h\n}\n"
    );
    let again = prism_with_input(&["fmt", "-"], &stdout(&output));
    assert_eq!(stdout(&again), stdout(&output));
}

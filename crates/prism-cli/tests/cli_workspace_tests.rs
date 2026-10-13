//! End-to-end integration tests for `prism check` driving workspace resolution.

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn create_temp_dir(test_name: &str) -> PathBuf {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("prism_cli_e2e_{}_{}", test_name, ts));
    fs::create_dir_all(&path).unwrap();
    path
}

fn prism_bin() -> &'static str {
    env!("CARGO_BIN_EXE_prism")
}

#[test]
fn test_cli_binary_check_single_file_success() {
    let temp = create_temp_dir("single_file");
    let file = temp.join("main.prism");
    fs::write(&file, "fun main(): Int = 42\n").unwrap();

    let output = Command::new(prism_bin())
        .arg("check")
        .arg(&file)
        .output()
        .expect("failed to execute prism CLI");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("✓ Check passed"));
    assert!(stdout.contains("Total units: 1"));

    fs::remove_dir_all(&temp).ok();
}

#[test]
fn test_cli_binary_root_anchored_import() {
    let temp = create_temp_dir("root_import");
    let core_dir = temp.join("core");
    let app_dir = temp.join("app");
    fs::create_dir_all(&core_dir).unwrap();
    fs::create_dir_all(&app_dir).unwrap();

    let fmt = core_dir.join("fmt.prism");
    fs::write(
        &fmt,
        "package core\npublic fun format_num(x: Int): Int = x\n",
    )
    .unwrap();

    let main = app_dir.join("main.prism");
    fs::write(
        &main,
        "package app\nimport /core/fmt\nfun main(): Int = format_num(10)\n",
    )
    .unwrap();

    // Check entry file with root-anchored import
    let output = Command::new(prism_bin())
        .arg("check")
        .arg(&main)
        .output()
        .expect("failed to execute prism CLI");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("✓ Check passed"));
    assert!(stdout.contains("Total units: 2"));
    assert!(stdout.contains("fmt.prism"));
    assert!(stdout.contains("package: core"));

    fs::remove_dir_all(&temp).ok();
}

#[test]
fn test_cli_binary_sibling_import() {
    let temp = create_temp_dir("sibling_import");
    let pkg_dir = temp.join("widgets");
    fs::create_dir_all(&pkg_dir).unwrap();

    let button = pkg_dir.join("button.prism");
    fs::write(
        &button,
        "package widgets\npackage fun render_btn(): Int = 1\n",
    )
    .unwrap();

    let page = pkg_dir.join("page.prism");
    fs::write(
        &page,
        "package widgets\nimport button\npublic fun render_page(): Int = render_btn()\n",
    )
    .unwrap();

    let output = Command::new(prism_bin())
        .arg("check")
        .arg(&page)
        .output()
        .expect("failed to execute prism CLI");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("✓ Check passed"));
    assert!(stdout.contains("Total units: 2"));

    fs::remove_dir_all(&temp).ok();
}

#[test]
fn test_cli_binary_check_directory_multi_package() {
    let temp = create_temp_dir("multi_package");
    let core_dir = temp.join("core");
    let app_dir = temp.join("app");
    fs::create_dir_all(&core_dir).unwrap();
    fs::create_dir_all(&app_dir).unwrap();

    let helper = core_dir.join("helper.prism");
    fs::write(
        &helper,
        "package core\npublic fun help(): Int = 99\n",
    )
    .unwrap();

    let entry = app_dir.join("entry.prism");
    fs::write(
        &entry,
        "package app\nimport /core/helper\nfun main(): Int = help()\n",
    )
    .unwrap();

    // Check whole workspace directory
    let output = Command::new(prism_bin())
        .arg("check")
        .arg(&temp)
        .output()
        .expect("failed to execute prism CLI");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("✓ Check passed"));
    assert!(stdout.contains("Total units: 2"));

    fs::remove_dir_all(&temp).ok();
}

#[test]
fn test_cli_binary_package_path_mismatch_e1001() {
    let temp = create_temp_dir("mismatch_e1001");
    let sub = temp.join("sub");
    fs::create_dir_all(&sub).unwrap();

    let file = sub.join("view.prism");
    // Directory is sub, but declared package is other.pkg
    fs::write(&file, "package other.pkg\nfun f(): Int = 1\n").unwrap();

    let output = Command::new(prism_bin())
        .arg("check")
        .arg(&file)
        .output()
        .expect("failed to execute prism CLI");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("error[E1001]: package path mismatch"));
    assert!(stderr.contains("does not match declared package 'other.pkg'"));

    fs::remove_dir_all(&temp).ok();
}

#[test]
fn test_cli_binary_file_not_found_e1002() {
    let temp = create_temp_dir("missing_e1002");
    let file = temp.join("main.prism");
    fs::write(&file, "import /core/missing\nfun main(): Int = 0\n").unwrap();

    let output = Command::new(prism_bin())
        .arg("check")
        .arg(&file)
        .output()
        .expect("failed to execute prism CLI");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("error[E1002]: imported file not found"));
    assert!(stderr.contains("cannot resolve import '/core/missing'"));

    fs::remove_dir_all(&temp).ok();
}

#[test]
fn test_cli_binary_package_heterogeneity_e1003() {
    let temp = create_temp_dir("heterogeneity_e1003");
    let pkg_dir = temp.join("pkg");
    fs::create_dir_all(&pkg_dir).unwrap();

    let f1 = pkg_dir.join("f1.prism");
    let f2 = pkg_dir.join("f2.prism");
    fs::write(&f1, "package pkg\nfun a(): Int = 1\n").unwrap();
    fs::write(&f2, "package other\nfun b(): Int = 2\n").unwrap();

    let output = Command::new(prism_bin())
        .arg("check")
        .arg(&pkg_dir)
        .output()
        .expect("failed to execute prism CLI");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("error[E1003]: conflicting package in directory"));

    fs::remove_dir_all(&temp).ok();
}

#[test]
fn test_cli_binary_cyclic_dependency_e1007() {
    let temp = create_temp_dir("cycle_e1007");
    let pkg_dir = temp.join("cycle");
    fs::create_dir_all(&pkg_dir).unwrap();

    let a = pkg_dir.join("a.prism");
    let b = pkg_dir.join("b.prism");
    fs::write(&a, "package cycle\nimport b\npublic fun a(): Int = 1\n").unwrap();
    fs::write(&b, "package cycle\nimport a\npublic fun b(): Int = 2\n").unwrap();

    let output = Command::new(prism_bin())
        .arg("check")
        .arg(&a)
        .output()
        .expect("failed to execute prism CLI");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("error[E1007]: cyclic dependency detected"));
    assert!(stderr.contains("a.prism ->"));

    fs::remove_dir_all(&temp).ok();
}

#[test]
fn test_cli_binary_abi_violation_e3001() {
    let temp = create_temp_dir("abi_e3001");
    let file = temp.join("api.prism");
    // Public function without explicit return type
    fs::write(&file, "public fun compute(x: Int) = x * 2\n").unwrap();

    let output = Command::new(prism_bin())
        .arg("check")
        .arg(&file)
        .output()
        .expect("failed to execute prism CLI");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("error[E3001]: ABI specification violation"));
    assert!(stderr.contains("compute"));

    fs::remove_dir_all(&temp).ok();
}

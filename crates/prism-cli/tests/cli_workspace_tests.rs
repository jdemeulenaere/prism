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
    fs::write(&helper, "package core\npublic fun help(): Int = 99\n").unwrap();

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
fn test_cli_binary_mutual_recursion_succeeds() {
    let temp = create_temp_dir("mutual_recursion");
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

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("✓ Check passed"));
    assert!(stdout.contains("Total units: 2"));

    fs::remove_dir_all(&temp).ok();
}

#[test]
#[cfg(unix)]
fn test_cli_binary_symlink_escape_rejected_e1005() {
    let temp = create_temp_dir("cli_symlink_escape");
    let outside = create_temp_dir("cli_outside");
    let secret = outside.join("secret.prism");
    fs::write(&secret, "package pkg\npublic fun secret(): Int = 42\n").unwrap();

    let pkg_dir = temp.join("pkg");
    fs::create_dir_all(&pkg_dir).unwrap();
    let link = pkg_dir.join("escaped.prism");
    std::os::unix::fs::symlink(&secret, &link).unwrap();

    let output = Command::new(prism_bin())
        .arg("check")
        .arg(&link)
        .output()
        .expect("failed to execute prism CLI");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("error[E1005]: import escapes workspace boundary"));

    fs::remove_dir_all(&temp).ok();
    fs::remove_dir_all(&outside).ok();
}

#[test]
fn test_cli_binary_syntax_error_e2001() {
    let temp = create_temp_dir("syntax_err");
    let file = temp.join("bad.prism");
    fs::write(&file, "fun main(\n").unwrap();

    let output = Command::new(prism_bin())
        .arg("check")
        .arg(&file)
        .output()
        .expect("failed to execute prism CLI");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("error[E2001]: syntax error"));

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

#[test]
fn test_cli_binary_return_type_mismatch_e4005() {
    let temp = create_temp_dir("return_type_mismatch_e4005");
    let file = temp.join("main.prism");
    fs::write(&file, "fun main(): Int = \"wrong type\"\n").unwrap();

    let output = Command::new(prism_bin())
        .arg("check")
        .arg(&file)
        .output()
        .expect("failed to execute prism CLI");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("error[E4005]: return type mismatch: expected 'Int', found 'String'"));
    assert!(stderr.contains("main.prism:1:19"));

    fs::remove_dir_all(&temp).ok();
}

#[test]
fn test_cli_binary_package_visibility_e4007() {
    let temp = create_temp_dir("package_visibility_e4007");
    let core_dir = temp.join("core");
    let app_dir = temp.join("app");
    fs::create_dir_all(&core_dir).unwrap();
    fs::create_dir_all(&app_dir).unwrap();

    let secret = core_dir.join("secret.prism");
    fs::write(
        &secret,
        "package core\npackage fun internal_fn(): Int = 42\n",
    )
    .unwrap();

    let main = app_dir.join("main.prism");
    fs::write(
        &main,
        "package app\nimport /core/secret\nfun main(): Int = internal_fn()\n",
    )
    .unwrap();

    let output = Command::new(prism_bin())
        .arg("check")
        .arg(&main)
        .output()
        .expect("failed to execute prism CLI");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("error[E4007]: symbol 'internal_fn' in package 'core' has package visibility and cannot be accessed from package 'app'"));

    fs::remove_dir_all(&temp).ok();
}

#[test]
fn test_cli_binary_compose_context_e4008() {
    let temp = create_temp_dir("compose_context_e4008");
    let file = temp.join("ui.prism");
    fs::write(
        &file,
        "compose fun Button(): Unit {}\nfun regular(): Unit {\n    Button()\n}\n",
    )
    .unwrap();

    let output = Command::new(prism_bin())
        .arg("check")
        .arg(&file)
        .output()
        .expect("failed to execute prism CLI");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("error[E4008]: composable function 'Button' can only be invoked within a composable context"));

    fs::remove_dir_all(&temp).ok();
}

#[test]
fn test_cli_binary_non_exhaustive_when_e4009() {
    let temp = create_temp_dir("when_e4009");
    let file = temp.join("check_when.prism");
    fs::write(
        &file,
        "fun classify(x: Int): String = when (x) { 0 -> \"zero\" }\n",
    )
    .unwrap();

    let output = Command::new(prism_bin())
        .arg("check")
        .arg(&file)
        .output()
        .expect("failed to execute prism CLI");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("error[E4009]: 'when' expression in value position must have an 'else' branch or be exhaustive"));

    fs::remove_dir_all(&temp).ok();
}

#[test]
fn test_cli_binary_val_immutability_e4003() {
    let temp = create_temp_dir("immutability_e4003");
    let file = temp.join("reassign.prism");
    fs::write(&file, "fun main(): Unit {\n    val x = 1\n    x = 2\n}\n").unwrap();

    let output = Command::new(prism_bin())
        .arg("check")
        .arg(&file)
        .output()
        .expect("failed to execute prism CLI");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("error[E4003]: cannot reassign to immutable 'val' binding 'x'"));

    fs::remove_dir_all(&temp).ok();
}

#[test]
fn test_cli_binary_struct_and_nominal_type_success() {
    let temp = create_temp_dir("struct_success");
    let file = temp.join("point.prism");
    fs::write(
        &file,
        "struct Point {\n    val x: Int,\n    val y: Int\n}\n\nfun main(): Int {\n    val p = Point(10, 20)\n    return p.x\n}\n",
    )
    .unwrap();

    let output = Command::new(prism_bin())
        .arg("check")
        .arg(&file)
        .output()
        .expect("failed to execute prism CLI");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("✓ Check passed"));

    fs::remove_dir_all(&temp).ok();
}

#[test]
fn test_cli_binary_default_params_success() {
    let temp = create_temp_dir("default_params");
    let file = temp.join("defaults.prism");
    fs::write(
        &file,
        "fun greet(msg: String, count: Int = 1): String = msg\n\nfun main(): String {\n    val a = greet(\"hello\")\n    val b = greet(\"world\", 2)\n    return a\n}\n",
    )
    .unwrap();

    let output = Command::new(prism_bin())
        .arg("check")
        .arg(&file)
        .output()
        .expect("failed to execute prism CLI");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("✓ Check passed"));

    fs::remove_dir_all(&temp).ok();
}

#[test]
fn test_cli_binary_exhaustive_enum_when_success() {
    let temp = create_temp_dir("exhaustive_enum_when");
    let file = temp.join("enum_when.prism");
    fs::write(
        &file,
        "enum Status {\n    Active,\n    Inactive\n}\n\nfun describe(s: Status): Int = when (s) {\n    Active -> 1,\n    Inactive -> 0\n}\n",
    )
    .unwrap();

    let output = Command::new(prism_bin())
        .arg("check")
        .arg(&file)
        .output()
        .expect("failed to execute prism CLI");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("✓ Check passed"));

    fs::remove_dir_all(&temp).ok();
}

#[test]
fn test_cli_binary_top_level_return_rejected_e4005() {
    let temp = create_temp_dir("top_level_return");
    let file = temp.join("top_return.prism");
    fs::write(&file, "val x = return 10\n").unwrap();

    let output = Command::new(prism_bin())
        .arg("check")
        .arg(&file)
        .output()
        .expect("failed to execute prism CLI");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("error[E4005]"));

    fs::remove_dir_all(&temp).ok();
}

#[test]
fn test_cli_binary_unannotated_private_block_function_success() {
    let temp = create_temp_dir("unannotated_block_fn");
    let file = temp.join("block_fn.prism");
    fs::write(
        &file,
        "fun do_side_effect() {\n    val x = 1\n}\n\nfun main(): Unit {\n    do_side_effect()\n}\n",
    )
    .unwrap();

    let output = Command::new(prism_bin())
        .arg("check")
        .arg(&file)
        .output()
        .expect("failed to execute prism CLI");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("✓ Check passed"));

    fs::remove_dir_all(&temp).ok();
}

#[test]
fn test_cli_binary_field_access_through_annotated_params_success() {
    let temp = create_temp_dir("annotated_field_access");
    let file = temp.join("users.prism");
    fs::write(
        &file,
        "struct User {\n    val id: Int\n    var name: String\n}\nfun byValue(user: User): String = user.name.clone()\nfun byRef(user: &User): String = user.name.clone()\nfun byMut(user: &mut User, n: String): Unit {\n    user.name = n\n}\n",
    )
    .unwrap();

    let output = Command::new(prism_bin())
        .arg("check")
        .arg(&file)
        .output()
        .expect("failed to execute prism CLI");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("✓ Check passed"));

    fs::remove_dir_all(&temp).ok();
}

#[test]
fn test_cli_binary_field_access_on_imported_struct_success() {
    let temp = create_temp_dir("imported_struct_field_access");
    let pkg_dir = temp.join("models");
    fs::create_dir_all(&pkg_dir).unwrap();

    fs::write(
        pkg_dir.join("user.prism"),
        "package models\npublic struct User {\n    val id: Int\n    var name: String\n}\n",
    )
    .unwrap();

    let service = pkg_dir.join("service.prism");
    fs::write(
        &service,
        "package models\nimport user\npublic fun rename(user: &mut User, n: String): Unit {\n    user.name = n\n}\npublic fun nameOf(user: &User): String = user.name.clone()\n",
    )
    .unwrap();

    let output = Command::new(prism_bin())
        .arg("check")
        .arg(&service)
        .output()
        .expect("failed to execute prism CLI");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("✓ Check passed"));
    assert!(stdout.contains("Total units: 2"));

    fs::remove_dir_all(&temp).ok();
}

#[test]
fn test_cli_binary_missing_return_type_e4011() {
    let temp = create_temp_dir("missing_return_e4011");
    let file = temp.join("api.prism");
    // Block-bodied exported function: passes ABI extraction but omits its return type
    fs::write(&file, "public fun greet(name: String) {\n}\n").unwrap();

    let output = Command::new(prism_bin())
        .arg("check")
        .arg(&file)
        .output()
        .expect("failed to execute prism CLI");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr
        .contains("error[E4011]: exported function 'greet' must declare an explicit return type"));
    assert!(stderr.contains("help: add an explicit return type such as ': Unit'"));
    assert!(!stderr.contains("E4004"));
    assert!(!stderr.contains("verify parameter counts"));

    fs::remove_dir_all(&temp).ok();
}

#[test]
fn test_cli_binary_syntax_error_location_and_lexer_message() {
    let temp = create_temp_dir("syntax_location");
    let lexer_err = temp.join("lexer.prism");
    fs::write(&lexer_err, "fun f(): Int = 1\nfun g(): String = \"\\q\"\n").unwrap();
    let too_deep = temp.join("deep.prism");
    fs::write(
        &too_deep,
        format!("fun f(): Int = {}1{}\n", "(".repeat(300), ")".repeat(300)),
    )
    .unwrap();

    let output = Command::new(prism_bin())
        .arg("check")
        .arg(&lexer_err)
        .output()
        .expect("failed to execute prism CLI");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("error[E2001]: syntax error"), "{}", stderr);
    assert!(stderr.contains("lexer.prism:2:19"), "{}", stderr);
    assert!(
        stderr.contains("Unknown escape sequence '\\q'"),
        "{}",
        stderr
    );
    assert!(!stderr.contains("<Error"), "{}", stderr);

    let output = Command::new(prism_bin())
        .arg("check")
        .arg(&too_deep)
        .output()
        .expect("failed to execute prism CLI");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("error[E2002]: syntax error"), "{}", stderr);

    fs::remove_dir_all(&temp).ok();
}

#[test]
fn test_cli_binary_imported_signatures_keep_their_declared_types() {
    let temp = create_temp_dir("imported_signature_types");
    for dir in ["lib", "model", "svc", "app"] {
        fs::create_dir_all(temp.join(dir)).unwrap();
    }
    fs::write(
        temp.join("lib/h.prism"),
        "package lib\npublic fun hof(f: ((Int) -> Int) -> Int): Int = f { k -> k + 1 }\npublic fun pick(f: (Int) -> Int?): Int = 1\n",
    )
    .unwrap();
    fs::write(
        temp.join("model/types.prism"),
        "package model\npublic struct Item {\n    val price: Int\n}\n",
    )
    .unwrap();
    // `svc/api` binds `Item` to `model/types` in its ABI.
    fs::write(
        temp.join("svc/api.prism"),
        "package svc\nimport /model/types\npublic fun first(): Item = Item(1)\n",
    )
    .unwrap();
    let main = temp.join("app/main.prism");
    fs::write(
        &main,
        "package app\nimport /lib/h\nimport /svc/api\nimport /model/types\nfun g(): Int = hof { k -> k(1) }\nfun p(): Int = pick { x -> null }\nfun q(): Int = first().price\n",
    )
    .unwrap();

    let output = Command::new(prism_bin())
        .arg("check")
        .arg(&main)
        .output()
        .expect("failed to execute prism CLI");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{}", stderr);

    fs::remove_dir_all(&temp).ok();
}

#[test]
fn test_cli_binary_member_of_unimported_type_e4014() {
    let temp = create_temp_dir("member_of_unimported_type_e4014");
    for dir in ["model", "svc", "app"] {
        fs::create_dir_all(temp.join(dir)).unwrap();
    }
    fs::write(
        temp.join("model/types.prism"),
        "package model\npublic struct Item {\n    val price: Int\n}\n",
    )
    .unwrap();
    fs::write(
        temp.join("svc/api.prism"),
        "package svc\nimport /model/types\npublic fun first(): Item = Item(1)\npublic fun send(i: Item): Unit { }\n",
    )
    .unwrap();
    let main = temp.join("app/main.prism");
    fs::write(
        &main,
        "package app\nimport /svc/api\nfun keep() {\n    send(first())\n}\nfun price(): Int = first().price\n",
    )
    .unwrap();

    let output = Command::new(prism_bin())
        .arg("check")
        .arg(&main)
        .output()
        .expect("failed to execute prism CLI");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("error[E4014]"), "{}", stderr);
    assert!(stderr.contains("add 'import /model/types'"), "{}", stderr);
    assert_eq!(stderr.matches("error[").count(), 1, "{}", stderr);

    fs::remove_dir_all(&temp).ok();
}

#[test]
fn test_cli_binary_nonexistent_target_e1000() {
    let temp = create_temp_dir("missing_target_e1000");

    for missing in [temp.join("nowhere.prism"), temp.join("nowhere")] {
        let output = Command::new(prism_bin())
            .arg("check")
            .arg(&missing)
            .output()
            .expect("failed to execute prism CLI");

        assert!(!output.status.success());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("error[E1000]"), "stderr: {}", stderr);
    }

    fs::remove_dir_all(&temp).ok();
}

#[test]
#[cfg(unix)]
fn test_cli_binary_symlinked_directory_entry_rejected_e1005() {
    let temp = create_temp_dir("cli_dir_symlink_escape");
    let outside = create_temp_dir("cli_dir_outside");
    fs::write(
        outside.join("secret.prism"),
        "package pkg\npublic fun secret(): Int = 42\n",
    )
    .unwrap();

    let pkg_dir = temp.join("pkg");
    fs::create_dir_all(&pkg_dir).unwrap();
    fs::write(
        pkg_dir.join("a.prism"),
        "package pkg\npublic fun a(): Int = 1\n",
    )
    .unwrap();
    std::os::unix::fs::symlink(outside.join("secret.prism"), pkg_dir.join("link.prism")).unwrap();

    // Both the package directory and the workspace root that contains it reject the entry.
    for target in [&pkg_dir, &temp] {
        let output = Command::new(prism_bin())
            .arg("check")
            .arg(target)
            .output()
            .expect("failed to execute prism CLI");

        assert!(!output.status.success());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("error[E1005]"), "stderr: {}", stderr);
    }

    fs::remove_dir_all(&temp).ok();
    fs::remove_dir_all(&outside).ok();
}

/// Runs `prism dump-typed-tree` on a single-file workspace holding `source`.
fn dump_typed_tree(test_name: &str, source: &str) -> std::process::Output {
    let temp = create_temp_dir(test_name);
    let file = temp.join("main.prism");
    fs::write(&file, source).unwrap();
    let output = Command::new(prism_bin())
        .arg("dump-typed-tree")
        .arg(&file)
        .output()
        .expect("failed to execute prism CLI");
    fs::remove_dir_all(&temp).ok();
    output
}

/// typed-tree: "Dumping a file".
#[test]
fn test_cli_binary_dump_typed_tree_is_deterministic() {
    let source = "fun f(a: Int): Int = a + 1\n";
    let first = dump_typed_tree("dump_first", source);
    let second = dump_typed_tree("dump_second", source);
    assert!(first.status.success());
    assert!(second.status.success());
    let text = String::from_utf8_lossy(&first.stdout);
    assert!(text.contains("fun main::f(a: Int): Int @1:1"), "{}", text);
    assert!(text.contains("binary +: Int @1:22"), "{}", text);
    assert_eq!(first.stdout, second.stdout);
}

/// typed-tree: "Dumping a file with type errors".
#[test]
fn test_cli_binary_dump_typed_tree_with_type_errors() {
    let output = dump_typed_tree("dump_type_error", "fun f(): Int = nope + 1\n");
    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stdout.contains("fun main::f(): Int @1:1"), "{}", stdout);
    assert!(stdout.contains("error: <error> @1:16"), "{}", stdout);
    assert!(stderr.contains("error[E4001]"), "{}", stderr);
}

/// typed-tree: "Dumping a file with a syntax error".
#[test]
fn test_cli_binary_dump_typed_tree_with_a_syntax_error() {
    let output = dump_typed_tree("dump_syntax_error", "fun f(: Int = 1\n");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.stdout.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(stderr.contains("error[E2"), "{}", stderr);
}

/// Runs `prism dump-mir` on a single-file workspace holding `source`.
fn dump_mir(test_name: &str, source: &str) -> std::process::Output {
    let temp = create_temp_dir(test_name);
    let file = temp.join("main.prism");
    fs::write(&file, source).unwrap();
    let output = Command::new(prism_bin())
        .arg("dump-mir")
        .arg(&file)
        .output()
        .expect("failed to execute prism CLI");
    fs::remove_dir_all(&temp).ok();
    output
}

/// mir: "Dumping a file".
#[test]
fn test_cli_binary_dump_mir_is_deterministic() {
    let source = "fun f(a: Int): Int = a + 1\n";
    let first = dump_mir("dump_mir_first", source);
    let second = dump_mir("dump_mir_second", source);
    assert!(first.status.success());
    assert!(second.status.success());
    let text = String::from_utf8_lossy(&first.stdout);
    assert!(text.contains("fun main::f(_1: Int): Int @1:1"), "{}", text);
    assert!(text.contains("_0 = copy _1 + const 1 @1:22"), "{}", text);
    assert_eq!(first.stdout, second.stdout);
}

/// mir: "Dumping a file with type errors".
#[test]
fn test_cli_binary_dump_mir_with_type_errors() {
    let output = dump_mir("dump_mir_type_error", "fun f(): Int = nope + 1\n");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.stdout.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(stderr.contains("error[E4001]"), "{}", stderr);
}

/// Runs `prism <args...> main.prism` on a single-file workspace holding `source`.
fn run_on_source(test_name: &str, args: &[&str], source: &str) -> std::process::Output {
    let temp = create_temp_dir(test_name);
    let file = temp.join("main.prism");
    fs::write(&file, source).unwrap();
    let output = Command::new(prism_bin())
        .args(args)
        .arg(&file)
        .output()
        .expect("failed to execute prism CLI");
    fs::remove_dir_all(&temp).ok();
    output
}

/// mir: "Dumping elaborated MIR".
#[test]
fn test_cli_binary_dump_elaborated_mir() {
    let take = "fun take(s: String) {}\n";
    let passing = format!("{}fun f(s: String) {{ take(s) }}\n", take);
    let output = run_on_source("dump_elaborated", &["dump-mir", "--elaborated"], &passing);
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{}", text);
    assert!(
        text.starts_with("module main\nphase elaborated\n"),
        "{}",
        text
    );
    let f = &text[text.find("fun main::f").unwrap()..];
    assert!(!f.contains("drop"), "{}", text);

    let failing = format!("{}fun g(s: String) {{ take(s); take(s) }}\n", take);
    let output = run_on_source(
        "dump_elaborated_errors",
        &["dump-mir", "--elaborated"],
        &failing,
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert!(
        output.stdout.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        stderr.contains("error[E5001]: use of moved value 's'"),
        "{}",
        stderr
    );

    let output = run_on_source("dump_built_with_borrow_errors", &["dump-mir"], &failing);
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{}", text);
    assert!(text.starts_with("module main\nfun "), "{}", text);
    assert!(text.contains("drop _1"), "{}", text);
}

/// diagnostics: "Rendered related location", through `prism check`.
#[test]
fn test_cli_binary_check_renders_a_use_after_move() {
    let output = run_on_source(
        "check_use_after_move",
        &["check"],
        "fun take(s: String) {}\nfun f(s: String) { take(s); take(s) }\n",
    );
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    let lines: Vec<&str> = stderr.lines().collect();
    assert_eq!(
        lines[0], "error[E5001]: use of moved value 's'",
        "{}",
        stderr
    );
    assert!(lines[1].ends_with("main.prism:2:34"), "{}", stderr);
    assert!(
        lines[3].ends_with("^ value used here after move"),
        "{}",
        stderr
    );
    assert!(lines[4].ends_with("main.prism:2:25"), "{}", stderr);
    assert!(lines[6].ends_with("^ value moved here"), "{}", stderr);
    assert_eq!(
        lines[7],
        "   = note: 's' has type 'String', which is not 'Copy'"
    );
    assert_eq!(lines[8], "   = help: clone it where it moves: 's.clone()'");
}

/// ownership: "Continue skips a declaration that the condition reads", through `prism check`.
#[test]
fn test_cli_binary_check_renders_an_uninitialized_read() {
    let output = run_on_source(
        "check_uninitialized",
        &["check"],
        "fun next(): Int = 0\nfun f(c: Bool) { do { val s = \"a\"; if (c) continue; val x = next() } while (x > 0) }\n",
    );
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.starts_with("error[E5002]: use of possibly uninitialized value 'x'\n"),
        "{}",
        stderr
    );
    assert!(stderr.contains("main.prism:2:43\n"), "{}", stderr);
    assert!(
        stderr.contains("^^^^^^^^ the declaration of 'x' is skipped here"),
        "{}",
        stderr
    );
}

/// Whether `line` holds `<label>0x` followed by 64 lowercase hexadecimal digits.
fn has_full_hash(text: &str, label: &str) -> bool {
    text.lines().any(|line| {
        line.split(label).nth(1).is_some_and(|rest| {
            let digits: String = rest.chars().take_while(|c| c.is_ascii_hexdigit()).collect();
            digits.len() == 64 && digits.chars().all(|c| !c.is_ascii_uppercase())
        })
    })
}

/// compilation-and-abi: "Hashes printed in full".
#[test]
fn hashes_printed_in_full() {
    let source = "public fun f(x: Int): Int = x\n";
    let abi = run_on_source("hashes_in_full_abi", &["abi"], source);
    assert!(abi.status.success());
    assert!(has_full_hash(
        &String::from_utf8_lossy(&abi.stdout),
        "ABI Hash: 0x"
    ));

    let check = run_on_source("hashes_in_full_check", &["check"], source);
    assert!(check.status.success());
    assert!(has_full_hash(
        &String::from_utf8_lossy(&check.stdout),
        "ABI: 0x"
    ));
}

/// compilation-and-abi: "Interface hash printed by `prism abi`".
#[test]
fn interface_hash_printed_by_prism_abi() {
    let output = run_on_source(
        "interface_hash",
        &["abi"],
        "public fun <T> keep(x: T): T = x\n",
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    let abi_line = lines.iter().position(|l| l.starts_with("ABI Hash: 0x"));
    let interface_line = lines
        .iter()
        .position(|l| l.starts_with("Interface Hash: 0x"));
    assert_eq!(interface_line, abi_line.map(|i| i + 1), "{}", stdout);
    assert!(has_full_hash(&stdout, "Interface Hash: 0x"), "{}", stdout);
}

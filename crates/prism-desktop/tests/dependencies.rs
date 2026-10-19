//! The scenario "Only the CLI depends on the window crate" (D2 of `desktop-window`): window,
//! rasterization and font crates are linked only into the `prism` binary, through
//! `prism-cli`'s default `window` feature, so no compiler crate, `prism-runtime`,
//! `prism-compose` or `prism-ui` ever builds one.

use std::path::{Path, PathBuf};

/// The crates only `prism-desktop` may name.
const WINDOW_CRATES: [&str; 4] = ["winit", "softbuffer", "tiny-skia", "fontdue"];

/// Whether `prism-cli` names `prism-desktop` yet: the CLI integration step (`desktop-window`
/// task 6.1) wires it in and sets this to `true`, which makes the dependency required.
const CLI_WIRED: bool = false;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .unwrap()
        .to_path_buf()
}

/// Every workspace member's name and manifest.
fn members() -> Vec<(String, String)> {
    let root = workspace_root();
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    let list = manifest
        .split("members = [")
        .nth(1)
        .and_then(|rest| rest.split(']').next())
        .expect("a members list");
    list.split(',')
        .map(|entry| entry.trim().trim_matches('"'))
        .filter(|entry| !entry.is_empty())
        .map(|path| {
            let name = path.rsplit('/').next().unwrap().to_string();
            let manifest = std::fs::read_to_string(root.join(path).join("Cargo.toml")).unwrap();
            (name, manifest)
        })
        .collect()
}

/// The dependency lines of a manifest: `<name> = ...` lines outside `[package]`,
/// `[features]` and profiles, comments left out.
fn dependency_lines(manifest: &str) -> Vec<(String, String)> {
    let mut section = String::new();
    let mut lines = Vec::new();
    for line in manifest.lines().map(str::trim) {
        if line.starts_with('[') {
            section = line.to_string();
            continue;
        }
        if line.is_empty() || line.starts_with('#') || !section.contains("dependencies") {
            continue;
        }
        if let Some((name, value)) = line.split_once('=') {
            lines.push((name.trim().to_string(), value.trim().to_string()));
        }
    }
    lines
}

#[test]
fn only_the_cli_depends_on_the_window_crate() {
    let members = members();
    assert!(members.iter().any(|(name, _)| name == "prism-desktop"));
    let mut cli_names_it = false;
    for (name, manifest) in &members {
        for (dependency, value) in dependency_lines(manifest) {
            if dependency == "prism-desktop" {
                assert_eq!(name, "prism-cli", "{} depends on prism-desktop", name);
                assert!(
                    value.contains("optional = true"),
                    "prism-cli's prism-desktop dependency is optional: {}",
                    value
                );
                cli_names_it = true;
            }
            if WINDOW_CRATES.contains(&dependency.as_str()) {
                assert_eq!(name, "prism-desktop", "{} depends on {}", name, dependency);
            }
        }
    }
    if cli_names_it || CLI_WIRED {
        let cli = &members
            .iter()
            .find(|(name, _)| name == "prism-cli")
            .unwrap()
            .1;
        assert!(cli_names_it, "prism-cli depends on prism-desktop");
        let features = cli
            .split("[features]")
            .nth(1)
            .expect("prism-cli has features");
        assert!(features.contains(r#"default = ["window"]"#), "{}", features);
        assert!(
            features.contains(r#"window = ["dep:prism-desktop"]"#),
            "{}",
            features
        );
    }
}

//! The compile-time benchmark of structural equality (the `code-generation` spec, "Large
//! Aggregates Are Compared by Equality Glue"): how long `prism run` spends in code generation
//! for one `==` over generated types of growing size, so that a super-linear lowering shows up
//! as a growth factor well above the size factor.
//!
//! Four shapes are generated, each at three sizes three or four times apart:
//!
//! - nested structs, a binary tree of struct types with `Int` leaves;
//! - a wide struct of `Int?` fields, each compared by presence then payload;
//! - a wide struct of `String` fields, each compared by a call of the runtime;
//! - nested enums, each variant holding the enum one level down.
//!
//! It is ignored by default, since timings of a debug build or a busy machine vary. Run it with
//!
//! ```text
//! cargo test --release -p prism-cli --test equality_timing -- --ignored --nocapture
//! ```
//!
//! It reports the code generation time of each program and the growth from one size to the
//! next, and fails when code generation grows more than twice as fast as the size only when
//! `PRISM_ASSERT_TIMING=1` is set.

use std::fmt::Write;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// A generated program: its name, its size in compared scalars, and its source.
struct Generated {
    name: String,
    size: usize,
    source: String,
}

/// Structs `S0` to `S<depth - 1>`, each holding two of the one below, `S0` two `Int`s: one
/// comparison of `S<depth - 1>` compares `2^depth` integers.
fn nested_structs(depth: usize) -> Generated {
    let mut source = String::from("struct S0 : Copy { val a: Int, val b: Int }\n");
    for level in 1..depth {
        writeln!(
            source,
            "struct S{level} : Copy {{ val a: S{below}, val b: S{below} }}",
            below = level - 1
        )
        .unwrap();
    }
    source.push_str("fun make0(x: Int): S0 = S0(x, x)\n");
    for level in 1..depth {
        writeln!(
            source,
            "fun make{level}(x: Int): S{level} = S{level}(make{below}(x), make{below}(x))",
            below = level - 1
        )
        .unwrap();
    }
    let top = depth - 1;
    writeln!(
        source,
        "import /prism/core\nfun main() {{ println(make{top}(1) == make{top}(1)); println(make{top}(1) == make{top}(2)) }}"
    )
    .unwrap();
    Generated {
        name: format!("nested-structs-{}", 1 << depth),
        size: 1 << depth,
        source,
    }
}

/// A struct of `width` `Int?` fields.
fn wide_nullables(width: usize) -> Generated {
    let fields: Vec<String> = (0..width)
        .map(|index| format!("val f{index}: Int?"))
        .collect();
    let arguments = vec!["x"; width].join(", ");
    let source = format!(
        "import /prism/core\nstruct W : Copy {{ {} }}\n\
         fun make(x: Int?): W = W({})\n\
         fun main() {{ println(make(1) == make(1)); println(make(1) == make(null)) }}\n",
        fields.join(", "),
        arguments
    );
    Generated {
        name: format!("wide-nullables-{}", width),
        size: width,
        source,
    }
}

/// A struct of `width` `String` fields, built from literals.
fn wide_strings(width: usize) -> Generated {
    let fields: Vec<String> = (0..width)
        .map(|index| format!("val f{index}: String"))
        .collect();
    let same = vec!["\"a\""; width].join(", ");
    let mut other = vec!["\"a\""; width];
    other[width - 1] = "\"b\"";
    let source = format!(
        "import /prism/core\nstruct W {{ {} }}\n\
         fun make(): W = W({})\n\
         fun other(): W = W({})\n\
         fun main() {{ println(make() == make()); println(make() == other()) }}\n",
        fields.join(", "),
        same,
        other.join(", ")
    );
    Generated {
        name: format!("wide-strings-{}", width),
        size: width,
        source,
    }
}

/// Enums `E0` to `E<depth - 1>`, each with a variant holding one and a variant holding two of
/// the one below.
fn nested_enums(depth: usize) -> Generated {
    let mut source = String::from("enum E0 : Copy { A(Int), B(Int, Int), C }\n");
    for level in 1..depth {
        writeln!(
            source,
            "enum E{level} : Copy {{ A(E{below}), B(E{below}, E{below}), C }}",
            below = level - 1
        )
        .unwrap();
    }
    source.push_str("fun make0(x: Int): E0 = E0.B(x, x)\n");
    for level in 1..depth {
        writeln!(
            source,
            "fun make{level}(x: Int): E{level} = E{level}.B(make{below}(x), make{below}(x))",
            below = level - 1
        )
        .unwrap();
    }
    let top = depth - 1;
    writeln!(
        source,
        "import /prism/core\nfun main() {{ println(make{top}(1) == make{top}(1)); println(make{top}(1) == make{top}(2)) }}"
    )
    .unwrap();
    // E0 compares a tag and three payloads, and each level a tag and three of the one below.
    let size = (1..depth).fold(4, |size, _| 1 + 3 * size);
    Generated {
        name: format!("nested-enums-{}", size),
        size,
        source,
    }
}

/// A temporary directory removed on drop.
struct TemporaryDirectory(PathBuf);

impl TemporaryDirectory {
    fn new() -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("prism_equality_timing_{}", nanos));
        fs::create_dir_all(&path).unwrap();
        TemporaryDirectory(path)
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// The milliseconds `prism run --timings` spends in code generation for `file`, the fastest of
/// three runs, after checking that it prints `true` then `false`.
fn code_generation_milliseconds(directory: &Path, file: &str) -> f64 {
    (0..3)
        .map(|_| {
            // Without the cache, so that every run compiles the program.
            let output = Command::new(env!("CARGO_BIN_EXE_prism"))
                .env("PRISM_CACHE", "off")
                .args(["run", "--timings", file])
                .current_dir(directory)
                .output()
                .expect("failed to run prism");
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert_eq!(
                output.status.code(),
                Some(0),
                "{} failed:\n{}{}",
                file,
                stdout,
                stderr
            );
            assert_eq!(stdout, "true\nfalse\n", "{} printed the wrong result", file);
            stderr
                .lines()
                .find_map(|line| line.strip_prefix("timing: code generation "))
                .and_then(|rest| rest.strip_suffix(" ms"))
                .and_then(|value| value.parse::<f64>().ok())
                .unwrap_or_else(|| panic!("no code generation timing in\n{}", stderr))
        })
        .fold(f64::INFINITY, f64::min)
}

#[test]
#[ignore = "a benchmark: run it with --ignored --nocapture"]
fn equality_compiles_in_linear_time() {
    let directory = TemporaryDirectory::new();
    let shapes: Vec<Vec<Generated>> = vec![
        [8, 10, 12].into_iter().map(nested_structs).collect(),
        [256, 1024, 4096].into_iter().map(wide_nullables).collect(),
        [256, 1024, 4096].into_iter().map(wide_strings).collect(),
        [4, 5, 6].into_iter().map(nested_enums).collect(),
    ];
    let build = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    println!(
        "code generation of one `==` and its operands, {} build:",
        build
    );
    let mut too_fast_growth = Vec::new();
    for programs in &shapes {
        let mut previous: Option<(usize, f64)> = None;
        for program in programs {
            let file = format!("{}.prism", program.name.replace('-', "_"));
            fs::write(directory.0.join(&file), &program.source).unwrap();
            let milliseconds = code_generation_milliseconds(&directory.0, &file);
            let growth = previous.map(|(size, time)| {
                let size_factor = program.size as f64 / size as f64;
                let time_factor = milliseconds / time;
                if time_factor > 2.0 * size_factor {
                    too_fast_growth.push(program.name.clone());
                }
                format!("   x{:.1} for x{:.1} the size", time_factor, size_factor)
            });
            println!(
                "  {:<24} {:>9.1} ms{}",
                program.name,
                milliseconds,
                growth.unwrap_or_default()
            );
            previous = Some((program.size, milliseconds));
        }
    }
    if std::env::var_os("PRISM_ASSERT_TIMING").is_some_and(|value| value == "1") {
        assert!(
            too_fast_growth.is_empty(),
            "code generation grows super-linearly for {:?}",
            too_fast_growth
        );
    }
}

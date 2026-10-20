//! Developer CLI for the Prism programming language (`prism`).

use prism_abi::FileAbi;
use prism_formatter::files::{FileError, FileOutcome};
use prism_formatter::FormatError;
use prism_lexer::Lexer;
use prism_parser::parse_source;
use std::env;
use std::fs;
use std::process;

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The `prism` binary's allocator: the system's, counting each thread's allocations once
/// `--frame-stats` enables it, and costing one relaxed atomic load per allocation otherwise
/// (D7 of `reactive-app-verification`). Library tests and other embedders keep their own.
#[global_allocator]
static ALLOCATOR: prism_runtime::allocation_counter::CountingAllocator =
    prism_runtime::allocation_counter::CountingAllocator;

fn print_usage() {
    eprintln!(
        r#"Prism Compiler CLI (prism) v{}

USAGE:
    prism <COMMAND> [OPTIONS] <FILE>

COMMANDS:
    check [--no-cache] [--explain-cache] <path>
                           Lex, parse, and validate source file or directory and ABI, skipping
                           files whose inputs a build cache already checked; --no-cache
                           ignores the cache, --explain-cache lists what was checked or reused,
                           then the program's instances and whether the cache holds their code
    dump-ast <file.prism>  Print the Abstract Syntax Tree for a source file
    dump-typed-tree <file.prism>
                           Print the typed tree of a source file, checked against the ABIs of
                           its imports
    dump-mir [--elaborated | --composed | --instances] <file.prism>
                           Print the MIR of a source file that typechecks; with
                           --elaborated, borrow-check it and print its elaborated MIR; with
                           --composed, print that MIR through the compose transform; with
                           --instances, compile the program the file is the entry of and
                           print the composed MIR of every instance of a generic it needs
    run [--timings] [--check-allocations] [--frame-stats] [--no-cache] [--explain-cache]
        [--watch [--edits <script>]] [--headless[=<width>x<height>] [--events <script>]]
        [--screenshot <file.png>] <file.prism> [-- <argument>...]
                           Compile the file and everything it imports in memory and run its
                           'fun main()', exiting with the program's status (101 after a
                           runtime error, 1 when it does not compile), loading from the build
                           cache every unit and instance whose code it holds; --timings prints
                           the time of each stage before main runs; --check-allocations tracks
                           every allocation and, when main returns, reports those still live
                           and any invalid free, exiting with 102; --no-cache ignores the
                           cache; --explain-cache lists on stderr what was checked, compiled or
                           loaded; --watch keeps the program running and reloads its code
                           when its files change, running main again after it returns, and
                           --edits drives those reloads from an edit script instead of file
                           events; runApp opens a window, unless --headless runs it without
                           one, in a window of the given size if one is given, and --events
                           drives it from an event script, read before the program compiles;
                           --screenshot writes each app's final frame to a PNG file;
                           --frame-stats makes each app report its frames' times and
                           allocations on stderr when it returns; the arguments after '--'
                           are the program's
    dump-clif [--instances] <file.prism>
                           Print the Cranelift IR the backend generates for a source file;
                           with --instances, compile the program the file is the entry of
                           and print the IR of every instance of a generic it needs
    dump-layout <file.prism>
                           Print the memory layout of every struct, enum and nullable type a
                           source file declares or its compiled bodies hold
    abi <file.prism>       Extract and display exported ABI symbols and ABI hash
    fmt [--check] <path>...
                           Format files, and the .prism files under directories, in place and
                           list the changed ones; with --check, only list the files that would
                           change and fail if any would; '-' formats standard input to output
    tokens <file.prism>    Tokenize source file and print stream of tokens
    version                Show version information
    help                   Show this help message

OPTIONS:
    -h, --help             Show help
    -v, --version          Show version

ENVIRONMENT:
    PRISM_CACHE            The build cache directory, or 'off' to disable the cache
                           (default: the platform's per-user cache directory, then 'prism')
"#,
        VERSION
    );
}

fn main() {
    let started = std::time::Instant::now();
    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        print_usage();
        process::exit(1);
    }

    match args[1].as_str() {
        "-v" | "--version" | "version" => {
            println!("prism version {}", VERSION);
        }
        "-h" | "--help" | "help" => {
            print_usage();
        }
        "check" => {
            let flag = |name: &str| args[2..].iter().any(|arg| arg == name);
            let (no_cache, explain_cache) = (flag("--no-cache"), flag("--explain-cache"));
            let mut paths = args[2..]
                .iter()
                .filter(|arg| !matches!(arg.as_str(), "--no-cache" | "--explain-cache"));
            match (paths.next(), paths.next()) {
                (Some(path), None) => run_check(path, no_cache, explain_cache),
                _ => {
                    eprintln!("Error: 'check' command requires a file path argument");
                    process::exit(1);
                }
            }
        }
        "dump-ast" => {
            if args.len() < 3 {
                eprintln!("Error: 'dump-ast' command requires a file path argument");
                process::exit(1);
            }
            run_dump_ast(&args[2]);
        }
        "dump-typed-tree" => {
            if args.len() < 3 {
                eprintln!("Error: 'dump-typed-tree' command requires a file path argument");
                process::exit(1);
            }
            run_dump_typed_tree(&args[2]);
        }
        "dump-mir" => {
            let flag = args.get(2).filter(|arg| {
                *arg == "--elaborated" || *arg == "--composed" || *arg == "--instances"
            });
            let path = args.get(if flag.is_some() { 3 } else { 2 });
            match path {
                Some(path) if flag.is_some_and(|flag| flag == "--instances") => {
                    run_dump(prism_cli::dump_instance_mir(std::path::Path::new(path)))
                }
                Some(path) => run_dump_mir(path, flag.map(String::as_str)),
                None => {
                    eprintln!("Error: 'dump-mir' command requires a file path argument");
                    process::exit(1);
                }
            }
        }
        "run" => run_program(&args[2..], started),
        "dump-clif" => {
            let instances = args.get(2).is_some_and(|arg| arg == "--instances");
            let expected = if instances { 4 } else { 3 };
            if args.len() != expected {
                eprintln!("Error: 'dump-clif' command requires a file path argument");
                process::exit(1);
            }
            if instances {
                run_dump(prism_cli::dump_instance_clif(std::path::Path::new(
                    &args[3],
                )));
            } else {
                run_dump_clif(&args[2]);
            }
        }
        "dump-layout" => {
            if args.len() != 3 {
                eprintln!("Error: 'dump-layout' command requires a file path argument");
                process::exit(1);
            }
            run_dump_layout(&args[2]);
        }
        "abi" => {
            if args.len() < 3 {
                eprintln!("Error: 'abi' command requires a file path argument");
                process::exit(1);
            }
            run_abi(&args[2]);
        }
        "fmt" => run_fmt(&args[2..]),
        "tokens" => {
            if args.len() < 3 {
                eprintln!("Error: 'tokens' command requires a file path argument");
                process::exit(1);
            }
            run_tokens(&args[2]);
        }
        other => {
            eprintln!("Unknown command or option: '{}'", other);
            print_usage();
            process::exit(1);
        }
    }
}

/// `prism run [--timings] [--check-allocations] [--frame-stats] [--watch [--edits <script>]]
/// [--headless[=<w>x<h>] [--events <script>]] [--screenshot <file.png>] <file.prism>
/// [-- <argument>...]` (D14 of `codegen-core`, D13 of `runtime-strings-and-drops`): options
/// before the file, and everything after `--` for the program. The build cache the environment
/// selects is used unless `--no-cache` (D9 of `instance-cache`). `--watch` runs a watch session
/// (`hot-reload`), which `--edits` drives from a script. `--headless` runs apps without a
/// window, driven by the event script of `--events`, which is parsed before anything compiles
/// (D16 of `ui-toolkit`). Otherwise, with the `window` feature, the main thread serves the
/// program's windows while the program runs on its own thread (D3 of `desktop-window`), and
/// `--screenshot` writes each app's final frame (D15).
fn run_program(args: &[String], started: std::time::Instant) {
    let usage = || -> ! {
        eprintln!(
            "Error: usage: prism run [--timings] [--check-allocations] [--frame-stats] [--no-cache] [--explain-cache] [--watch [--edits <script>]] [--headless[=<width>x<height>] [--events <script>]] [--screenshot <file.png>] <file.prism> [-- <argument>...]"
        );
        process::exit(1);
    };
    let mut headless: Option<Option<(i64, i64)>> = None;
    let mut events: Option<String> = None;
    let mut timings = false;
    let mut check_allocations = false;
    let mut frame_stats = false;
    let mut no_cache = false;
    let mut explain_cache = false;
    let mut watch = false;
    let mut edits: Option<String> = None;
    let mut screenshot: Option<std::path::PathBuf> = None;
    let mut rest = args.iter();
    let file = loop {
        match rest.next().map(String::as_str) {
            Some("--timings") => timings = true,
            Some("--watch") => watch = true,
            Some("--edits") => match rest.next() {
                Some(script) => edits = Some(script.clone()),
                None => usage(),
            },
            Some("--check-allocations") => check_allocations = true,
            Some("--frame-stats") => frame_stats = true,
            Some("--no-cache") => no_cache = true,
            Some("--explain-cache") => explain_cache = true,
            Some("--headless") => headless = Some(None),
            Some(option) if option.starts_with("--headless=") => {
                match prism_cli::parse_headless_size(&option["--headless=".len()..]) {
                    Some(size) => headless = Some(Some(size)),
                    None => usage(),
                }
            }
            Some("--events") => match rest.next() {
                Some(script) => events = Some(script.clone()),
                None => usage(),
            },
            // Screenshots need the rasterizer, which only the `window` feature links (D15 of
            // `desktop-window`).
            Some("--screenshot") if cfg!(feature = "window") => match rest.next() {
                Some(path) => screenshot = Some(std::path::PathBuf::from(path)),
                None => usage(),
            },
            Some(option) if option.starts_with('-') => usage(),
            Some(file) => break file,
            None => usage(),
        }
    };
    let arguments: Vec<String> = match rest.next().map(String::as_str) {
        None => Vec::new(),
        Some("--") => rest.cloned().collect(),
        Some(_) => usage(),
    };
    // `--edits` drives a watch session's reloads; without `--watch` there is none.
    if edits.is_some() && !watch {
        usage();
    }
    // `--events` drives a headless run; without `--headless` there is none.
    if events.is_some() && headless.is_none() {
        usage();
    }
    // The script is read and parsed before anything is compiled, so that its errors come
    // before the program's output (D16 of `ui-toolkit`).
    let headless = headless.map(|size| {
        let commands = match events.as_deref() {
            Some(path) => match prism_cli::read_event_script(std::path::Path::new(path), watch) {
                Ok(commands) => commands,
                Err(message) => {
                    eprintln!("Error: {}", message);
                    process::exit(1);
                }
            },
            None => Vec::new(),
        };
        prism_cli::HeadlessOptions { size, commands }
    });
    // The program thread's allocations are counted from its start (D7 of
    // `reactive-app-verification`).
    if frame_stats {
        prism_runtime::allocation_counter::enable();
    }
    let options = prism_cli::RunOptions {
        timings,
        check_allocations,
        frame_stats,
        arguments,
        started,
        cache: if no_cache {
            None
        } else {
            prism_cli::BuildCache::from_environment()
        },
        explain_cache,
        headless,
        screenshot,
        ..prism_cli::RunOptions::default()
    };
    #[cfg(feature = "window")]
    if options.headless.is_none() {
        let (server, windows) = prism_desktop::MainThreadServer::new();
        let file = std::path::Path::new(file);
        let status = if watch {
            prism_cli::windows::watch_with_windows(file, options, edits, server, windows)
        } else {
            prism_cli::windows::run_with_windows(file, &options, server, windows)
        };
        process::exit(status);
    }
    if watch {
        let status = prism_cli::run_watch(
            std::path::Path::new(file),
            options,
            edits.as_deref().map(std::path::Path::new),
        );
        process::exit(status);
    }
    match prism_cli::run_file(std::path::Path::new(file), &options) {
        Ok(status) => process::exit(status),
        Err(err) => {
            eprint!("{}", err.render());
            process::exit(1);
        }
    }
}

/// Prints a dump, or reports its errors with a failure status.
fn run_dump(dump: Result<String, prism_cli::WorkspaceError>) {
    match dump {
        Ok(text) => print!("{}", text),
        Err(err) => {
            eprint!("{}", err.render());
            process::exit(1);
        }
    }
}

fn run_dump_clif(path: &str) {
    match prism_cli::dump_clif(std::path::Path::new(path)) {
        Ok(text) => print!("{}", text),
        Err(err) => {
            eprint!("{}", err.render());
            process::exit(1);
        }
    }
}

fn run_dump_layout(path: &str) {
    match prism_cli::dump_layout(std::path::Path::new(path)) {
        Ok(text) => print!("{}", text),
        Err(err) => {
            eprint!("{}", err.render());
            process::exit(1);
        }
    }
}

fn read_file_or_exit(path: &str) -> String {
    match fs::read_to_string(path) {
        Ok(content) => content,
        Err(err) => {
            eprintln!("Error: Failed to read file '{}': {}", path, err);
            process::exit(1);
        }
    }
}

/// Checks `path` with the build cache the environment selects, unless `no_cache`, and with
/// `explain_cache` lists after the report which files were checked or reused (`build-cache`).
fn run_check(path: &str, no_cache: bool, explain_cache: bool) {
    let p = std::path::Path::new(path);
    let options = prism_cli::CheckOptions {
        cache: if no_cache {
            None
        } else {
            prism_cli::BuildCache::from_environment()
        },
        explain_cache,
        ..prism_cli::CheckOptions::default()
    };
    match prism_cli::check_path_with(p, &options) {
        Ok(report) => {
            println!(
                "✓ Check passed: '{}' (Workspace root: {}, Total units: {})",
                path,
                prism_cli::display_path(&report.workspace_root),
                report.total_files
            );
            for file in &report.checked_files {
                println!(
                    "  - {} (package: {}, exported: {}, ABI: 0x{})",
                    prism_cli::display_path(&file.path),
                    file.package,
                    file.exported_symbols,
                    file.abi_hash
                );
            }
            if explain_cache {
                print_cache_explanation(&report, options.cache.as_ref());
            }
        }
        Err(err) => {
            eprint!("{}", err.render());
            process::exit(1);
        }
    }
}

/// Prints the cache directory, then each file in check order, library modules first, as
/// checked or reused under its path relative to the workspace root, then the program's
/// instances once, in symbol order (D10 of `instance-cache`).
fn print_cache_explanation(report: &prism_cli::CheckReport, cache: Option<&prism_cli::BuildCache>) {
    let files: Vec<(&std::path::Path, prism_cli::CacheOutcome)> = report
        .library_files
        .iter()
        .chain(&report.checked_files)
        .map(|file| (file.path.as_path(), file.cache))
        .collect();
    print!(
        "{}",
        prism_cli::explain_cache(
            cache.map(prism_cli::BuildCache::directory),
            &report.workspace_root,
            &files,
            &report.instances
        )
    );
}

fn run_dump_ast(path: &str) {
    let content = read_file_or_exit(path);
    let ast = match parse_source(&content) {
        Ok(ast) => ast,
        Err(err) => {
            eprintln!("Syntax Error: {}", err);
            process::exit(1);
        }
    };
    println!("{:#?}", ast);
}

/// Prints the typed tree of `path`, then its type errors, if any, with a failure status.
/// Resolve, parse and ABI errors are reported as by `check`, with no tree.
fn run_dump_typed_tree(path: &str) {
    match prism_cli::dump_typed_tree(std::path::Path::new(path)) {
        Ok(dump) => {
            print!("{}", dump.text);
            if let Some(errors) = dump.type_errors {
                eprint!("{}", errors.render());
                process::exit(1);
            }
        }
        Err(err) => {
            eprint!("{}", err.render());
            process::exit(1);
        }
    }
}

/// Prints the MIR of `path`, or with `--elaborated` its borrow-checked and elaborated MIR, or with
/// `--composed` that MIR through the compose transform. A file with type errors, or with errors
/// before type checking, is not lowered, and after `--elaborated` or `--composed` a file with
/// borrow errors prints no MIR: its errors are reported with a failure status.
fn run_dump_mir(path: &str, flag: Option<&str>) {
    let path = std::path::Path::new(path);
    let dump = match flag {
        Some("--elaborated") => prism_cli::dump_elaborated_mir(path),
        Some("--composed") => prism_cli::dump_composed_mir(path),
        _ => prism_cli::dump_mir(path),
    };
    match dump {
        Ok(dump) => print!("{}", dump.text),
        Err(err) => {
            eprint!("{}", err.render());
            process::exit(1);
        }
    }
}

fn run_abi(path: &str) {
    let content = read_file_or_exit(path);
    let ast = match parse_source(&content) {
        Ok(ast) => ast,
        Err(err) => {
            eprintln!("Syntax Error: {}", err);
            process::exit(1);
        }
    };

    // Without a workspace, the module path is the declared package plus the file name.
    let stem = std::path::Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    let module = ast
        .package
        .as_ref()
        .map(|p| format!("{}/{}", p.package_string().replace('.', "/"), stem))
        .unwrap_or_else(|| stem.to_string());
    // A file of the root package `prism` is shown as a library module, which may declare
    // external and intrinsic declarations (D12 of `standard-library`).
    let in_library = ast
        .package
        .as_ref()
        .is_some_and(|package| package.path.first().map(String::as_str) == Some("prism"));
    // The reachable definitions are the file's own, as `FileAbi::extract` finds them.
    let own = prism_abi::type_definitions(&ast, &module, &prism_abi::no_imports);
    let definition = |type_module: &str, name: &str| {
        own.iter()
            .find(|d| d.module == type_module && d.name == name)
            .cloned()
    };
    let abi = match FileAbi::extract_with_origin(
        &ast,
        &module,
        &prism_abi::no_imports,
        &definition,
        in_library,
    ) {
        Ok(abi) => abi,
        Err(err) => {
            eprintln!("ABI Error: {}", err);
            process::exit(1);
        }
    };

    println!("Package: {}", abi.package.as_deref().unwrap_or("<root>"));
    println!("ABI Hash: 0x{}", abi.abi_hash_hex());
    println!("Exported Symbols ({}):", abi.symbols.len());
    for sym in &abi.symbols {
        match sym {
            prism_abi::ExportedSymbol::Function {
                visibility,
                kind,
                is_compose,
                is_capture,
                is_operator,
                receiver,
                context,
                name,
                type_parameters,
                params,
                return_type,
            } => {
                let context = context
                    .as_ref()
                    .map_or_else(String::new, |c| format!("context({}) ", c));
                let capture = if *is_capture { "capture " } else { "" };
                let operator = if *is_operator { "operator " } else { "" };
                let comp = match kind.as_str() {
                    "" if *is_compose => format!("{}compose ", operator),
                    "" => operator.to_string(),
                    library if *is_compose => {
                        format!("{} {}{}compose ", library, operator, capture)
                    }
                    library => format!("{} {}{}", library, operator, capture),
                };
                let (mode, receiver) = match receiver {
                    Some(r) if r.mode.as_str().is_empty() => (String::new(), format!("{}.", r.ty)),
                    Some(r) => (format!("{} ", r.mode.as_str()), format!("{}.", r.ty)),
                    None => (String::new(), String::new()),
                };
                let param_str = params
                    .iter()
                    .map(|p| {
                        let modifier = p.kind;
                        match &p.default {
                            Some(d) => format!("{}: {}{} = {}", p.name, modifier, p.ty, d),
                            None => format!("{}: {}{}", p.name, modifier, p.ty),
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                let type_parameters = match type_parameter_list(type_parameters) {
                    list if list.is_empty() => list,
                    list => format!("{} ", list),
                };
                println!(
                    "  {}{} {}{}fun {}{}{}({}): {}",
                    context,
                    visibility,
                    comp,
                    mode,
                    type_parameters,
                    receiver,
                    name,
                    param_str,
                    return_type
                );
            }
            prism_abi::ExportedSymbol::ExtensionProperty { .. } => {
                if let Some(text) = sym.declaration_text() {
                    println!("  {}", text);
                }
            }
            prism_abi::ExportedSymbol::Struct {
                visibility,
                name,
                type_parameters,
                marker,
                marker_parameters,
                fields,
                intrinsic: true,
                ..
            } if fields.is_empty() => {
                println!(
                    "  {} intrinsic struct {}{}{}",
                    visibility,
                    name,
                    type_parameter_list(type_parameters),
                    generic_marker_suffix(*marker, type_parameters, marker_parameters),
                );
            }
            prism_abi::ExportedSymbol::Struct {
                visibility,
                name,
                type_parameters,
                marker,
                marker_parameters,
                fields,
                equality,
                drop_hook,
                clone_hook,
                ..
            } => {
                let f_str = fields
                    .iter()
                    .map(|(f, is_var, t)| {
                        format!("{} {}: {}", if *is_var { "var" } else { "val" }, f, t)
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                println!(
                    "  {} struct {}{}{} {{ {} }}{}",
                    visibility,
                    name,
                    type_parameter_list(type_parameters),
                    generic_marker_suffix(*marker, type_parameters, marker_parameters),
                    f_str,
                    definition_suffix(*equality, *drop_hook, *clone_hook)
                );
            }
            prism_abi::ExportedSymbol::Enum {
                visibility,
                name,
                type_parameters,
                marker,
                marker_parameters,
                variants,
                equality,
                drop_hook,
                clone_hook,
            } => {
                let v_str = variants
                    .iter()
                    .map(|(v, flds)| {
                        if flds.is_empty() {
                            v.clone()
                        } else {
                            let flds: Vec<String> = flds.iter().map(ToString::to_string).collect();
                            format!("{}({})", v, flds.join(", "))
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                println!(
                    "  {} enum {}{}{} {{ {} }}{}",
                    visibility,
                    name,
                    type_parameter_list(type_parameters),
                    generic_marker_suffix(*marker, type_parameters, marker_parameters),
                    v_str,
                    definition_suffix(*equality, *drop_hook, *clone_hook)
                );
            }
            prism_abi::ExportedSymbol::Var {
                visibility,
                name,
                ty,
            } => {
                println!("  {} val {}: {}", visibility, name, ty);
            }
        }
    }
    println!("Reachable Types ({}):", abi.type_definitions.len());
    for definition in &abi.type_definitions {
        let members = match &definition.members {
            prism_abi::AbiTypeMembers::Fields(fields) => fields
                .iter()
                .map(|(f, is_var, t)| {
                    format!("{} {}: {}", if *is_var { "var" } else { "val" }, f, t)
                })
                .collect::<Vec<_>>(),
            prism_abi::AbiTypeMembers::Variants(variants) => variants
                .iter()
                .map(|(v, payloads)| {
                    if payloads.is_empty() {
                        v.clone()
                    } else {
                        let payloads: Vec<String> =
                            payloads.iter().map(ToString::to_string).collect();
                        format!("{}({})", v, payloads.join(", "))
                    }
                })
                .collect(),
        };
        let kind = match definition.kind() {
            prism_abi::NominalKind::Struct => "struct",
            prism_abi::NominalKind::Enum => "enum",
        };
        println!(
            "  {} {}::{}{}{} {{ {} }}{}",
            kind,
            definition.module,
            definition.name,
            type_parameter_list(&definition.type_parameters),
            generic_marker_suffix(
                definition.marker,
                &definition.type_parameters,
                &definition.marker_parameters
            ),
            members.join(", "),
            definition_suffix(
                definition.equality,
                definition.drop_hook,
                definition.clone_hook
            )
        );
    }
}

/// What `prism abi` prints after a struct or enum whose file declares its `equals` operator,
/// its drop hook or its clone hook (D10 of `drop-and-clone-hooks`): ` with operator equals, a
/// drop hook, a clone hook`, naming those it has.
fn definition_suffix(equality: bool, drop_hook: bool, clone_hook: bool) -> String {
    let parts: Vec<&str> = [
        (equality, "operator equals"),
        (drop_hook, "a drop hook"),
        (clone_hook, "a clone hook"),
    ]
    .into_iter()
    .filter_map(|(present, part)| present.then_some(part))
    .collect();
    if parts.is_empty() {
        String::new()
    } else {
        format!(" with {}", parts.join(", "))
    }
}

/// `prism fmt [--check] <path>...` (the `formatting` spec). Exits with status 1 when a file
/// could not be formatted, or with `--check` when a file is not formatted.
fn run_fmt(arguments: &[String]) {
    let check = arguments.iter().any(|argument| argument == "--check");
    let paths: Vec<&String> = arguments
        .iter()
        .filter(|argument| argument.as_str() != "--check")
        .collect();
    if let Some(option) = paths
        .iter()
        .find(|path| path.starts_with("--") || path.starts_with('-') && path.len() > 1)
    {
        eprintln!("Error: unknown option '{}' for 'fmt'", option);
        process::exit(1);
    }
    if paths.is_empty() {
        eprintln!(
            "Error: 'fmt' command requires a file or directory path argument, or '-' for standard input"
        );
        process::exit(1);
    }
    let mut failed = false;
    let mut unformatted = false;
    for path in paths {
        if path == "-" {
            match fmt_standard_input(check) {
                Ok(changed) => unformatted |= changed,
                Err(()) => failed = true,
            }
            continue;
        }
        let files = match prism_formatter::files::collect_files(&[path.into()]) {
            Ok(files) => files,
            Err((path, err)) => {
                report_io_error(&path, &err);
                failed = true;
                continue;
            }
        };
        for file in files {
            match prism_formatter::files::format_file(&file, !check) {
                FileOutcome::Unchanged => {}
                FileOutcome::Changed => {
                    println!("{}", file.display());
                    unformatted = true;
                }
                FileOutcome::Failed { source, error } => {
                    report_format_error(&file, source.as_deref(), &error);
                    failed = true;
                }
            }
        }
    }
    if failed || check && unformatted {
        process::exit(1);
    }
}

/// Formats standard input to standard output, or with `check` only reports whether it is
/// formatted. Returns whether the input was not formatted.
fn fmt_standard_input(check: bool) -> Result<bool, ()> {
    let path = std::path::Path::new("<stdin>");
    let mut source = String::new();
    if let Err(err) = std::io::Read::read_to_string(&mut std::io::stdin(), &mut source) {
        report_io_error(path, &err);
        return Err(());
    }
    match prism_formatter::format_source(&source) {
        Ok(formatted) => {
            let changed = formatted != source;
            if !check {
                print!("{}", formatted);
            }
            Ok(changed)
        }
        Err(err) => {
            report_format_error(path, Some(&source), &FileError::Format(err));
            Err(())
        }
    }
}

fn report_io_error(path: &std::path::Path, err: &std::io::Error) {
    let err = prism_resolver::ResolveError::Io {
        path: path.to_path_buf(),
        message: err.to_string(),
    };
    eprint!("{}", prism_cli::format_resolve_error(&err));
}

fn report_format_error(path: &std::path::Path, source: Option<&str>, error: &FileError) {
    match error {
        FileError::Io(err) => report_io_error(path, err),
        FileError::Format(FormatError::Syntax(err)) => {
            eprint!("{}", prism_cli::format_parse_error(path, source, err));
        }
        FileError::Format(FormatError::Internal(reason)) => {
            eprintln!(
                "internal error: cannot format '{}': {}. This is a bug in prism fmt.",
                path.display(),
                reason
            );
        }
    }
}

fn run_tokens(path: &str) {
    let content = read_file_or_exit(path);
    let tokens = Lexer::new(&content).tokenize_all();
    for tok in tokens {
        println!("{:?} at {}", tok.kind, tok.span);
    }
}

/// A declaration's type parameters with their bounds (`<A, B : Copy>`), or nothing.
fn type_parameter_list(parameters: &[prism_abi::AbiTypeParameter]) -> String {
    if parameters.is_empty() {
        return String::new();
    }
    let parameters: Vec<String> = parameters
        .iter()
        .map(|p| match p.bound {
            Some(bound) => format!("{} : {}", p.name, bound.as_str()),
            None => p.name.clone(),
        })
        .collect();
    format!("<{}>", parameters.join(", "))
}

/// The marker suffix of a generic struct or enum, with the type parameters that condition it
/// (` : Copy if T`), as the typed-tree dump prints it (D19 of `generics`).
fn generic_marker_suffix(
    marker: Option<prism_ast::TypeMarker>,
    parameters: &[prism_abi::AbiTypeParameter],
    marker_parameters: &[u32],
) -> String {
    let suffix = marker_suffix(marker);
    if marker.is_none() || marker_parameters.is_empty() {
        return suffix;
    }
    let names: Vec<&str> = marker_parameters
        .iter()
        .filter_map(|index| parameters.get(*index as usize))
        .map(|p| p.name.as_str())
        .collect();
    format!("{} if {}", suffix, names.join(", "))
}

/// The ` : Copy` or ` : Clone` suffix of a struct or enum in the ABI dump.
fn marker_suffix(marker: Option<prism_ast::TypeMarker>) -> String {
    marker.map_or(String::new(), |m| format!(" : {}", m.as_str()))
}

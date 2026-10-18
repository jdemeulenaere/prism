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

fn print_usage() {
    eprintln!(
        r#"Prism Compiler CLI (prism) v{}

USAGE:
    prism <COMMAND> [OPTIONS] <FILE>

COMMANDS:
    check [--no-cache] [--explain-cache] <path>
                           Lex, parse, and validate source file or directory and ABI, skipping
                           files whose inputs a build cache already checked; --no-cache
                           ignores the cache, --explain-cache lists what was checked or reused
    dump-ast <file.prism>  Print the Abstract Syntax Tree for a source file
    dump-typed-tree <file.prism>
                           Print the typed tree of a source file, checked against the ABIs of
                           its imports
    dump-mir [--elaborated] <file.prism>
                           Print the MIR of a source file that typechecks; with
                           --elaborated, borrow-check it and print its elaborated MIR
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
            let elaborated = args.get(2).is_some_and(|arg| arg == "--elaborated");
            let path = args.get(if elaborated { 3 } else { 2 });
            match path {
                Some(path) => run_dump_mir(path, elaborated),
                None => {
                    eprintln!("Error: 'dump-mir' command requires a file path argument");
                    process::exit(1);
                }
            }
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

/// Prints the cache directory, then each file in check order as checked or reused, with the
/// instances it requests, under its path relative to the workspace root.
fn print_cache_explanation(report: &prism_cli::CheckReport, cache: Option<&prism_cli::BuildCache>) {
    match cache {
        Some(cache) => println!("Cache: {}", cache.directory().display()),
        None => println!("Cache: off"),
    }
    let root = match report.workspace_root.as_os_str().is_empty() {
        true => std::path::Path::new("."),
        false => report.workspace_root.as_path(),
    };
    let root = root.canonicalize().ok();
    for file in &report.checked_files {
        let canonical = file.path.canonicalize().ok();
        let relative = root
            .as_ref()
            .zip(canonical.as_ref())
            .and_then(|(root, path)| path.strip_prefix(root).ok())
            .unwrap_or(&file.path);
        let shown: Vec<String> = relative
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        let shown = shown.join("/");
        let outcome = match file.cache {
            prism_cli::CacheOutcome::Checked => "checked",
            prism_cli::CacheOutcome::Reused => "reused",
        };
        println!("  {} {}", outcome, shown);
        for instance in &file.instances {
            let outcome = if instance.reused {
                "reused instance"
            } else {
                "instantiate"
            };
            println!("  {} {} for {}", outcome, instance.instance, shown);
        }
    }
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

/// Prints the MIR of `path`, or with `elaborated` its borrow-checked and elaborated MIR. A file
/// with type errors, or with errors before type checking, is not lowered, and with `elaborated`
/// a file with borrow errors prints no MIR: its errors are reported with a failure status.
fn run_dump_mir(path: &str, elaborated: bool) {
    let path = std::path::Path::new(path);
    let dump = if elaborated {
        prism_cli::dump_elaborated_mir(path)
    } else {
        prism_cli::dump_mir(path)
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
    let abi = match FileAbi::extract(&ast, &module, &prism_abi::no_imports) {
        Ok(abi) => abi,
        Err(err) => {
            eprintln!("ABI Error: {}", err);
            process::exit(1);
        }
    };

    println!("Package: {}", abi.package.as_deref().unwrap_or("<root>"));
    println!("ABI Hash: 0x{}", abi.abi_hash_hex());
    println!("Interface Hash: 0x{}", abi.interface_hash_hex());
    println!("Exported Symbols ({}):", abi.symbols.len());
    for sym in &abi.symbols {
        match sym {
            prism_abi::ExportedSymbol::Function {
                visibility,
                is_compose,
                receiver,
                name,
                type_parameters,
                params,
                return_type,
                ..
            } => {
                let comp = if *is_compose { "compose " } else { "" };
                let (mode, receiver) = match receiver {
                    Some(r) if r.mode.as_str().is_empty() => (String::new(), format!("{}.", r.ty)),
                    Some(r) => (format!("{} ", r.mode.as_str()), format!("{}.", r.ty)),
                    None => (String::new(), String::new()),
                };
                let param_str = params
                    .iter()
                    .map(|(p, t, def)| {
                        if let Some(d) = def {
                            format!("{}: {} = {}", p, t, d)
                        } else {
                            format!("{}: {}", p, t)
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                let type_parameters = match type_parameter_list(type_parameters) {
                    list if list.is_empty() => list,
                    list => format!("{} ", list),
                };
                println!(
                    "  {} {}{}fun {}{}{}({}): {}",
                    visibility, comp, mode, type_parameters, receiver, name, param_str, return_type
                );
            }
            prism_abi::ExportedSymbol::Struct {
                visibility,
                name,
                type_parameters,
                marker,
                marker_parameters,
                fields,
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
                    "  {} struct {}{}{} {{ {} }}",
                    visibility,
                    name,
                    type_parameter_list(type_parameters),
                    generic_marker_suffix(*marker, type_parameters, marker_parameters),
                    f_str
                );
            }
            prism_abi::ExportedSymbol::Enum {
                visibility,
                name,
                type_parameters,
                marker,
                marker_parameters,
                variants,
                ..
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
                    "  {} enum {}{}{} {{ {} }}",
                    visibility,
                    name,
                    type_parameter_list(type_parameters),
                    generic_marker_suffix(*marker, type_parameters, marker_parameters),
                    v_str
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

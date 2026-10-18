//! Developer CLI for the Prism programming language (`prism`).

use prism_abi::FileAbi;
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
    check <path>           Lex, parse, and validate source file or directory and ABI
    dump-ast <file.prism>  Print the Abstract Syntax Tree for a source file
    dump-typed-tree <file.prism>
                           Print the typed tree of a source file, checked against the ABIs of
                           its imports
    dump-mir [--elaborated] <file.prism>
                           Print the MIR of a source file that typechecks; with
                           --elaborated, borrow-check it and print its elaborated MIR
    abi <file.prism>       Extract and display exported ABI symbols and ABI hash
    tokens <file.prism>    Tokenize source file and print stream of tokens
    version                Show version information
    help                   Show this help message

OPTIONS:
    -h, --help             Show help
    -v, --version          Show version
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
            if args.len() < 3 {
                eprintln!("Error: 'check' command requires a file path argument");
                process::exit(1);
            }
            run_check(&args[2]);
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

fn run_check(path: &str) {
    let p = std::path::Path::new(path);
    match prism_cli::check_path(p) {
        Ok(report) => {
            println!(
                "✓ Check passed: '{}' (Workspace root: {}, Total units: {})",
                path,
                report.workspace_root.display(),
                report.total_files
            );
            for file in &report.checked_files {
                println!(
                    "  - {} (package: {}, exported: {}, ABI: 0x{})",
                    file.path.display(),
                    file.package,
                    file.exported_symbols,
                    file.abi_hash
                );
            }
        }
        Err(err) => {
            eprint!("{}", err.render());
            process::exit(1);
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

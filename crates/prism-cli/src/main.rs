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
    check <file.prism>     Lex, parse, and validate source file and ABI
    dump-ast <file.prism>  Print the Abstract Syntax Tree for a source file
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
    let content = read_file_or_exit(path);
    let ast = match parse_source(&content) {
        Ok(ast) => ast,
        Err(err) => {
            eprintln!("Syntax Error: {}", err);
            process::exit(1);
        }
    };

    let abi = match FileAbi::extract(&ast) {
        Ok(abi) => abi,
        Err(err) => {
            eprintln!("ABI Error: {}", err);
            process::exit(1);
        }
    };

    let pkg = abi.package.as_deref().unwrap_or("<root>");
    println!(
        "✓ Check passed: '{}' (Package: {}, Exported symbols: {}, ABI Hash: 0x{})",
        path,
        pkg,
        abi.symbols.len(),
        abi.abi_hash_hex()
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

fn run_abi(path: &str) {
    let content = read_file_or_exit(path);
    let ast = match parse_source(&content) {
        Ok(ast) => ast,
        Err(err) => {
            eprintln!("Syntax Error: {}", err);
            process::exit(1);
        }
    };

    let abi = match FileAbi::extract(&ast) {
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
                is_compose,
                name,
                params,
                return_type,
                ..
            } => {
                let comp = if *is_compose { "compose " } else { "" };
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
                println!(
                    "  {} {}fun {}({}): {}",
                    visibility, comp, name, param_str, return_type
                );
            }
            prism_abi::ExportedSymbol::Struct {
                visibility,
                name,
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
                println!("  {} struct {} {{ {} }}", visibility, name, f_str);
            }
            prism_abi::ExportedSymbol::Enum {
                visibility,
                name,
                variants,
                ..
            } => {
                let v_str = variants
                    .iter()
                    .map(|(v, flds)| {
                        if flds.is_empty() {
                            v.clone()
                        } else {
                            format!("{}({})", v, flds.join(", "))
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                println!("  {} enum {} {{ {} }}", visibility, name, v_str);
            }
            prism_abi::ExportedSymbol::Var {
                visibility,
                name,
                is_var,
                ty,
            } => {
                println!(
                    "  {} {} {}: {}",
                    visibility,
                    if *is_var { "var" } else { "val" },
                    name,
                    ty
                );
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

//! `prism-runtime`: the functions that generated Prism code calls, and the process state behind
//! them (D17 of `codegen-core`).
//!
//! Generated code calls the `extern "C"` functions named `prism_rt_*` directly, by symbol: the
//! Tier 1 JIT registers every entry of [`RUNTIME_FUNCTIONS`] with its symbol table, and declares
//! each import with the signature the entry records. The runtime is never reloaded, so these
//! calls do not go through the function table.
//!
//! A run is bracketed by [`start`], which records the program's arguments and whether stdout is
//! a terminal, and [`finish`], which flushes the program's output. A runtime error
//! ([`prism_rt_panic`]) flushes the output, reports the error and ends the process with status
//! [`PANIC_EXIT_STATUS`], without unwinding (D12).
//!
//! Generated code also reads one runtime static, [`STACK_LIMIT`], to detect a stack overflow
//! before it reaches the guard page.
//!
//! This crate depends on nothing, so Tier 2 can link it into native executables without
//! Cranelift (D1).

mod double;
mod functions;
mod output;
mod panic;
mod stack;

pub use double::format_double;
pub use functions::{
    runtime_function, RuntimeAddress, RuntimeFunction, RuntimeType, RUNTIME_FUNCTIONS,
};
pub use output::{
    finish, format_bool, format_char, format_int, prism_rt_print_bool, prism_rt_print_char,
    prism_rt_print_double, prism_rt_print_int, prism_rt_print_newline, start, OUTPUT_BUFFER_SIZE,
};
pub use panic::{
    panic_message, prism_rt_double_remainder, prism_rt_panic, prism_rt_unlinked_function,
    PanicKind, PANIC_EXIT_STATUS,
};
pub use stack::{
    clear_stack_limit, set_stack_limit, STACK_LIMIT, STACK_LIMIT_SYMBOL, STACK_RESERVE,
};

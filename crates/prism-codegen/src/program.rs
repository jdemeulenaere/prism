//! A linked program and how it runs (D19 and D27 of `codegen-core`).

use crate::error::BackendError;
use crate::instances::{check_instances, collect_instances};
use crate::link::{FunctionTable, Linker};
use crate::symbols::function_symbol;
use cranelift_jit::JITModule;
use prism_mir::MirFile;
use prism_typed_tree::DeclarationPath;
use std::sync::Arc;

/// The stack size of the thread a program runs on, the same on every platform.
pub const PROGRAM_STACK_SIZE: usize = 8 * 1024 * 1024;

/// The files of a program, compiled and linked in memory.
pub struct Program {
    table: Arc<FunctionTable>,
    /// The loads whose code the table points into; their memory is never freed while the
    /// program lives.
    _modules: Vec<JITModule>,
}

impl Program {
    pub(crate) fn new(table: Arc<FunctionTable>, module: JITModule) -> Self {
        Self {
            table,
            _modules: vec![module],
        }
    }

    /// Compiles and links `files`, each of which passed [`crate::check_file`], with the
    /// instances of generics they need, which must pass [`crate::check_instances`].
    pub fn compile(files: &[MirFile]) -> Result<Program, BackendError> {
        let instances = collect_instances(files)
            .map_err(|errors| BackendError::new(format!("instances rejected: {:?}", errors)))?;
        check_instances(&instances)
            .map_err(|errors| BackendError::new(format!("instances rejected: {:?}", errors)))?;
        let mut linker = Linker::new()?;
        for file in files {
            linker.add_file(file)?;
        }
        for instance in &instances {
            linker.add_instance(instance)?;
        }
        linker.link()
    }

    /// The program's function table.
    pub fn table(&self) -> &FunctionTable {
        &self.table
    }

    /// The address the slot of `symbol` holds: the linked function, or the unlinked stub.
    pub fn function_address(&self, symbol: &str) -> Option<*const u8> {
        self.table.get(symbol)
    }

    /// Runs the program from `main`, a function without parameters or result, on a thread of
    /// its own with a fixed stack, with `arguments` recorded by the runtime and the allocation
    /// check on when `check_allocations` is. Returns the exit status the runtime's `finish`
    /// gives: 0 when `main` returns, 102 when the check finds allocations still live. A runtime
    /// error ends the process with status 101 instead, and an invalid free under the check with
    /// status 102.
    pub fn run(
        &self,
        main: &DeclarationPath,
        arguments: Vec<String>,
        check_allocations: bool,
    ) -> Result<i32, BackendError> {
        let symbol = function_symbol(main);
        let address = self
            .function_address(&symbol)
            .ok_or_else(|| BackendError::new(format!("{} is not linked", symbol)))?
            as usize;
        let thread = std::thread::Builder::new()
            .name("main".to_string())
            .stack_size(PROGRAM_STACK_SIZE)
            .spawn(move || {
                // The stack grows down from about here: generated code raises "stack overflow"
                // before it reaches the end (D27).
                let top = 0u8;
                let top = std::hint::black_box(std::ptr::addr_of!(top)) as usize;
                prism_runtime::set_stack_limit(top, PROGRAM_STACK_SIZE);
                prism_runtime::start(arguments, check_allocations);
                // SAFETY: `address` is the compiled `main`, whose signature `find_main` checked:
                // no parameters, no result, the host's C calling convention.
                let main: extern "C" fn() = unsafe { std::mem::transmute(address) };
                main();
                prism_runtime::clear_stack_limit();
                prism_runtime::finish()
            })
            .map_err(|error| BackendError::new(format!("starting the program: {}", error)))?;
        thread
            .join()
            .map_err(|_| BackendError::new("the program's thread panicked"))
    }
}

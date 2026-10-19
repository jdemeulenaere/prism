//! A linked program and how it runs (D19 and D27 of `codegen-core`).

use crate::error::BackendError;
use crate::instances::{check_instances, collect_instances};
use crate::link::{FunctionTable, Linker};
use crate::reload::LiveProgram;
use crate::symbols::function_symbol;
use prism_mir::MirFile;
use prism_typed_tree::DeclarationPath;

/// The stack size of the thread a program runs on, the same on every platform.
pub const PROGRAM_STACK_SIZE: usize = 8 * 1024 * 1024;

/// The files of a program, compiled and linked in memory, with the loads a watch session
/// reloads into it.
pub struct Program {
    live: LiveProgram,
}

/// A program whose `main` runs on its own thread ([`Program::start`]).
pub struct RunningProgram {
    thread: std::thread::JoinHandle<i32>,
}

impl RunningProgram {
    /// Waits for `main` to return and gives the exit status [`prism_runtime::finish`] gave.
    pub fn join(self) -> Result<i32, BackendError> {
        self.thread
            .join()
            .map_err(|_| BackendError::new("the program's thread panicked"))
    }

    /// Whether the program's thread has ended.
    pub fn is_finished(&self) -> bool {
        self.thread.is_finished()
    }
}

impl Program {
    pub(crate) fn new(live: LiveProgram) -> Self {
        Self { live }
    }

    /// The program's code and tables across reloads.
    pub fn live(&self) -> &LiveProgram {
        &self.live
    }

    /// The program's code and tables across reloads, to prepare and hold a reload.
    pub fn live_mut(&mut self) -> &mut LiveProgram {
        &mut self.live
    }

    /// Compiles and links `files`, each of which passed [`crate::check_file`], with the
    /// instances of generics they need, which must pass [`crate::check_instances`].
    pub fn compile(files: &[MirFile]) -> Result<Program, BackendError> {
        let instances = collect_instances(&files.iter().collect::<Vec<_>>())
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
        self.live.table()
    }

    /// The address the slot of `symbol` holds: the linked function, or the unlinked stub.
    pub fn function_address(&self, symbol: &str) -> Option<*const u8> {
        self.live.table().get(symbol)
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
        self.start(main, arguments, check_allocations, |_| {})?
            .join()
    }

    /// Starts the program from `main` as [`Program::run`] does, without waiting for it:
    /// `on_return` is called on the program's thread with the exit status once `main` returned
    /// and the run finished. Under watch mode a runtime error parks the thread instead
    /// (`prism_runtime::set_watch_mode`), and `on_return` is never called.
    pub fn start(
        &self,
        main: &DeclarationPath,
        arguments: Vec<String>,
        check_allocations: bool,
        on_return: impl FnOnce(i32) + Send + 'static,
    ) -> Result<RunningProgram, BackendError> {
        let symbol = function_symbol(main);
        let address = self
            .function_address(&symbol)
            .ok_or_else(|| BackendError::new(format!("{} is not linked", symbol)))?
            as usize;
        let thread = std::thread::Builder::new()
            .name("main".to_string())
            .stack_size(PROGRAM_STACK_SIZE)
            .spawn(move || {
                // Under the allocation check, the cells the program owns are freed, then the
                // compositions still live when `main` returns are reported, before the leaks
                // (D13 of `compose-runtime`, D12 of `state-handles`). The report reads this
                // thread's cells and compositions, and `finish` runs on this thread.
                prism_runtime::set_finish_report(Some(prism_compose::finish_report));
                // A reload applied on this thread invalidates the scopes of the compositions it
                // affects (D3 of `compose-hot-reload`).
                prism_runtime::reload::set_applied_hook(Some(
                    prism_compose::reload::reload_applied,
                ));
                prism_runtime::start(arguments, check_allocations);
                // The stack grows down from about here: generated code raises "stack overflow"
                // before it reaches the end (D27).
                let top = 0u8;
                let top = std::hint::black_box(std::ptr::addr_of!(top)) as usize;
                prism_runtime::set_stack_limit(top, PROGRAM_STACK_SIZE);
                // SAFETY: `address` is the compiled `main`, whose signature `find_main` checked:
                // no parameters, no result, the host's C calling convention.
                let main: extern "C" fn() = unsafe { std::mem::transmute(address) };
                main();
                prism_runtime::clear_stack_limit();
                let status = prism_runtime::finish();
                on_return(status);
                status
            })
            .map_err(|error| BackendError::new(format!("starting the program: {}", error)))?;
        Ok(RunningProgram { thread })
    }
}

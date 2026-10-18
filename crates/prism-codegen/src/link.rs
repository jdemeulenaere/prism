//! The function table and the linker (D3 and D4 of `codegen-core`).
//!
//! The function table holds one pointer-sized slot per symbol of a function, default value
//! generator or top-level value accessor, allocated in chunks that never move, so a slot keeps
//! its address for as long as the table lives. A slot nothing defines holds the address of
//! `prism_rt_unlinked_function`. Code refers to a slot only by the symbol `<symbol>#slot`, which
//! the JIT resolves through the table when a load is finalized; after finalizing, the linker
//! writes the address of each function the load defines into its slot.
//!
//! One table serves a whole program for the life of the process (`prism run` links one
//! program), and `hot-reload` will repoint its slots. It is owned by the [`Linker`] and then the
//! [`crate::Program`] rather than being a global, so a test process can link several programs.

use crate::error::BackendError;
use crate::isa::isa;
use crate::program::Program;
use crate::symbols::slot_owner;
use crate::unit::CodeUnit;
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{default_libcall_names, FuncId, Module};
use prism_mir::MirFile;
use prism_runtime::{prism_rt_unlinked_function, RUNTIME_FUNCTIONS};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// The number of slots in one chunk of the table.
const CHUNK_SIZE: usize = 256;

/// The process-wide table of function slots.
pub struct FunctionTable {
    inner: Mutex<TableInner>,
}

struct TableInner {
    /// The chunks of slots; a chunk is never moved or freed while the table lives.
    chunks: Vec<Box<[AtomicUsize]>>,
    /// The slots used in the last chunk.
    used: usize,
    /// The slot of each symbol.
    slots: HashMap<String, *const AtomicUsize>,
}

// SAFETY: the raw pointers point into the table's own chunks, which are only freed with the
// table, and every access to a slot is atomic.
unsafe impl Send for TableInner {}

impl FunctionTable {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(TableInner {
                chunks: Vec::new(),
                used: CHUNK_SIZE,
                slots: HashMap::new(),
            }),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, TableInner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The slot of `symbol`, allocated on first use and holding the unlinked stub until a
    /// function is linked into it.
    pub fn slot(&self, symbol: &str) -> *const AtomicUsize {
        let mut inner = self.lock();
        if let Some(slot) = inner.slots.get(symbol) {
            return *slot;
        }
        if inner.used == CHUNK_SIZE {
            let unlinked = prism_rt_unlinked_function as *const u8 as usize;
            let chunk: Box<[AtomicUsize]> = (0..CHUNK_SIZE)
                .map(|_| AtomicUsize::new(unlinked))
                .collect();
            inner.chunks.push(chunk);
            inner.used = 0;
        }
        let index = inner.used;
        inner.used += 1;
        let slot: *const AtomicUsize = &inner.chunks.last().expect("a chunk exists")[index];
        inner.slots.insert(symbol.to_string(), slot);
        slot
    }

    /// The address `symbol`'s slot holds, if it has a slot.
    pub fn get(&self, symbol: &str) -> Option<*const u8> {
        let slot = *self.lock().slots.get(symbol)?;
        // SAFETY: the slot lives as long as the table.
        Some(unsafe { (*slot).load(Ordering::Acquire) } as *const u8)
    }

    /// Points `symbol`'s slot at `address`.
    pub fn set(&self, symbol: &str, address: *const u8) {
        let slot = self.slot(symbol);
        // SAFETY: the slot lives as long as the table.
        unsafe { (*slot).store(address as usize, Ordering::Release) };
    }
}

/// Builds one load of a program: the code units of its files, defined into one JIT module and
/// finalized together (D3).
pub struct Linker {
    table: Arc<FunctionTable>,
    module: JITModule,
    /// The functions the load defines, by symbol.
    defined: Vec<(String, FuncId)>,
}

impl Linker {
    /// A linker with a new function table, whose JIT resolves the runtime functions by name
    /// and every `<symbol>#slot` to the table's slot of `symbol`.
    pub fn new() -> Result<Self, BackendError> {
        let table = FunctionTable::new();
        let mut builder = JITBuilder::with_isa(isa()?, default_libcall_names());
        builder.symbols(
            RUNTIME_FUNCTIONS
                .iter()
                .map(|function| (function.name, function.address.as_ptr())),
        );
        let lookup_table = Arc::clone(&table);
        builder.symbol_lookup_fn(Box::new(move |name| {
            slot_owner(name).map(|symbol| lookup_table.slot(symbol).cast::<u8>())
        }));
        Ok(Self {
            table,
            module: JITModule::new(builder),
            defined: Vec::new(),
        })
    }

    /// Lowers `file`, which passed [`crate::check_file`], into its code unit and compiles its
    /// functions.
    pub fn add_file(&mut self, file: &MirFile) -> Result<(), BackendError> {
        let unit = CodeUnit::lower(file, &mut self.module)?;
        let mut context = self.module.make_context();
        for function in unit.functions {
            context.func = function.function;
            self.module
                .define_function(function.id, &mut context)
                .map_err(|error| {
                    BackendError::new(format!("compiling {}: {:?}", function.symbol, error))
                })?;
            self.module.clear_context(&mut context);
            self.defined.push((function.symbol, function.id));
        }
        Ok(())
    }

    /// Finalizes the load, resolving every slot and runtime symbol, and writes the address of
    /// each defined function into its slot.
    pub fn link(mut self) -> Result<Program, BackendError> {
        self.module.finalize_definitions()?;
        for (symbol, id) in &self.defined {
            let address = self.module.get_finalized_function(*id);
            self.table.set(symbol, address);
        }
        Ok(Program::new(self.table, self.module))
    }
}

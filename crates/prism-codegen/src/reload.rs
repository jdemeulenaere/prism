//! A running program's code across reloads (D3, D5, D10 and D11 of `hot-reload`).
//!
//! A [`LiveProgram`] owns a program's function table, its value table and every JIT module whose
//! code the program may still run. A watch session prepares each reload on its own thread over
//! the same tables ([`LiveProgram::linker`], [`crate::Linker::prepare`]): a new module, finalized,
//! whose slot writes and table updates ([`ReloadWrites`]) are applied on the program thread
//! through the runtime's reload hand-off ([`prism_runtime::reload`]). Modules are never freed
//! while the program runs, since running frames, function values, drop entries, slots and
//! strings may refer to their code and data; they are freed with the program.

use crate::error::BackendError;
use crate::link::{FunctionTable, Linker, StoredValue, ValueTable};
use cranelift_jit::JITModule;
use prism_abi::ContentHash;
use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// What a symbol whose slot a reload split is the code of, for the reload's note.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotKind {
    /// A function, extension, default generator, initializer or accessor: its signature changed.
    Function,
    /// A closure: its captures changed.
    Closure,
}

/// The writes that make a load current, applied together on the program thread: each slot's
/// new code address, and the symbol-to-slot and symbol-to-storage mappings later loads resolve
/// through.
pub struct ReloadWrites {
    table: Arc<FunctionTable>,
    values: Arc<ValueTable>,
    slots: Vec<SlotWrite>,
    stored: Vec<(String, StoredValue)>,
}

struct SlotWrite {
    symbol: String,
    slot: usize,
    address: usize,
    contract: Option<ContentHash>,
}

// SAFETY: the slot addresses point into the table's chunks, which the writes keep alive through
// their `Arc`, and the code addresses are of finalized modules the session keeps mapped.
unsafe impl Send for ReloadWrites {}

impl ReloadWrites {
    pub(crate) fn new(table: Arc<FunctionTable>, values: Arc<ValueTable>) -> Self {
        Self {
            table,
            values,
            slots: Vec::new(),
            stored: Vec::new(),
        }
    }

    pub(crate) fn slot(
        &mut self,
        symbol: String,
        slot: *const AtomicUsize,
        address: *const u8,
        contract: Option<ContentHash>,
    ) {
        self.slots.push(SlotWrite {
            symbol,
            slot: slot as usize,
            address: address as usize,
            contract,
        });
    }

    pub(crate) fn value(&mut self, symbol: String, stored: StoredValue) {
        self.stored.push((symbol, stored));
    }

    /// The number of slots the writes fill.
    pub fn slot_count(&self) -> usize {
        self.slots.len()
    }

    /// Writes every slot with a release store, then maps every symbol to its slot and every
    /// value to its storage for later loads.
    pub fn apply(self) {
        for write in &self.slots {
            let slot = write.slot as *const AtomicUsize;
            // SAFETY: the slot lives as long as the table, which `self` keeps alive.
            unsafe { (*slot).store(write.address, Ordering::Release) };
        }
        for write in &self.slots {
            self.table.commit(
                &write.symbol,
                write.slot as *const AtomicUsize,
                write.contract,
            );
        }
        for (symbol, stored) in &self.stored {
            self.values.commit(symbol, *stored);
        }
    }
}

/// A reload compiled and finalized into its own module, not applied yet.
pub struct PreparedReload {
    writes: ReloadWrites,
    /// The symbols whose slot the reload splits and that get a note, in the order they were
    /// defined: restart functions split only with their composable,
    /// whose note covers them (D8 of `compose-hot-reload`).
    pub splits: Vec<(String, SlotKind)>,
    /// The slot each split symbol, restart functions included, was mapped to before the reload.
    retired: Vec<usize>,
    module: JITModule,
    /// The units of the reload, by name, with the symbols whose slots each refers to.
    units: Vec<(String, BTreeSet<String>)>,
}

impl PreparedReload {
    pub(crate) fn new(
        writes: ReloadWrites,
        splits: Vec<(String, SlotKind)>,
        retired: Vec<usize>,
        module: JITModule,
        units: Vec<(String, BTreeSet<String>)>,
    ) -> Self {
        Self {
            writes,
            splits,
            retired,
            module,
            units,
        }
    }

    /// The names of the units the reload defines, in order.
    pub fn unit_names(&self) -> Vec<String> {
        self.units.iter().map(|(name, _)| name.clone()).collect()
    }

    /// The addresses of the slots the reload's splits retire: the slot each split symbol was
    /// mapped to before it, in the order they were defined (D8 of `compose-hot-reload`).
    pub fn retired_slots(&self) -> Vec<usize> {
        self.retired.clone()
    }
}

/// The units of a load, by name, with the symbols whose slots each refers to.
type LoadUnits = Vec<(String, BTreeSet<String>)>;

/// The code and tables of a program that a watch session keeps running across reloads.
pub struct LiveProgram {
    table: Arc<FunctionTable>,
    values: Arc<ValueTable>,
    /// Every module whose code the program may run, the first load's first.
    modules: Vec<JITModule>,
    /// The module of the reload requested last, until the session knows whether the program
    /// applied it.
    pending: Option<(JITModule, LoadUnits)>,
    /// The symbols whose slots each unit of the running code refers to, by unit, for the
    /// split-safety check.
    references: HashMap<String, BTreeSet<String>>,
}

impl LiveProgram {
    pub(crate) fn new(
        table: Arc<FunctionTable>,
        values: Arc<ValueTable>,
        module: JITModule,
        units: Vec<(String, BTreeSet<String>)>,
    ) -> Self {
        Self {
            table,
            values,
            modules: vec![module],
            pending: None,
            references: units.into_iter().collect(),
        }
    }

    /// The program's function table.
    pub fn table(&self) -> &Arc<FunctionTable> {
        &self.table
    }

    /// The program's value table.
    pub fn values(&self) -> &Arc<ValueTable> {
        &self.values
    }

    /// A linker for the next reload, over the program's tables.
    pub fn linker(&self) -> Result<Linker, BackendError> {
        Linker::with_table(Arc::clone(&self.table), Arc::clone(&self.values))
    }

    /// The number of modules the program keeps mapped, a requested reload's included.
    pub fn module_count(&self) -> usize {
        self.modules.len() + usize::from(self.pending.is_some())
    }

    /// Hands `reload` over for the program thread: keeps its module mapped until
    /// [`LiveProgram::settle`] knows whether it was applied, and gives the writes to apply.
    ///
    /// In debug builds, checks that the split is safe (D5 of `hot-reload`): no unit of the
    /// running code outside the reload refers to a symbol whose slot the reload splits, since
    /// such a unit would keep calling the old slot from code compiled for the new contract's
    /// callers.
    pub fn hold(&mut self, reload: PreparedReload) -> ReloadWrites {
        let PreparedReload {
            writes,
            splits,
            module,
            units,
            ..
        } = reload;
        if cfg!(debug_assertions) {
            let reloaded: BTreeSet<&str> = units.iter().map(|(name, _)| name.as_str()).collect();
            for (symbol, _) in &splits {
                for (unit, references) in &self.references {
                    assert!(
                        reloaded.contains(unit.as_str()) || !references.contains(symbol),
                        "the unit {} refers to {}, whose slot the reload splits, and is not \
                         in the reload",
                        unit,
                        symbol
                    );
                }
            }
        }
        debug_assert!(self.pending.is_none(), "a held reload was not settled");
        self.pending = Some((module, units));
        writes
    }

    /// Settles the reload held last: keeps its module when the program applied it, and frees it
    /// when it was withdrawn before the program ran any of its code.
    pub fn settle(&mut self, applied: bool) {
        let Some((module, units)) = self.pending.take() else {
            return;
        };
        if applied {
            self.modules.push(module);
            self.references.extend(units);
        } else {
            // SAFETY: no slot was ever pointed at the module's code, so nothing runs it.
            unsafe { module.free_memory() };
        }
    }
}

impl Drop for LiveProgram {
    fn drop(&mut self) {
        // The program's thread has returned from `main` or is parked forever after a runtime
        // error, so none of its code runs again (D10 and D13 of `hot-reload`).
        self.settle(false);
        for module in self.modules.drain(..) {
            // SAFETY: as above, no code of the program runs any more.
            unsafe { module.free_memory() };
        }
    }
}

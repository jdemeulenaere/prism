//! The function table, the value table and the linker (D3 and D4 of `codegen-core`, D5 and D8
//! of `hot-reload`).
//!
//! The function table holds one pointer-sized slot per symbol of a function, default value
//! generator or top-level value accessor, allocated in chunks that never move, so a slot keeps
//! its address for as long as the table lives. A slot nothing defines holds the address of
//! `prism_rt_unlinked_function`. Code refers to a slot only by the symbol `<symbol>#slot`, which
//! the JIT resolves when a load is finalized to the slot the table maps the symbol to; after
//! finalizing, the linker writes the address of each function the load defines into its slot.
//!
//! Each slot records the contract of the code it holds (`crate::contract`). A later load of a
//! watch session that defines a symbol again with the same contract repoints its slot; one with a
//! different contract gets a new slot for the symbol, which the code of that load and of every
//! later one calls, while code finalized before keeps calling the old slot and its old code
//! (D5 of `hot-reload`).
//!
//! The value table holds the storage and initialization state of every top-level value, keyed by
//! the value's symbol and its value key (D8 of `hot-reload`): a unit imports `<value>#storage`
//! and `<value>#state`, and the linker resolves them to the storage the table holds for that key,
//! or to new zeroed storage when the key changed.
//!
//! One table serves a whole program, and a watch session keeps it across loads. It is owned by
//! the [`Linker`] and then the [`crate::Program`] rather than being a global, so a test process
//! can link several programs.

use crate::artifact::{
    CompiledFunction, DataContents, DataObject, Declaration, Relocation, RelocationTarget,
    UnitArtifact, ValueRecord,
};
use crate::error::BackendError;
use crate::instances::InstanceUnit;
use crate::isa::{is_large, isa, large_function_isa};
use crate::program::Program;
use crate::reload::{LiveProgram, PreparedReload, ReloadWrites, SlotKind};
use crate::symbols::{slot_owner, slot_symbol};
use crate::unit::CodeUnit;
use cranelift_codegen::binemit::Reloc;
use cranelift_codegen::control::ControlPlane;
use cranelift_codegen::ir;
use cranelift_codegen::isa::{OwnedTargetIsa, TargetIsa};
use cranelift_codegen::{Context, FinalizedMachReloc};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{
    default_libcall_names, DataDescription, DataId, FuncId, FuncOrDataId, Init, Linkage, Module,
    ModuleDeclarations, ModuleReloc, ModuleRelocTarget, ModuleResult,
};
use prism_abi::{ContentHash, DeterministicHasher};
use prism_mir::MirFile;
use prism_runtime::{prism_rt_unlinked_function, STACK_LIMIT, STACK_LIMIT_SYMBOL};
use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// The number of slots in one chunk of the table.
const CHUNK_SIZE: usize = 256;

/// A program's table of function slots.
pub struct FunctionTable {
    inner: Mutex<TableInner>,
}

struct TableInner {
    /// The chunks of slots; a chunk is never moved or freed while the table lives.
    chunks: Vec<Box<[AtomicUsize]>>,
    /// The slots used in the last chunk.
    used: usize,
    /// The slot each symbol is mapped to, with the contract of the code it holds once a load
    /// filled it.
    slots: HashMap<String, SlotEntry>,
}

#[derive(Debug, Clone, Copy)]
struct SlotEntry {
    slot: *const AtomicUsize,
    contract: Option<ContentHash>,
}

// SAFETY: the raw pointers point into the table's own chunks, which are only freed with the
// table, and every access to a slot is atomic.
unsafe impl Send for TableInner {}

/// What defining a symbol does to its slot ([`FunctionTable::slot_for`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotChange {
    /// The symbol had no code yet: an added declaration, or the first load.
    New,
    /// The code has the contract of the slot's code: the slot is repointed.
    Repointed,
    /// The code has another contract: the symbol gets a new slot, and the old slot keeps its
    /// code for the code finalized before.
    Split,
}

impl TableInner {
    fn allocate(&mut self) -> *const AtomicUsize {
        if self.used == CHUNK_SIZE {
            let unlinked = prism_rt_unlinked_function as *const u8 as usize;
            let chunk: Box<[AtomicUsize]> = (0..CHUNK_SIZE)
                .map(|_| AtomicUsize::new(unlinked))
                .collect();
            self.chunks.push(chunk);
            self.used = 0;
        }
        let index = self.used;
        self.used += 1;
        &self.chunks.last().expect("a chunk exists")[index]
    }
}

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

    /// The slot `symbol` is mapped to, allocated on first use and holding the unlinked stub
    /// until a function is linked into it.
    pub fn slot(&self, symbol: &str) -> *const AtomicUsize {
        let mut inner = self.lock();
        if let Some(entry) = inner.slots.get(symbol) {
            return entry.slot;
        }
        let slot = inner.allocate();
        inner.slots.insert(
            symbol.to_string(),
            SlotEntry {
                slot,
                contract: None,
            },
        );
        slot
    }

    /// The slot code of `contract` for `symbol` goes in, and how it changes the symbol's slot:
    /// its current slot when the symbol has no code yet or the contracts are equal, otherwise a
    /// fresh slot. The mapping is not changed: [`FunctionTable::commit`] does that once the load
    /// is applied, so that a load that is never applied leaves the table as it was.
    pub fn slot_for(
        &self,
        symbol: &str,
        contract: Option<ContentHash>,
    ) -> (*const AtomicUsize, SlotChange) {
        let mut inner = self.lock();
        match inner.slots.get(symbol).copied() {
            Some(SlotEntry {
                slot,
                contract: None,
            }) => (slot, SlotChange::New),
            Some(entry) if contract.is_none() || entry.contract == contract => {
                (entry.slot, SlotChange::Repointed)
            }
            Some(_) => (inner.allocate(), SlotChange::Split),
            None => {
                let slot = inner.allocate();
                inner.slots.insert(
                    symbol.to_string(),
                    SlotEntry {
                        slot,
                        contract: None,
                    },
                );
                (slot, SlotChange::New)
            }
        }
    }

    /// Maps `symbol` to `slot`, whose code has `contract`, for every later load.
    pub fn commit(&self, symbol: &str, slot: *const AtomicUsize, contract: Option<ContentHash>) {
        self.lock()
            .slots
            .insert(symbol.to_string(), SlotEntry { slot, contract });
    }

    /// The contract of the code `symbol`'s slot holds, once a load filled it.
    pub fn contract(&self, symbol: &str) -> Option<ContentHash> {
        self.lock().slots.get(symbol)?.contract
    }

    /// The address `symbol`'s slot holds, if it has a slot.
    pub fn get(&self, symbol: &str) -> Option<*const u8> {
        let slot = self.lock().slots.get(symbol)?.slot;
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

/// The initializer hash of each top-level value a load defines, by its symbol (D8 of
/// `hot-reload`): what a watch session supplies so that an edited initializer gives its value
/// new storage. A value without one is keyed by its symbol and type alone.
pub type ValueKeys = HashMap<String, ContentHash>;

/// The key of the storage of the top-level value `symbol` whose type has `type_fingerprint` and
/// whose declared type and initializer hash to `initializer` (D8 of `hot-reload`).
pub fn value_key(
    symbol: &str,
    type_fingerprint: &ContentHash,
    initializer: Option<&ContentHash>,
) -> ContentHash {
    let mut hasher = DeterministicHasher::new();
    hasher.write_str("value");
    hasher.write_str(symbol);
    hasher.write_hash(type_fingerprint);
    match initializer {
        Some(hash) => {
            hasher.write_u8(1);
            hasher.write_hash(hash);
        }
        None => hasher.write_u8(0),
    }
    hasher.finish()
}

/// The storage of one top-level value: its key, and the addresses of its storage and of its
/// state byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoredValue {
    pub key: ContentHash,
    pub storage: usize,
    pub state: usize,
}

/// A program's top-level value storage (D8 of `hot-reload`). Storage is zeroed when allocated,
/// never moves, and is freed only with the table, so code and borrows that still refer to
/// storage a reload replaced stay valid.
pub struct ValueTable {
    inner: Mutex<ValuesInner>,
}

#[derive(Default)]
struct ValuesInner {
    /// The storage each symbol currently resolves to.
    current: HashMap<String, StoredValue>,
    /// Every block allocated, with its layout.
    blocks: Vec<(usize, std::alloc::Layout)>,
}

impl ValueTable {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(ValuesInner::default()),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ValuesInner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The storage of `value` under `key`: the symbol's current storage when its key is `key`,
    /// otherwise new zeroed storage, which [`ValueTable::commit`] makes current.
    pub fn storage_for(&self, value: &ValueRecord, key: ContentHash) -> StoredValue {
        let mut inner = self.lock();
        if let Some(stored) = inner.current.get(&value.symbol) {
            if stored.key == key {
                return *stored;
            }
        }
        let align = value.align.max(1) as usize;
        let size = (value.size.max(1) as usize).next_multiple_of(align);
        let layout = std::alloc::Layout::from_size_align(size + 1, align)
            .expect("a value's storage has a valid layout");
        // SAFETY: the layout has a non-zero size.
        let block = unsafe { std::alloc::alloc_zeroed(layout) };
        if block.is_null() {
            std::alloc::handle_alloc_error(layout);
        }
        inner.blocks.push((block as usize, layout));
        StoredValue {
            key,
            storage: block as usize,
            state: block as usize + size,
        }
    }

    /// Makes `stored` the storage of `symbol` for every later load.
    pub fn commit(&self, symbol: &str, stored: StoredValue) {
        self.lock().current.insert(symbol.to_string(), stored);
    }

    /// The storage `symbol` currently resolves to.
    pub fn get(&self, symbol: &str) -> Option<StoredValue> {
        self.lock().current.get(symbol).copied()
    }
}

impl Drop for ValueTable {
    fn drop(&mut self) {
        let inner = self
            .inner
            .get_mut()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for (block, layout) in inner.blocks.drain(..) {
            // SAFETY: each block was allocated with its layout and is freed once, with the
            // program, when no code of it runs any more.
            unsafe { std::alloc::dealloc(block as *mut u8, layout) };
        }
    }
}

/// A function a load defines into a slot.
struct DefinedFunction {
    symbol: String,
    id: FuncId,
    slot: *const AtomicUsize,
    contract: Option<ContentHash>,
    change: SlotChange,
    /// For a split, the slot the symbol was mapped to before the load (D8 of
    /// `compose-hot-reload`).
    retired: Option<*const AtomicUsize>,
}

/// Builds one load of a program: the code units of its files, defined into one JIT module and
/// finalized together (D3). Every unit is defined from its relocatable artifact
/// ([`UnitArtifact`], D8 of `instance-cache`), whether it was just compiled or read from the
/// build cache. The first load of a program is linked ([`Linker::link`]); each later load of a
/// watch session is prepared over the program's tables ([`Linker::with_table`],
/// [`Linker::prepare`]) and applied by the program thread.
pub struct Linker {
    table: Arc<FunctionTable>,
    values: Arc<ValueTable>,
    module: JITModule,
    /// The ISA of large functions ([`crate::isa::is_large`]).
    large: OwnedTargetIsa,
    /// The address this load resolves a symbol to ahead of the table: the new slot of each
    /// split symbol, and the storage and state of each value the load defines.
    resolved: Arc<Mutex<HashMap<String, usize>>>,
    /// The functions the load defines into slots.
    defined: Vec<DefinedFunction>,
    /// The storage each value of the load resolves to.
    stored: Vec<(String, StoredValue)>,
    /// The initializer hash of each value, which keys its storage.
    value_keys: ValueKeys,
    /// The units of the load, by name, with the symbols whose slots each refers to.
    units: Vec<(String, BTreeSet<String>)>,
    /// The number of functions compiled by Cranelift in this load, cached ones excluded.
    compiled_functions: usize,
}

impl Linker {
    /// A linker with a new function table and value table, whose JIT resolves the runtime
    /// functions by name and every `<symbol>#slot` to the table's slot of `symbol`.
    pub fn new() -> Result<Self, BackendError> {
        Self::with_table(FunctionTable::new(), ValueTable::new())
    }

    /// A linker of a later load of the program whose tables are `table` and `values`: its new
    /// module resolves slots and storage through them, as [`Linker::new`]'s does.
    pub fn with_table(
        table: Arc<FunctionTable>,
        values: Arc<ValueTable>,
    ) -> Result<Self, BackendError> {
        let mut builder = JITBuilder::with_isa(isa()?, default_libcall_names());
        builder.symbols(
            crate::externals::entry_points()
                .map(|function| (function.name, function.address.as_ptr()))
                .chain(std::iter::once((
                    STACK_LIMIT_SYMBOL,
                    std::ptr::addr_of!(STACK_LIMIT).cast::<u8>(),
                ))),
        );
        let resolved: Arc<Mutex<HashMap<String, usize>>> = Arc::default();
        let lookup_resolved = Arc::clone(&resolved);
        let lookup_table = Arc::clone(&table);
        builder.symbol_lookup_fn(Box::new(move |name| {
            let resolved = lookup_resolved
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .get(name)
                .copied();
            if let Some(address) = resolved {
                return Some(address as *const u8);
            }
            slot_owner(name).map(|symbol| lookup_table.slot(symbol).cast::<u8>())
        }));
        Ok(Self {
            table,
            values,
            module: JITModule::new(builder),
            large: large_function_isa()?,
            resolved,
            defined: Vec::new(),
            stored: Vec::new(),
            value_keys: ValueKeys::new(),
            units: Vec::new(),
            compiled_functions: 0,
        })
    }

    /// Keys the storage of the values defined from now on by their initializers' hashes.
    pub fn set_value_keys(&mut self, keys: ValueKeys) {
        self.value_keys = keys;
    }

    /// The number of functions Cranelift compiled for this load: zero when every unit came
    /// from the build cache.
    pub fn compiled_functions(&self) -> usize {
        self.compiled_functions
    }

    /// Lowers `file`, which passed [`crate::check_file`], into its code unit, compiles its
    /// functions and defines them.
    pub fn add_file(&mut self, file: &MirFile) -> Result<(), BackendError> {
        let artifact = self.compile_unit(file, &file.module)?;
        self.define_checked(&file.module, &artifact)
    }

    /// Lowers the instance unit `instance`, which passed [`crate::check_instances`], under its
    /// symbol, compiles its function, which gets its own slot, and its glue (D3 of
    /// `monomorphization`), and defines them.
    pub fn add_instance(&mut self, instance: &InstanceUnit) -> Result<(), BackendError> {
        let artifact = self.compile_instance(instance)?;
        self.define_checked(&instance.symbol, &artifact)
    }

    /// The artifact of the instance unit `instance`, compiled but not defined.
    pub fn compile_instance(
        &mut self,
        instance: &InstanceUnit,
    ) -> Result<UnitArtifact, BackendError> {
        self.compile_unit(&instance.file, &instance.symbol)
    }

    /// Defines `artifact`, round-tripping it through its encoding first in debug builds, so
    /// that every test exercises the codec (D8 of `instance-cache`).
    fn define_checked(&mut self, unit: &str, artifact: &UnitArtifact) -> Result<(), BackendError> {
        #[cfg(debug_assertions)]
        if let Some(bytes) = artifact.encode() {
            let decoded = UnitArtifact::decode(&bytes).expect("an encoded artifact decodes");
            assert_eq!(
                &decoded, artifact,
                "an artifact round-trips through its encoding"
            );
            return self.define_artifact(unit, &decoded);
        }
        self.define_artifact(unit, artifact)
    }

    /// Lowers `file` into its code unit under the unit name `unit_name` and compiles each of
    /// its functions with Cranelift, with the large-function ISA when [`is_large`] holds, into
    /// a relocatable artifact. Nothing is defined in the module: its symbols are declared, and
    /// the relocations name their targets by those declarations' names.
    pub fn compile_unit(
        &mut self,
        file: &MirFile,
        unit_name: &str,
    ) -> Result<UnitArtifact, BackendError> {
        let mut recording = RecordingModule {
            inner: &mut self.module,
            data: Vec::new(),
        };
        let unit = CodeUnit::lower_named(file, unit_name, &mut recording)?;
        let data = std::mem::take(&mut recording.data);
        let mut artifact = ArtifactBuilder::new(&self.module);
        let mut context = self.module.make_context();
        let glue = unit.glue.into_iter().map(|function| (function, false));
        for (function, has_slot) in unit
            .functions
            .into_iter()
            .map(|function| (function, true))
            .chain(glue)
        {
            context.func = function.function;
            let compiled = compile_function(self.module.isa(), &*self.large, &mut context)
                .map_err(|error| {
                    BackendError::new(format!("compiling {}: {:?}", function.symbol, error))
                })?;
            self.compiled_functions += 1;
            artifact.function(
                function.id,
                has_slot,
                function.contract,
                compiled,
                &context.func,
            )?;
            self.module.clear_context(&mut context);
        }
        for (id, description) in &data {
            artifact.data(*id, description)?;
        }
        Ok(artifact.finish(unit.values))
    }

    /// Defines every function and data object of `artifact`, the unit `unit`, in the load:
    /// declares each of its symbols with its linkage and signature, maps its relocations back to
    /// the module's ids, and defines the code and data from their bytes. Each function with a
    /// slot is given the slot its contract selects ([`FunctionTable::slot_for`]), and each value
    /// the storage its key selects; they are filled and made current by [`Linker::link`], or by
    /// applying [`Linker::prepare`]'s reload.
    pub fn define_artifact(
        &mut self,
        unit: &str,
        artifact: &UnitArtifact,
    ) -> Result<(), BackendError> {
        let mut ids: HashMap<&str, FuncOrDataId> = HashMap::new();
        for declaration in &artifact.declarations {
            let id = match declaration {
                Declaration::Function {
                    name,
                    linkage,
                    signature,
                } => FuncOrDataId::Func(self.module.declare_function(name, *linkage, signature)?),
                Declaration::Data {
                    name,
                    linkage,
                    writable,
                    tls,
                } => FuncOrDataId::Data(self.module.declare_data(name, *linkage, *writable, *tls)?),
            };
            ids.insert(declaration.name(), id);
        }
        let function_id = |name: &str| match ids.get(name) {
            Some(FuncOrDataId::Func(id)) => Ok(*id),
            _ => Err(BackendError::new(format!(
                "{} is not a declared function",
                name
            ))),
        };
        let data_id = |name: &str| match ids.get(name) {
            Some(FuncOrDataId::Data(id)) => Ok(*id),
            _ => Err(BackendError::new(format!(
                "{} is not a declared data object",
                name
            ))),
        };
        let mut references: BTreeSet<String> = BTreeSet::new();
        for function in &artifact.functions {
            let id = function_id(&function.name)?;
            let relocations = function
                .relocations
                .iter()
                .map(|relocation| {
                    let name = match &relocation.target {
                        RelocationTarget::Function(name) => {
                            ModuleRelocTarget::user(0, function_id(name)?.as_u32())
                        }
                        RelocationTarget::Data(name) => {
                            if let Some(symbol) = slot_owner(name) {
                                references.insert(symbol.to_string());
                            }
                            ModuleRelocTarget::user(1, data_id(name)?.as_u32())
                        }
                        RelocationTarget::LibCall(name) => {
                            ModuleRelocTarget::LibCall(name.parse().map_err(|()| {
                                BackendError::new(format!("unknown library call {}", name))
                            })?)
                        }
                        RelocationTarget::FunctionOffset(name, offset) => {
                            ModuleRelocTarget::FunctionOffset(function_id(name)?, *offset)
                        }
                    };
                    Ok(ModuleReloc {
                        offset: relocation.offset,
                        kind: relocation.kind,
                        name,
                        addend: relocation.addend,
                    })
                })
                .collect::<Result<Vec<ModuleReloc>, BackendError>>()?;
            self.module
                .define_function_bytes(id, function.alignment, &function.code, &relocations)
                .map_err(|error| {
                    BackendError::new(format!("defining {}: {:?}", function.name, error))
                })?;
            // Glue is called directly within its unit and has no slot.
            if function.has_slot {
                // The mapping changes only when the load is applied, so the symbol's slot is
                // still the one a split retires.
                let previous = self.table.slot(&function.name);
                let (slot, change) = self.table.slot_for(&function.name, function.contract);
                if change == SlotChange::Split {
                    self.resolve(slot_symbol(&function.name), slot as usize);
                }
                self.defined.push(DefinedFunction {
                    symbol: function.name.clone(),
                    id,
                    slot,
                    contract: function.contract,
                    change,
                    retired: (change == SlotChange::Split).then_some(previous),
                });
            }
        }
        for object in &artifact.data {
            let id = data_id(&object.name)?;
            let mut description = DataDescription::new();
            match &object.contents {
                DataContents::Bytes(bytes) => description.define(bytes.clone().into_boxed_slice()),
                DataContents::Zeros(size) => description.define_zeroinit(*size),
            }
            if let Some(alignment) = object.alignment {
                description.set_align(alignment);
            }
            for relocation in &object.relocations {
                match &relocation.target {
                    RelocationTarget::Function(name) => {
                        let target = description.import_function(ModuleRelocTarget::user(
                            0,
                            function_id(name)?.as_u32(),
                        ));
                        description.write_function_addr(relocation.offset, target);
                    }
                    RelocationTarget::Data(name) => {
                        if let Some(symbol) = slot_owner(name) {
                            references.insert(symbol.to_string());
                        }
                        let target = description.import_global_value(ModuleRelocTarget::user(
                            1,
                            data_id(name)?.as_u32(),
                        ));
                        description.write_data_addr(relocation.offset, target, relocation.addend);
                    }
                    RelocationTarget::LibCall(_) | RelocationTarget::FunctionOffset(..) => {
                        return Err(BackendError::new(format!(
                            "a relocation of data object {} names no function or data object",
                            object.name
                        )))
                    }
                }
            }
            self.module.define_data(id, &description)?;
        }
        for value in &artifact.values {
            let key = value_key(
                &value.symbol,
                &value.type_fingerprint,
                self.value_keys.get(&value.symbol),
            );
            let stored = self.values.storage_for(value, key);
            self.resolve(format!("{}#storage", value.symbol), stored.storage);
            self.resolve(format!("{}#state", value.symbol), stored.state);
            self.stored.push((value.symbol.clone(), stored));
        }
        self.units.push((unit.to_string(), references));
        Ok(())
    }

    /// Resolves `name` to `address` in this load, ahead of the table.
    fn resolve(&self, name: String, address: usize) {
        self.resolved
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(name, address);
    }

    /// Finalizes the load, resolving every slot, value and runtime symbol, and gives the writes
    /// that make it current.
    fn finalize(&mut self) -> Result<ReloadWrites, BackendError> {
        self.module.finalize_definitions()?;
        let mut writes = ReloadWrites::new(Arc::clone(&self.table), Arc::clone(&self.values));
        for function in &self.defined {
            let address = self.module.get_finalized_function(function.id);
            writes.slot(
                function.symbol.clone(),
                function.slot,
                address,
                function.contract,
            );
        }
        for (symbol, stored) in &self.stored {
            writes.value(symbol.clone(), *stored);
        }
        Ok(writes)
    }

    /// Finalizes the first load of a program and makes it current: writes the address of each
    /// defined function into its slot and maps every symbol and value of the load.
    pub fn link(mut self) -> Result<Program, BackendError> {
        let writes = self.finalize()?;
        writes.apply();
        let units = std::mem::take(&mut self.units);
        let live = LiveProgram::new(self.table, self.values, self.module, units);
        Ok(Program::new(live))
    }

    /// Finalizes a later load over the program's tables without writing any slot: the reload
    /// it gives is applied on the program thread ([`PreparedReload`]).
    pub fn prepare(mut self) -> Result<PreparedReload, BackendError> {
        let writes = self.finalize()?;
        // A restart function splits only with its composable, whose
        // own note says so (D8 of `compose-hot-reload`).
        let splits = self
            .defined
            .iter()
            .filter(|function| function.change == SlotChange::Split)
            .filter(|function| {
                !function.symbol.ends_with("#restart")
                    && !function.symbol.ends_with("#root-content")
            })
            .map(|function| {
                let kind = if function.symbol.contains("#closure") {
                    SlotKind::Closure
                } else {
                    SlotKind::Function
                };
                (function.symbol.clone(), kind)
            })
            .collect();
        let retired = self
            .defined
            .iter()
            .filter_map(|function| function.retired)
            .map(|slot| slot as usize)
            .collect();
        Ok(PreparedReload::new(
            writes,
            splits,
            retired,
            self.module,
            self.units,
        ))
    }
}

/// A module that forwards every declaration to `inner` and records the data objects a unit
/// defines instead of defining them, so that [`Linker::compile_unit`] can put them in the unit's
/// artifact.
struct RecordingModule<'m> {
    inner: &'m mut JITModule,
    data: Vec<(DataId, DataDescription)>,
}

impl Module for RecordingModule<'_> {
    fn isa(&self) -> &dyn TargetIsa {
        self.inner.isa()
    }

    fn declarations(&self) -> &ModuleDeclarations {
        self.inner.declarations()
    }

    fn declare_function(
        &mut self,
        name: &str,
        linkage: Linkage,
        signature: &ir::Signature,
    ) -> ModuleResult<FuncId> {
        self.inner.declare_function(name, linkage, signature)
    }

    fn declare_anonymous_function(&mut self, signature: &ir::Signature) -> ModuleResult<FuncId> {
        self.inner.declare_anonymous_function(signature)
    }

    fn declare_data(
        &mut self,
        name: &str,
        linkage: Linkage,
        writable: bool,
        tls: bool,
    ) -> ModuleResult<DataId> {
        self.inner.declare_data(name, linkage, writable, tls)
    }

    fn declare_anonymous_data(&mut self, writable: bool, tls: bool) -> ModuleResult<DataId> {
        self.inner.declare_anonymous_data(writable, tls)
    }

    fn define_function_with_control_plane(
        &mut self,
        _: FuncId,
        _: &mut Context,
        _: &mut ControlPlane,
    ) -> ModuleResult<()> {
        Err(compiled_into_artifact())
    }

    fn define_function_bytes(
        &mut self,
        _: FuncId,
        _: u64,
        _: &[u8],
        _: &[ModuleReloc],
    ) -> ModuleResult<()> {
        Err(compiled_into_artifact())
    }

    fn define_data(&mut self, id: DataId, data: &DataDescription) -> ModuleResult<()> {
        self.data.push((id, data.clone()));
        Ok(())
    }
}

/// The error of defining a function while a unit is lowered: [`CodeUnit::lower_named`] never
/// does, since a unit's functions are compiled into its artifact.
fn compiled_into_artifact() -> cranelift_module::ModuleError {
    cranelift_module::ModuleError::InvalidImportDefinition(
        "a unit's functions are compiled into its artifact".to_string(),
    )
}

/// Builds a [`UnitArtifact`] from compiled functions and recorded data, naming every id the
/// unit refers to by its declaration in `module`.
struct ArtifactBuilder<'m> {
    module: &'m JITModule,
    declarations: Vec<Declaration>,
    declared: HashMap<FuncOrDataId, ()>,
    functions: Vec<CompiledFunction>,
    data: Vec<DataObject>,
}

impl<'m> ArtifactBuilder<'m> {
    fn new(module: &'m JITModule) -> Self {
        Self {
            module,
            declarations: Vec::new(),
            declared: HashMap::new(),
            functions: Vec::new(),
            data: Vec::new(),
        }
    }

    /// The name of the function `id`, declared in the artifact.
    fn function_name(&mut self, id: FuncId) -> Result<String, BackendError> {
        let declaration = self.module.declarations().get_function_decl(id);
        let name = declaration
            .name
            .clone()
            .ok_or_else(|| BackendError::new("an anonymous function"))?;
        if self.declared.insert(FuncOrDataId::Func(id), ()).is_none() {
            self.declarations.push(Declaration::Function {
                name: name.clone(),
                linkage: declaration.linkage,
                signature: declaration.signature.clone(),
            });
        }
        Ok(name)
    }

    /// The name of the data object `id`, declared in the artifact.
    fn data_name(&mut self, id: DataId) -> Result<String, BackendError> {
        let declaration = self.module.declarations().get_data_decl(id);
        let name = declaration
            .name
            .clone()
            .ok_or_else(|| BackendError::new("an anonymous data object"))?;
        if self.declared.insert(FuncOrDataId::Data(id), ()).is_none() {
            self.declarations.push(Declaration::Data {
                name: name.clone(),
                linkage: declaration.linkage,
                writable: declaration.writable,
                tls: declaration.tls,
            });
        }
        Ok(name)
    }

    /// The symbolic target of the module's relocation target `target`.
    fn target(&mut self, target: &ModuleRelocTarget) -> Result<RelocationTarget, BackendError> {
        Ok(match target {
            ModuleRelocTarget::User {
                namespace: 0,
                index,
            } => RelocationTarget::Function(self.function_name(FuncId::from_u32(*index))?),
            ModuleRelocTarget::User {
                namespace: 1,
                index,
            } => RelocationTarget::Data(self.data_name(DataId::from_u32(*index))?),
            ModuleRelocTarget::LibCall(libcall) => RelocationTarget::LibCall(libcall.to_string()),
            ModuleRelocTarget::FunctionOffset(id, offset) => {
                RelocationTarget::FunctionOffset(self.function_name(*id)?, *offset)
            }
            ModuleRelocTarget::User { .. } | ModuleRelocTarget::KnownSymbol(_) => {
                return Err(BackendError::new(format!(
                    "a relocation to {} has no name",
                    target
                )))
            }
        })
    }

    /// Records the compiled function `id`, whose IR is `function`.
    fn function(
        &mut self,
        id: FuncId,
        has_slot: bool,
        contract: Option<ContentHash>,
        compiled: CompiledCode,
        function: &ir::Function,
    ) -> Result<(), BackendError> {
        let name = self.function_name(id)?;
        let relocations = compiled
            .relocations
            .iter()
            .map(|relocation| {
                let module_relocation = ModuleReloc::from_mach_reloc(relocation, function, id);
                Ok(Relocation {
                    offset: module_relocation.offset,
                    kind: module_relocation.kind,
                    addend: module_relocation.addend,
                    target: self.target(&module_relocation.name)?,
                })
            })
            .collect::<Result<Vec<Relocation>, BackendError>>()?;
        self.functions.push(CompiledFunction {
            name,
            has_slot,
            contract,
            alignment: compiled.alignment,
            code: compiled.code,
            relocations,
        });
        Ok(())
    }

    /// Records the data object `id` the unit defined as `description`.
    fn data(&mut self, id: DataId, description: &DataDescription) -> Result<(), BackendError> {
        let name = self.data_name(id)?;
        let contents = match &description.init {
            Init::Bytes { contents } => DataContents::Bytes(contents.to_vec()),
            Init::Zeros { size } => DataContents::Zeros(*size),
            Init::Uninitialized => {
                return Err(BackendError::new(format!("{} is not initialized", name)))
            }
        };
        let pointer = self.module.isa().pointer_bytes();
        let pointer_reloc = if pointer == 8 {
            Reloc::Abs8
        } else {
            Reloc::Abs4
        };
        let relocations = description
            .all_relocs(pointer_reloc)
            .map(|relocation| {
                Ok(Relocation {
                    offset: relocation.offset,
                    kind: relocation.kind,
                    addend: relocation.addend,
                    target: self.target(&relocation.name)?,
                })
            })
            .collect::<Result<Vec<Relocation>, BackendError>>()?;
        self.data.push(DataObject {
            name,
            alignment: description.align,
            contents,
            relocations,
        });
        Ok(())
    }

    fn finish(self, values: Vec<ValueRecord>) -> UnitArtifact {
        UnitArtifact {
            declarations: self.declarations,
            functions: self.functions,
            data: self.data,
            values,
        }
    }
}

/// One function's machine code, compiled but not defined.
pub struct CompiledCode {
    pub alignment: u64,
    pub code: Vec<u8>,
    pub relocations: Vec<FinalizedMachReloc>,
}

/// Compiles the function in `context` with `normal`, or with `large`, the single-pass
/// allocator's ISA, when it is large ([`crate::isa::is_large`]), so that compiling one function
/// stays linear in its size however large it is.
#[allow(clippy::result_large_err)]
pub fn compile_function(
    normal: &dyn TargetIsa,
    large: &dyn TargetIsa,
    context: &mut Context,
) -> Result<CompiledCode, cranelift_module::ModuleError> {
    let isa = if is_large(&context.func) {
        large
    } else {
        normal
    };
    context
        .compile(isa, &mut ControlPlane::default())
        .map_err(|error| cranelift_module::ModuleError::Compilation(error.inner))?;
    let compiled = context
        .compiled_code()
        .expect("a function just compiled has code");
    Ok(CompiledCode {
        alignment: u64::from(compiled.buffer.alignment),
        code: compiled.code_buffer().to_vec(),
        relocations: compiled.buffer.relocs().to_vec(),
    })
}

/// Compiles the function in `context` ([`compile_function`]) and defines it as `id` in
/// `module` from its bytes. The error is the module's own, as `Module::define_function`'s.
#[allow(clippy::result_large_err)]
pub fn define_function<M: Module>(
    module: &mut M,
    large: &dyn TargetIsa,
    id: FuncId,
    context: &mut Context,
) -> ModuleResult<()> {
    let compiled = compile_function(module.isa(), large, context)?;
    let relocations: Vec<ModuleReloc> = compiled
        .relocations
        .iter()
        .map(|relocation| ModuleReloc::from_mach_reloc(relocation, &context.func, id))
        .collect();
    module.define_function_bytes(id, compiled.alignment, &compiled.code, &relocations)
}

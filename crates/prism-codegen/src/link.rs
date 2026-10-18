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

use crate::artifact::{
    CompiledFunction, DataContents, DataObject, Declaration, Relocation, RelocationTarget,
    UnitArtifact,
};
use crate::error::BackendError;
use crate::instances::InstanceUnit;
use crate::isa::{is_large, isa, large_function_isa};
use crate::program::Program;
use crate::symbols::slot_owner;
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
use prism_mir::MirFile;
use prism_runtime::{
    prism_rt_unlinked_function, RUNTIME_FUNCTIONS, STACK_LIMIT, STACK_LIMIT_SYMBOL,
};
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
/// finalized together (D3). Every unit is defined from its relocatable artifact
/// ([`UnitArtifact`], D8 of `instance-cache`), whether it was just compiled or read from the
/// build cache.
pub struct Linker {
    table: Arc<FunctionTable>,
    module: JITModule,
    /// The ISA of large functions ([`crate::isa::is_large`]).
    large: OwnedTargetIsa,
    /// The functions the load defines, by symbol.
    defined: Vec<(String, FuncId)>,
    /// The number of functions compiled by Cranelift in this load, cached ones excluded.
    compiled_functions: usize,
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
                .map(|function| (function.name, function.address.as_ptr()))
                .chain(std::iter::once((
                    STACK_LIMIT_SYMBOL,
                    std::ptr::addr_of!(STACK_LIMIT).cast::<u8>(),
                ))),
        );
        let lookup_table = Arc::clone(&table);
        builder.symbol_lookup_fn(Box::new(move |name| {
            slot_owner(name).map(|symbol| lookup_table.slot(symbol).cast::<u8>())
        }));
        Ok(Self {
            table,
            module: JITModule::new(builder),
            large: large_function_isa()?,
            defined: Vec::new(),
            compiled_functions: 0,
        })
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
        self.define_checked(&artifact)
    }

    /// Lowers the instance unit `instance`, which passed [`crate::check_instances`], under its
    /// symbol, compiles its function, which gets its own slot, and its glue (D3 of
    /// `monomorphization`), and defines them.
    pub fn add_instance(&mut self, instance: &InstanceUnit) -> Result<(), BackendError> {
        let artifact = self.compile_instance(instance)?;
        self.define_checked(&artifact)
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
    fn define_checked(&mut self, artifact: &UnitArtifact) -> Result<(), BackendError> {
        #[cfg(debug_assertions)]
        if let Some(bytes) = artifact.encode() {
            let decoded = UnitArtifact::decode(&bytes).expect("an encoded artifact decodes");
            assert_eq!(
                &decoded, artifact,
                "an artifact round-trips through its encoding"
            );
            return self.define_artifact(&decoded);
        }
        self.define_artifact(artifact)
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
            artifact.function(function.id, has_slot, compiled, &context.func)?;
            self.module.clear_context(&mut context);
        }
        for (id, description) in &data {
            artifact.data(*id, description)?;
        }
        Ok(artifact.finish())
    }

    /// Defines every function and data object of `artifact` in the load: declares each of its
    /// symbols with its linkage and signature, maps its relocations back to the module's ids,
    /// and defines the code and data from their bytes. A function with a slot is linked into it
    /// by [`Linker::link`].
    pub fn define_artifact(&mut self, artifact: &UnitArtifact) -> Result<(), BackendError> {
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
                self.defined.push((function.name.clone(), id));
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

    fn finish(self) -> UnitArtifact {
        UnitArtifact {
            declarations: self.declarations,
            functions: self.functions,
            data: self.data,
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

//! One file's code unit (D3 of `codegen-core`): the functions lowered from the file's elaborated
//! MIR alone, its data objects, and its imports of runtime functions and of the slots of the
//! function table.
//!
//! A unit refers to the functions and top-level values of other files, and to its own, only
//! through the slots `<symbol>#slot`, imported as data symbols that the linker resolves. Its own
//! data (the file name and the details of runtime errors, the storage and state of its top-level
//! values) is named after its module, so the units of a program never collide. Each closure of
//! the unit has a descriptor, `<closure symbol>#descriptor`, whose first word is relocated to
//! the closure's slot (D3 of `closures`).
//!
//! A top-level value's storage and state belong to the program, not to the unit (D8 of
//! `hot-reload`): the unit imports `<value>#storage` and `<value>#state` as data symbols, which
//! the linker resolves to the program's value table, and lists each value it accesses with its
//! size, alignment and type fingerprint ([`ValueRecord`]). Each function with a slot records the
//! contract of its code ([`crate::contract`]), which decides whether a reload repoints its slot.

use crate::artifact::ValueRecord;
use crate::contract::{accessor_contract, slot_contract, TypeFingerprints};
use crate::error::BackendError;
use crate::glue::{GlueKind, GlueTable};
use crate::layout::{EnvironmentKind, LayoutTable};
use crate::lower::BodyLowering;
use crate::representation::{
    body_signature, closure_signature, representation, signature, Representation,
};
use crate::supported::is_compiled;
use crate::symbols::{
    accessor_symbol, descriptor_symbol, function_symbol, initializer_symbol, slot_symbol,
    state_symbol, storage_symbol, symbols_of,
};
use cranelift_codegen::ir::{
    self, condcodes::IntCC, types, AbiParam, Function, InstBuilder, MemFlagsData, UserFuncName,
};
use cranelift_codegen::isa::CallConv;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::{DataDescription, DataId, FuncId, Linkage, Module};
use prism_abi::ContentHash;
use prism_mir::{BodyId, BodyOwner, MirFile};
use prism_runtime::{RuntimeType, STACK_LIMIT_SYMBOL};
use prism_typed_tree::{ArenaId, DeclarationPath, Type};
use std::collections::HashMap;

/// A function of a unit, lowered to Cranelift IR and declared in the module, not yet compiled.
pub struct LoweredFunction {
    pub symbol: String,
    pub id: FuncId,
    pub function: Function,
    /// The contract of its code, for a function with a slot; `None` for glue (D5 of
    /// `hot-reload`).
    pub contract: Option<ContentHash>,
}

/// The code unit of one file, or of one instance of a generic (D3 of `monomorphization`).
pub struct CodeUnit {
    /// The file's module path, or the module of the instance's generic.
    pub module: String,
    /// The unit's name, which prefixes its data and glue: the module for a file's unit, the
    /// instance's symbol for an instance's.
    pub name: String,
    /// Its functions, in body order, each top-level value's accessor after its initializer.
    /// Each has a slot in the function table.
    pub functions: Vec<LoweredFunction>,
    /// The drop and clone glue its functions call, in order of their symbols: local to the unit,
    /// with no slot (D9 of `runtime-strings-and-drops`).
    pub glue: Vec<LoweredFunction>,
    /// The top-level values whose storage it imports, in body order (D8 of `hot-reload`).
    pub values: Vec<ValueRecord>,
}

/// The values of a top-level `val`'s initialization state byte (D18).
pub(crate) mod state {
    /// The state of zero-initialized storage.
    pub const UNINITIALIZED: i64 = 0;
    pub const INITIALIZING: i64 = 1;
    pub const INITIALIZED: i64 = 2;
}

// The state byte is zero-initialized data, which must read as uninitialized.
const _: () = assert!(state::UNINITIALIZED == 0);

impl CodeUnit {
    /// Lowers every compiled body of `file`, which passed [`crate::check_file`], into functions
    /// declared in `module`, and defines the unit's data objects there.
    pub fn lower<M: Module>(file: &MirFile, module: &mut M) -> Result<CodeUnit, BackendError> {
        Self::lower_named(file, &file.module, module)
    }

    /// [`CodeUnit::lower`] under the unit name `name`, which prefixes the unit's data and glue,
    /// so that the units of one module (its file's and its generics' instances) never collide.
    /// Runtime errors still report the file of `file`'s module.
    pub fn lower_named<M: Module>(
        file: &MirFile,
        name: &str,
        module: &mut M,
    ) -> Result<CodeUnit, BackendError> {
        // The backend compiles composed MIR alone (D20 of `compose-transform`).
        if file.phase != prism_mir::MirPhase::Composed {
            return Err(BackendError::new(format!(
                "{} reached the backend without being composed",
                file.module
            )));
        }
        let mut context = UnitContext::new(file, name, module)?;
        let mut builder_context = FunctionBuilderContext::new();
        let mut functions = Vec::new();
        for (body_id, body) in file.bodies.iter() {
            if !is_compiled(body) {
                continue;
            }
            let symbol = context.symbol(body_id).to_string();
            // Restart functions and root content adapters are closures too (D19 of
            // `compose-transform`).
            let signature = if body.owner.is_closure() {
                closure_signature(body, context.call_conv, context.pointer)
            } else {
                body_signature(body, context.call_conv, context.pointer)
            };
            let id = context.declare_function(&symbol, &signature)?;
            let environment = if body.owner.is_closure() {
                context
                    .layouts
                    .environment_layout(body)
                    .ok()
                    .map(|layout| layout.kind)
            } else {
                None
            };
            // A restart function's contract covers its composable's (D8 of
            // `compose-hot-reload`, adopted by `compose-transform`).
            let restarted = match body.owner {
                BodyOwner::Restart { function } => Some(&file.bodies[function]),
                _ => None,
            };
            let contract = slot_contract(body, environment, restarted, &mut context.fingerprints);
            let mut function =
                Function::with_name_signature(UserFuncName::user(0, id.as_u32()), signature);
            BodyLowering::lower(&mut context, body, &mut function, &mut builder_context)?;
            functions.push(LoweredFunction {
                symbol,
                id,
                function,
                contract: Some(contract),
            });
            if let BodyOwner::Value(path) = &body.owner {
                let accessor = context.accessor(path, body.return_type(), &mut builder_context)?;
                functions.push(accessor);
            }
        }
        let glue = context.lower_glue(&mut builder_context)?;
        Ok(CodeUnit {
            module: file.module.clone(),
            name: name.to_string(),
            functions,
            glue,
            values: std::mem::take(&mut context.values),
        })
    }
}

/// What lowering the bodies of one file shares: the module it declares into, and the unit's
/// data objects and imports, each declared once.
pub(crate) struct UnitContext<'m, M: Module> {
    pub module: &'m mut M,
    pub file: &'m MirFile,
    /// The unit's name (see [`CodeUnit::name`]).
    pub name: String,
    pub pointer: ir::Type,
    pub call_conv: CallConv,
    /// The layouts of the file's types, shared by its bodies.
    pub layouts: LayoutTable<'m>,
    /// The unit's drop and clone glue.
    pub glue: GlueTable,
    /// The file's path relative to the workspace root, which runtime errors report.
    file_name: (DataId, usize),
    texts: HashMap<String, DataId>,
    runtime: HashMap<&'static str, FuncId>,
    slots: HashMap<String, DataId>,
    /// The descriptor of each closure body the unit builds a value of.
    descriptors: HashMap<BodyId, DataId>,
    /// The slot type record of each type the unit keeps or remembers (D20 of
    /// `compose-transform`).
    slot_types: HashMap<Type, DataId>,
    /// The symbol of each body of the file, by id, built once ([`symbols_of`]).
    symbols: Vec<String>,
    stack_limit: Option<DataId>,
    /// The fingerprints of the file's types, for contracts and value records.
    pub fingerprints: TypeFingerprints<'m>,
    /// The top-level values the unit imports the storage of.
    values: Vec<ValueRecord>,
}

impl<'m, M: Module> UnitContext<'m, M> {
    fn new(file: &'m MirFile, unit_name: &str, module: &'m mut M) -> Result<Self, BackendError> {
        let pointer = module.target_config().pointer_type();
        let call_conv = module.target_config().default_call_conv;
        let name = format!("{}.prism", file.module);
        let mut context = Self {
            module,
            file,
            name: unit_name.to_string(),
            pointer,
            call_conv,
            layouts: LayoutTable::new(&file.types, pointer),
            glue: GlueTable::default(),
            file_name: (DataId::from_u32(0), 0),
            texts: HashMap::new(),
            runtime: HashMap::new(),
            slots: HashMap::new(),
            descriptors: HashMap::new(),
            slot_types: HashMap::new(),
            symbols: symbols_of(file),
            stack_limit: None,
            fingerprints: TypeFingerprints::new(&file.types),
            values: Vec::new(),
        };
        let id = context.define_bytes(&format!("{}#file", unit_name), name.as_bytes())?;
        context.file_name = (id, name.len());
        Ok(context)
    }

    /// The symbol of the body `body` of the unit's file.
    pub fn symbol(&self, body: BodyId) -> &str {
        &self.symbols[body.index()]
    }

    fn declare_function(
        &mut self,
        symbol: &str,
        signature: &ir::Signature,
    ) -> Result<FuncId, BackendError> {
        Ok(self
            .module
            .declare_function(symbol, Linkage::Export, signature)?)
    }

    /// Defines a read-only data object holding `bytes`.
    fn define_bytes(&mut self, name: &str, bytes: &[u8]) -> Result<DataId, BackendError> {
        let id = self
            .module
            .declare_data(name, Linkage::Local, false, false)?;
        let mut description = DataDescription::new();
        // A data object cannot be empty; a zero-length text keeps one unused byte.
        let contents = if bytes.is_empty() { &[0u8][..] } else { bytes };
        description.define(contents.to_vec().into_boxed_slice());
        self.module.define_data(id, &description)?;
        Ok(id)
    }

    /// Imports the writable data object `name`, which the linker resolves to storage of the
    /// program's value table.
    fn import_storage(&mut self, name: &str) -> Result<DataId, BackendError> {
        Ok(self
            .module
            .declare_data(name, Linkage::Import, true, false)?)
    }

    /// The file name's data object and length.
    pub fn file_name(&self) -> (DataId, usize) {
        self.file_name
    }

    /// The read-only data object holding `text`, defined on first use.
    pub fn text(&mut self, text: &str) -> Result<DataId, BackendError> {
        if let Some(id) = self.texts.get(text) {
            return Ok(*id);
        }
        let name = format!("{}#text{}", self.name, self.texts.len());
        let id = self.define_bytes(&name, text.as_bytes())?;
        self.texts.insert(text.to_string(), id);
        Ok(id)
    }

    /// The runtime function `name`, imported with the signature its table entry records.
    pub fn runtime_function(&mut self, name: &'static str) -> Result<FuncId, BackendError> {
        if let Some(id) = self.runtime.get(name) {
            return Ok(*id);
        }
        let function = crate::externals::entry_point(name)
            .ok_or_else(|| BackendError::new(format!("no runtime function {}", name)))?;
        let mut signature = ir::Signature::new(self.call_conv);
        let machine = |ty: RuntimeType| match ty {
            RuntimeType::I8 => types::I8,
            RuntimeType::I32 => types::I32,
            RuntimeType::I64 => types::I64,
            RuntimeType::F64 => types::F64,
            RuntimeType::Pointer => self.pointer,
        };
        for parameter in function.parameters {
            signature
                .params
                .push(crate::representation::abi_param(machine(*parameter)));
        }
        if let Some(result) = function.result {
            signature
                .returns
                .push(crate::representation::abi_param(machine(result)));
        }
        let id = self
            .module
            .declare_function(name, Linkage::Import, &signature)?;
        self.runtime.insert(name, id);
        Ok(id)
    }

    /// The imported runtime static holding the stack limit (D27).
    pub fn stack_limit(&mut self) -> Result<DataId, BackendError> {
        if let Some(id) = self.stack_limit {
            return Ok(id);
        }
        let id = self
            .module
            .declare_data(STACK_LIMIT_SYMBOL, Linkage::Import, false, false)?;
        self.stack_limit = Some(id);
        Ok(id)
    }

    /// The imported slot of the function table holding the address of `symbol`.
    pub fn slot(&mut self, symbol: &str) -> Result<DataId, BackendError> {
        if let Some(id) = self.slots.get(symbol) {
            return Ok(*id);
        }
        let id = self
            .module
            .declare_data(&slot_symbol(symbol), Linkage::Import, true, false)?;
        self.slots.insert(symbol.to_string(), id);
        Ok(id)
    }

    /// The slot type record of `ty`, `<unit>#slot-type(<type>)`, defined on first use (D20 of
    /// `compose-transform`), laid out as `prism_compose::SlotType`: the type's fingerprint, its
    /// size and alignment, its drop glue or null, its equality glue, and its name.
    pub fn slot_type(&mut self, ty: &Type) -> Result<DataId, BackendError> {
        if let Some(id) = self.slot_types.get(ty) {
            return Ok(*id);
        }
        let layout = self.layouts.layout(ty).map_err(|construct| {
            BackendError::new(format!("no layout for {}: {}", ty, construct.description()))
        })?;
        let name = crate::glue::qualified_name(ty);
        let text = self.text(&name)?;
        let fingerprint = self.fingerprints.fingerprint(ty);
        // A kept borrow is its identity, the reference and its version: two words, no drop,
        // compared word by word, never through its referent (D7 of `strong-skipping`).
        let identity = crate::representation::is_identity_type(ty);
        let drop = match self.glue(GlueKind::Drop, ty)? {
            _ if identity => None,
            Some(crate::glue::Glue::Runtime(function)) => Some(self.runtime_function(function)?),
            Some(crate::glue::Glue::Generated(id)) => Some(id),
            None => None,
        };
        let equals = if identity {
            self.identity_glue(ty)?
        } else {
            self.equality_glue(ty)?
        };
        let (size, align) = if identity {
            (
                2 * self.pointer.bytes() as usize,
                u64::from(self.pointer.bytes()),
            )
        } else {
            (layout.storage_size() as usize, u64::from(layout.align))
        };
        let symbol = format!("{}#slot-type({})", self.name, name);
        let id = self
            .module
            .declare_data(&symbol, Linkage::Local, false, false)?;
        let word = self.pointer.bytes() as usize;
        let mut bytes = vec![0u8; 32 + 6 * word];
        bytes[..32].copy_from_slice(&fingerprint.0);
        bytes[32..32 + word].copy_from_slice(&(size as u64).to_le_bytes()[..word]);
        bytes[32 + word..32 + 2 * word].copy_from_slice(&align.to_le_bytes()[..word]);
        bytes[32 + 5 * word..32 + 6 * word]
            .copy_from_slice(&(name.len() as u64).to_le_bytes()[..word]);
        let mut description = DataDescription::new();
        description.define(bytes.into_boxed_slice());
        description.set_align(u64::from(self.pointer.bytes()));
        if let Some(drop) = drop {
            let drop = self.module.declare_func_in_data(drop, &mut description);
            description.write_function_addr((32 + 2 * word) as u32, drop);
        }
        let equals = self.module.declare_func_in_data(equals, &mut description);
        description.write_function_addr((32 + 3 * word) as u32, equals);
        let text = self.module.declare_data_in_data(text, &mut description);
        description.write_data_addr((32 + 4 * word) as u32, text, 0);
        self.module.define_data(id, &description)?;
        self.slot_types.insert(ty.clone(), id);
        Ok(id)
    }

    /// The descriptor of the closure `body` of the unit's file, defined on first use (D3 of
    /// `closures`, D9 of `compose-transform`): four pointer-sized words. The address of the
    /// closure's slot, written as a relocation to the imported `<closure symbol>#slot` so that
    /// a repointed slot is followed by every value built before; the environment's drop entry;
    /// and the environment equality and duplicate entries of a keepable closure, or null.
    pub fn descriptor(&mut self, body: BodyId) -> Result<DataId, BackendError> {
        if let Some(id) = self.descriptors.get(&body) {
            return Ok(*id);
        }
        let symbol = self.symbol(body).to_string();
        let slot = self.slot(&symbol)?;
        let id =
            self.module
                .declare_data(&descriptor_symbol(&symbol), Linkage::Local, false, false)?;
        let mut description = DataDescription::new();
        let word = self.pointer.bytes();
        description.define(vec![0u8; 4 * word as usize].into_boxed_slice());
        description.set_align(u64::from(word));
        let slot_global = self.module.declare_data_in_data(slot, &mut description);
        description.write_data_addr(0, slot_global, 0);
        // The drop entry: the environment's drop function when it is allocated, a direct code
        // address so that a value keeps the drop of the layout it was built with; null
        // otherwise (D8 and D9 of `escaping-closures`).
        let environment = self
            .layouts
            .environment_layout(&self.file.bodies[body])
            .map_err(|construct| {
                BackendError::new(format!(
                    "no layout for a closure's environment: {}",
                    construct.description()
                ))
            })?;
        if environment.kind == EnvironmentKind::Allocated {
            let drop = self.environment_drop(body)?;
            let drop = self.module.declare_func_in_data(drop, &mut description);
            description.write_function_addr(word, drop);
        }
        // A closure capturing nothing, or owning only `Copy` captures, is keepable: its
        // environments compare by their captures, and an allocated one is duplicated by
        // copying them (D9 of `compose-transform`).
        let capture_count = self.file.bodies[body].capture_count;
        let keepable = environment.captures.iter().all(|field| field.owned)
            && environment
                .captures
                .iter()
                .all(|field| self.file.types.is_copy(&field.ty));
        if environment.captures.is_empty() {
            let equals = self.runtime_function("prism_rt_empty_environments_equal")?;
            let equals = self.module.declare_func_in_data(equals, &mut description);
            description.write_function_addr(2 * word, equals);
        } else if keepable && capture_count > 0 {
            let equals = self.environment_equality(body)?;
            let equals = self.module.declare_func_in_data(equals, &mut description);
            description.write_function_addr(2 * word, equals);
            if environment.kind == EnvironmentKind::Allocated {
                let duplicate = self.environment_duplicate(body)?;
                let duplicate = self
                    .module
                    .declare_func_in_data(duplicate, &mut description);
                description.write_function_addr(3 * word, duplicate);
            }
        }
        self.module.define_data(id, &description)?;
        self.descriptors.insert(body, id);
        Ok(id)
    }

    /// The generated accessor of the top-level `val` `path` of type `ty` (D18): it returns the
    /// address of the value's storage, running the initializer through its slot on the first
    /// call, and a null pointer when called again while the initializer runs. A value in memory
    /// is built in place: the storage's address is the initializer's result address (D17 of
    /// `codegen-aggregates`). A value that owns storage is never dropped, but registered with the
    /// runtime with its drop glue once initialized, so the allocation check can release it before
    /// it reports what is still live (D12 and D13 of `runtime-strings-and-drops`).
    fn accessor(
        &mut self,
        path: &DeclarationPath,
        ty: &Type,
        builder_context: &mut FunctionBuilderContext,
    ) -> Result<LoweredFunction, BackendError> {
        let layout = self.layouts.layout(ty).map_err(|construct| {
            BackendError::new(format!("no layout for {}: {}", ty, construct.description()))
        })?;
        let storage = self.import_storage(&storage_symbol(path))?;
        let state = self.import_storage(&state_symbol(path))?;
        let type_fingerprint = self.fingerprints.fingerprint(ty);
        self.values.push(ValueRecord {
            symbol: function_symbol(path),
            size: layout.storage_size(),
            align: layout.align,
            type_fingerprint,
        });
        let contract = accessor_contract(ty, &mut self.fingerprints);
        let initializer = self.slot(&initializer_symbol(path))?;
        let symbol = accessor_symbol(path);
        let mut accessor_signature = ir::Signature::new(self.call_conv);
        accessor_signature.returns.push(AbiParam::new(self.pointer));
        let id = self.declare_function(&symbol, &accessor_signature)?;
        let mut function =
            Function::with_name_signature(UserFuncName::user(0, id.as_u32()), accessor_signature);
        let initializer_signature = signature(&[], ty, self.call_conv, self.pointer);
        let value_representation = representation(ty, self.pointer);
        let drop_glue = self.glue(GlueKind::Drop, ty)?;
        let register = match drop_glue {
            Some(_) => Some(self.runtime_function("prism_rt_register_top_level_value")?),
            None => None,
        };
        let begin_program = self.runtime_function("prism_compose_begin_program")?;
        let end_program = self.runtime_function("prism_compose_end_program")?;
        {
            let pointer = self.pointer;
            let mut builder = FunctionBuilder::new(&mut function, builder_context);
            let flags = MemFlagsData::trusted();
            let entry = builder.create_block();
            let not_initialized = builder.create_block();
            let reentered = builder.create_block();
            let initialize = builder.create_block();
            let done = builder.create_block();
            builder.set_cold_block(reentered);
            builder.set_cold_block(initialize);

            builder.switch_to_block(entry);
            let storage_address = data_address(&mut builder, storage, pointer);
            let state_address = data_address(&mut builder, state, pointer);
            let current = builder.ins().load(types::I8, flags, state_address, 0);
            let initialized = builder
                .ins()
                .icmp_imm_s(IntCC::Equal, current, state::INITIALIZED);
            builder
                .ins()
                .brif(initialized, done, &[], not_initialized, &[]);

            builder.switch_to_block(not_initialized);
            let initializing = builder
                .ins()
                .icmp_imm_s(IntCC::Equal, current, state::INITIALIZING);
            builder
                .ins()
                .brif(initializing, reentered, &[], initialize, &[]);

            builder.switch_to_block(reentered);
            let null = builder.ins().iconst(pointer, 0);
            builder.ins().return_(&[null]);

            builder.switch_to_block(initialize);
            let marker = builder.ins().iconst(types::I8, state::INITIALIZING);
            builder.ins().store(flags, marker, state_address, 0);
            // The initializer is program code: the state cells it creates belong to the
            // program, even when it first runs during a pass (D11 of `state-handles`). The
            // runtime opens that scope only once compose is in use, and says whether it did.
            let begin_program = self
                .module
                .declare_func_in_func(begin_program, builder.func);
            let begin_call = builder.ins().call(begin_program, &[]);
            let pushed = builder.inst_results(begin_call)[0];
            let slot_address = data_address(&mut builder, initializer, pointer);
            let target = builder.ins().load(pointer, flags, slot_address, 0);
            let signature_ref = builder.import_signature(initializer_signature);
            match value_representation {
                Some(Representation::Memory) => {
                    builder
                        .ins()
                        .call_indirect(signature_ref, target, &[storage_address]);
                }
                Some(Representation::Scalar(_)) => {
                    let call = builder.ins().call_indirect(signature_ref, target, &[]);
                    let value = builder.inst_results(call)[0];
                    builder.ins().store(flags, value, storage_address, 0);
                }
                Some(Representation::Empty) | None => {
                    builder.ins().call_indirect(signature_ref, target, &[]);
                }
            }
            if let (Some(glue), Some(register)) = (drop_glue, register) {
                let glue = self.glue_reference(glue, builder.func)?;
                let glue_address = builder.ins().func_addr(pointer, glue);
                let register = self.module.declare_func_in_func(register, builder.func);
                builder
                    .ins()
                    .call(register, &[storage_address, glue_address]);
            }
            let end_program = self.module.declare_func_in_func(end_program, builder.func);
            builder.ins().call(end_program, &[pushed]);
            let marker = builder.ins().iconst(types::I8, state::INITIALIZED);
            builder.ins().store(flags, marker, state_address, 0);
            builder.ins().jump(done, &[]);

            builder.switch_to_block(done);
            builder.ins().return_(&[storage_address]);
            builder.seal_all_blocks();
            builder.finalize(self.module.target_config());
        }
        Ok(LoweredFunction {
            symbol,
            id,
            function,
            contract: Some(contract),
        })
    }
}

/// The address of the data object `id` in the function being built.
pub(crate) fn data_address(
    builder: &mut FunctionBuilder,
    id: DataId,
    pointer: ir::Type,
) -> ir::Value {
    let global = data_global(builder.func, id);
    builder.ins().symbol_value(pointer, global)
}

/// The global value of the data object `id` in `function`. The reference is never colocated:
/// the JIT does not promise that code and data are within reach of a PC-relative address, so
/// every data address is absolute (or, on x86-64, loaded from a GOT).
pub(crate) fn data_global(function: &mut Function, id: DataId) -> ir::GlobalValue {
    let name = function.declare_imported_user_function(ir::UserExternalName {
        namespace: 1,
        index: id.as_u32(),
    });
    function.create_global_value(ir::GlobalValueData::Symbol {
        name: ir::ExternalName::user(name),
        offset: ir::immediates::Imm64::new(0),
        colocated: false,
        tls: false,
    })
}

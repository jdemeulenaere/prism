//! One file's code unit (D3 of `codegen-core`): the functions lowered from the file's elaborated
//! MIR alone, its data objects, and its imports of runtime functions and of the slots of the
//! function table.
//!
//! A unit refers to the functions and top-level values of other files, and to its own, only
//! through the slots `<symbol>#slot`, imported as data symbols that the linker resolves. Its own
//! data (the file name and the details of runtime errors, the storage and state of its top-level
//! values) is named after its module, so the units of a program never collide.

use crate::error::BackendError;
use crate::glue::{GlueKind, GlueTable};
use crate::layout::LayoutTable;
use crate::lower::BodyLowering;
use crate::representation::{body_signature, representation, signature, Representation};
use crate::supported::is_compiled;
use crate::symbols::{
    accessor_symbol, initializer_symbol, slot_symbol, state_symbol, storage_symbol, symbol_of,
};
use cranelift_codegen::ir::{
    self, condcodes::IntCC, types, AbiParam, Function, InstBuilder, MemFlagsData, UserFuncName,
};
use cranelift_codegen::isa::CallConv;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::{DataDescription, DataId, FuncId, Linkage, Module};
use prism_mir::{BodyOwner, MirFile};
use prism_runtime::{RuntimeType, RUNTIME_FUNCTIONS, STACK_LIMIT_SYMBOL};
use prism_typed_tree::{DeclarationPath, Type};
use std::collections::HashMap;

/// A function of a unit, lowered to Cranelift IR and declared in the module, not yet compiled.
pub struct LoweredFunction {
    pub symbol: String,
    pub id: FuncId,
    pub function: Function,
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
        let mut context = UnitContext::new(file, name, module)?;
        let mut builder_context = FunctionBuilderContext::new();
        let mut functions = Vec::new();
        for (_, body) in file.bodies.iter() {
            if !is_compiled(body) {
                continue;
            }
            let symbol = symbol_of(body, file);
            let signature = body_signature(body, context.call_conv, context.pointer);
            let id = context.declare_function(&symbol, &signature)?;
            let mut function =
                Function::with_name_signature(UserFuncName::user(0, id.as_u32()), signature);
            BodyLowering::lower(&mut context, body, &mut function, &mut builder_context)?;
            functions.push(LoweredFunction {
                symbol,
                id,
                function,
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
    stack_limit: Option<DataId>,
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
            stack_limit: None,
        };
        let id = context.define_bytes(&format!("{}#file", unit_name), name.as_bytes())?;
        context.file_name = (id, name.len());
        Ok(context)
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

    /// Defines a writable, zero-initialized data object of `size` bytes aligned to `align`.
    fn define_zeroed(&mut self, name: &str, size: u32, align: u32) -> Result<DataId, BackendError> {
        let id = self
            .module
            .declare_data(name, Linkage::Local, true, false)?;
        let mut description = DataDescription::new();
        description.define_zeroinit(size as usize);
        description.set_align(u64::from(align));
        self.module.define_data(id, &description)?;
        Ok(id)
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
        let function = RUNTIME_FUNCTIONS
            .iter()
            .find(|function| function.name == name)
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
        let storage =
            self.define_zeroed(&storage_symbol(path), layout.storage_size(), layout.align)?;
        let state = self.define_zeroed(&state_symbol(path), 1, 1)?;
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

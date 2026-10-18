//! Relocatable unit artifacts (D8 of `instance-cache`).
//!
//! A [`UnitArtifact`] is one compiled code unit, a file's or an instance's, in a form that does
//! not depend on the JIT module it was compiled in: the machine code of each function, each data
//! object's contents, and relocations that name their targets by symbol, with the declaration of
//! every symbol the unit defines or refers to. The [`crate::Linker`] defines every unit of a load
//! from its artifact, whether it was just compiled or read from the build cache, so cached code is
//! linked exactly as fresh code is. Its encoding (fixed-width little-endian integers, LEB128
//! lengths, through `prism_mir::codec`) is what code entries store; a unit holding anything the
//! encoding cannot name (a type, call convention, relocation or target it does not know) is
//! uncacheable and is compiled on every run.

use cranelift_codegen::binemit::Reloc;
use cranelift_codegen::ir::{self, AbiParam, ArgumentExtension, ArgumentPurpose, LibCall};
use cranelift_codegen::isa::CallConv;
use cranelift_module::Linkage;
use prism_abi::ContentHash;
use prism_mir::codec::{Codec, Decoder, Encoder};
use std::str::FromStr;

/// One compiled unit, independent of any module.
#[derive(Debug, Clone, PartialEq)]
pub struct UnitArtifact {
    /// Every symbol the unit defines or refers to, each once, in order of first use.
    pub declarations: Vec<Declaration>,
    /// The unit's functions, its slotted ones first, then its glue.
    pub functions: Vec<CompiledFunction>,
    /// The unit's data objects.
    pub data: Vec<DataObject>,
    /// The top-level values whose storage the unit imports (D8 of `hot-reload`).
    pub values: Vec<ValueRecord>,
}

/// A top-level value whose storage a unit imports from the program's value table: its symbol
/// (`<module>::<name>`, without suffix), the size and alignment of its storage, and its type's
/// fingerprint, one of the three parts of its value key (D8 of `hot-reload`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValueRecord {
    pub symbol: String,
    pub size: u32,
    pub align: u32,
    pub type_fingerprint: ContentHash,
}

/// How a symbol is declared in a module.
#[derive(Debug, Clone, PartialEq)]
pub enum Declaration {
    Function {
        name: String,
        linkage: Linkage,
        signature: ir::Signature,
    },
    Data {
        name: String,
        linkage: Linkage,
        writable: bool,
        tls: bool,
    },
}

impl Declaration {
    pub fn name(&self) -> &str {
        match self {
            Declaration::Function { name, .. } | Declaration::Data { name, .. } => name,
        }
    }
}

/// The machine code of one function.
#[derive(Debug, Clone, PartialEq)]
pub struct CompiledFunction {
    /// Its symbol, declared among the artifact's declarations.
    pub name: String,
    /// Whether the function has a slot in the function table; glue has none.
    pub has_slot: bool,
    /// The contract of the code, for a function with a slot (D5 of `hot-reload`).
    pub contract: Option<ContentHash>,
    /// The code's alignment in bytes.
    pub alignment: u64,
    pub code: Vec<u8>,
    pub relocations: Vec<Relocation>,
}

/// The contents of one data object.
#[derive(Debug, Clone, PartialEq)]
pub struct DataObject {
    /// Its name, declared among the artifact's declarations.
    pub name: String,
    /// Its alignment in bytes, or `None` for the module's default.
    pub alignment: Option<u64>,
    pub contents: DataContents,
    /// Pointers written into it: a function's or another data object's address, at an offset.
    pub relocations: Vec<Relocation>,
}

/// How a data object is initialized.
#[derive(Debug, Clone, PartialEq)]
pub enum DataContents {
    Bytes(Vec<u8>),
    Zeros(usize),
}

/// A place in code or data to patch with a target's address.
#[derive(Debug, Clone, PartialEq)]
pub struct Relocation {
    pub offset: u32,
    pub kind: Reloc,
    pub addend: i64,
    pub target: RelocationTarget,
}

/// What a relocation refers to, by name.
#[derive(Debug, Clone, PartialEq)]
pub enum RelocationTarget {
    /// A function, of the unit or imported (a runtime function).
    Function(String),
    /// A data object, of the unit or imported (a slot `<symbol>#slot`, the stack limit).
    Data(String),
    /// A Cranelift library call (`Probestack`, `Memcpy`, ...).
    LibCall(String),
    /// An offset into the named function's own code.
    FunctionOffset(String, u32),
}

impl UnitArtifact {
    /// The artifact's encoding, or `None` when it holds something the encoding cannot name, so
    /// that the unit is uncacheable.
    pub fn encode(&self) -> Option<Vec<u8>> {
        let mut encoder = Encoder::new();
        encode_artifact(self, &mut encoder)?;
        Some(encoder.finish())
    }

    /// The artifact `bytes` encode, or `None` when they are malformed.
    pub fn decode(bytes: &[u8]) -> Option<UnitArtifact> {
        let mut decoder = Decoder::new(bytes);
        let artifact = decode_artifact(&mut decoder)?;
        decoder.is_finished().then_some(artifact)
    }

    /// The number of functions the artifact defines.
    pub fn function_count(&self) -> usize {
        self.functions.len()
    }
}

// --- Encoding

fn encode_artifact(artifact: &UnitArtifact, encoder: &mut Encoder) -> Option<()> {
    let UnitArtifact {
        declarations,
        functions,
        data,
        values,
    } = artifact;
    encoder.length(declarations.len());
    for declaration in declarations {
        match declaration {
            Declaration::Function {
                name,
                linkage,
                signature,
            } => {
                encoder.u8(0);
                encoder.string(name);
                encoder.u8(linkage_tag(*linkage));
                encode_signature(signature, encoder)?;
            }
            Declaration::Data {
                name,
                linkage,
                writable,
                tls,
            } => {
                encoder.u8(1);
                encoder.string(name);
                encoder.u8(linkage_tag(*linkage));
                encoder.bool(*writable);
                encoder.bool(*tls);
            }
        }
    }
    encoder.length(functions.len());
    for function in functions {
        let CompiledFunction {
            name,
            has_slot,
            contract,
            alignment,
            code,
            relocations,
        } = function;
        encoder.string(name);
        encoder.bool(*has_slot);
        encode_hash(contract.as_ref(), encoder);
        encoder.u64(*alignment);
        encoder.bytes(code);
        encode_relocations(relocations, encoder)?;
    }
    encoder.length(data.len());
    for object in data {
        let DataObject {
            name,
            alignment,
            contents,
            relocations,
        } = object;
        encoder.string(name);
        alignment.encode(encoder);
        match contents {
            DataContents::Bytes(bytes) => {
                encoder.u8(0);
                encoder.bytes(bytes);
            }
            DataContents::Zeros(size) => {
                encoder.u8(1);
                encoder.length(*size);
            }
        }
        encode_relocations(relocations, encoder)?;
    }
    encoder.length(values.len());
    for value in values {
        let ValueRecord {
            symbol,
            size,
            align,
            type_fingerprint,
        } = value;
        encoder.string(symbol);
        encoder.u32(*size);
        encoder.u32(*align);
        encoder.bytes(&type_fingerprint.0);
    }
    Some(())
}

/// An optional hash: a presence byte, then its 32 bytes.
fn encode_hash(hash: Option<&ContentHash>, encoder: &mut Encoder) {
    match hash {
        Some(hash) => {
            encoder.u8(1);
            encoder.bytes(&hash.0);
        }
        None => encoder.u8(0),
    }
}

fn decode_hash(decoder: &mut Decoder) -> Option<ContentHash> {
    Some(ContentHash(decoder.bytes()?.try_into().ok()?))
}

fn decode_optional_hash(decoder: &mut Decoder) -> Option<Option<ContentHash>> {
    match decoder.u8()? {
        0 => Some(None),
        1 => Some(Some(decode_hash(decoder)?)),
        _ => None,
    }
}

fn decode_artifact(decoder: &mut Decoder) -> Option<UnitArtifact> {
    let count = decoder.sequence_length()?;
    let mut declarations = Vec::new();
    for _ in 0..count {
        declarations.push(match decoder.u8()? {
            0 => Declaration::Function {
                name: decoder.string()?,
                linkage: linkage_of(decoder.u8()?)?,
                signature: decode_signature(decoder)?,
            },
            1 => Declaration::Data {
                name: decoder.string()?,
                linkage: linkage_of(decoder.u8()?)?,
                writable: decoder.bool()?,
                tls: decoder.bool()?,
            },
            _ => return None,
        });
    }
    let count = decoder.sequence_length()?;
    let mut functions = Vec::new();
    for _ in 0..count {
        functions.push(CompiledFunction {
            name: decoder.string()?,
            has_slot: decoder.bool()?,
            contract: decode_optional_hash(decoder)?,
            alignment: decoder.u64()?,
            code: decoder.bytes()?.to_vec(),
            relocations: decode_relocations(decoder)?,
        });
    }
    let count = decoder.sequence_length()?;
    let mut data = Vec::new();
    for _ in 0..count {
        data.push(DataObject {
            name: decoder.string()?,
            alignment: decoder.decode()?,
            contents: match decoder.u8()? {
                0 => DataContents::Bytes(decoder.bytes()?.to_vec()),
                1 => DataContents::Zeros(decoder.length()?),
                _ => return None,
            },
            relocations: decode_relocations(decoder)?,
        });
    }
    let count = decoder.sequence_length()?;
    let mut values = Vec::new();
    for _ in 0..count {
        values.push(ValueRecord {
            symbol: decoder.string()?,
            size: decoder.u32()?,
            align: decoder.u32()?,
            type_fingerprint: decode_hash(decoder)?,
        });
    }
    Some(UnitArtifact {
        declarations,
        functions,
        data,
        values,
    })
}

fn linkage_tag(linkage: Linkage) -> u8 {
    match linkage {
        Linkage::Import => 0,
        Linkage::Local => 1,
        Linkage::Preemptible => 2,
        Linkage::Hidden => 3,
        Linkage::Export => 4,
    }
}

fn linkage_of(tag: u8) -> Option<Linkage> {
    Some(match tag {
        0 => Linkage::Import,
        1 => Linkage::Local,
        2 => Linkage::Preemptible,
        3 => Linkage::Hidden,
        4 => Linkage::Export,
        _ => return None,
    })
}

/// The machine types a Prism signature uses, by tag. Any other type makes the unit uncacheable.
const TYPES: [ir::Type; 8] = [
    ir::types::I8,
    ir::types::I16,
    ir::types::I32,
    ir::types::I64,
    ir::types::I128,
    ir::types::F32,
    ir::types::F64,
    ir::types::F16,
];

fn encode_signature(signature: &ir::Signature, encoder: &mut Encoder) -> Option<()> {
    let ir::Signature {
        params,
        returns,
        call_conv,
    } = signature;
    encoder.string(&call_conv.to_string());
    for list in [params, returns] {
        encoder.length(list.len());
        for parameter in list {
            let AbiParam {
                value_type,
                purpose,
                extension,
            } = parameter;
            encoder.u8(TYPES.iter().position(|ty| ty == value_type)? as u8);
            match purpose {
                ArgumentPurpose::Normal => encoder.u8(0),
                ArgumentPurpose::StructArgument(size) => {
                    encoder.u8(1);
                    encoder.u32(*size);
                }
                ArgumentPurpose::StructReturn => encoder.u8(2),
                ArgumentPurpose::VMContext => encoder.u8(3),
            }
            encoder.u8(match extension {
                ArgumentExtension::None => 0,
                ArgumentExtension::Uext => 1,
                ArgumentExtension::Sext => 2,
            });
        }
    }
    Some(())
}

fn decode_signature(decoder: &mut Decoder) -> Option<ir::Signature> {
    let call_conv = CallConv::from_str(&decoder.string()?).ok()?;
    let mut lists: [Vec<AbiParam>; 2] = [Vec::new(), Vec::new()];
    for list in &mut lists {
        let count = decoder.sequence_length()?;
        for _ in 0..count {
            let value_type = *TYPES.get(decoder.u8()? as usize)?;
            let purpose = match decoder.u8()? {
                0 => ArgumentPurpose::Normal,
                1 => ArgumentPurpose::StructArgument(decoder.u32()?),
                2 => ArgumentPurpose::StructReturn,
                3 => ArgumentPurpose::VMContext,
                _ => return None,
            };
            let extension = match decoder.u8()? {
                0 => ArgumentExtension::None,
                1 => ArgumentExtension::Uext,
                2 => ArgumentExtension::Sext,
                _ => return None,
            };
            list.push(AbiParam {
                value_type,
                purpose,
                extension,
            });
        }
    }
    let [params, returns] = lists;
    Some(ir::Signature {
        params,
        returns,
        call_conv,
    })
}

fn encode_relocations(relocations: &[Relocation], encoder: &mut Encoder) -> Option<()> {
    encoder.length(relocations.len());
    for relocation in relocations {
        let Relocation {
            offset,
            kind,
            addend,
            target,
        } = relocation;
        encoder.u32(*offset);
        encoder.u8(reloc_tag(*kind));
        encoder.i64(*addend);
        match target {
            RelocationTarget::Function(name) => {
                encoder.u8(0);
                encoder.string(name);
            }
            RelocationTarget::Data(name) => {
                encoder.u8(1);
                encoder.string(name);
            }
            RelocationTarget::LibCall(name) => {
                LibCall::from_str(name).ok()?;
                encoder.u8(2);
                encoder.string(name);
            }
            RelocationTarget::FunctionOffset(name, offset) => {
                encoder.u8(3);
                encoder.string(name);
                encoder.u32(*offset);
            }
        }
    }
    Some(())
}

fn decode_relocations(decoder: &mut Decoder) -> Option<Vec<Relocation>> {
    let count = decoder.sequence_length()?;
    let mut relocations = Vec::new();
    for _ in 0..count {
        let offset = decoder.u32()?;
        let kind = reloc_of(decoder.u8()?)?;
        let addend = decoder.i64()?;
        let target = match decoder.u8()? {
            0 => RelocationTarget::Function(decoder.string()?),
            1 => RelocationTarget::Data(decoder.string()?),
            2 => {
                let name = decoder.string()?;
                LibCall::from_str(&name).ok()?;
                RelocationTarget::LibCall(name)
            }
            3 => RelocationTarget::FunctionOffset(decoder.string()?, decoder.u32()?),
            _ => return None,
        };
        relocations.push(Relocation {
            offset,
            kind,
            addend,
            target,
        });
    }
    Some(relocations)
}

/// Every relocation kind, in tag order.
const RELOCS: [Reloc; 31] = [
    Reloc::Abs4,
    Reloc::Abs8,
    Reloc::X86PCRel4,
    Reloc::X86CallPCRel4,
    Reloc::X86CallPLTRel4,
    Reloc::X86GOTPCRel4,
    Reloc::X86SecRel,
    Reloc::Arm32Call,
    Reloc::Arm64Call,
    Reloc::S390xPCRel32Dbl,
    Reloc::S390xPLTRel32Dbl,
    Reloc::ElfX86_64TlsGd,
    Reloc::MachOX86_64Tlv,
    Reloc::MachOAarch64TlsAdrPage21,
    Reloc::MachOAarch64TlsAdrPageOff12,
    Reloc::Aarch64TlsDescAdrPage21,
    Reloc::Aarch64TlsDescLd64Lo12,
    Reloc::Aarch64TlsDescAddLo12,
    Reloc::Aarch64TlsDescCall,
    Reloc::Aarch64AdrGotPage21,
    Reloc::Aarch64AdrPrelPgHi21,
    Reloc::Aarch64AddAbsLo12Nc,
    Reloc::Aarch64Ld64GotLo12Nc,
    Reloc::RiscvCallPlt,
    Reloc::RiscvTlsGdHi20,
    Reloc::RiscvPCRelLo12I,
    Reloc::RiscvGotHi20,
    Reloc::RiscvPCRelHi20,
    Reloc::S390xTlsGd64,
    Reloc::S390xTlsGdCall,
    Reloc::PulleyPcRel,
];

/// The tag of `kind`: its position in [`RELOCS`], matched exhaustively so that a relocation kind
/// added to Cranelift is a compile error here.
fn reloc_tag(kind: Reloc) -> u8 {
    match kind {
        Reloc::Abs4 => 0,
        Reloc::Abs8 => 1,
        Reloc::X86PCRel4 => 2,
        Reloc::X86CallPCRel4 => 3,
        Reloc::X86CallPLTRel4 => 4,
        Reloc::X86GOTPCRel4 => 5,
        Reloc::X86SecRel => 6,
        Reloc::Arm32Call => 7,
        Reloc::Arm64Call => 8,
        Reloc::S390xPCRel32Dbl => 9,
        Reloc::S390xPLTRel32Dbl => 10,
        Reloc::ElfX86_64TlsGd => 11,
        Reloc::MachOX86_64Tlv => 12,
        Reloc::MachOAarch64TlsAdrPage21 => 13,
        Reloc::MachOAarch64TlsAdrPageOff12 => 14,
        Reloc::Aarch64TlsDescAdrPage21 => 15,
        Reloc::Aarch64TlsDescLd64Lo12 => 16,
        Reloc::Aarch64TlsDescAddLo12 => 17,
        Reloc::Aarch64TlsDescCall => 18,
        Reloc::Aarch64AdrGotPage21 => 19,
        Reloc::Aarch64AdrPrelPgHi21 => 20,
        Reloc::Aarch64AddAbsLo12Nc => 21,
        Reloc::Aarch64Ld64GotLo12Nc => 22,
        Reloc::RiscvCallPlt => 23,
        Reloc::RiscvTlsGdHi20 => 24,
        Reloc::RiscvPCRelLo12I => 25,
        Reloc::RiscvGotHi20 => 26,
        Reloc::RiscvPCRelHi20 => 27,
        Reloc::S390xTlsGd64 => 28,
        Reloc::S390xTlsGdCall => 29,
        Reloc::PulleyPcRel => 30,
        Reloc::PulleyCallIndirectHost => 31,
    }
}

fn reloc_of(tag: u8) -> Option<Reloc> {
    match tag {
        31 => Some(Reloc::PulleyCallIndirectHost),
        tag => RELOCS.get(tag as usize).copied(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn artifact() -> UnitArtifact {
        let mut signature = ir::Signature::new(CallConv::SystemV);
        signature.params.push(AbiParam::new(ir::types::I64));
        signature.params.push(AbiParam::special(
            ir::types::I64,
            ArgumentPurpose::StructReturn,
        ));
        signature.returns.push(AbiParam::new(ir::types::I8).uext());
        UnitArtifact {
            declarations: vec![
                Declaration::Function {
                    name: "main::main".to_string(),
                    linkage: Linkage::Export,
                    signature: signature.clone(),
                },
                Declaration::Function {
                    name: "prism_rt_print_int".to_string(),
                    linkage: Linkage::Import,
                    signature,
                },
                Declaration::Data {
                    name: "main::f#slot".to_string(),
                    linkage: Linkage::Import,
                    writable: true,
                    tls: false,
                },
            ],
            functions: vec![CompiledFunction {
                name: "main::main".to_string(),
                has_slot: true,
                contract: Some(ContentHash([7; 32])),
                alignment: 16,
                code: vec![1, 2, 3, 4],
                relocations: vec![
                    Relocation {
                        offset: 2,
                        kind: Reloc::Abs8,
                        addend: -4,
                        target: RelocationTarget::Data("main::f#slot".to_string()),
                    },
                    Relocation {
                        offset: 0,
                        kind: Reloc::Arm64Call,
                        addend: 0,
                        target: RelocationTarget::LibCall("Probestack".to_string()),
                    },
                    Relocation {
                        offset: 1,
                        kind: Reloc::PulleyCallIndirectHost,
                        addend: 0,
                        target: RelocationTarget::FunctionOffset("main::main".to_string(), 3),
                    },
                ],
            }],
            data: vec![
                DataObject {
                    name: "main#file".to_string(),
                    alignment: None,
                    contents: DataContents::Bytes(b"main.prism".to_vec()),
                    relocations: Vec::new(),
                },
                DataObject {
                    name: "main#descriptor".to_string(),
                    alignment: Some(8),
                    contents: DataContents::Zeros(16),
                    relocations: vec![Relocation {
                        offset: 8,
                        kind: Reloc::Abs8,
                        addend: 0,
                        target: RelocationTarget::Function("main::main".to_string()),
                    }],
                },
            ],
            values: vec![ValueRecord {
                symbol: "main::LIMIT".to_string(),
                size: 8,
                align: 8,
                type_fingerprint: ContentHash([9; 32]),
            }],
        }
    }

    #[test]
    fn an_artifact_round_trips() {
        let bytes = artifact().encode().unwrap();
        assert_eq!(UnitArtifact::decode(&bytes), Some(artifact()));
    }

    #[test]
    fn a_truncated_artifact_decodes_to_none() {
        let bytes = artifact().encode().unwrap();
        for length in 0..bytes.len() {
            assert_eq!(UnitArtifact::decode(&bytes[..length]), None, "{}", length);
        }
    }

    #[test]
    fn what_the_encoding_cannot_name_is_uncacheable() {
        let mut unknown = artifact();
        unknown.functions[0].relocations[1].target =
            RelocationTarget::LibCall("NoSuchCall".to_string());
        assert_eq!(unknown.encode(), None);
        let mut vector = artifact();
        if let Declaration::Function { signature, .. } = &mut vector.declarations[0] {
            signature.params.push(AbiParam::new(ir::types::I64X2));
        }
        assert_eq!(vector.encode(), None);
    }

    #[test]
    fn every_relocation_kind_has_a_tag() {
        for (index, kind) in RELOCS.iter().enumerate() {
            assert_eq!(reloc_tag(*kind) as usize, index);
            assert_eq!(reloc_of(index as u8), Some(*kind));
        }
        assert_eq!(
            reloc_of(reloc_tag(Reloc::PulleyCallIndirectHost)),
            Some(Reloc::PulleyCallIndirectHost)
        );
        assert_eq!(reloc_of(32), None);
    }
}

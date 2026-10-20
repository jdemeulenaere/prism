//! The canonical binary encoding of generic records and file summaries (D4 of `instance-cache`).
//!
//! An [`Encoder`] appends to a byte vector and a [`Decoder`] reads a byte slice, returning `None`
//! on any malformed input, never panicking. The format has no self-description: fixed-width
//! little-endian `u32`, `u64`, `i64` and `f64`, LEB128 for lengths, indices and source
//! positions, UTF-8 strings after their length, and a tag byte per enum variant. Every
//! [`Codec`] implementation over an enum matches it exhaustively, without a wildcard arm, so a
//! construct added to MIR or to the typed tree is a compile error here until it is encoded. A
//! [`TypeTable`] is encoded in [`NominalId`] order, and derived indices (its member indices) are
//! rebuilt on decode, so the bytes are a function of the value alone: two encodings of one
//! record are identical, which makes the encoding's hash the record's Monomorphization Hash
//! (D2).

use crate::mir::{
    AggregateKind, BasicBlock, Body, BodyId, BodyOwner, BorrowOrigin, Callee, Capture, CaptureMode,
    Constant, LocalDeclaration, LocalId, LocalKind, Operand, Place, Projection, Rvalue,
    StateOperation, Statement, StatementKind, TemplatePart, Terminator, TerminatorKind,
};
use crate::record::GenericRecord;
use prism_ast::{Span, TypeMarker};
use prism_typed_tree::ParameterKind;
use prism_typed_tree::{
    Arena, ArenaId, BinaryOperator, BuiltinFunction, CaptureProjection, DeclarationPath, NominalId,
    OwnershipReason, OwnershipReasonKind, Type, TypeDef, TypeParameter, TypeTable, UnaryOperator,
    Widening, WideningReason,
};

/// Appends a value's encoding to a byte vector.
#[derive(Debug, Default)]
pub struct Encoder {
    bytes: Vec<u8>,
}

impl Encoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// The bytes written.
    pub fn finish(self) -> Vec<u8> {
        self.bytes
    }

    pub fn u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    pub fn bool(&mut self, value: bool) {
        self.u8(u8::from(value));
    }

    pub fn u32(&mut self, value: u32) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    pub fn u64(&mut self, value: u64) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    pub fn i64(&mut self, value: i64) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    pub fn f64(&mut self, value: f64) {
        self.u64(value.to_bits());
    }

    /// A length, an index or a position, as LEB128.
    pub fn length(&mut self, value: usize) {
        let mut value = value as u64;
        loop {
            let byte = (value & 0x7F) as u8;
            value >>= 7;
            if value == 0 {
                self.u8(byte);
                return;
            }
            self.u8(byte | 0x80);
        }
    }

    pub fn string(&mut self, value: &str) {
        self.bytes(value.as_bytes());
    }

    /// `value` after its length.
    pub fn bytes(&mut self, value: &[u8]) {
        self.length(value.len());
        self.bytes.extend_from_slice(value);
    }

    /// Encodes `value`.
    pub fn encode<T: Codec>(&mut self, value: &T) {
        value.encode(self);
    }
}

/// Reads values from a byte slice, each read returning `None` when the input is malformed.
#[derive(Debug)]
pub struct Decoder<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Decoder<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    /// Whether every byte was read.
    pub fn is_finished(&self) -> bool {
        self.position == self.bytes.len()
    }

    fn remaining(&self) -> usize {
        self.bytes.len() - self.position
    }

    fn take(&mut self, count: usize) -> Option<&'a [u8]> {
        let end = self.position.checked_add(count)?;
        let slice = self.bytes.get(self.position..end)?;
        self.position = end;
        Some(slice)
    }

    pub fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }

    pub fn bool(&mut self) -> Option<bool> {
        match self.u8()? {
            0 => Some(false),
            1 => Some(true),
            _ => None,
        }
    }

    pub fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }

    pub fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }

    pub fn i64(&mut self) -> Option<i64> {
        Some(i64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }

    pub fn f64(&mut self) -> Option<f64> {
        Some(f64::from_bits(self.u64()?))
    }

    /// A LEB128 length, index or position that fits a `usize`.
    pub fn length(&mut self) -> Option<usize> {
        let mut value: u64 = 0;
        for shift in (0..64).step_by(7) {
            let byte = self.u8()?;
            let bits = u64::from(byte & 0x7F);
            if shift == 63 && bits > 1 {
                return None;
            }
            value |= bits << shift;
            if byte & 0x80 == 0 {
                return usize::try_from(value).ok();
            }
        }
        None
    }

    /// The length of a sequence whose every element takes at least one byte, so that a corrupt
    /// length fails at once rather than after a large allocation.
    pub fn sequence_length(&mut self) -> Option<usize> {
        let length = self.length()?;
        (length <= self.remaining()).then_some(length)
    }

    /// Bytes written by [`Encoder::bytes`].
    pub fn bytes(&mut self) -> Option<&'a [u8]> {
        let length = self.length()?;
        self.take(length)
    }

    pub fn string(&mut self) -> Option<String> {
        Some(std::str::from_utf8(self.bytes()?).ok()?.to_string())
    }

    /// Decodes a `T`.
    pub fn decode<T: Codec>(&mut self) -> Option<T> {
        T::decode(self)
    }
}

/// A value with a canonical encoding.
pub trait Codec: Sized {
    fn encode(&self, encoder: &mut Encoder);
    fn decode(decoder: &mut Decoder) -> Option<Self>;
}

/// The encoding of `value`.
pub fn encode<T: Codec>(value: &T) -> Vec<u8> {
    let mut encoder = Encoder::new();
    value.encode(&mut encoder);
    encoder.finish()
}

/// The value `bytes` encodes, or `None` when they are malformed or hold anything after it.
pub fn decode<T: Codec>(bytes: &[u8]) -> Option<T> {
    let mut decoder = Decoder::new(bytes);
    let value = T::decode(&mut decoder)?;
    decoder.is_finished().then_some(value)
}

// --- Primitives and containers

impl Codec for bool {
    fn encode(&self, encoder: &mut Encoder) {
        encoder.bool(*self);
    }
    fn decode(decoder: &mut Decoder) -> Option<Self> {
        decoder.bool()
    }
}

impl Codec for ParameterKind {
    fn encode(&self, encoder: &mut Encoder) {
        encoder.u8(match self {
            ParameterKind::Plain => 0,
            ParameterKind::Escaping => 1,
            ParameterKind::Attached => 2,
        });
    }
    fn decode(decoder: &mut Decoder) -> Option<Self> {
        Some(match decoder.u8()? {
            0 => ParameterKind::Plain,
            1 => ParameterKind::Escaping,
            2 => ParameterKind::Attached,
            _ => return None,
        })
    }
}

impl Codec for u32 {
    fn encode(&self, encoder: &mut Encoder) {
        encoder.u32(*self);
    }
    fn decode(decoder: &mut Decoder) -> Option<Self> {
        decoder.u32()
    }
}

impl Codec for u64 {
    fn encode(&self, encoder: &mut Encoder) {
        encoder.u64(*self);
    }
    fn decode(decoder: &mut Decoder) -> Option<Self> {
        decoder.u64()
    }
}

impl Codec for usize {
    fn encode(&self, encoder: &mut Encoder) {
        encoder.length(*self);
    }
    fn decode(decoder: &mut Decoder) -> Option<Self> {
        decoder.length()
    }
}

impl Codec for String {
    fn encode(&self, encoder: &mut Encoder) {
        encoder.string(self);
    }
    fn decode(decoder: &mut Decoder) -> Option<Self> {
        decoder.string()
    }
}

impl Codec for [u8; 32] {
    fn encode(&self, encoder: &mut Encoder) {
        encoder.bytes.extend_from_slice(self);
    }
    fn decode(decoder: &mut Decoder) -> Option<Self> {
        decoder.take(32)?.try_into().ok()
    }
}

impl<T: Codec> Codec for Vec<T> {
    fn encode(&self, encoder: &mut Encoder) {
        encoder.length(self.len());
        for item in self {
            item.encode(encoder);
        }
    }
    fn decode(decoder: &mut Decoder) -> Option<Self> {
        let length = decoder.sequence_length()?;
        let mut items = Vec::new();
        for _ in 0..length {
            items.push(T::decode(decoder)?);
        }
        Some(items)
    }
}

impl<T: Codec> Codec for Option<T> {
    fn encode(&self, encoder: &mut Encoder) {
        match self {
            None => encoder.u8(0),
            Some(value) => {
                encoder.u8(1);
                value.encode(encoder);
            }
        }
    }
    fn decode(decoder: &mut Decoder) -> Option<Self> {
        match decoder.u8()? {
            0 => Some(None),
            1 => Some(Some(T::decode(decoder)?)),
            _ => None,
        }
    }
}

impl<T: Codec> Codec for Box<T> {
    fn encode(&self, encoder: &mut Encoder) {
        self.as_ref().encode(encoder);
    }
    fn decode(decoder: &mut Decoder) -> Option<Self> {
        Some(Box::new(T::decode(decoder)?))
    }
}

impl<A: Codec, B: Codec> Codec for (A, B) {
    fn encode(&self, encoder: &mut Encoder) {
        self.0.encode(encoder);
        self.1.encode(encoder);
    }
    fn decode(decoder: &mut Decoder) -> Option<Self> {
        Some((A::decode(decoder)?, B::decode(decoder)?))
    }
}

impl<I: ArenaId, T: Codec> Codec for Arena<I, T> {
    fn encode(&self, encoder: &mut Encoder) {
        encoder.length(self.len());
        for (_, item) in self.iter() {
            item.encode(encoder);
        }
    }
    fn decode(decoder: &mut Decoder) -> Option<Self> {
        let length = decoder.sequence_length()?;
        let mut arena = Arena::new();
        for _ in 0..length {
            arena.alloc(T::decode(decoder)?);
        }
        Some(arena)
    }
}

/// Implements [`Codec`] for an id of a [`prism_typed_tree::define_id`] arena as its index.
macro_rules! id_codec {
    ($($id:ty),*) => {$(
        impl Codec for $id {
            fn encode(&self, encoder: &mut Encoder) {
                encoder.length(self.index());
            }
            fn decode(decoder: &mut Decoder) -> Option<Self> {
                let index = u32::try_from(decoder.length()?).ok()?;
                Some(<$id>::from_index(index as usize))
            }
        }
    )*};
}

id_codec!(
    LocalId,
    BodyId,
    crate::mir::BlockId,
    crate::mir::GroupId,
    prism_typed_tree::LocalId
);

// --- Source positions and the typed tree's types

impl Codec for Span {
    fn encode(&self, encoder: &mut Encoder) {
        encoder.length(self.start);
        encoder.length(self.end);
        encoder.length(self.line as usize);
        encoder.length(self.column as usize);
    }
    fn decode(decoder: &mut Decoder) -> Option<Self> {
        Some(Span {
            start: decoder.length()?,
            end: decoder.length()?,
            line: u32::try_from(decoder.length()?).ok()?,
            column: u32::try_from(decoder.length()?).ok()?,
        })
    }
}

impl Codec for TypeMarker {
    fn encode(&self, encoder: &mut Encoder) {
        encoder.u8(match self {
            TypeMarker::Copy => 0,
            TypeMarker::Clone => 1,
        });
    }
    fn decode(decoder: &mut Decoder) -> Option<Self> {
        match decoder.u8()? {
            0 => Some(TypeMarker::Copy),
            1 => Some(TypeMarker::Clone),
            _ => None,
        }
    }
}

impl Codec for TypeParameter {
    fn encode(&self, encoder: &mut Encoder) {
        let TypeParameter { index, name, bound } = self;
        encoder.length(*index as usize);
        name.encode(encoder);
        bound.encode(encoder);
    }
    fn decode(decoder: &mut Decoder) -> Option<Self> {
        Some(TypeParameter {
            index: u32::try_from(decoder.length()?).ok()?,
            name: decoder.decode()?,
            bound: decoder.decode()?,
        })
    }
}

impl Codec for NominalId {
    fn encode(&self, encoder: &mut Encoder) {
        let NominalId { module, name } = self;
        module.encode(encoder);
        name.encode(encoder);
    }
    fn decode(decoder: &mut Decoder) -> Option<Self> {
        Some(NominalId {
            module: decoder.decode()?,
            name: decoder.decode()?,
        })
    }
}

impl Codec for Type {
    fn encode(&self, encoder: &mut Encoder) {
        match self {
            Type::Int => encoder.u8(0),
            Type::Float => encoder.u8(1),
            Type::Bool => encoder.u8(2),
            Type::Char => encoder.u8(3),
            Type::String => encoder.u8(4),
            Type::Unit => encoder.u8(5),
            Type::Nothing => encoder.u8(6),
            Type::Reference { mutable, inner } => {
                encoder.u8(7);
                mutable.encode(encoder);
                inner.encode(encoder);
            }
            Type::Nullable(inner) => {
                encoder.u8(8);
                inner.encode(encoder);
            }
            Type::Heap(inner) => {
                encoder.u8(9);
                inner.encode(encoder);
            }
            Type::List(inner) => {
                encoder.u8(10);
                inner.encode(encoder);
            }
            Type::Struct { id, arguments } => {
                encoder.u8(11);
                id.encode(encoder);
                arguments.encode(encoder);
            }
            Type::Enum { id, arguments } => {
                encoder.u8(12);
                id.encode(encoder);
                arguments.encode(encoder);
            }
            Type::Opaque(name) => {
                encoder.u8(13);
                name.encode(encoder);
            }
            Type::Fn {
                is_compose,
                receiver,
                params,
                kinds: parameter_kinds,
                ret,
            } => {
                // A function type with a receiver has its own tag, so plain ones encode as
                // they always did.
                encoder.u8(if *receiver { 17 } else { 14 });
                is_compose.encode(encoder);
                params.encode(encoder);
                parameter_kinds.encode(encoder);
                ret.encode(encoder);
            }
            Type::Parameter(parameter) => {
                encoder.u8(15);
                parameter.encode(encoder);
            }
            Type::Error => encoder.u8(16),
        }
    }
    fn decode(decoder: &mut Decoder) -> Option<Self> {
        Some(match decoder.u8()? {
            0 => Type::Int,
            1 => Type::Float,
            2 => Type::Bool,
            3 => Type::Char,
            4 => Type::String,
            5 => Type::Unit,
            6 => Type::Nothing,
            7 => Type::Reference {
                mutable: decoder.decode()?,
                inner: decoder.decode()?,
            },
            8 => Type::Nullable(decoder.decode()?),
            9 => Type::Heap(decoder.decode()?),
            10 => Type::List(decoder.decode()?),
            11 => Type::Struct {
                id: decoder.decode()?,
                arguments: decoder.decode()?,
            },
            12 => Type::Enum {
                id: decoder.decode()?,
                arguments: decoder.decode()?,
            },
            13 => Type::Opaque(decoder.decode()?),
            tag @ (14 | 17) => {
                let receiver = tag == 17;
                let is_compose = decoder.decode()?;
                let params: Vec<Type> = decoder.decode()?;
                let parameter_kinds: Vec<ParameterKind> = decoder.decode()?;
                if parameter_kinds.len() != params.len() || (receiver && params.is_empty()) {
                    return None;
                }
                Type::Fn {
                    is_compose,
                    receiver,
                    params,
                    kinds: parameter_kinds,
                    ret: decoder.decode()?,
                }
            }
            15 => Type::Parameter(decoder.decode()?),
            16 => Type::Error,
            _ => return None,
        })
    }
}

impl Codec for DeclarationPath {
    fn encode(&self, encoder: &mut Encoder) {
        let DeclarationPath {
            module,
            receiver,
            name,
            context,
        } = self;
        module.encode(encoder);
        receiver.encode(encoder);
        name.encode(encoder);
        context.encode(encoder);
    }
    fn decode(decoder: &mut Decoder) -> Option<Self> {
        Some(DeclarationPath {
            module: decoder.decode()?,
            receiver: decoder.decode()?,
            name: decoder.decode()?,
            context: decoder.decode()?,
        })
    }
}

impl Codec for TypeDef {
    fn encode(&self, encoder: &mut Encoder) {
        match self {
            TypeDef::Struct { parameters, fields } => {
                encoder.u8(0);
                parameters.encode(encoder);
                encoder.length(fields.len());
                for (name, is_var, ty) in fields {
                    name.encode(encoder);
                    is_var.encode(encoder);
                    ty.encode(encoder);
                }
            }
            TypeDef::Enum {
                parameters,
                variants,
            } => {
                encoder.u8(1);
                parameters.encode(encoder);
                variants.encode(encoder);
            }
        }
    }
    fn decode(decoder: &mut Decoder) -> Option<Self> {
        match decoder.u8()? {
            0 => {
                let parameters = decoder.decode()?;
                let length = decoder.sequence_length()?;
                let mut fields = Vec::new();
                for _ in 0..length {
                    fields.push((decoder.decode()?, decoder.decode()?, decoder.decode()?));
                }
                Some(TypeDef::Struct { parameters, fields })
            }
            1 => Some(TypeDef::Enum {
                parameters: decoder.decode()?,
                variants: decoder.decode()?,
            }),
            _ => None,
        }
    }
}

/// A table is its ids in [`NominalId`] order, each with its definition and its marker entry, if
/// recorded; its member indices are rebuilt from the definitions.
impl Codec for TypeTable {
    fn encode(&self, encoder: &mut Encoder) {
        let ids: Vec<&NominalId> = self.ids().collect();
        encoder.length(ids.len());
        for id in ids {
            id.encode(encoder);
            match self.definition(id) {
                None => encoder.u8(0),
                Some(definition) => {
                    encoder.u8(1);
                    definition.encode(encoder);
                }
            }
            match self.marker_entry(id) {
                None => encoder.u8(0),
                Some(entry) => {
                    encoder.u8(1);
                    entry.marker.encode(encoder);
                    entry.parameters.encode(encoder);
                }
            }
        }
    }
    fn decode(decoder: &mut Decoder) -> Option<Self> {
        let length = decoder.sequence_length()?;
        let mut table = TypeTable::new();
        let mut previous: Option<NominalId> = None;
        for _ in 0..length {
            let id: NominalId = decoder.decode()?;
            // Ids are strictly increasing, so the table decodes to exactly what was encoded.
            if previous.as_ref().is_some_and(|previous| *previous >= id) {
                return None;
            }
            let definition: Option<TypeDef> = decoder.decode()?;
            let marker = match decoder.u8()? {
                0 => None,
                1 => Some((decoder.decode()?, decoder.decode()?)),
                _ => return None,
            };
            if definition.is_none() && marker.is_none() {
                return None;
            }
            if let Some(definition) = definition {
                table.insert_definition(id.clone(), definition);
            }
            if let Some((marker, parameters)) = marker {
                table.insert_marker(id.clone(), marker, parameters);
            }
            previous = Some(id);
        }
        Some(table)
    }
}

impl Codec for BinaryOperator {
    fn encode(&self, encoder: &mut Encoder) {
        encoder.u8(match self {
            BinaryOperator::Add => 0,
            BinaryOperator::Subtract => 1,
            BinaryOperator::Multiply => 2,
            BinaryOperator::Divide => 3,
            BinaryOperator::Remainder => 4,
            BinaryOperator::Equal => 5,
            BinaryOperator::NotEqual => 6,
            BinaryOperator::Less => 7,
            BinaryOperator::LessOrEqual => 8,
            BinaryOperator::Greater => 9,
            BinaryOperator::GreaterOrEqual => 10,
        });
    }
    fn decode(decoder: &mut Decoder) -> Option<Self> {
        Some(match decoder.u8()? {
            0 => BinaryOperator::Add,
            1 => BinaryOperator::Subtract,
            2 => BinaryOperator::Multiply,
            3 => BinaryOperator::Divide,
            4 => BinaryOperator::Remainder,
            5 => BinaryOperator::Equal,
            6 => BinaryOperator::NotEqual,
            7 => BinaryOperator::Less,
            8 => BinaryOperator::LessOrEqual,
            9 => BinaryOperator::Greater,
            10 => BinaryOperator::GreaterOrEqual,
            _ => return None,
        })
    }
}

impl Codec for UnaryOperator {
    fn encode(&self, encoder: &mut Encoder) {
        encoder.u8(match self {
            UnaryOperator::Negate => 0,
            UnaryOperator::Not => 1,
        });
    }
    fn decode(decoder: &mut Decoder) -> Option<Self> {
        Some(match decoder.u8()? {
            0 => UnaryOperator::Negate,
            1 => UnaryOperator::Not,
            _ => return None,
        })
    }
}

impl Codec for StateOperation {
    fn encode(&self, encoder: &mut Encoder) {
        let code = StateOperation::ALL
            .iter()
            .position(|operation| operation == self)
            .expect("every operation is listed");
        encoder.u8(code as u8);
    }
    fn decode(decoder: &mut Decoder) -> Option<Self> {
        StateOperation::ALL.get(decoder.u8()? as usize).copied()
    }
}

impl Codec for BuiltinFunction {
    fn encode(&self, encoder: &mut Encoder) {
        encoder.u8(match self {
            BuiltinFunction::Clone => 0,
            BuiltinFunction::Print => 1,
            BuiltinFunction::PrintLine => 2,
            BuiltinFunction::StringLength => 3,
            BuiltinFunction::StringCharacter => 4,
            BuiltinFunction::ToString => 5,
            BuiltinFunction::HeapAllocate => 6,
            BuiltinFunction::Unbox => 7,
            BuiltinFunction::ListOf => 8,
            BuiltinFunction::ListSize => 9,
            BuiltinFunction::ListIndices => 10,
            BuiltinFunction::ListIsEmpty => 11,
            BuiltinFunction::ListIsNotEmpty => 12,
            BuiltinFunction::ListContains => 13,
            BuiltinFunction::ListIndexOf => 14,
            BuiltinFunction::ListAdd => 15,
            BuiltinFunction::ListInsert => 16,
            BuiltinFunction::ListRemoveAt => 17,
            BuiltinFunction::ListRemoveLast => 18,
            BuiltinFunction::ListReverse => 19,
            BuiltinFunction::ListClear => 20,
            BuiltinFunction::Composition => 21,
            BuiltinFunction::SetContent => 22,
            BuiltinFunction::Recompose => 23,
            BuiltinFunction::DumpComposition => 24,
            BuiltinFunction::Remember => 25,
            BuiltinFunction::Key => 26,
            BuiltinFunction::CurrentRecomposeScope => 27,
            BuiltinFunction::InvalidateScope => 28,
            BuiltinFunction::MutableStateOf => 29,
            BuiltinFunction::StateValue => 30,
            BuiltinFunction::StateRead => 31,
            BuiltinFunction::StateUpdate => 32,
            BuiltinFunction::StateEdit => 33,
            BuiltinFunction::SideEffect => 34,
            BuiltinFunction::UiNode => 35,
            BuiltinFunction::UiLeafNode => 36,
            BuiltinFunction::RunApp => 37,
            BuiltinFunction::DpOfInt => 38,
            BuiltinFunction::TextUnitOfInt => 39,
        });
    }
    fn decode(decoder: &mut Decoder) -> Option<Self> {
        Some(match decoder.u8()? {
            0 => BuiltinFunction::Clone,
            1 => BuiltinFunction::Print,
            2 => BuiltinFunction::PrintLine,
            3 => BuiltinFunction::StringLength,
            4 => BuiltinFunction::StringCharacter,
            5 => BuiltinFunction::ToString,
            6 => BuiltinFunction::HeapAllocate,
            7 => BuiltinFunction::Unbox,
            8 => BuiltinFunction::ListOf,
            9 => BuiltinFunction::ListSize,
            10 => BuiltinFunction::ListIndices,
            11 => BuiltinFunction::ListIsEmpty,
            12 => BuiltinFunction::ListIsNotEmpty,
            13 => BuiltinFunction::ListContains,
            14 => BuiltinFunction::ListIndexOf,
            15 => BuiltinFunction::ListAdd,
            16 => BuiltinFunction::ListInsert,
            17 => BuiltinFunction::ListRemoveAt,
            18 => BuiltinFunction::ListRemoveLast,
            19 => BuiltinFunction::ListReverse,
            20 => BuiltinFunction::ListClear,
            21 => BuiltinFunction::Composition,
            22 => BuiltinFunction::SetContent,
            23 => BuiltinFunction::Recompose,
            24 => BuiltinFunction::DumpComposition,
            25 => BuiltinFunction::Remember,
            26 => BuiltinFunction::Key,
            27 => BuiltinFunction::CurrentRecomposeScope,
            28 => BuiltinFunction::InvalidateScope,
            29 => BuiltinFunction::MutableStateOf,
            30 => BuiltinFunction::StateValue,
            31 => BuiltinFunction::StateRead,
            32 => BuiltinFunction::StateUpdate,
            33 => BuiltinFunction::StateEdit,
            34 => BuiltinFunction::SideEffect,
            35 => BuiltinFunction::UiNode,
            36 => BuiltinFunction::UiLeafNode,
            37 => BuiltinFunction::RunApp,
            38 => BuiltinFunction::DpOfInt,
            39 => BuiltinFunction::TextUnitOfInt,
            _ => return None,
        })
    }
}

impl Codec for CaptureProjection {
    fn encode(&self, encoder: &mut Encoder) {
        match self {
            CaptureProjection::Field(index) => {
                encoder.u8(0);
                index.encode(encoder);
            }
            CaptureProjection::Deref => encoder.u8(1),
        }
    }
    fn decode(decoder: &mut Decoder) -> Option<Self> {
        Some(match decoder.u8()? {
            0 => CaptureProjection::Field(decoder.decode()?),
            1 => CaptureProjection::Deref,
            _ => return None,
        })
    }
}

impl Codec for Widening {
    fn encode(&self, encoder: &mut Encoder) {
        let Widening {
            projections,
            reason,
        } = self;
        projections.encode(encoder);
        encoder.u8(match reason {
            WideningReason::NotCopy => 0,
            WideningReason::VariableBinding => 1,
            WideningReason::MutableReference => 2,
            WideningReason::VariableField => 3,
        });
    }
    fn decode(decoder: &mut Decoder) -> Option<Self> {
        let projections = decoder.decode()?;
        let reason = match decoder.u8()? {
            0 => WideningReason::NotCopy,
            1 => WideningReason::VariableBinding,
            2 => WideningReason::MutableReference,
            3 => WideningReason::VariableField,
            _ => return None,
        };
        Some(Widening {
            projections,
            reason,
        })
    }
}

impl Codec for OwnershipReason {
    fn encode(&self, encoder: &mut Encoder) {
        let OwnershipReason { span, kind } = self;
        span.encode(encoder);
        match kind {
            OwnershipReasonKind::Returned => encoder.u8(0),
            OwnershipReasonKind::TopLevelValue => encoder.u8(1),
            OwnershipReasonKind::StoredInField => encoder.u8(2),
            OwnershipReasonKind::StoredInPayload => encoder.u8(3),
            OwnershipReasonKind::StoredInBox => encoder.u8(4),
            OwnershipReasonKind::WrittenThroughReference => encoder.u8(5),
            OwnershipReasonKind::AssignedToCapturedVariable => encoder.u8(6),
            OwnershipReasonKind::PassedToGenericParameter => encoder.u8(7),
            OwnershipReasonKind::PassedToEscapingParameter => encoder.u8(8),
            OwnershipReasonKind::CapturedByOwningClosure => encoder.u8(9),
            OwnershipReasonKind::StoredOutsideBlock { name } => {
                encoder.u8(10);
                name.encode(encoder);
            }
            OwnershipReasonKind::StoredInState => encoder.u8(11),
            OwnershipReasonKind::PassedToAttachedParameter => encoder.u8(12),
        }
    }
    fn decode(decoder: &mut Decoder) -> Option<Self> {
        let span = decoder.decode()?;
        let kind = match decoder.u8()? {
            0 => OwnershipReasonKind::Returned,
            1 => OwnershipReasonKind::TopLevelValue,
            2 => OwnershipReasonKind::StoredInField,
            3 => OwnershipReasonKind::StoredInPayload,
            4 => OwnershipReasonKind::StoredInBox,
            5 => OwnershipReasonKind::WrittenThroughReference,
            6 => OwnershipReasonKind::AssignedToCapturedVariable,
            7 => OwnershipReasonKind::PassedToGenericParameter,
            8 => OwnershipReasonKind::PassedToEscapingParameter,
            9 => OwnershipReasonKind::CapturedByOwningClosure,
            10 => OwnershipReasonKind::StoredOutsideBlock {
                name: decoder.decode()?,
            },
            11 => OwnershipReasonKind::StoredInState,
            12 => OwnershipReasonKind::PassedToAttachedParameter,
            _ => return None,
        };
        Some(OwnershipReason { span, kind })
    }
}

// --- MIR

impl Codec for BodyOwner {
    fn encode(&self, encoder: &mut Encoder) {
        match self {
            BodyOwner::Function(path) => {
                encoder.u8(0);
                path.encode(encoder);
            }
            BodyOwner::Value(path) => {
                encoder.u8(1);
                path.encode(encoder);
            }
            BodyOwner::Default {
                function,
                parameter,
                name,
            } => {
                encoder.u8(2);
                function.encode(encoder);
                parameter.encode(encoder);
                name.encode(encoder);
            }
            BodyOwner::Closure { parent, index } => {
                encoder.u8(3);
                parent.encode(encoder);
                index.encode(encoder);
            }
            BodyOwner::Restart { function } => {
                encoder.u8(4);
                function.encode(encoder);
            }
            BodyOwner::RootContent { parent } => {
                encoder.u8(5);
                parent.encode(encoder);
            }
        }
    }
    fn decode(decoder: &mut Decoder) -> Option<Self> {
        Some(match decoder.u8()? {
            0 => BodyOwner::Function(decoder.decode()?),
            1 => BodyOwner::Value(decoder.decode()?),
            2 => BodyOwner::Default {
                function: decoder.decode()?,
                parameter: decoder.decode()?,
                name: decoder.decode()?,
            },
            3 => BodyOwner::Closure {
                parent: decoder.decode()?,
                index: decoder.decode()?,
            },
            4 => BodyOwner::Restart {
                function: decoder.decode()?,
            },
            5 => BodyOwner::RootContent {
                parent: decoder.decode()?,
            },
            _ => return None,
        })
    }
}

impl Codec for CaptureMode {
    fn encode(&self, encoder: &mut Encoder) {
        encoder.u8(match self {
            CaptureMode::Shared => 0,
            CaptureMode::Mutable => 1,
            CaptureMode::Owned => 2,
        });
    }
    fn decode(decoder: &mut Decoder) -> Option<Self> {
        Some(match decoder.u8()? {
            0 => CaptureMode::Shared,
            1 => CaptureMode::Mutable,
            2 => CaptureMode::Owned,
            _ => return None,
        })
    }
}

impl Codec for LocalKind {
    fn encode(&self, encoder: &mut Encoder) {
        encoder.u8(match self {
            LocalKind::Return => 0,
            LocalKind::Argument => 1,
            LocalKind::Capture => 2,
            LocalKind::User => 3,
            LocalKind::Temporary => 4,
            LocalKind::DropFlag => 5,
        });
    }
    fn decode(decoder: &mut Decoder) -> Option<Self> {
        Some(match decoder.u8()? {
            0 => LocalKind::Return,
            1 => LocalKind::Argument,
            2 => LocalKind::Capture,
            3 => LocalKind::User,
            4 => LocalKind::Temporary,
            5 => LocalKind::DropFlag,
            _ => return None,
        })
    }
}

impl Codec for LocalDeclaration {
    fn encode(&self, encoder: &mut Encoder) {
        let LocalDeclaration {
            ty,
            kind,
            mutable,
            name,
            binding,
            parameter_kind,
            span,
        } = self;
        ty.encode(encoder);
        kind.encode(encoder);
        mutable.encode(encoder);
        name.encode(encoder);
        binding.encode(encoder);
        parameter_kind.encode(encoder);
        span.encode(encoder);
    }
    fn decode(decoder: &mut Decoder) -> Option<Self> {
        Some(LocalDeclaration {
            ty: decoder.decode()?,
            kind: decoder.decode()?,
            mutable: decoder.decode()?,
            name: decoder.decode()?,
            binding: decoder.decode()?,
            parameter_kind: decoder.decode()?,
            span: decoder.decode()?,
        })
    }
}

impl Codec for Projection {
    fn encode(&self, encoder: &mut Encoder) {
        match self {
            Projection::Deref => encoder.u8(0),
            Projection::Field(index) => {
                encoder.u8(1);
                index.encode(encoder);
            }
            Projection::Payload => encoder.u8(2),
            Projection::VariantField { variant, field } => {
                encoder.u8(3);
                variant.encode(encoder);
                field.encode(encoder);
            }
            Projection::Index(local) => {
                encoder.u8(4);
                local.encode(encoder);
            }
        }
    }
    fn decode(decoder: &mut Decoder) -> Option<Self> {
        Some(match decoder.u8()? {
            0 => Projection::Deref,
            1 => Projection::Field(decoder.decode()?),
            2 => Projection::Payload,
            3 => Projection::VariantField {
                variant: decoder.decode()?,
                field: decoder.decode()?,
            },
            4 => Projection::Index(decoder.decode()?),
            _ => return None,
        })
    }
}

impl Codec for Place {
    fn encode(&self, encoder: &mut Encoder) {
        let Place { local, projections } = self;
        local.encode(encoder);
        projections.encode(encoder);
    }
    fn decode(decoder: &mut Decoder) -> Option<Self> {
        Some(Place {
            local: decoder.decode()?,
            projections: decoder.decode()?,
        })
    }
}

impl Codec for Constant {
    fn encode(&self, encoder: &mut Encoder) {
        match self {
            Constant::Int(value) => {
                encoder.u8(0);
                encoder.i64(*value);
            }
            Constant::Float(value) => {
                encoder.u8(1);
                encoder.f64(*value);
            }
            Constant::Bool(value) => {
                encoder.u8(2);
                value.encode(encoder);
            }
            Constant::Char(value) => {
                encoder.u8(3);
                encoder.u32(u32::from(*value));
            }
            Constant::String(value) => {
                encoder.u8(4);
                value.encode(encoder);
            }
            Constant::Unit => encoder.u8(5),
            Constant::Null(ty) => {
                encoder.u8(6);
                ty.encode(encoder);
            }
            Constant::TopLevelValue(path) => {
                encoder.u8(7);
                path.encode(encoder);
            }
        }
    }
    fn decode(decoder: &mut Decoder) -> Option<Self> {
        Some(match decoder.u8()? {
            0 => Constant::Int(decoder.i64()?),
            1 => Constant::Float(decoder.f64()?),
            2 => Constant::Bool(decoder.decode()?),
            3 => Constant::Char(char::from_u32(decoder.u32()?)?),
            4 => Constant::String(decoder.decode()?),
            5 => Constant::Unit,
            6 => Constant::Null(decoder.decode()?),
            7 => Constant::TopLevelValue(decoder.decode()?),
            _ => return None,
        })
    }
}

impl Codec for Operand {
    fn encode(&self, encoder: &mut Encoder) {
        match self {
            Operand::Copy(place, span) => {
                encoder.u8(0);
                place.encode(encoder);
                span.encode(encoder);
            }
            Operand::Move(place, span) => {
                encoder.u8(1);
                place.encode(encoder);
                span.encode(encoder);
            }
            Operand::Constant(constant) => {
                encoder.u8(2);
                constant.encode(encoder);
            }
        }
    }
    fn decode(decoder: &mut Decoder) -> Option<Self> {
        Some(match decoder.u8()? {
            0 => Operand::Copy(decoder.decode()?, decoder.decode()?),
            1 => Operand::Move(decoder.decode()?, decoder.decode()?),
            2 => Operand::Constant(decoder.decode()?),
            _ => return None,
        })
    }
}

impl Codec for BorrowOrigin {
    fn encode(&self, encoder: &mut Encoder) {
        encoder.u8(match self {
            BorrowOrigin::Written => 0,
            BorrowOrigin::Argument => 1,
            BorrowOrigin::Receiver => 2,
            BorrowOrigin::Conversion => 3,
            BorrowOrigin::Operator => 4,
            BorrowOrigin::Default => 5,
            BorrowOrigin::Pattern => 6,
            BorrowOrigin::Guard => 7,
            BorrowOrigin::Iteration => 8,
        });
    }
    fn decode(decoder: &mut Decoder) -> Option<Self> {
        Some(match decoder.u8()? {
            0 => BorrowOrigin::Written,
            1 => BorrowOrigin::Argument,
            2 => BorrowOrigin::Receiver,
            3 => BorrowOrigin::Conversion,
            4 => BorrowOrigin::Operator,
            5 => BorrowOrigin::Default,
            6 => BorrowOrigin::Pattern,
            7 => BorrowOrigin::Guard,
            8 => BorrowOrigin::Iteration,
            _ => return None,
        })
    }
}

impl Codec for AggregateKind {
    fn encode(&self, encoder: &mut Encoder) {
        match self {
            AggregateKind::Struct { id, arguments } => {
                encoder.u8(0);
                id.encode(encoder);
                arguments.encode(encoder);
            }
            AggregateKind::Variant {
                enum_id,
                index,
                arguments,
            } => {
                encoder.u8(1);
                enum_id.encode(encoder);
                index.encode(encoder);
                arguments.encode(encoder);
            }
        }
    }
    fn decode(decoder: &mut Decoder) -> Option<Self> {
        Some(match decoder.u8()? {
            0 => AggregateKind::Struct {
                id: decoder.decode()?,
                arguments: decoder.decode()?,
            },
            1 => AggregateKind::Variant {
                enum_id: decoder.decode()?,
                index: decoder.decode()?,
                arguments: decoder.decode()?,
            },
            _ => return None,
        })
    }
}

impl Codec for Capture {
    fn encode(&self, encoder: &mut Encoder) {
        match self {
            Capture::Borrowed(place) => {
                encoder.u8(0);
                place.encode(encoder);
            }
            Capture::Owned(operand) => {
                encoder.u8(1);
                operand.encode(encoder);
            }
        }
    }
    fn decode(decoder: &mut Decoder) -> Option<Self> {
        Some(match decoder.u8()? {
            0 => Capture::Borrowed(decoder.decode()?),
            1 => Capture::Owned(decoder.decode()?),
            _ => return None,
        })
    }
}

impl Codec for TemplatePart {
    fn encode(&self, encoder: &mut Encoder) {
        match self {
            TemplatePart::Text(text) => {
                encoder.u8(0);
                text.encode(encoder);
            }
            TemplatePart::Value(operand) => {
                encoder.u8(1);
                operand.encode(encoder);
            }
        }
    }
    fn decode(decoder: &mut Decoder) -> Option<Self> {
        Some(match decoder.u8()? {
            0 => TemplatePart::Text(decoder.decode()?),
            1 => TemplatePart::Value(decoder.decode()?),
            _ => return None,
        })
    }
}

impl Codec for Rvalue {
    fn encode(&self, encoder: &mut Encoder) {
        match self {
            Rvalue::Use(operand) => {
                encoder.u8(0);
                operand.encode(encoder);
            }
            Rvalue::Borrow {
                mutable,
                origin,
                place,
            } => {
                encoder.u8(1);
                mutable.encode(encoder);
                origin.encode(encoder);
                place.encode(encoder);
            }
            Rvalue::Binary(operator, left, right) => {
                encoder.u8(2);
                operator.encode(encoder);
                left.encode(encoder);
                right.encode(encoder);
            }
            Rvalue::Unary(operator, operand) => {
                encoder.u8(3);
                operator.encode(encoder);
                operand.encode(encoder);
            }
            Rvalue::Discriminant(place) => {
                encoder.u8(4);
                place.encode(encoder);
            }
            Rvalue::Aggregate(kind, operands) => {
                encoder.u8(5);
                kind.encode(encoder);
                operands.encode(encoder);
            }
            Rvalue::WrapNullable(operand) => {
                encoder.u8(6);
                operand.encode(encoder);
            }
            Rvalue::Subtype(operand) => {
                encoder.u8(7);
                operand.encode(encoder);
            }
            Rvalue::Closure { body, captures } => {
                encoder.u8(8);
                body.encode(encoder);
                captures.encode(encoder);
            }
            Rvalue::Template(parts) => {
                encoder.u8(9);
                parts.encode(encoder);
            }
            Rvalue::Bits(operator, left, right) => {
                encoder.u8(10);
                operator.encode(encoder);
                left.encode(encoder);
                right.encode(encoder);
            }
        }
    }
    fn decode(decoder: &mut Decoder) -> Option<Self> {
        Some(match decoder.u8()? {
            0 => Rvalue::Use(decoder.decode()?),
            1 => Rvalue::Borrow {
                mutable: decoder.decode()?,
                origin: decoder.decode()?,
                place: decoder.decode()?,
            },
            2 => Rvalue::Binary(decoder.decode()?, decoder.decode()?, decoder.decode()?),
            3 => Rvalue::Unary(decoder.decode()?, decoder.decode()?),
            4 => Rvalue::Discriminant(decoder.decode()?),
            5 => Rvalue::Aggregate(decoder.decode()?, decoder.decode()?),
            6 => Rvalue::WrapNullable(decoder.decode()?),
            7 => Rvalue::Subtype(decoder.decode()?),
            8 => Rvalue::Closure {
                body: decoder.decode()?,
                captures: decoder.decode()?,
            },
            9 => Rvalue::Template(decoder.decode()?),
            10 => Rvalue::Bits(decoder.decode()?, decoder.decode()?, decoder.decode()?),
            _ => return None,
        })
    }
}

impl Codec for crate::mir::BitOperator {
    fn encode(&self, encoder: &mut Encoder) {
        use crate::mir::BitOperator;
        encoder.u8(match self {
            BitOperator::And => 0,
            BitOperator::Or => 1,
            BitOperator::ShiftLeft => 2,
            BitOperator::ShiftRight => 3,
        });
    }
    fn decode(decoder: &mut Decoder) -> Option<Self> {
        use crate::mir::BitOperator;
        Some(match decoder.u8()? {
            0 => BitOperator::And,
            1 => BitOperator::Or,
            2 => BitOperator::ShiftLeft,
            3 => BitOperator::ShiftRight,
            _ => return None,
        })
    }
}

impl Codec for crate::mir::GroupKey {
    fn encode(&self, encoder: &mut Encoder) {
        self.hash.encode(encoder);
        self.label.encode(encoder);
    }
    fn decode(decoder: &mut Decoder) -> Option<Self> {
        Some(crate::mir::GroupKey {
            hash: decoder.decode()?,
            label: decoder.decode()?,
        })
    }
}

impl Codec for crate::mir::ComposerOperation {
    fn encode(&self, encoder: &mut Encoder) {
        use crate::mir::ComposerOperation as Operation;
        match self {
            Operation::StartRestartGroup { key } => {
                encoder.u8(0);
                key.encode(encoder);
            }
            Operation::StartGroup { key } => {
                encoder.u8(1);
                key.encode(encoder);
            }
            Operation::StartMovableGroup { key } => {
                encoder.u8(2);
                key.encode(encoder);
            }
            Operation::EndGroup => encoder.u8(3),
            Operation::EndRestartGroup => encoder.u8(4),
            Operation::Skipping => encoder.u8(5),
            Operation::SkipToGroupEnd => encoder.u8(6),
            Operation::Changed(ty) => {
                encoder.u8(7);
                ty.encode(encoder);
            }
            Operation::SetSlot(ty) => {
                encoder.u8(8);
                ty.encode(encoder);
            }
            Operation::KeptParameter { index, ty } => {
                encoder.u8(9);
                index.encode(encoder);
                ty.encode(encoder);
            }
            Operation::Keepable => encoder.u8(10),
            Operation::Duplicate => encoder.u8(11),
            Operation::CurrentScope => encoder.u8(12),
            Operation::StartComposition => encoder.u8(13),
            Operation::SetContent => encoder.u8(14),
            Operation::Recompose => encoder.u8(15),
            Operation::Dump => encoder.u8(16),
            Operation::Invalidate => encoder.u8(17),
            Operation::Keep(ty) => {
                encoder.u8(18);
                ty.encode(encoder);
            }
            Operation::BeginRemember => encoder.u8(19),
            Operation::EndRemember => encoder.u8(20),
            Operation::SideEffect => encoder.u8(21),
            Operation::StartNodeGroup { key } => {
                encoder.u8(22);
                key.encode(encoder);
            }
        }
    }
    fn decode(decoder: &mut Decoder) -> Option<Self> {
        use crate::mir::ComposerOperation as Operation;
        Some(match decoder.u8()? {
            0 => Operation::StartRestartGroup {
                key: decoder.decode()?,
            },
            1 => Operation::StartGroup {
                key: decoder.decode()?,
            },
            2 => Operation::StartMovableGroup {
                key: decoder.decode()?,
            },
            3 => Operation::EndGroup,
            4 => Operation::EndRestartGroup,
            5 => Operation::Skipping,
            6 => Operation::SkipToGroupEnd,
            7 => Operation::Changed(decoder.decode()?),
            8 => Operation::SetSlot(decoder.decode()?),
            9 => Operation::KeptParameter {
                index: decoder.decode()?,
                ty: decoder.decode()?,
            },
            10 => Operation::Keepable,
            11 => Operation::Duplicate,
            12 => Operation::CurrentScope,
            13 => Operation::StartComposition,
            14 => Operation::SetContent,
            15 => Operation::Recompose,
            16 => Operation::Dump,
            17 => Operation::Invalidate,
            18 => Operation::Keep(decoder.decode()?),
            19 => Operation::BeginRemember,
            20 => Operation::EndRemember,
            21 => Operation::SideEffect,
            22 => Operation::StartNodeGroup {
                key: decoder.decode()?,
            },
            _ => return None,
        })
    }
}

impl Codec for StatementKind {
    fn encode(&self, encoder: &mut Encoder) {
        match self {
            StatementKind::Assign(place, rvalue) => {
                encoder.u8(0);
                place.encode(encoder);
                rvalue.encode(encoder);
            }
            StatementKind::StorageLive(local) => {
                encoder.u8(1);
                local.encode(encoder);
            }
            StatementKind::StorageDead(local) => {
                encoder.u8(2);
                local.encode(encoder);
            }
            StatementKind::Read(place) => {
                encoder.u8(3);
                place.encode(encoder);
            }
            StatementKind::Destructure {
                source,
                variant,
                parts,
            } => {
                encoder.u8(4);
                source.encode(encoder);
                variant.encode(encoder);
                parts.encode(encoder);
            }
            StatementKind::GroupStart(group) => {
                encoder.u8(5);
                group.encode(encoder);
            }
            StatementKind::GroupEnd(group) => {
                encoder.u8(6);
                group.encode(encoder);
            }
        }
    }
    fn decode(decoder: &mut Decoder) -> Option<Self> {
        Some(match decoder.u8()? {
            0 => StatementKind::Assign(decoder.decode()?, decoder.decode()?),
            1 => StatementKind::StorageLive(decoder.decode()?),
            2 => StatementKind::StorageDead(decoder.decode()?),
            3 => StatementKind::Read(decoder.decode()?),
            4 => StatementKind::Destructure {
                source: decoder.decode()?,
                variant: decoder.decode()?,
                parts: decoder.decode()?,
            },
            5 => StatementKind::GroupStart(decoder.decode()?),
            6 => StatementKind::GroupEnd(decoder.decode()?),
            _ => return None,
        })
    }
}

impl Codec for crate::mir::GroupStep {
    fn encode(&self, encoder: &mut Encoder) {
        use crate::mir::GroupStep;
        let (tag, ordinal, branch) = match *self {
            GroupStep::If { ordinal, branch } => (0, ordinal, branch),
            GroupStep::When { ordinal, branch } => (1, ordinal, branch),
            GroupStep::And { ordinal } => (2, ordinal, 0),
            GroupStep::Or { ordinal } => (3, ordinal, 0),
            GroupStep::Elvis { ordinal } => (4, ordinal, 0),
            GroupStep::SafeCall { ordinal } => (5, ordinal, 0),
            GroupStep::Loop { ordinal } => (6, ordinal, 0),
            GroupStep::WhenTests { ordinal } => (7, ordinal, 0),
        };
        encoder.u8(tag);
        encoder.u32(ordinal);
        encoder.u32(branch);
    }
    fn decode(decoder: &mut Decoder) -> Option<Self> {
        use crate::mir::GroupStep;
        let tag = decoder.u8()?;
        let ordinal = decoder.u32()?;
        let branch = decoder.u32()?;
        Some(match tag {
            0 => GroupStep::If { ordinal, branch },
            1 => GroupStep::When { ordinal, branch },
            2 => GroupStep::And { ordinal },
            3 => GroupStep::Or { ordinal },
            4 => GroupStep::Elvis { ordinal },
            5 => GroupStep::SafeCall { ordinal },
            6 => GroupStep::Loop { ordinal },
            7 => GroupStep::WhenTests { ordinal },
            _ => return None,
        })
    }
}

impl Codec for crate::mir::GroupRegion {
    fn encode(&self, encoder: &mut Encoder) {
        let crate::mir::GroupRegion {
            parent,
            step,
            span,
            continued,
        } = self;
        parent.encode(encoder);
        step.encode(encoder);
        span.encode(encoder);
        continued.encode(encoder);
    }
    fn decode(decoder: &mut Decoder) -> Option<Self> {
        Some(crate::mir::GroupRegion {
            parent: decoder.decode()?,
            step: decoder.decode()?,
            span: decoder.decode()?,
            continued: decoder.decode()?,
        })
    }
}

impl Codec for crate::mir::ClosureStep {
    fn encode(&self, encoder: &mut Encoder) {
        match self {
            crate::mir::ClosureStep::Argument {
                callee,
                ordinal,
                parameter,
            } => {
                encoder.u8(0);
                callee.encode(encoder);
                ordinal.encode(encoder);
                parameter.encode(encoder);
            }
            crate::mir::ClosureStep::Value { ordinal } => {
                encoder.u8(1);
                ordinal.encode(encoder);
            }
        }
    }
    fn decode(decoder: &mut Decoder) -> Option<Self> {
        Some(match decoder.u8()? {
            0 => crate::mir::ClosureStep::Argument {
                callee: decoder.decode()?,
                ordinal: decoder.decode()?,
                parameter: decoder.decode()?,
            },
            1 => crate::mir::ClosureStep::Value {
                ordinal: decoder.decode()?,
            },
            _ => return None,
        })
    }
}

impl Codec for crate::mir::ClosureOrigin {
    fn encode(&self, encoder: &mut Encoder) {
        let crate::mir::ClosureOrigin { region, step } = self;
        region.encode(encoder);
        step.encode(encoder);
    }
    fn decode(decoder: &mut Decoder) -> Option<Self> {
        Some(crate::mir::ClosureOrigin {
            region: decoder.decode()?,
            step: decoder.decode()?,
        })
    }
}

impl Codec for Statement {
    fn encode(&self, encoder: &mut Encoder) {
        let Statement { kind, span } = self;
        kind.encode(encoder);
        span.encode(encoder);
    }
    fn decode(decoder: &mut Decoder) -> Option<Self> {
        Some(Statement {
            kind: decoder.decode()?,
            span: decoder.decode()?,
        })
    }
}

impl Codec for Callee {
    fn encode(&self, encoder: &mut Encoder) {
        match self {
            Callee::Function(path) => {
                encoder.u8(0);
                path.encode(encoder);
            }
            Callee::Builtin(builtin) => {
                encoder.u8(1);
                builtin.encode(encoder);
            }
            Callee::Value(place, span) => {
                encoder.u8(2);
                place.encode(encoder);
                span.encode(encoder);
            }
            Callee::Default {
                function,
                parameter,
                name,
            } => {
                encoder.u8(3);
                function.encode(encoder);
                parameter.encode(encoder);
                name.encode(encoder);
            }
            Callee::External(path) => {
                encoder.u8(4);
                path.encode(encoder);
            }
            Callee::Composer(operation) => {
                encoder.u8(5);
                operation.encode(encoder);
            }
            Callee::State(operation) => {
                encoder.u8(6);
                operation.encode(encoder);
            }
        }
    }
    fn decode(decoder: &mut Decoder) -> Option<Self> {
        Some(match decoder.u8()? {
            0 => Callee::Function(decoder.decode()?),
            1 => Callee::Builtin(decoder.decode()?),
            2 => Callee::Value(decoder.decode()?, decoder.decode()?),
            3 => Callee::Default {
                function: decoder.decode()?,
                parameter: decoder.decode()?,
                name: decoder.decode()?,
            },
            4 => Callee::External(decoder.decode()?),
            5 => Callee::Composer(decoder.decode()?),
            6 => Callee::State(decoder.decode()?),
            _ => return None,
        })
    }
}

impl Codec for TerminatorKind {
    fn encode(&self, encoder: &mut Encoder) {
        match self {
            TerminatorKind::Goto(target) => {
                encoder.u8(0);
                target.encode(encoder);
            }
            TerminatorKind::SwitchInt {
                value,
                targets,
                otherwise,
            } => {
                encoder.u8(1);
                value.encode(encoder);
                targets.encode(encoder);
                otherwise.encode(encoder);
            }
            TerminatorKind::Call {
                callee,
                arguments,
                parameters,
                parameter_kinds,
                type_arguments,
                destination,
                target,
                is_compose,
            } => {
                encoder.u8(2);
                callee.encode(encoder);
                arguments.encode(encoder);
                parameters.encode(encoder);
                parameter_kinds.encode(encoder);
                type_arguments.encode(encoder);
                destination.encode(encoder);
                target.encode(encoder);
                is_compose.encode(encoder);
            }
            TerminatorKind::Drop { place, target } => {
                encoder.u8(3);
                place.encode(encoder);
                target.encode(encoder);
            }
            TerminatorKind::Return => encoder.u8(4),
            TerminatorKind::Unreachable => encoder.u8(5),
            TerminatorKind::Remembered {
                composer,
                slot_type,
                destination,
                found,
                missing,
            } => {
                encoder.u8(6);
                composer.encode(encoder);
                slot_type.encode(encoder);
                destination.encode(encoder);
                found.encode(encoder);
                missing.encode(encoder);
            }
        }
    }
    fn decode(decoder: &mut Decoder) -> Option<Self> {
        Some(match decoder.u8()? {
            0 => TerminatorKind::Goto(decoder.decode()?),
            1 => TerminatorKind::SwitchInt {
                value: decoder.decode()?,
                targets: decoder.decode()?,
                otherwise: decoder.decode()?,
            },
            2 => TerminatorKind::Call {
                callee: decoder.decode()?,
                arguments: decoder.decode()?,
                parameters: decoder.decode()?,
                parameter_kinds: decoder.decode()?,
                type_arguments: decoder.decode()?,
                destination: decoder.decode()?,
                target: decoder.decode()?,
                is_compose: decoder.decode()?,
            },
            3 => TerminatorKind::Drop {
                place: decoder.decode()?,
                target: decoder.decode()?,
            },
            4 => TerminatorKind::Return,
            5 => TerminatorKind::Unreachable,
            6 => TerminatorKind::Remembered {
                composer: decoder.decode()?,
                slot_type: decoder.decode()?,
                destination: decoder.decode()?,
                found: decoder.decode()?,
                missing: decoder.decode()?,
            },
            _ => return None,
        })
    }
}

impl Codec for Terminator {
    fn encode(&self, encoder: &mut Encoder) {
        let Terminator { kind, span } = self;
        kind.encode(encoder);
        span.encode(encoder);
    }
    fn decode(decoder: &mut Decoder) -> Option<Self> {
        Some(Terminator {
            kind: decoder.decode()?,
            span: decoder.decode()?,
        })
    }
}

impl Codec for BasicBlock {
    fn encode(&self, encoder: &mut Encoder) {
        let BasicBlock {
            statements,
            terminator,
        } = self;
        statements.encode(encoder);
        terminator.encode(encoder);
    }
    fn decode(decoder: &mut Decoder) -> Option<Self> {
        Some(BasicBlock {
            statements: decoder.decode()?,
            terminator: decoder.decode()?,
        })
    }
}

impl Codec for Body {
    fn encode(&self, encoder: &mut Encoder) {
        let Body {
            owner,
            is_compose,
            type_parameters,
            type_arguments,
            locals,
            argument_count,
            capture_count,
            capture_modes,
            ownership_reason,
            capture_widenings,
            blocks,
            groups,
            origin,
            body_key,
            span,
        } = self;
        owner.encode(encoder);
        is_compose.encode(encoder);
        type_parameters.encode(encoder);
        type_arguments.encode(encoder);
        locals.encode(encoder);
        argument_count.encode(encoder);
        capture_count.encode(encoder);
        capture_modes.encode(encoder);
        ownership_reason.encode(encoder);
        capture_widenings.encode(encoder);
        blocks.encode(encoder);
        groups.encode(encoder);
        origin.encode(encoder);
        body_key.encode(encoder);
        span.encode(encoder);
    }
    fn decode(decoder: &mut Decoder) -> Option<Self> {
        Some(Body {
            owner: decoder.decode()?,
            is_compose: decoder.decode()?,
            type_parameters: decoder.decode()?,
            type_arguments: decoder.decode()?,
            locals: decoder.decode()?,
            argument_count: decoder.decode()?,
            capture_count: decoder.decode()?,
            capture_modes: decoder.decode()?,
            ownership_reason: decoder.decode()?,
            capture_widenings: decoder.decode()?,
            blocks: decoder.decode()?,
            groups: decoder.decode()?,
            origin: decoder.decode()?,
            body_key: decoder.decode()?,
            span: decoder.decode()?,
        })
    }
}

impl Codec for GenericRecord {
    fn encode(&self, encoder: &mut Encoder) {
        let GenericRecord {
            path,
            function,
            defaults,
            closures,
            types,
            function_id,
            default_ids,
            closure_ids,
        } = self;
        path.encode(encoder);
        function.encode(encoder);
        defaults.encode(encoder);
        closures.encode(encoder);
        types.encode(encoder);
        function_id.encode(encoder);
        default_ids.encode(encoder);
        closure_ids.encode(encoder);
    }
    fn decode(decoder: &mut Decoder) -> Option<Self> {
        let record = GenericRecord {
            path: decoder.decode()?,
            function: decoder.decode()?,
            defaults: decoder.decode()?,
            closures: decoder.decode()?,
            types: decoder.decode()?,
            function_id: decoder.decode()?,
            default_ids: decoder.decode()?,
            closure_ids: decoder.decode()?,
        };
        (record.defaults.len() == record.default_ids.len()
            && record.closures.len() == record.closure_ids.len())
        .then_some(record)
    }
}

impl GenericRecord {
    /// The record's canonical encoding, whose hash is its Monomorphization Hash (D2 and D4 of
    /// `instance-cache`).
    pub fn encode(&self) -> Vec<u8> {
        encode(self)
    }

    /// The record `bytes` encode, or `None` when they are malformed.
    pub fn decode(bytes: &[u8]) -> Option<GenericRecord> {
        decode(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lengths_round_trip_in_leb128() {
        for value in [
            0usize,
            1,
            127,
            128,
            300,
            16_384,
            u32::MAX as usize,
            usize::MAX,
        ] {
            let bytes = encode(&value);
            assert_eq!(decode::<usize>(&bytes), Some(value), "{}", value);
        }
        assert_eq!(encode(&127usize), [127]);
        assert_eq!(encode(&128usize), [0x80, 1]);
        // Eleven continuation bytes overflow a u64.
        assert_eq!(decode::<usize>(&[0xFF; 11]), None);
        assert_eq!(decode::<usize>(&[0x80]), None);
    }

    #[test]
    fn a_corrupt_sequence_length_fails_at_once() {
        let mut bytes = Vec::new();
        let mut encoder = Encoder::new();
        encoder.length(1 << 40);
        bytes.extend(encoder.finish());
        assert_eq!(decode::<Vec<String>>(&bytes), None);
    }

    #[test]
    fn trailing_bytes_are_rejected() {
        let mut bytes = encode(&"abc".to_string());
        assert_eq!(decode::<String>(&bytes), Some("abc".to_string()));
        bytes.push(0);
        assert_eq!(decode::<String>(&bytes), None);
    }

    #[test]
    fn types_round_trip() {
        let parameter = TypeParameter {
            index: 1,
            name: "T".to_string(),
            bound: Some(TypeMarker::Copy),
        };
        let ty = Type::Fn {
            is_compose: true,
            receiver: true,
            params: vec![
                Type::Reference {
                    mutable: true,
                    inner: Box::new(Type::List(Box::new(Type::Parameter(parameter)))),
                },
                Type::Nullable(Box::new(Type::Struct {
                    id: NominalId::new("lib/box", "Box"),
                    arguments: vec![Type::Heap(Box::new(Type::String))],
                })),
            ],
            kinds: vec![ParameterKind::Plain, ParameterKind::Escaping],
            ret: Box::new(Type::Enum {
                id: NominalId::new("a", "E"),
                arguments: vec![],
            }),
        };
        assert_eq!(decode::<Type>(&encode(&ty)), Some(ty));
    }
}

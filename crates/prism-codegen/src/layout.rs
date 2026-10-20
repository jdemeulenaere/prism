//! Memory layouts of values (D5 to D8 of `codegen-aggregates`), and `prism dump-layout`'s text
//! (D18).
//!
//! A struct is stored inline, its fields placed by decreasing alignment (declaration order among
//! equals); an enum is the smallest unsigned tag numbering its variants, at offset 0, followed by
//! the current variant's payload, laid out like a struct's fields after the tag; a `T?` is a tag
//! byte (0 for `null`) followed by `T`, except that a nullable reference or `Heap<T>` is one
//! pointer whose `null` is 0. `Int`, `Double` and pointers take 8 bytes, `Char` 4, `Bool` 1,
//! `Unit` and fieldless structs none. A `String` is four pointer-sized words (D2 of
//! `runtime-strings-and-drops`), and a function value two: its closure's descriptor and its
//! environment (D2 of `closures`).
//!
//! A closure's environment (D8 and D9 of `escaping-closures`) is laid out like a struct whose
//! fields are its owned captures, by value, and one pointer per borrowed capture
//! ([`LayoutTable::environment_layout`]).
//!
//! Layouts depend only on the type table of the file, which holds the definition of every type
//! its bodies can hold a value of (D2 and D4), so a file lays out every value from its own
//! declarations and its direct imports' ABIs.

use crate::supported::Construct;
use cranelift_codegen::ir::{types, Type as MachineType};
use prism_mir::{Body, CaptureMode, MirFile};
use prism_typed_tree::{NominalId, Type, TypeDef, TypeTable};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write;
use std::rc::Rc;

/// How deep a layout may nest structs and enums by value. Infinitely sized types are rejected
/// by the type checker (`E4023`), so only a broken type table reaches the limit.
const MAXIMUM_DEPTH: usize = 256;

/// The size, alignment and shape of the values of a type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    /// In bytes, a multiple of `align`.
    pub size: u32,
    /// In bytes, a power of two.
    pub align: u32,
    pub shape: Shape,
}

/// A field of a struct or a payload of an enum variant or nullable: its type, with type
/// arguments substituted, and its offset from the start of the value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    /// The field's name, or empty for a payload.
    pub name: String,
    pub ty: Type,
    pub offset: u32,
}

/// A variant of an enum: its name and its payloads, in declaration order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VariantLayout {
    pub name: String,
    pub payloads: Vec<Member>,
}

/// What a value is made of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Shape {
    /// `Unit`: no bytes.
    Empty,
    /// `Nothing` and enums without variants: there is no value.
    Uninhabited,
    /// One machine value: `Int`, `Double`, `Bool`, `Char`, a reference or a `Heap<T>`.
    Scalar(MachineType),
    /// A nullable reference or `Heap<T>`: one pointer, 0 for `null`.
    NullPointer,
    /// A struct's fields, in declaration order, with their offsets.
    Struct { fields: Vec<Member> },
    /// An enum: its tag at offset 0, then each variant's payloads, in declaration order.
    Enum {
        tag: MachineType,
        variants: Vec<VariantLayout>,
    },
    /// A `T?` other than a nullable pointer: a tag byte at offset 0 (0 for `null`, 1 otherwise),
    /// then the payload.
    Nullable { payload: Member },
    /// A `String`: the runtime's `PrismString`, four pointer-sized words ([`StringField`]).
    String,
    /// A function value: two pointer-sized words ([`FunctionField`]).
    Function,
    /// A `List<T>`: the runtime's `PrismList`, three pointer-sized words ([`ListField`]).
    List,
}

/// A word of a function value (D2 of `closures`), in layout order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FunctionField {
    /// The address of the descriptor of the closure that built the value, never null.
    Descriptor = 0,
    /// The address of the closure's environment, or null when it captures nothing.
    Environment = 1,
}

impl FunctionField {
    /// The field's offset in a function value on a target whose pointers are `pointer`.
    pub fn offset(self, pointer: MachineType) -> u32 {
        self as u32 * pointer.bytes()
    }
}

/// A word of a `String` value (D2 of `runtime-strings-and-drops`), in layout order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StringField {
    /// The first byte of the UTF-8 text, never null.
    Pointer = 0,
    /// The text's length in bytes.
    ByteLength = 1,
    /// The number of characters (Unicode scalar values) of the text.
    CharacterCount = 2,
    /// The size of the owned buffer in bytes, or 0 when the value owns none.
    Capacity = 3,
}

impl StringField {
    /// The field's offset in a `String` on a target whose pointers are `pointer`.
    pub fn offset(self, pointer: MachineType) -> u32 {
        self as u32 * pointer.bytes()
    }
}

/// A word of a `List<T>` value (D13 of `collections`), in layout order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListField {
    /// The address of the first element, never null: the element type's alignment when the
    /// list owns no buffer.
    Pointer = 0,
    /// The number of elements.
    Size = 1,
    /// The number of elements the buffer holds: 0 when the list owns none, and the largest
    /// `Int` for zero-sized elements, which never need one.
    Capacity = 2,
}

impl ListField {
    /// The field's offset in a `List<T>` on a target whose pointers are `pointer`.
    pub fn offset(self, pointer: MachineType) -> u32 {
        self as u32 * pointer.bytes()
    }

    /// The field's name in `prism dump-layout`.
    pub fn name(self) -> &'static str {
        match self {
            ListField::Pointer => "pointer",
            ListField::Size => "size",
            ListField::Capacity => "capacity",
        }
    }
}

impl Layout {
    /// The machine type of a value held in one machine value, a scalar or a nullable pointer.
    pub fn machine_type(&self, pointer: MachineType) -> Option<MachineType> {
        match self.shape {
            Shape::Scalar(machine) => Some(machine),
            Shape::NullPointer => Some(pointer),
            _ => None,
        }
    }

    /// The size of storage holding a value: at least one byte, so every value has an address.
    pub fn storage_size(&self) -> u32 {
        self.size.max(1)
    }

    fn scalar(machine: MachineType) -> Self {
        Layout {
            size: machine.bytes(),
            align: machine.bytes(),
            shape: Shape::Scalar(machine),
        }
    }
}

/// The layouts of the types of one file, computed on demand and memoized.
pub struct LayoutTable<'t> {
    types: &'t TypeTable,
    pointer: MachineType,
    layouts: RefCell<HashMap<Type, Rc<Layout>>>,
    /// The structs and enums being laid out, which a layout must not contain by value.
    in_progress: RefCell<HashSet<Type>>,
}

impl<'t> LayoutTable<'t> {
    /// The layouts of the types of `types`, for a target whose pointers are `pointer`.
    pub fn new(types: &'t TypeTable, pointer: MachineType) -> Self {
        Self {
            types,
            pointer,
            layouts: RefCell::new(HashMap::new()),
            in_progress: RefCell::new(HashSet::new()),
        }
    }

    /// The type table the layouts come from.
    pub fn types(&self) -> &'t TypeTable {
        self.types
    }

    /// The layout of `ty`, or the construct it waits for: a type parameter (only generic bodies
    /// hold one) or an unknown type.
    pub fn layout(&self, ty: &Type) -> Result<Rc<Layout>, Construct> {
        if let Some(layout) = self.layouts.borrow().get(ty) {
            return Ok(layout.clone());
        }
        let layout = Rc::new(self.compute(ty)?);
        self.layouts.borrow_mut().insert(ty.clone(), layout.clone());
        Ok(layout)
    }

    fn compute(&self, ty: &Type) -> Result<Layout, Construct> {
        let pointer = self.pointer;
        Ok(match ty {
            Type::Int => Layout::scalar(types::I64),
            Type::Float => Layout::scalar(types::F64),
            Type::Bool => Layout::scalar(types::I8),
            Type::Char => Layout::scalar(types::I32),
            Type::Reference { .. } | Type::Heap(_) => Layout::scalar(pointer),
            Type::Unit => Layout {
                size: 0,
                align: 1,
                shape: Shape::Empty,
            },
            Type::Nothing => Layout {
                size: 0,
                align: 1,
                shape: Shape::Uninhabited,
            },
            Type::Nullable(inner) => match inner.as_ref() {
                Type::Reference { .. } | Type::Heap(_) => Layout {
                    size: pointer.bytes(),
                    align: pointer.bytes(),
                    shape: Shape::NullPointer,
                },
                inner => {
                    let payload = self.layout(inner)?;
                    let offset = round_up(1, payload.align);
                    let align = payload.align.max(1);
                    Layout {
                        size: round_up(offset + payload.size, align),
                        align,
                        shape: Shape::Nullable {
                            payload: Member {
                                name: String::new(),
                                ty: inner.clone(),
                                offset,
                            },
                        },
                    }
                }
            },
            Type::Struct { .. } | Type::Enum { .. } => self.nominal(ty)?,
            Type::String => Layout {
                size: 4 * pointer.bytes(),
                align: pointer.bytes(),
                shape: Shape::String,
            },
            Type::Fn { .. } => Layout {
                size: 2 * pointer.bytes(),
                align: pointer.bytes(),
                shape: Shape::Function,
            },
            Type::List(_) => Layout {
                size: 3 * pointer.bytes(),
                align: pointer.bytes(),
                shape: Shape::List,
            },
            Type::Parameter(parameter) => {
                return Err(Construct::TypeParameter(parameter.name.clone()))
            }
            Type::Opaque(name) => return Err(Construct::UnknownType(name.clone())),
            Type::Error => return Err(Construct::UnknownType("<error>".to_string())),
        })
    }

    /// The layout of the struct or enum `ty`, whose arguments are substituted into its
    /// definition.
    fn nominal(&self, ty: &Type) -> Result<Layout, Construct> {
        let unknown = || Construct::UnknownType(ty.to_string());
        if self.in_progress.borrow().len() > MAXIMUM_DEPTH
            || !self.in_progress.borrow_mut().insert(ty.clone())
        {
            return Err(unknown());
        }
        let result = match ty {
            // An intrinsic struct is one word, a runtime handle, laid out as a hidden `Int`
            // field so that copies and `==` compare the handle (D16 of `compose-transform`).
            Type::Struct { id, .. } if id.intrinsic().is_some() => {
                self.struct_layout(&[("handle".to_string(), false, Type::Int)])
            }
            Type::Struct { id, arguments } => match self.types.instantiated_fields(id, arguments) {
                Some(fields) => self.struct_layout(&fields),
                None => Err(unknown()),
            },
            Type::Enum { id, arguments } => match self.types.instantiated_variants(id, arguments) {
                Some(variants) => self.enum_layout(&variants),
                None => Err(unknown()),
            },
            _ => Err(unknown()),
        };
        self.in_progress.borrow_mut().remove(ty);
        result
    }

    /// The environment of the closure `closure`, a closure body of elaborated MIR: its owned
    /// captures by value and its borrowed ones as pointers, in capture order, placed like a
    /// struct's fields, and how the function value holds it (D8 and D9 of `escaping-closures`).
    pub fn environment_layout(&self, closure: &Body) -> Result<EnvironmentLayout, Construct> {
        let mut members = Vec::with_capacity(closure.capture_count);
        let mut owned = Vec::with_capacity(closure.capture_count);
        let mut needs_drop = false;
        for index in 0..closure.capture_count {
            let local = prism_mir::LocalId((closure.argument_count + 1 + index) as u32);
            let ty = closure.locals[local].ty.clone();
            // A weak capture is an owned `Copy` word (D10 of `owned-state`).
            let is_owned = matches!(
                closure.capture_modes.get(index),
                Some(CaptureMode::Owned | CaptureMode::Weak)
            );
            if is_owned {
                needs_drop |= crate::glue::needs_glue(crate::glue::GlueKind::Drop, &ty, self.types);
                members.push((String::new(), ty));
            } else {
                members.push((
                    String::new(),
                    Type::Reference {
                        mutable: false,
                        inner: Box::new(Type::Unit),
                    },
                ));
            }
            owned.push(is_owned);
        }
        let (placed, end, align) = self.place(&members, 0)?;
        let size = round_up(end, align);
        let word = self.pointer.bytes();
        let kind = if members.is_empty() {
            EnvironmentKind::None
        } else if !owned.contains(&true) {
            EnvironmentKind::Frame
        } else if size <= word && align <= word && !needs_drop {
            EnvironmentKind::Inline
        } else {
            EnvironmentKind::Allocated
        };
        let captures = placed
            .into_iter()
            .zip(owned)
            .map(|(member, owned)| EnvironmentField {
                offset: member.offset,
                owned,
                ty: member.ty,
            })
            .collect();
        Ok(EnvironmentLayout {
            kind,
            size,
            align,
            captures,
        })
    }

    fn struct_layout(&self, fields: &[(String, bool, Type)]) -> Result<Layout, Construct> {
        let members: Vec<(String, Type)> = fields
            .iter()
            .map(|(name, _, ty)| (name.clone(), ty.clone()))
            .collect();
        let (members, end, align) = self.place(&members, 0)?;
        Ok(Layout {
            size: round_up(end, align),
            align,
            shape: Shape::Struct { fields: members },
        })
    }

    fn enum_layout(&self, variants: &[(String, Vec<Type>)]) -> Result<Layout, Construct> {
        if variants.is_empty() {
            return Ok(Layout {
                size: 0,
                align: 1,
                shape: Shape::Uninhabited,
            });
        }
        let tag = tag_type(variants.len());
        let mut align = tag.bytes();
        let mut end = tag.bytes();
        let mut laid_out = Vec::new();
        for (name, payloads) in variants {
            let members: Vec<(String, Type)> = payloads
                .iter()
                .map(|ty| (String::new(), ty.clone()))
                .collect();
            let (payloads, variant_end, variant_align) = self.place(&members, tag.bytes())?;
            align = align.max(variant_align);
            end = end.max(variant_end);
            laid_out.push(VariantLayout {
                name: name.clone(),
                payloads,
            });
        }
        Ok(Layout {
            size: round_up(end, align),
            align,
            shape: Shape::Enum {
                tag,
                variants: laid_out,
            },
        })
    }

    /// Places `members` from offset `start` by decreasing alignment, members of equal alignment
    /// in order, each at the next multiple of its alignment (D5). Returns them in their own
    /// order with their offsets, the end of the last one, and the largest alignment (1 without
    /// members).
    fn place(
        &self,
        members: &[(String, Type)],
        start: u32,
    ) -> Result<(Vec<Member>, u32, u32), Construct> {
        let layouts: Vec<Rc<Layout>> = members
            .iter()
            .map(|(_, ty)| self.layout(ty))
            .collect::<Result<_, _>>()?;
        let mut order: Vec<usize> = (0..members.len()).collect();
        order.sort_by_key(|index| std::cmp::Reverse(layouts[*index].align));
        let mut offsets = vec![0; members.len()];
        let mut end = start;
        let mut align = 1;
        for index in order {
            let layout = &layouts[index];
            let offset = round_up(end, layout.align);
            offsets[index] = offset;
            end = offset + layout.size;
            align = align.max(layout.align);
        }
        let placed = members
            .iter()
            .zip(offsets)
            .map(|((name, ty), offset)| Member {
                name: name.clone(),
                ty: ty.clone(),
                offset,
            })
            .collect();
        Ok((placed, end, align))
    }
}

/// How a function value holds its closure's environment (D8 and D9 of `escaping-closures`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvironmentKind {
    /// No captures: the environment word is null.
    None,
    /// Only borrowed captures: pointers in a stack slot of the building function (D5 of
    /// `closures`).
    Frame,
    /// Owned captures fitting in one word, none needing a drop: the environment word holds
    /// the environment's bytes.
    Inline,
    /// One heap allocation of the environment's size, freed by its drop function.
    Allocated,
}

/// One capture's place in an environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvironmentField {
    pub offset: u32,
    /// Whether the capture is held by value; otherwise the field is the captured place's
    /// address.
    pub owned: bool,
    /// The capture's type when owned, a pointer type otherwise.
    pub ty: Type,
}

/// The layout of a closure's environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvironmentLayout {
    pub kind: EnvironmentKind,
    pub size: u32,
    pub align: u32,
    /// One field per capture, in capture order.
    pub captures: Vec<EnvironmentField>,
}

/// The smallest unsigned integer numbering `count` variants: 8, 16 or 32 bits (D6).
fn tag_type(count: usize) -> MachineType {
    if count <= 1 << 8 {
        types::I8
    } else if count <= 1 << 16 {
        types::I16
    } else {
        types::I32
    }
}

/// `value` rounded up to a multiple of `align`, a power of two.
fn round_up(value: u32, align: u32) -> u32 {
    let align = align.max(1);
    value.div_ceil(align) * align
}

/// The layouts of `file` as `prism dump-layout` prints them (D18): first each struct and enum
/// of `declared`, the file's own in declaration order, then every other struct, enum and
/// nullable type that a compiled body holds, in the order of their rendered names. Each entry
/// gives the size and alignment, then the offset of every field, tag and payload; a type with
/// no layout yet names the construct it waits for, and a generic declaration is laid out per
/// instance.
pub fn dump_layouts(file: &MirFile, declared: &[NominalId], pointer: MachineType) -> String {
    let types = &file.types;
    let layouts = LayoutTable::new(types, pointer);
    let mut text = String::new();
    for id in declared {
        let Some(definition) = types.definition(id) else {
            continue;
        };
        let (kind, ty) = match definition {
            TypeDef::Struct { .. } => ("struct", Type::plain_struct(id.clone())),
            TypeDef::Enum { .. } => ("enum", Type::plain_enum(id.clone())),
        };
        let parameters = definition.parameters();
        if parameters.is_empty() {
            write_entry(&mut text, &layouts, &ty);
        } else {
            let rendered: Vec<String> = parameters.iter().map(|p| p.name.clone()).collect();
            writeln!(
                text,
                "{} {}::{}<{}> (generic: laid out per instance)",
                kind,
                id.module,
                id.name,
                rendered.join(", ")
            )
            .unwrap();
        }
    }
    let declared: HashSet<&NominalId> = declared.iter().collect();
    let mut held: BTreeMap<String, Type> = BTreeMap::new();
    for (_, body) in file.bodies.iter() {
        if !crate::supported::is_compiled(body) {
            continue;
        }
        for ty in body_types(body) {
            collect_held(&ty, types, &mut held, 0);
        }
    }
    for ty in held.values() {
        let own = matches!(ty, Type::Struct { id, arguments } | Type::Enum { id, arguments }
            if arguments.is_empty() && declared.contains(id));
        if !own {
            write_entry(&mut text, &layouts, ty);
        }
    }
    text
}

fn body_types(body: &Body) -> impl Iterator<Item = Type> + '_ {
    body.locals.iter().map(|(_, local)| local.ty.clone())
}

/// Records in `held` every struct, enum, nullable and list type a value of `ty` holds: `ty`
/// itself, the referent of a reference, and the fields and payloads of structs, enums and
/// nullables, but not what a `Heap<T>` points at or the elements of a list, which live in their
/// buffer.
fn collect_held(ty: &Type, types: &TypeTable, held: &mut BTreeMap<String, Type>, depth: usize) {
    if depth > MAXIMUM_DEPTH {
        return;
    }
    let members: Vec<Type> = match ty {
        Type::Reference { inner, .. } => vec![inner.as_ref().clone()],
        Type::Nullable(inner) => vec![inner.as_ref().clone()],
        Type::Struct { id, arguments } => types
            .instantiated_fields(id, arguments)
            .unwrap_or_default()
            .into_iter()
            .map(|(_, _, ty)| ty)
            .collect(),
        Type::Enum { id, arguments } => types
            .instantiated_variants(id, arguments)
            .unwrap_or_default()
            .into_iter()
            .flat_map(|(_, payloads)| payloads)
            .collect(),
        Type::List(_) => Vec::new(),
        _ => return,
    };
    if matches!(
        ty,
        Type::Nullable(_) | Type::Struct { .. } | Type::Enum { .. } | Type::List(_)
    ) && held.insert(rendered_name(ty), ty.clone()).is_some()
    {
        return;
    }
    for member in members {
        collect_held(&member, types, held, depth + 1);
    }
}

/// The name an entry of the dump gives `ty`: a struct or enum qualified by its module, with its
/// type arguments as written; any other type as written.
fn rendered_name(ty: &Type) -> String {
    match ty {
        Type::Struct { id, arguments } | Type::Enum { id, arguments } => {
            let mut name = format!("{}::{}", id.module, id.name);
            if !arguments.is_empty() {
                let arguments: Vec<String> = arguments.iter().map(Type::to_string).collect();
                write!(name, "<{}>", arguments.join(", ")).unwrap();
            }
            name
        }
        other => other.to_string(),
    }
}

/// Writes the entry of `ty`: its header, then one line per field, variant or payload.
fn write_entry(text: &mut String, layouts: &LayoutTable, ty: &Type) {
    let kind = match ty {
        Type::Struct { .. } => "struct",
        Type::Enum { .. } => "enum",
        Type::List(_) => "list",
        _ => "nullable",
    };
    let marker = ty
        .nominal_id()
        .and_then(|id| layouts.types().marker(id))
        .map_or(String::new(), |marker| format!(" : {}", marker.as_str()));
    let name = format!("{} {}{}", kind, rendered_name(ty), marker);
    let layout = match layouts.layout(ty) {
        Ok(layout) => layout,
        Err(construct) => {
            writeln!(
                text,
                "{} (no layout yet: {})",
                name,
                construct.description()
            )
            .unwrap();
            return;
        }
    };
    let size = format!("size {}, align {}", layout.size, layout.align);
    match &layout.shape {
        Shape::Struct { fields } => {
            writeln!(text, "{} ({})", name, size).unwrap();
            for field in fields {
                writeln!(text, "  {}: {} at {}", field.name, field.ty, field.offset).unwrap();
            }
        }
        Shape::Enum { tag, variants } => {
            writeln!(text, "{} ({}, tag u{} at 0)", name, size, tag.bits()).unwrap();
            for variant in variants {
                if variant.payloads.is_empty() {
                    writeln!(text, "  {}", variant.name).unwrap();
                } else {
                    let payloads: Vec<String> = variant
                        .payloads
                        .iter()
                        .map(|payload| format!("{} at {}", payload.ty, payload.offset))
                        .collect();
                    writeln!(text, "  {}({})", variant.name, payloads.join(", ")).unwrap();
                }
            }
        }
        Shape::Nullable { payload } => {
            writeln!(text, "{} ({}, tag u8 at 0)", name, size).unwrap();
            writeln!(text, "  payload {} at {}", payload.ty, payload.offset).unwrap();
        }
        Shape::NullPointer => writeln!(text, "{} ({}, null pointer)", name, size).unwrap(),
        Shape::Uninhabited => writeln!(text, "{} ({}, uninhabited)", name, size).unwrap(),
        Shape::List => {
            writeln!(text, "{} ({})", name, size).unwrap();
            let pointer = layouts.pointer;
            let element = match ty {
                Type::List(element) => element.to_string(),
                other => other.to_string(),
            };
            for field in [ListField::Pointer, ListField::Size, ListField::Capacity] {
                let ty = match field {
                    ListField::Pointer => format!("&{}", element),
                    ListField::Size | ListField::Capacity => "Int".to_string(),
                };
                let offset = field.offset(pointer);
                writeln!(text, "  {}: {} at {}", field.name(), ty, offset).unwrap();
            }
        }
        Shape::Empty | Shape::Scalar(_) | Shape::String | Shape::Function => {
            writeln!(text, "{} ({})", name, size).unwrap()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::lower_source;

    const POINTER: MachineType = types::I64;

    /// The layout of `ty` in the type table of the MIR of `source`.
    fn layout_in(source: &str, ty: &Type) -> Layout {
        let file = lower_source(source);
        let layouts = LayoutTable::new(&file.types, POINTER);
        let layout = layouts.layout(ty).unwrap_or_else(|c| panic!("{:?}", c));
        (*layout).clone()
    }

    fn main_struct(name: &str) -> Type {
        Type::plain_struct(NominalId::new("main", name))
    }

    fn main_enum(name: &str) -> Type {
        Type::plain_enum(NominalId::new("main", name))
    }

    fn offsets(layout: &Layout) -> Vec<u32> {
        match &layout.shape {
            Shape::Struct { fields } => fields.iter().map(|f| f.offset).collect(),
            Shape::Nullable { payload } => vec![payload.offset],
            other => panic!("no offsets in {:?}", other),
        }
    }

    fn variant_offsets(layout: &Layout) -> Vec<Vec<u32>> {
        match &layout.shape {
            Shape::Enum { variants, .. } => variants
                .iter()
                .map(|v| v.payloads.iter().map(|p| p.offset).collect())
                .collect(),
            other => panic!("not an enum: {:?}", other),
        }
    }

    /// Each kind of environment of `environment_layout` (D8 and D9 of `escaping-closures`).
    #[test]
    fn environments_by_kind() {
        let file = crate::test_support::lower_source(
            "struct Handle : Copy { val id: Int }\n\
             fun run(f: () -> Int): Int = f()\n\
             fun store(f: escaping () -> Int): Int = f()\n\
             fun none(): Int = store { 1 }\n\
             fun frame(n: Int): Int { var count = n; return run { count += 1; count } }\n\
             fun inline(h: Handle): Int = store { h.id }\n\
             fun allocated(s: String, n: Int): Int = store { s.length + n }\n",
        );
        let table = LayoutTable::new(&file.types, types::I64);
        let kind_of = |name: &str| {
            let closure = file
                .bodies
                .iter()
                .find(|(_, body)| {
                    matches!(&body.owner, prism_mir::BodyOwner::Closure { parent, .. }
                        if matches!(&file.bodies[*parent].owner,
                            prism_mir::BodyOwner::Function(path) if path.name == name))
                })
                .map(|(_, body)| body)
                .unwrap_or_else(|| panic!("no closure in {}", name));
            table.environment_layout(closure).unwrap()
        };
        assert_eq!(kind_of("none").kind, EnvironmentKind::None);
        let frame = kind_of("frame");
        assert_eq!((frame.kind, frame.size), (EnvironmentKind::Frame, 8));
        let inline = kind_of("inline");
        assert_eq!((inline.kind, inline.size), (EnvironmentKind::Inline, 8));
        let allocated = kind_of("allocated");
        assert_eq!(allocated.kind, EnvironmentKind::Allocated);
        // The string (32 bytes) and the `Int`, by decreasing alignment, in capture order.
        assert_eq!((allocated.size, allocated.align), (40, 8));
        let offsets: Vec<u32> = allocated.captures.iter().map(|c| c.offset).collect();
        assert_eq!(offsets, [0, 32]);
        assert!(allocated.captures.iter().all(|c| c.owned));
    }

    /// D13 of `collections`: three words, whatever the element, laid out like any field.
    #[test]
    fn lists_are_three_words() {
        let list = Type::List(Box::new(Type::Char));
        let layout = layout_in("fun f() {}\n", &list);
        assert_eq!(
            (layout.size, layout.align, &layout.shape),
            (24, 8, &Shape::List)
        );
        assert_eq!(
            [ListField::Pointer, ListField::Size, ListField::Capacity].map(|f| f.offset(POINTER)),
            [0, 8, 16]
        );
        let model = layout_in(
            "struct Model { val flag: Bool, val todos: List<Int> }\n",
            &main_struct("Model"),
        );
        assert_eq!((model.size, offsets(&model)), (32, vec![24, 0]));
    }

    #[test]
    fn structs_are_laid_out_inline_by_decreasing_alignment() {
        let point = layout_in(
            "struct Point { val x: Double, val y: Double }\n",
            &main_struct("Point"),
        );
        assert_eq!(
            (point.size, point.align, offsets(&point)),
            (16, 8, vec![0, 8])
        );
        let s = layout_in(
            "struct S { val a: Bool, val n: Int, val b: Bool }\n",
            &main_struct("S"),
        );
        assert_eq!((s.size, s.align, offsets(&s)), (16, 8, vec![8, 0, 9]));
        let empty = layout_in("struct E {}\n", &main_struct("E"));
        assert_eq!((empty.size, empty.align, empty.storage_size()), (0, 1, 1));
        let nested = layout_in(
            "struct C { val c: Char, val b: Bool }\nstruct N { val b: Bool, val c: C, val n: Int }\n",
            &main_struct("N"),
        );
        // `n` (align 8) at 0, `c` (size 5 rounded to 8, align 4) at 8, `b` at 16.
        assert_eq!((nested.size, offsets(&nested)), (24, vec![16, 8, 0]));
    }

    #[test]
    fn enums_are_a_tag_and_payloads() {
        let color = layout_in("enum Color { Red, Green, Blue }\n", &main_enum("Color"));
        assert_eq!((color.size, color.align), (1, 1));
        assert!(matches!(color.shape, Shape::Enum { tag, .. } if tag == types::I8));
        let expr = layout_in(
            "enum Expr { Literal(Int), Add(Heap<Expr>, Heap<Expr>) }\n",
            &main_enum("Expr"),
        );
        assert_eq!((expr.size, expr.align), (24, 8));
        assert_eq!(variant_offsets(&expr), [vec![8], vec![8, 16]]);
        let flag = layout_in("enum Flag { On(Bool), Off }\n", &main_enum("Flag"));
        assert_eq!((flag.size, flag.align), (2, 1));
        assert_eq!(variant_offsets(&flag), [vec![1], vec![]]);
        let never = layout_in("enum Never {}\n", &main_enum("Never"));
        assert_eq!((never.size, never.shape), (0, Shape::Uninhabited));
    }

    #[test]
    fn an_enum_of_300_variants_has_a_16_bit_tag() {
        let variants: Vec<String> = (0..300).map(|i| format!("V{}", i)).collect();
        let source = format!("enum Many {{ {}, Last(Bool) }}\n", variants.join(", "));
        let many = layout_in(&source, &main_enum("Many"));
        assert!(matches!(many.shape, Shape::Enum { tag, .. } if tag == types::I16));
        assert_eq!((many.size, many.align), (4, 2));
        assert_eq!(variant_offsets(&many)[300], [2]);
    }

    #[test]
    fn nullables_have_a_tag_or_are_a_null_pointer() {
        let source = "struct Point { val x: Double, val y: Double }\n";
        let nullable = |ty: Type| layout_in(source, &ty.nullable());
        let int = nullable(Type::Int);
        assert_eq!((int.size, int.align, offsets(&int)), (16, 8, vec![8]));
        let boolean = nullable(Type::Bool);
        assert_eq!(
            (boolean.size, boolean.align, offsets(&boolean)),
            (2, 1, vec![1])
        );
        let point = nullable(main_struct("Point"));
        assert_eq!((point.size, offsets(&point)), (24, vec![8]));
        let unit = nullable(Type::Unit);
        assert_eq!((unit.size, unit.align), (1, 1));
        for inner in [
            Type::Reference {
                mutable: false,
                inner: Box::new(main_struct("Point")),
            },
            Type::Reference {
                mutable: true,
                inner: Box::new(main_struct("Point")),
            },
            Type::Heap(Box::new(Type::Int)),
        ] {
            let layout = nullable(inner);
            assert_eq!(
                (layout.size, layout.align, layout.shape),
                (8, 8, Shape::NullPointer)
            );
        }
        let heap = layout_in(source, &Type::Heap(Box::new(Type::Int)));
        assert_eq!((heap.size, heap.shape), (8, Shape::Scalar(POINTER)));
    }

    #[test]
    fn instances_of_generic_types_are_laid_out_by_substitution() {
        let source = "struct Pair<A, B>(val a: A, val b: B)\nenum Option<T> { Some(T), None }\n";
        let pair = Type::Struct {
            id: NominalId::new("main", "Pair"),
            arguments: vec![Type::Int, Type::Bool],
        };
        let layout = layout_in(source, &pair);
        assert_eq!((layout.size, offsets(&layout)), (16, vec![0, 8]));
        let option = Type::Enum {
            id: NominalId::new("main", "Option"),
            arguments: vec![Type::Char],
        };
        let layout = layout_in(source, &option);
        assert_eq!(
            (layout.size, variant_offsets(&layout)),
            (8, vec![vec![4], vec![]])
        );
    }

    #[test]
    fn strings_are_four_words() {
        let source = "struct User { val name: String, val age: Int }\n";
        let string = layout_in(source, &Type::String);
        assert_eq!(
            (string.size, string.align, string.shape),
            (32, 8, Shape::String)
        );
        let user = layout_in(source, &main_struct("User"));
        assert_eq!(
            (user.size, user.align, offsets(&user)),
            (40, 8, vec![0, 32])
        );
        let nullable = layout_in(source, &Type::String.nullable());
        assert_eq!((nullable.size, offsets(&nullable)), (40, vec![8]));
        assert_eq!(
            [
                StringField::Pointer,
                StringField::ByteLength,
                StringField::CharacterCount,
                StringField::Capacity
            ]
            .map(|field| field.offset(POINTER)),
            [0, 8, 16, 24]
        );
    }

    #[test]
    fn function_values_are_two_words() {
        let source = "struct Task { val run: () -> Unit, val id: Int }\n";
        let task = layout_in(source, &main_struct("Task"));
        assert_eq!(
            (task.size, task.align, offsets(&task)),
            (24, 8, vec![0, 16])
        );
        let function = Type::function(false, Vec::new(), Type::Unit);
        let layout = layout_in(source, &function);
        assert_eq!(
            (layout.size, layout.align, layout.shape),
            (16, 8, Shape::Function)
        );
        let nullable = layout_in(source, &function.nullable());
        assert_eq!((nullable.size, offsets(&nullable)), (24, vec![8]));
        assert_eq!(
            [FunctionField::Descriptor, FunctionField::Environment].map(|f| f.offset(POINTER)),
            [0, 8]
        );
    }

    #[test]
    fn types_without_a_layout_name_their_construct() {
        let file = lower_source("struct Odd { val w: Widget }\nstruct Box<T>(val v: T)\n");
        let layouts = LayoutTable::new(&file.types, POINTER);
        assert_eq!(
            layouts.layout(&main_struct("Odd")),
            Err(Construct::UnknownType("Widget".to_string()))
        );
        let generic = Type::Struct {
            id: NominalId::new("main", "Box"),
            arguments: Vec::new(),
        };
        assert!(matches!(
            layouts.layout(&generic),
            Err(Construct::TypeParameter(_))
        ));
    }

    #[test]
    fn dumping_layouts() {
        let source = "struct Point : Copy { val x: Double, val y: Double }\n\
             enum Expr { Literal(Int), Add(Heap<Expr>, Heap<Expr>) }\n\
             struct User { val name: String }\n\
             struct Task { val run: () -> Unit }\n\
             struct Box<T>(val v: T)\n\
             fun f(a: (&Point)?, n: Int?, s: &String?): Int { val b = Box(true); return 1 }\n";
        let file = lower_source(source);
        let declared: Vec<NominalId> = ["Point", "Expr", "User", "Task", "Box"]
            .iter()
            .map(|name| NominalId::new("main", *name))
            .collect();
        let first = dump_layouts(&file, &declared, POINTER);
        let second = dump_layouts(&lower_source(source), &declared, POINTER);
        assert_eq!(first, second);
        assert_eq!(
            first,
            "struct main::Point : Copy (size 16, align 8)\n\
             \x20 x: Double at 0\n\
             \x20 y: Double at 8\n\
             enum main::Expr (size 24, align 8, tag u8 at 0)\n\
             \x20 Literal(Int at 8)\n\
             \x20 Add(Heap<Expr> at 8, Heap<Expr> at 16)\n\
             struct main::User (size 32, align 8)\n\
             \x20 name: String at 0\n\
             struct main::Task (size 16, align 8)\n\
             \x20 run: () -> Unit at 0\n\
             struct main::Box<T> (generic: laid out per instance)\n\
             nullable (&Point)? (size 8, align 8, null pointer)\n\
             nullable Int? (size 16, align 8, tag u8 at 0)\n\
             \x20 payload Int at 8\n\
             nullable String? (size 40, align 8, tag u8 at 0)\n\
             \x20 payload String at 8\n\
             struct main::Box<Bool> (size 1, align 1)\n\
             \x20 v: Bool at 0\n"
        );
    }
}

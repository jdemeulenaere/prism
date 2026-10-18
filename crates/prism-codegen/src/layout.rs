//! Memory layouts of values (D5 to D8 of `codegen-aggregates`), and `prism dump-layout`'s text
//! (D18).
//!
//! A struct is stored inline, its fields placed by decreasing alignment (declaration order among
//! equals); an enum is the smallest unsigned tag numbering its variants, at offset 0, followed by
//! the current variant's payload, laid out like a struct's fields after the tag; a `T?` is a tag
//! byte (0 for `null`) followed by `T`, except that a nullable reference or `Heap<T>` is one
//! pointer whose `null` is 0. `Int`, `Double` and pointers take 8 bytes, `Char` 4, `Bool` 1,
//! `Unit` and fieldless structs none. `String` and function types have no layout yet.
//!
//! Layouts depend only on the type table of the file, which holds the definition of every type
//! its bodies can hold a value of (D2 and D4), so a file lays out every value from its own
//! declarations and its direct imports' ABIs.

use crate::supported::Construct;
use cranelift_codegen::ir::{types, Type as MachineType};
use prism_mir::{Body, MirFile};
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

    /// The layout of `ty`, or the construct it waits for: strings, function values, a type
    /// parameter (only generic bodies hold one) or an unknown type.
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
            Type::String => return Err(Construct::Strings),
            Type::Fn { .. } => return Err(Construct::FunctionValues),
            Type::Parameter(_) => return Err(Construct::GenericCalls),
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

/// Records in `held` every struct, enum and nullable type a value of `ty` holds: `ty` itself,
/// the referent of a reference, and the fields and payloads of structs, enums and nullables,
/// but not what a `Heap<T>` points at.
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
        _ => return,
    };
    if matches!(
        ty,
        Type::Nullable(_) | Type::Struct { .. } | Type::Enum { .. }
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
        Shape::Empty | Shape::Scalar(_) => writeln!(text, "{} ({})", name, size).unwrap(),
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
    fn types_without_a_layout_name_their_construct() {
        let file = lower_source("struct User { val name: String }\nstruct Box<T>(val v: T)\n");
        let layouts = LayoutTable::new(&file.types, POINTER);
        assert_eq!(
            layouts.layout(&main_struct("User")),
            Err(Construct::Strings)
        );
        let generic = Type::Struct {
            id: NominalId::new("main", "Box"),
            arguments: Vec::new(),
        };
        assert!(matches!(
            layouts.layout(&generic),
            Err(Construct::GenericCalls)
        ));
    }

    #[test]
    fn dumping_layouts() {
        let source = "struct Point : Copy { val x: Double, val y: Double }\n\
             enum Expr { Literal(Int), Add(Heap<Expr>, Heap<Expr>) }\n\
             struct User { val name: String }\n\
             struct Box<T>(val v: T)\n\
             fun f(a: (&Point)?, n: Int?): Int { val b = Box(true); return 1 }\n";
        let file = lower_source(source);
        let declared: Vec<NominalId> = ["Point", "Expr", "User", "Box"]
            .iter()
            .map(|name| NominalId::new("main", *name))
            .collect();
        let first = dump_layouts(&file, &declared, POINTER);
        let second = dump_layouts(&lower_source(source), &declared, POINTER);
        assert_eq!(first, second);
        assert_eq!(
            first,
            "struct main::Point : Copy (size 16, align 8)\n\
             \x20 x: Float at 0\n\
             \x20 y: Float at 8\n\
             enum main::Expr (size 24, align 8, tag u8 at 0)\n\
             \x20 Literal(Int at 8)\n\
             \x20 Add(Heap<Expr> at 8, Heap<Expr> at 16)\n\
             struct main::User (no layout yet: strings)\n\
             struct main::Box<T> (generic: laid out per instance)\n\
             nullable (&Point)? (size 8, align 8, null pointer)\n\
             nullable Int? (size 16, align 8, tag u8 at 0)\n\
             \x20 payload Int at 8\n\
             struct main::Box<Bool> (size 1, align 1)\n\
             \x20 v: Bool at 0\n"
        );
    }
}

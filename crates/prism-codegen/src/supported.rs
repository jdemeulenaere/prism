//! What the Tier 1 backend compiles, and `E6001` for the rest (D9 of `codegen-core`, D19 of
//! `codegen-aggregates`).
//!
//! [`check_file`] runs on a file's elaborated MIR before any code is generated. Generic bodies
//! and closure bodies are never compiled ([`is_compiled`]); their uses are what it rejects. For
//! every other body it reports the first unsupported construct: an unsupported argument or
//! return type at the declaration, a composable body at the declaration, then the first
//! statement or terminator, in block order, that uses an unsupported type or operation. A body
//! that passes is one [`crate::lower`] can lower.
//!
//! Scalars, `Unit`, strings, boxes, references, structs, enums and nullables are supported, unless a
//! struct, enum or nullable holds something that is not: a type is named by the first construct
//! its fields or payloads need, in declaration order, as a reference is named by its referent
//! (D18 of `runtime-strings-and-drops`).

use crate::error::{CodegenError, CodegenErrorKind};
use crate::representation::{representation, Representation};
use cranelift_codegen::ir::types;
use prism_ast::Span;
use prism_mir::{
    Body, BodyOwner, Callee, Constant, MirFile, Operand, Place, Rvalue, StatementKind,
    TerminatorKind,
};
use prism_typed_tree::{Type, TypeTable};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

/// How deep [`classify`] looks into nested structs and enums. Infinitely sized types are
/// rejected by the type checker (`E4023`), and a box's contents are looked into once per type,
/// so only a broken type table reaches it.
const MAXIMUM_DEPTH: usize = 256;

/// A construct the backend does not compile yet, as `E6001` names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Construct {
    FunctionValues,
    GenericCalls,
    ComposableFunctions,
    /// A conversion between two types that are otherwise supported, which the backend does not
    /// lower; the type system produces none over supported types.
    Conversion {
        from: String,
        to: String,
    },
    /// A value of an opaque type, which names no declaration, or of a struct or enum whose
    /// definition the file does not know.
    UnknownType(String),
}

impl Construct {
    /// How the diagnostic names the construct.
    pub fn description(&self) -> String {
        match self {
            Construct::FunctionValues => "function values and closures".to_string(),
            Construct::GenericCalls => "calls of generic functions".to_string(),
            Construct::ComposableFunctions => "composable functions".to_string(),
            Construct::Conversion { from, to } => {
                format!("conversions from '{}' to '{}'", from, to)
            }
            Construct::UnknownType(name) => format!("values of unknown type '{}'", name),
        }
    }

    /// The planned change that will support the construct, if one is planned.
    pub fn change(&self) -> Option<&'static str> {
        match self {
            Construct::FunctionValues => Some("closures"),
            Construct::GenericCalls => Some("monomorphization"),
            Construct::ComposableFunctions => Some("compose-transform"),
            Construct::Conversion { .. } | Construct::UnknownType(_) => None,
        }
    }

    /// The `E6001` diagnostic for this construct at `span`.
    pub fn error(&self, span: Span) -> CodegenError {
        let description = self.description();
        let notes = match self.change() {
            Some(change) => vec![format!(
                "support for {} is planned in the '{}' change",
                description, change
            )],
            None => Vec::new(),
        };
        CodegenError {
            kind: CodegenErrorKind::UnsupportedByBackend,
            message: format!("the Tier 1 backend does not support {} yet", description),
            span,
            notes,
            help: None,
        }
    }
}

/// Whether the backend compiles `body`: every body except generic bodies, which wait for
/// monomorphization, and closure bodies, which wait for closure conversion.
pub fn is_compiled(body: &Body) -> bool {
    body.type_parameters.is_empty() && !matches!(body.owner, BodyOwner::Closure { .. })
}

/// Checks every compiled body of `file`, reporting one `E6001` per body with an unsupported
/// construct.
pub fn check_file(file: &MirFile) -> Result<(), Vec<CodegenError>> {
    let errors: Vec<CodegenError> = file
        .bodies
        .iter()
        .filter(|(_, body)| is_compiled(body))
        .filter_map(|(_, body)| {
            first_unsupported(body, &file.types).map(|(construct, span)| construct.error(span))
        })
        .collect();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// The first unsupported construct of `body`, and where it is reported.
pub fn first_unsupported(body: &Body, types: &TypeTable) -> Option<(Construct, Span)> {
    let signature = body
        .arguments()
        .map(|local| &body.locals[local].ty)
        .chain(std::iter::once(body.return_type()))
        .find_map(|ty| classify(ty, types));
    if let Some(construct) = signature {
        return Some((construct, body.span));
    }
    if body.is_compose {
        return Some((Construct::ComposableFunctions, body.span));
    }
    let checker = BodyChecker {
        body,
        types,
        classified: RefCell::new(HashMap::new()),
    };
    for (_, block) in body.blocks.iter() {
        for statement in &block.statements {
            if let Some(construct) = checker.statement(&statement.kind) {
                return Some((construct, statement.span));
            }
        }
        if let Some(construct) = checker.terminator(&block.terminator.kind) {
            return Some((construct, block.terminator.span));
        }
    }
    None
}

/// The construct that makes a value of `ty` unsupported, if any: for a struct, an enum or a
/// nullable, the first one its fields or payloads need, in declaration order (D14 and D19 of
/// `codegen-aggregates`).
pub fn classify(ty: &Type, types: &TypeTable) -> Option<Construct> {
    classify_at(ty, types, &mut HashSet::new(), 0)
}

/// [`classify`] of `ty`, where `boxed` holds the contents of the boxes already looked into: a
/// type reaching itself through a box needs nothing more the second time (D17 of `heap-boxes`).
fn classify_at(
    ty: &Type,
    types: &TypeTable,
    boxed: &mut HashSet<Type>,
    depth: usize,
) -> Option<Construct> {
    let unknown = || Some(Construct::UnknownType(ty.to_string()));
    if depth > MAXIMUM_DEPTH {
        return unknown();
    }
    match ty {
        Type::Int
        | Type::Float
        | Type::Bool
        | Type::Char
        | Type::String
        | Type::Unit
        | Type::Nothing => None,
        // A reference and a nullable are named by what they hold.
        Type::Reference { inner, .. } | Type::Nullable(inner) => {
            classify_at(inner, types, boxed, depth + 1)
        }
        // A box is named by its contents (D3 of `heap-boxes`).
        Type::Heap(inner) => {
            if boxed.insert(inner.as_ref().clone()) {
                classify_at(inner, types, boxed, depth + 1)
            } else {
                None
            }
        }
        Type::Struct { id, arguments } => match types.instantiated_fields(id, arguments) {
            Some(fields) => fields
                .iter()
                .find_map(|(_, _, field)| classify_at(field, types, boxed, depth + 1)),
            None => unknown(),
        },
        Type::Enum { id, arguments } => match types.instantiated_variants(id, arguments) {
            Some(variants) => variants
                .iter()
                .flat_map(|(_, payloads)| payloads)
                .find_map(|payload| classify_at(payload, types, boxed, depth + 1)),
            None => unknown(),
        },
        Type::Fn { .. } => Some(Construct::FunctionValues),
        Type::Parameter(_) => Some(Construct::GenericCalls),
        Type::Opaque(name) => Some(Construct::UnknownType(name.clone())),
        Type::Error => Some(Construct::UnknownType("<error>".to_string())),
    }
}

/// Whether the backend lowers the `Subtype` step from `from` to `to` (D11 of
/// `codegen-aggregates`): between two types of one machine representation (`&mut T` to `&T`,
/// also inside a nullable), from `Nothing?` to any nullable (writing `null`), and from `Nothing`,
/// which has no value.
pub fn is_lowered_subtype(from: &Type, to: &Type) -> bool {
    // Pointer width does not matter to whether two types share a representation.
    let pointer = types::I64;
    match (from, to) {
        (Type::Nothing, _) => true,
        (Type::Nullable(inner), Type::Nullable(_)) if **inner == Type::Nothing => true,
        _ => matches!(
            (representation(from, pointer), representation(to, pointer)),
            (Some(Representation::Scalar(a)), Some(Representation::Scalar(b))) if a == b
        ),
    }
}

struct BodyChecker<'a> {
    body: &'a Body,
    types: &'a TypeTable,
    /// The classification of each type already classified: a body names the same types again
    /// and again, and classifying an enum walks all of its variants.
    classified: RefCell<HashMap<Type, Option<Construct>>>,
}

impl BodyChecker<'_> {
    /// [`classify`] of `ty`, computed once per type.
    fn classify(&self, ty: &Type) -> Option<Construct> {
        if let Some(construct) = self.classified.borrow().get(ty) {
            return construct.clone();
        }
        let construct = classify(ty, self.types);
        self.classified
            .borrow_mut()
            .insert(ty.clone(), construct.clone());
        construct
    }

    fn local_type(&self, place: &Place) -> &Type {
        &self.body.locals[place.local].ty
    }

    fn place_type(&self, place: &Place) -> Option<Type> {
        place.try_ty(self.body, self.types)
    }

    /// A place is supported when its local is. Its projections are: the MIR has no index
    /// projection (D5 of `runtime-strings-and-drops`).
    fn place(&self, place: &Place) -> Option<Construct> {
        self.classify(self.local_type(place))
    }

    fn operand(&self, operand: &Operand) -> Option<Construct> {
        match operand {
            Operand::Copy(place, _) | Operand::Move(place, _) => self.place(place),
            Operand::Constant(constant) => match constant {
                Constant::Int(_)
                | Constant::Float(_)
                | Constant::Bool(_)
                | Constant::Char(_)
                | Constant::String(_)
                | Constant::Unit
                | Constant::TopLevelValue(_) => None,
                Constant::Null(ty) => self.classify(ty),
                // A variant constructor reaches the backend only as a value; its calls are
                // aggregates.
                Constant::Function(_) | Constant::VariantConstructor { .. } => {
                    Some(Construct::FunctionValues)
                }
            },
        }
    }

    fn statement(&self, statement: &StatementKind) -> Option<Construct> {
        match statement {
            StatementKind::Assign(place, rvalue) => {
                self.place(place).or_else(|| self.rvalue(place, rvalue))
            }
            StatementKind::Read(place) => self.place(place),
            StatementKind::Destructure { source, parts, .. } => {
                self.place(&Place::local(*source)).or_else(|| {
                    parts
                        .iter()
                        .find_map(|(_, target)| self.place(&Place::local(*target)))
                })
            }
            StatementKind::StorageLive(_) | StatementKind::StorageDead(_) => None,
        }
    }

    fn rvalue(&self, destination: &Place, rvalue: &Rvalue) -> Option<Construct> {
        match rvalue {
            Rvalue::Use(operand) | Rvalue::Unary(_, operand) | Rvalue::WrapNullable(operand) => {
                self.operand(operand)
            }
            Rvalue::Binary(_, left, right) => self.operand(left).or_else(|| self.operand(right)),
            Rvalue::Borrow { place, .. } | Rvalue::Discriminant(place) => self.place(place),
            Rvalue::Aggregate(kind, operands) => self
                .classify(&kind.ty())
                .or_else(|| operands.iter().find_map(|operand| self.operand(operand))),
            Rvalue::Subtype(operand) => self.operand(operand).or_else(|| {
                let from = operand.try_ty(self.body, self.types)?;
                let to = self.place_type(destination)?;
                (!is_lowered_subtype(&from, &to)).then(|| Construct::Conversion {
                    from: from.to_string(),
                    to: to.to_string(),
                })
            }),
            Rvalue::Closure { .. } => Some(Construct::FunctionValues),
        }
    }

    fn terminator(&self, terminator: &TerminatorKind) -> Option<Construct> {
        match terminator {
            TerminatorKind::Goto(_) | TerminatorKind::Return | TerminatorKind::Unreachable => None,
            TerminatorKind::SwitchInt { value, .. } => self.operand(value),
            // A supported value's drop runs its drop glue, if it has any (D9 of
            // `runtime-strings-and-drops`).
            TerminatorKind::Drop { place, .. } => self.place(place),
            TerminatorKind::Call {
                callee,
                arguments,
                parameters,
                type_arguments,
                destination,
                is_compose,
                ..
            } => {
                if *is_compose {
                    return Some(Construct::ComposableFunctions);
                }
                // A built-in's type argument, the contents of `Heap(...)`, is no generic call.
                if !type_arguments.is_empty() && !matches!(callee, Callee::Builtin(_)) {
                    return Some(Construct::GenericCalls);
                }
                match callee {
                    Callee::Value(..) => return Some(Construct::FunctionValues),
                    Callee::Builtin(_) | Callee::Function(_) | Callee::Default { .. } => {}
                }
                arguments
                    .iter()
                    .find_map(|argument| self.operand(argument))
                    .or_else(|| {
                        parameters
                            .iter()
                            .find_map(|ty| self.classify(&ty.substitute(type_arguments)))
                    })
                    .or_else(|| self.place(destination))
                    .or_else(|| {
                        self.place_type(destination)
                            .and_then(|ty| self.classify(&ty))
                    })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::lower_source;

    /// The descriptions of the errors of `source`, with the line and column of each.
    fn errors(source: &str) -> Vec<(String, u32, u32)> {
        match check_file(&lower_source(source)) {
            Ok(()) => Vec::new(),
            Err(errors) => errors
                .into_iter()
                .map(|error| {
                    assert_eq!(error.code(), "E6001");
                    (error.message, error.span.line, error.span.column)
                })
                .collect(),
        }
    }

    fn single(source: &str) -> (String, u32, u32) {
        let mut errors = errors(source);
        assert_eq!(errors.len(), 1, "{:?}", errors);
        errors.remove(0)
    }

    fn names(message: &str, construct: &str) {
        assert_eq!(
            message,
            format!("the Tier 1 backend does not support {} yet", construct)
        );
    }

    #[test]
    fn scalar_programs_are_supported() {
        let source = "val LIMIT: Int = 10\n\
             fun f(a: Int, b: Int = a + 1): Int = a * b\n\
             fun bump(r: &mut Int) { *r = r + 1 }\n\
             fun g(n: Int, d: Double, c: Char): Bool {\n\
                 var sum = 0\n\
                 for (i in 0..<n) { if (i == 2) continue; sum += i }\n\
                 val r = 1..3\n\
                 for (j in r) { sum += j }\n\
                 var k = 0; while (k < 3) { k += 1; bump(&mut k) }\n\
                 do { k -= 1 } while (k > 0)\n\
                 val x = when (n) { 0 -> 10; in 1..10 -> 20; else -> 30 }\n\
                 println(n.clone() + LIMIT + f(3) + x); print(d % 2.0); println(c)\n\
                 return n > 1 && !(d < 0.5) || c == 'x'\n\
             }\n\
             fun <T> keep(x: T): T = x\n";
        assert_eq!(errors(source), []);
    }

    #[test]
    fn a_struct_is_rejected_at_its_construction() {
        let (message, line, column) = single(
            "struct P { val run: () -> Int }\nfun one(): Int = 1\nfun main() { val p = P(one); println(1) }\n",
        );
        names(&message, "function values and closures");
        assert_eq!(line, 3);
        let _ = column;
    }

    #[test]
    fn every_row_of_the_table() {
        let cases: &[(&str, &str)] = &[
            (
                "struct P { val n: Int, val name: String?, val h: Heap<Int>, val f: () -> Int }\nfun f(p: &P): Int = 1\n",
                "function values and closures",
            ),
            (
                "struct P { val n: Int, val h: Heap<() -> Int>? }\nfun f(p: &P): Int = 1\n",
                "function values and closures",
            ),
            (
                "enum Job { Run(Heap<Job>, () -> Int), Idle }\nfun f() { val j = Job.Idle }\n",
                "function values and closures",
            ),
            (
                "fun f() { val g = { 1 } }\n",
                "function values and closures",
            ),
            (
                "fun g() {}\nfun f() { val h = g }\n",
                "function values and closures",
            ),
            (
                "struct C { val f: () -> Int }\nfun f(c: &C) {}\n",
                "function values and closures",
            ),
            (
                "fun <T> id(x: T): T = x\nfun f() { println(id(1)) }\n",
                "calls of generic functions",
            ),
            ("compose fun Label() {}\n", "composable functions"),
            ("fun f(x: File) {}\n", "values of unknown type 'File'"),
        ];
        for (source, construct) in cases {
            let (message, _, _) = single(source);
            names(&message, construct);
        }
    }

    #[test]
    fn strings_are_supported() {
        let source = "struct User { val name: String, val nick: String? }\n\
             enum Message { Text(String, Int), Empty }\n\
             val ADMIN: String = \"ro\" + \"ot\"\n\
             fun greet(name: String, n: Int): String = name\n\
             fun join(a: &String, b: &String): String = a + b\n\
             fun main() {\n\
                 var s = \"a\"; s += \"b\"; println(s); print(s == ADMIN)\n\
                 val u = User(s.clone(), null); println(u.name.length); println(u.name[0])\n\
                 val m = Message.Text(1.toString(), 2); println(m == Message.Empty)\n\
                 println(greet(join(&ADMIN, &u.name), 1))\n\
             }\n";
        assert_eq!(errors(source), []);
    }

    #[test]
    fn aggregates_are_supported() {
        let source = "struct Point : Copy { var x: Int, var y: Int }\n\
             struct Counter { var n: Int }\n\
             enum Shape : Copy { Circle(Double), Square(Double), Empty }\n\
             struct Pair<A, B>(val a: A, val b: B)\n\
             val ORIGIN: Point = Point(0, 0)\n\
             fun count(r: Range): Int { var n = 0; for (i in r) n += 1; return n }\n\
             fun span(): Range = 1..3\n\
             fun mirror(p: Point): Point = Point(p.y, p.x)\n\
             fun take(c: Counter): Int = c.n\n\
             fun area(s: Shape): Double = when (s) { Shape.Circle(1.0) -> 1.0; else -> 0.0 }\n\
             fun age(n: Int?): Int = n ?: 0\n\
             fun view(r: (&Point)?): Int = r?.x ?: 0\n\
             fun main() {\n\
                 var p = Point(1, 2); p.y = 5; p = mirror(p)\n\
                 val c = Counter(4); println(take(c))\n\
                 val q = Pair(1, true); println(q.b)\n\
                 println(p == ORIGIN); println(Shape.Empty != Shape.Circle(1.0))\n\
                 println(count(span())); println(age(null)); println(view(&p))\n\
                 val t = p.clone(); println(t.x)\n\
             }\n";
        assert_eq!(errors(source), []);
    }

    #[test]
    fn notes_name_the_change() {
        let error = check_file(&lower_source("fun f() { val g = { 1 } }\n")).unwrap_err();
        assert_eq!(
            error[0].notes,
            ["support for function values and closures is planned in the 'closures' change"]
        );
    }

    #[test]
    fn one_error_per_body() {
        // The scenario "One error per body" of the `code-generation` spec.
        let errors = errors(
            "fun f(): Int { val a = { 1 }; val b = { 2 }; return 1 }\n\
             fun g(): Int { val c = { 3 }; return 2 }\n\
             fun main() {}\n",
        );
        let positions: Vec<(u32, u32)> = errors.iter().map(|e| (e.1, e.2)).collect();
        assert_eq!(positions, [(1, 24), (2, 24)]);
    }

    #[test]
    fn unsupported_signatures_are_reported_at_the_declaration() {
        let (message, line, column) = single("fun main() {}\nfun first(g: () -> Int): Int = 1\n");
        names(&message, "function values and closures");
        assert_eq!((line, column), (2, 1));
    }

    #[test]
    fn generic_and_closure_bodies_are_not_checked() {
        assert_eq!(
            errors("fun <T> keep(x: T): T { val g = { 1 }; return x }\n"),
            []
        );
    }

    #[test]
    fn boxes_are_supported() {
        let source = "enum Expr { Lit(Int), Add(Heap<Expr>, Heap<Expr>) }\n\
             struct Node { val value: Int, val next: Heap<Node>? }\n\
             fun f(h: Heap<Int>): Int = h.unbox()\n\
             fun main() {\n\
                 val e = Expr.Add(Heap(Expr.Lit(1)), Heap(Expr.Lit(2)))\n\
                 val n = Node(1, Heap(Node(2, null)))\n\
                 val h: Heap<Int>? = null\n\
                 println(f(Heap(3)))\n\
             }\n";
        assert_eq!(errors(source), []);
    }
}

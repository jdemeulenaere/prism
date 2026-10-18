//! What the Tier 1 backend compiles, and `E6001` for the rest (D9 of `codegen-core`).
//!
//! [`check_file`] runs on a file's elaborated MIR before any code is generated. Generic bodies
//! and closure bodies are never compiled ([`is_compiled`]); their uses are what it rejects. For
//! every other body it reports the first unsupported construct: an unsupported argument or
//! return type at the declaration, a composable body at the declaration, then the first
//! statement or terminator, in block order, that uses an unsupported type or operation. A body
//! that passes is one [`crate::lower`] can lower.

use crate::error::{CodegenError, CodegenErrorKind};
use prism_ast::Span;
use prism_mir::{
    AggregateKind, Body, BodyOwner, Callee, Constant, MirFile, Operand, Place, Projection, Rvalue,
    StatementKind, TerminatorKind,
};
use prism_typed_tree::{BuiltinFunction, NominalId, Type, TypeTable};

/// A construct the backend does not compile yet, as `E6001` names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Construct {
    Structs,
    Enums,
    NullableValues,
    /// A `Range` parameter, result, borrow or top-level value: only `Range` locals, which are
    /// scalar-replaced, are supported.
    RangeOutsideLocals,
    Strings,
    HeapValues,
    Drops,
    FunctionValues,
    GenericCalls,
    ComposableFunctions,
    /// A conversion between two types neither of which is otherwise rejected.
    Conversion {
        from: String,
        to: String,
    },
    /// A value of an opaque type, which names no declaration.
    UnknownType(String),
}

impl Construct {
    /// How the diagnostic names the construct.
    pub fn description(&self) -> String {
        match self {
            Construct::Structs => "structs".to_string(),
            Construct::Enums => "enums".to_string(),
            Construct::NullableValues => "nullable values".to_string(),
            Construct::RangeOutsideLocals => "'Range' values outside local variables".to_string(),
            Construct::Strings => "strings".to_string(),
            Construct::HeapValues => "'Heap' values".to_string(),
            Construct::Drops => "drops".to_string(),
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
            Construct::Structs
            | Construct::Enums
            | Construct::NullableValues
            | Construct::RangeOutsideLocals
            | Construct::Conversion { .. } => Some("codegen-aggregates"),
            Construct::Strings | Construct::HeapValues | Construct::Drops => {
                Some("runtime-strings-and-drops")
            }
            Construct::FunctionValues => Some("closures"),
            Construct::GenericCalls => Some("monomorphization"),
            Construct::ComposableFunctions => Some("compose-transform"),
            Construct::UnknownType(_) => None,
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
        .find_map(|ty| classify(ty, false));
    if let Some(construct) = signature {
        return Some((construct, body.span));
    }
    if body.is_compose {
        return Some((Construct::ComposableFunctions, body.span));
    }
    let checker = BodyChecker { body, types };
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

/// The construct that makes a value of `ty` unsupported, if any. A `Range` is supported only as
/// a local (`range_allowed`).
pub fn classify(ty: &Type, range_allowed: bool) -> Option<Construct> {
    match ty {
        Type::Int | Type::Float | Type::Bool | Type::Char | Type::Unit | Type::Nothing => None,
        Type::String => Some(Construct::Strings),
        // A reference is named by what it refers to.
        Type::Reference { inner, .. } => match inner.as_ref() {
            Type::Unit => None,
            inner => classify(inner, false),
        },
        Type::Nullable(_) => Some(Construct::NullableValues),
        Type::Heap(_) => Some(Construct::HeapValues),
        Type::Struct { id, .. } if *id == NominalId::builtin_range() => {
            (!range_allowed).then_some(Construct::RangeOutsideLocals)
        }
        Type::Struct { .. } => Some(Construct::Structs),
        Type::Enum { .. } => Some(Construct::Enums),
        Type::Fn { .. } => Some(Construct::FunctionValues),
        Type::Parameter(_) => Some(Construct::GenericCalls),
        Type::Opaque(name) => Some(Construct::UnknownType(name.clone())),
        Type::Error => Some(Construct::UnknownType("<error>".to_string())),
    }
}

fn is_range(ty: &Type) -> bool {
    matches!(ty, Type::Struct { id, .. } if *id == NominalId::builtin_range())
}

struct BodyChecker<'a> {
    body: &'a Body,
    types: &'a TypeTable,
}

impl BodyChecker<'_> {
    fn local_type(&self, place: &Place) -> &Type {
        &self.body.locals[place.local].ty
    }

    fn place_type(&self, place: &Place) -> Option<Type> {
        place.try_ty(self.body, self.types)
    }

    /// A place is supported when its local is (a `Range` local included) and its projections
    /// are dereferences of references or field reads of a `Range` local.
    fn place(&self, place: &Place) -> Option<Construct> {
        let root = self.local_type(place);
        if let Some(construct) = classify(root, true) {
            return Some(construct);
        }
        for (index, projection) in place.projections.iter().enumerate() {
            match projection {
                Projection::Deref => {}
                Projection::Field(_) if index == 0 && is_range(root) => {}
                Projection::Field(_) => return Some(Construct::Structs),
                Projection::Payload => return Some(Construct::NullableValues),
                Projection::Index(_) => return Some(Construct::Strings),
            }
        }
        None
    }

    fn operand(&self, operand: &Operand) -> Option<Construct> {
        match operand {
            Operand::Copy(place, _) | Operand::Move(place, _) => self.place(place),
            Operand::Constant(constant) => match constant {
                Constant::Int(_)
                | Constant::Float(_)
                | Constant::Bool(_)
                | Constant::Char(_)
                | Constant::Unit
                | Constant::TopLevelValue(_) => None,
                Constant::String(_) => Some(Construct::Strings),
                Constant::Null(_) => Some(Construct::NullableValues),
                Constant::Function(_) => Some(Construct::FunctionValues),
                Constant::VariantConstructor { .. } => Some(Construct::Enums),
            },
        }
    }

    fn statement(&self, statement: &StatementKind) -> Option<Construct> {
        match statement {
            StatementKind::Assign(place, rvalue) => {
                self.place(place).or_else(|| self.rvalue(place, rvalue))
            }
            StatementKind::Read(place) => self.place(place),
            StatementKind::StorageLive(_) | StatementKind::StorageDead(_) => None,
        }
    }

    fn rvalue(&self, destination: &Place, rvalue: &Rvalue) -> Option<Construct> {
        match rvalue {
            Rvalue::Use(operand) | Rvalue::Unary(_, operand) => self.operand(operand),
            Rvalue::Binary(_, left, right) => self.operand(left).or_else(|| self.operand(right)),
            Rvalue::Borrow { place, .. } => self.place(place).or_else(|| {
                // A `Range` local is scalar-replaced, so neither it nor its fields have an
                // address.
                let borrows_range = is_range(self.local_type(place))
                    && place.projections.first() != Some(&Projection::Deref);
                borrows_range.then_some(Construct::RangeOutsideLocals)
            }),
            Rvalue::Discriminant(place) => match self.place_type(place) {
                Some(Type::Nullable(_)) => Some(Construct::NullableValues),
                _ => Some(Construct::Enums),
            },
            Rvalue::Aggregate(AggregateKind::Struct { id, .. }, operands)
                if *id == NominalId::builtin_range() =>
            {
                operands.iter().find_map(|operand| self.operand(operand))
            }
            Rvalue::Aggregate(AggregateKind::Struct { .. }, _) => Some(Construct::Structs),
            Rvalue::Aggregate(AggregateKind::Variant { .. }, _) => Some(Construct::Enums),
            Rvalue::WrapNullable(_) => Some(Construct::NullableValues),
            Rvalue::Subtype(operand) => self.operand(operand).or_else(|| {
                let from = operand.try_ty(self.body, self.types)?;
                let to = self.place_type(destination)?;
                match (&from, &to) {
                    (
                        Type::Reference {
                            mutable: true,
                            inner: from_inner,
                        },
                        Type::Reference {
                            mutable: false,
                            inner: to_inner,
                        },
                    ) if from_inner == to_inner => None,
                    _ => Some(Construct::Conversion {
                        from: from.to_string(),
                        to: to.to_string(),
                    }),
                }
            }),
            Rvalue::Closure { .. } => Some(Construct::FunctionValues),
        }
    }

    fn terminator(&self, terminator: &TerminatorKind) -> Option<Construct> {
        match terminator {
            TerminatorKind::Goto(_) | TerminatorKind::Return | TerminatorKind::Unreachable => None,
            TerminatorKind::SwitchInt { value, .. } => self.operand(value),
            TerminatorKind::Drop { .. } => Some(Construct::Drops),
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
                if !type_arguments.is_empty() {
                    return Some(Construct::GenericCalls);
                }
                match callee {
                    Callee::Value(..) => return Some(Construct::FunctionValues),
                    Callee::Builtin(BuiltinFunction::Clone)
                    | Callee::Builtin(BuiltinFunction::Print)
                    | Callee::Builtin(BuiltinFunction::PrintLine)
                    | Callee::Function(_)
                    | Callee::Default { .. } => {}
                }
                arguments
                    .iter()
                    .find_map(|argument| self.operand(argument))
                    .or_else(|| parameters.iter().find_map(|ty| classify(ty, false)))
                    .or_else(|| self.place(destination))
                    .or_else(|| {
                        self.place_type(destination)
                            .and_then(|ty| classify(&ty, false))
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
        let (message, line, column) =
            single("struct P { val x: Int }\nfun main() { val p = P(1); println(p.x) }\n");
        names(&message, "structs");
        assert_eq!((line, column), (2, 22));
    }

    #[test]
    fn every_row_of_the_table() {
        let cases: &[(&str, &str)] = &[
            ("enum E { A, B }\nfun f() { val e = E.A }\n", "enums"),
            ("fun f() { val n: Int? = null }\n", "nullable values"),
            ("fun f(n: Int?) {}\n", "nullable values"),
            (
                "fun f(r: Range): Int = 1\n",
                "'Range' values outside local variables",
            ),
            (
                "fun f(): Range = 1..2\n",
                "'Range' values outside local variables",
            ),
            (
                "fun f() { val r = 1..2; val b = &r }\n",
                "'Range' values outside local variables",
            ),
            ("fun f() { val s = \"x\" }\n", "strings"),
            ("fun f(s: &String) {}\n", "strings"),
            ("fun f() { println(\"hi\") }\n", "strings"),
            ("fun f(h: Heap<Int>) {}\n", "'Heap' values"),
            (
                "fun f() { val g = { 1 } }\n",
                "function values and closures",
            ),
            (
                "fun g() {}\nfun f() { val h = g }\n",
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
    fn notes_name_the_change() {
        let error = check_file(&lower_source("fun f() { val s = \"x\" }\n")).unwrap_err();
        assert_eq!(
            error[0].notes,
            ["support for strings is planned in the 'runtime-strings-and-drops' change"]
        );
    }

    #[test]
    fn one_error_per_body() {
        // The scenario "One error per body" of the `code-generation` spec.
        let errors = errors(
            "fun f(): Int { val a = \"x\"; val b = \"y\"; return 1 }\n\
             fun g(): Int { val c = \"z\"; return 2 }\n\
             fun main() {}\n",
        );
        let positions: Vec<(u32, u32)> = errors.iter().map(|e| (e.1, e.2)).collect();
        assert_eq!(positions, [(1, 24), (2, 24)]);
    }

    #[test]
    fn unsupported_signatures_are_reported_at_the_declaration() {
        let (message, line, column) = single("fun main() {}\nfun first(s: String): Int = 1\n");
        names(&message, "strings");
        assert_eq!((line, column), (2, 1));
    }

    #[test]
    fn generic_and_closure_bodies_are_not_checked() {
        assert_eq!(
            errors("fun <T> keep(x: T): T { val s = \"x\"; return x }\n"),
            []
        );
    }
}

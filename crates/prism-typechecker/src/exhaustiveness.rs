//! Exhaustiveness and usefulness of `when` patterns (D11 and D12 of `payload-patterns`), by the
//! usefulness algorithm of Maranget's "Warnings for pattern matching" (2007).
//!
//! The algorithm works over a reduced form of the typed tree's patterns: a constructor with its
//! sub-patterns, a wildcard, or an opaque pattern that covers nothing (a range, a value that is
//! not a literal, a guarded branch's patterns are left out altogether). The constructors of a
//! type are an enum's variants, a struct's one constructor, `true` and `false` for `Bool`, and
//! `null` and "present" (with the payload as its one field) for a nullable. The literals of an
//! open type (`Int`, `Double`, `Char`, `String`) are constructors of a set that is never complete.
//! A `Heap<T>` column is its contents' column, since a box has one constructor (D9 of
//! `heap-boxes`), and the borrow a `(&T)?` holds is its referent's column likewise; patterns
//! are finite, so the algorithm still terminates.

use prism_ast::Literal;
use prism_typed_tree::{ExpressionId, Pattern, Type, TypeTable};

/// A constructor of a type.
#[derive(Debug, Clone, PartialEq)]
enum Constructor {
    /// Variant `index` of the enum.
    Variant(usize),
    /// The one constructor of a struct.
    Struct,
    Bool(bool),
    Null,
    /// A nullable that is not `null`; its one field is the payload.
    Present,
    /// A literal of an open type, whose constructor set is never complete.
    Literal(Literal),
}

/// A reduced pattern.
#[derive(Debug, Clone)]
enum Reduced {
    Wildcard,
    Constructor(Constructor, Vec<Reduced>),
    /// A pattern that covers no value for certain: a range, or a value that is neither a literal
    /// nor a variant.
    Opaque,
}

/// A value no row matches, printed as a pattern.
#[derive(Debug, Clone)]
enum Witness {
    Wildcard,
    Constructor(Constructor, Vec<Witness>),
}

/// The patterns of one branch, and whether it has a guard.
pub struct Row<'a> {
    pub patterns: Vec<&'a Pattern>,
    pub guarded: bool,
}

/// Checks whether `rows`, the branches of a `when` over a subject of type `subject`, cover every
/// value of it. Returns the pattern of one value no unguarded branch covers when they do not.
/// `literal` gives the literal a value pattern's node is, through its coercions.
pub fn missing_value(
    types: &TypeTable,
    subject: &Type,
    rows: &[Row],
    literal: &dyn Fn(ExpressionId) -> Option<Literal>,
) -> Option<String> {
    let checker = Matrix { types };
    let matrix: Vec<Vec<Reduced>> = rows
        .iter()
        .filter(|row| !row.guarded)
        .flat_map(|row| row.patterns.iter())
        .map(|pattern| vec![reduce(types, pattern, subject, literal)])
        .collect();
    let witness = checker.useful(&matrix, &[Reduced::Wildcard], std::slice::from_ref(subject))?;
    Some(checker.render(&witness[0], subject))
}

/// The positions `(branch, pattern)` of the patterns that match no value the patterns before them
/// leave: an unguarded branch's earlier patterns and earlier branches. They are computed for the
/// warnings channel to come and are not reported (D12).
pub fn useless_patterns(
    types: &TypeTable,
    subject: &Type,
    rows: &[Row],
    literal: &dyn Fn(ExpressionId) -> Option<Literal>,
) -> Vec<(usize, usize)> {
    let checker = Matrix { types };
    let mut matrix: Vec<Vec<Reduced>> = Vec::new();
    let mut useless = Vec::new();
    for (branch, row) in rows.iter().enumerate() {
        for (index, pattern) in row.patterns.iter().enumerate() {
            let reduced = reduce(types, pattern, subject, literal);
            let useful = matches!(reduced, Reduced::Opaque)
                || checker
                    .useful(
                        &matrix,
                        std::slice::from_ref(&reduced),
                        std::slice::from_ref(subject),
                    )
                    .is_some();
            if !useful {
                useless.push((branch, index));
            }
            if !row.guarded {
                matrix.push(vec![reduced]);
            }
        }
    }
    useless
}

/// The reduced form of `pattern`, matching a value of type `ty`.
fn reduce(
    types: &TypeTable,
    pattern: &Pattern,
    ty: &Type,
    literal: &dyn Fn(ExpressionId) -> Option<Literal>,
) -> Reduced {
    let reduce_fields = |fields: &[Pattern], parts: Vec<Type>| -> Vec<Reduced> {
        fields
            .iter()
            .enumerate()
            .map(|(index, field)| {
                let part = parts.get(index).cloned().unwrap_or(Type::Error);
                reduce(types, field, &part, literal)
            })
            .collect()
    };
    match pattern {
        Pattern::Wildcard | Pattern::Binding { .. } | Pattern::Else => Reduced::Wildcard,
        Pattern::Variant {
            index,
            fields,
            enum_id,
            arguments,
            ..
        } => {
            let parts = types
                .instantiated_variant(enum_id, arguments, *index)
                .unwrap_or_default();
            Reduced::Constructor(Constructor::Variant(*index), reduce_fields(fields, parts))
        }
        Pattern::Struct {
            id,
            arguments,
            fields,
        } => {
            let parts = types
                .instantiated_fields(id, arguments)
                .map_or_else(Vec::new, |fields| {
                    fields.into_iter().map(|(_, _, ty)| ty).collect()
                });
            Reduced::Constructor(Constructor::Struct, reduce_fields(fields, parts))
        }
        // A box has one constructor, so its column is its contents' (D9 of `heap-boxes`); a
        // borrow's is its referent's.
        Pattern::Deref(inner) => reduce(types, inner, peel_indirections(ty), literal),
        Pattern::NonNull(inner) => {
            let payload = match ty {
                Type::Nullable(payload) => payload.as_ref().clone(),
                _ => Type::Error,
            };
            Reduced::Constructor(
                Constructor::Present,
                vec![reduce(types, inner, &payload, literal)],
            )
        }
        Pattern::Value(value) => match literal(*value) {
            Some(Literal::Null) => Reduced::Constructor(Constructor::Null, Vec::new()),
            Some(value) => {
                let constructor = match value {
                    Literal::Bool(b) => Constructor::Bool(b),
                    other => Constructor::Literal(other),
                };
                let reduced = Reduced::Constructor(constructor, Vec::new());
                match ty {
                    // A literal of `T` matches a non-null `T?` holding it.
                    Type::Nullable(_) => Reduced::Constructor(Constructor::Present, vec![reduced]),
                    _ => reduced,
                }
            }
            None => Reduced::Opaque,
        },
        Pattern::Range { .. } | Pattern::Condition(_) => Reduced::Opaque,
    }
}

/// The union of the increasing lists `a` and `b`, which share no element, in increasing order.
fn merged(a: &[usize], b: &[usize]) -> Vec<usize> {
    let mut merged = Vec::with_capacity(a.len() + b.len());
    let (mut i, mut j) = (0, 0);
    while i < a.len() || j < b.len() {
        if j == b.len() || (i < a.len() && a[i] < b[j]) {
            merged.push(a[i]);
            i += 1;
        } else {
            merged.push(b[j]);
            j += 1;
        }
    }
    merged
}

/// `ty` without its `Heap<T>` and reference layers: a box is matched, and printed, as its
/// contents, and a borrow as its referent.
fn peel_indirections(mut ty: &Type) -> &Type {
    while let Type::Heap(inner) | Type::Reference { inner, .. } = ty {
        ty = inner;
    }
    ty
}

/// The usefulness algorithm over a matrix of reduced rows.
struct Matrix<'a> {
    types: &'a TypeTable,
}

impl Matrix<'_> {
    /// Every constructor of `ty` with its field types, or `None` for an open type (or one whose
    /// definition is unknown), whose constructor set is never complete.
    fn constructors(&self, ty: &Type) -> Option<Vec<(Constructor, Vec<Type>)>> {
        match peel_indirections(ty) {
            Type::Enum { id, arguments } => {
                let variants = self.types.instantiated_variants(id, arguments)?;
                Some(
                    variants
                        .into_iter()
                        .enumerate()
                        .map(|(index, (_, payloads))| (Constructor::Variant(index), payloads))
                        .collect(),
                )
            }
            Type::Struct { id, arguments } => {
                let fields = self.types.instantiated_fields(id, arguments)?;
                Some(vec![(
                    Constructor::Struct,
                    fields.into_iter().map(|(_, _, ty)| ty).collect(),
                )])
            }
            Type::Bool => Some(vec![
                (Constructor::Bool(true), Vec::new()),
                (Constructor::Bool(false), Vec::new()),
            ]),
            Type::Nullable(inner) => Some(vec![
                (Constructor::Null, Vec::new()),
                (Constructor::Present, vec![inner.as_ref().clone()]),
            ]),
            _ => None,
        }
    }

    /// The field types of `constructor` of `ty`. A variant's are looked up alone, so the cost
    /// does not grow with the enum's other variants.
    fn fields_of(&self, constructor: &Constructor, ty: &Type) -> Vec<Type> {
        match (constructor, peel_indirections(ty)) {
            (Constructor::Literal(_), _) => Vec::new(),
            (Constructor::Variant(index), Type::Enum { id, arguments }) => self
                .types
                .instantiated_variant(id, arguments, *index)
                .unwrap_or_default(),
            (_, Type::Enum { .. }) => Vec::new(),
            _ => self
                .constructors(ty)
                .and_then(|all| {
                    all.into_iter()
                        .find(|(c, _)| c == constructor)
                        .map(|(_, fields)| fields)
                })
                .unwrap_or_default(),
        }
    }

    /// The position of `constructor` in the list [`Self::constructors`] gives for its type, when
    /// that list can contain it.
    fn position(constructor: &Constructor) -> Option<usize> {
        match constructor {
            Constructor::Variant(index) => Some(*index),
            Constructor::Struct | Constructor::Bool(true) | Constructor::Null => Some(0),
            Constructor::Bool(false) | Constructor::Present => Some(1),
            Constructor::Literal(_) => None,
        }
    }

    /// The rows of `matrix` that match `constructor`, with its fields in place of their first
    /// column. An opaque first column matches nothing for certain.
    fn specialize(
        &self,
        matrix: &[Vec<Reduced>],
        constructor: &Constructor,
        arity: usize,
    ) -> Vec<Vec<Reduced>> {
        self.specialize_rows(matrix.iter(), constructor, arity)
    }

    /// [`Self::specialize`] over `rows` only, in order: the rows that can match.
    fn specialize_rows<'r>(
        &self,
        rows: impl Iterator<Item = &'r Vec<Reduced>>,
        constructor: &Constructor,
        arity: usize,
    ) -> Vec<Vec<Reduced>> {
        rows.filter_map(|row| {
            let mut fields = match &row[0] {
                Reduced::Wildcard => vec![Reduced::Wildcard; arity],
                Reduced::Constructor(c, fields) if c == constructor => {
                    let mut fields = fields.clone();
                    fields.resize(arity, Reduced::Wildcard);
                    fields
                }
                _ => return None,
            };
            fields.extend(row[1..].iter().cloned());
            Some(fields)
        })
        .collect()
    }

    /// The rows of `matrix` whose first column is a wildcard, without it.
    fn default_matrix(&self, matrix: &[Vec<Reduced>]) -> Vec<Vec<Reduced>> {
        matrix
            .iter()
            .filter(|row| matches!(row[0], Reduced::Wildcard))
            .map(|row| row[1..].to_vec())
            .collect()
    }

    /// Whether `vector`, of the column types `types`, matches a value no row of `matrix`
    /// matches: returns such values' patterns, one per column, or `None`.
    fn useful(
        &self,
        matrix: &[Vec<Reduced>],
        vector: &[Reduced],
        types: &[Type],
    ) -> Option<Vec<Witness>> {
        let Some((head, rest)) = vector.split_first() else {
            return matrix.is_empty().then(Vec::new);
        };
        let ty = &types[0];
        match head {
            Reduced::Opaque => None,
            Reduced::Constructor(constructor, fields) => {
                let field_types = self.fields_of(constructor, ty);
                let arity = fields.len();
                let mut column_types = field_types;
                column_types.resize(arity, Type::Error);
                column_types.extend(types[1..].iter().cloned());
                let mut next = fields.clone();
                next.extend(rest.iter().cloned());
                let specialized = self.specialize(matrix, constructor, arity);
                let witness = self.useful(&specialized, &next, &column_types)?;
                Some(Self::rebuild(constructor.clone(), arity, witness))
            }
            Reduced::Wildcard => {
                // An enum without variants has no constructor, yet is never covered: a `when`
                // over it needs `else` (D11).
                let all = self.constructors(ty).filter(|all| !all.is_empty());
                // The rows headed by each constructor of `all`, and the rows headed by a
                // wildcard, in order: each specialization below reads only its own rows and
                // the wildcards, so the cost does not grow with constructors times rows.
                let mut headed: Vec<Vec<usize>> =
                    vec![Vec::new(); all.as_ref().map_or(0, Vec::len)];
                let mut wildcards = Vec::new();
                for (index, row) in matrix.iter().enumerate() {
                    match &row[0] {
                        Reduced::Constructor(c, _) => {
                            let position = Self::position(c).filter(|position| {
                                all.as_ref()
                                    .and_then(|all| all.get(*position))
                                    .is_some_and(|(listed, _)| listed == c)
                            });
                            if let Some(position) = position {
                                headed[position].push(index);
                            }
                        }
                        Reduced::Wildcard => wildcards.push(index),
                        Reduced::Opaque => {}
                    }
                }
                let complete = all.is_some() && headed.iter().all(|rows| !rows.is_empty());
                if let (true, Some(all)) = (complete, &all) {
                    for (position, (constructor, field_types)) in all.iter().enumerate() {
                        let arity = field_types.len();
                        let mut next = vec![Reduced::Wildcard; arity];
                        next.extend(rest.iter().cloned());
                        let mut column_types = field_types.clone();
                        column_types.extend(types[1..].iter().cloned());
                        let rows = merged(&headed[position], &wildcards)
                            .into_iter()
                            .map(|index| &matrix[index]);
                        let specialized = self.specialize_rows(rows, constructor, arity);
                        if let Some(witness) = self.useful(&specialized, &next, &column_types) {
                            return Some(Self::rebuild(constructor.clone(), arity, witness));
                        }
                    }
                    return None;
                }
                let default = self.default_matrix(matrix);
                let mut witness = self.useful(&default, rest, &types[1..])?;
                // A constructor no row names is missing; an open type's is any other value.
                let missing = all.and_then(|all| {
                    all.into_iter()
                        .zip(&headed)
                        .find(|(_, rows)| rows.is_empty())
                        .map(|((c, fields), _)| {
                            Witness::Constructor(c, vec![Witness::Wildcard; fields.len()])
                        })
                });
                witness.insert(0, missing.unwrap_or(Witness::Wildcard));
                Some(witness)
            }
        }
    }

    /// `constructor` applied to the first `arity` witnesses, followed by the rest.
    fn rebuild(constructor: Constructor, arity: usize, mut witness: Vec<Witness>) -> Vec<Witness> {
        let rest = witness.split_off(arity.min(witness.len()));
        let mut rebuilt = vec![Witness::Constructor(constructor, witness)];
        rebuilt.extend(rest);
        rebuilt
    }

    /// The witness `witness` of a value of type `ty`, as a pattern: `Shape.Rect(_, _)`,
    /// `Point(0, _)`, `null`, `false`, `_`.
    fn render(&self, witness: &Witness, ty: &Type) -> String {
        match witness {
            Witness::Wildcard => "_".to_string(),
            Witness::Constructor(constructor, fields) => {
                let field_types = self.fields_of(constructor, ty);
                let rendered: Vec<String> = fields
                    .iter()
                    .enumerate()
                    .map(|(index, field)| {
                        let field_ty = field_types.get(index).cloned().unwrap_or(Type::Error);
                        self.render(field, &field_ty)
                    })
                    .collect();
                let arguments = if rendered.is_empty() {
                    String::new()
                } else {
                    format!("({})", rendered.join(", "))
                };
                match (constructor, peel_indirections(ty)) {
                    (Constructor::Variant(index), Type::Enum { id, .. }) => {
                        let name = self
                            .types
                            .enum_variants(id)
                            .and_then(|variants| variants.get(*index))
                            .map_or("?", |(name, _)| name.as_str());
                        format!("{}.{}{}", id.name, name, arguments)
                    }
                    (Constructor::Struct, Type::Struct { id, .. }) => {
                        format!("{}{}", id.name, arguments)
                    }
                    (Constructor::Bool(value), _) => value.to_string(),
                    (Constructor::Null, _) => "null".to_string(),
                    (Constructor::Present, _) => rendered
                        .into_iter()
                        .next()
                        .unwrap_or_else(|| "_".to_string()),
                    (Constructor::Literal(literal), _) => format!("{:?}", literal),
                    _ => "_".to_string(),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TypeChecker;
    use prism_typed_tree::{Body, ExpressionKind, Item, TypedFile};

    /// The typed tree of `src`, which must typecheck except for `E4009`, and the messages of its
    /// `E4009` errors.
    fn check(src: &str) -> (TypedFile, Vec<String>) {
        let file = prism_parser::parse_source(src).expect("source must parse");
        let mut checker = TypeChecker::new(None, "main");
        let tree = checker.check_module(&file);
        let mut missing = Vec::new();
        for error in &checker.errors {
            assert_eq!(error.code(), "E4009", "{}: {}", src, error);
            missing.push(error.message());
        }
        (tree, missing)
    }

    /// The value the only `when` of `src`'s function `f` leaves uncovered, if any.
    fn missing(src: &str) -> Option<String> {
        let (_, messages) = check(src);
        messages.first().map(|message| {
            let start = message.find(": '").expect("a witness") + 3;
            let end = message[start..].find('\'').expect("a witness") + start;
            message[start..end].to_string()
        })
    }

    fn function_body<'a>(tree: &'a TypedFile, name: &str) -> &'a Body {
        tree.items
            .iter()
            .find_map(|item| match item {
                Item::Function(f) if f.path.name == name => Some(&tree.bodies[f.body]),
                _ => None,
            })
            .expect("the function")
    }

    /// The useless patterns of the first `when` of function `f` in `src`.
    fn useless(src: &str) -> Vec<(usize, usize)> {
        let (tree, _) = check(src);
        let body = function_body(&tree, "f");
        let (subject, branches) = body
            .expressions
            .iter()
            .find_map(|(_, e)| match &e.kind {
                ExpressionKind::When {
                    subject: Some(subject),
                    branches,
                    ..
                } => Some((*subject, branches)),
                _ => None,
            })
            .expect("a when");
        let rows: Vec<Row> = branches
            .iter()
            .map(|b| Row {
                patterns: b.patterns.iter().collect(),
                guarded: b.guard.is_some(),
            })
            .collect();
        let literal = |mut id: ExpressionId| loop {
            match &body.expressions[id].kind {
                ExpressionKind::Coerce { value, .. } => id = *value,
                ExpressionKind::Literal(literal) => return Some(literal.clone()),
                _ => return None,
            }
        };
        useless_patterns(&tree.types, body.type_of(subject), &rows, &literal)
    }

    const NESTED: &str = "enum Inner { A, B }\nenum Outer { Wrap(Inner), Empty }\n";

    #[test]
    fn boxes_are_matched_as_their_contents() {
        let types = "enum Bit { Zero, One }\nenum Pair { Of(Heap<Bit>, Heap<Bit>) }\n";
        assert_eq!(
            missing(&format!(
                "{}fun f(p: &Pair): Int = when (p) {{ Pair.Of(Bit.Zero, _) -> 0; Pair.Of(Bit.One, Bit.Zero) -> 1; Pair.Of(Bit.One, Bit.One) -> 2 }}\n",
                types
            )),
            None
        );
        assert_eq!(
            missing(&format!(
                "{}fun f(p: &Pair): Int = when (p) {{ Pair.Of(Bit.Zero, _) -> 0; Pair.Of(_, Bit.Zero) -> 1 }}\n",
                types
            )),
            Some("Pair.Of(Bit.One, Bit.One)".to_string())
        );
        // A boxed subject, and a box nobody looks into, are covered by a wildcard.
        assert_eq!(
            missing("enum E { A, B(Heap<E>) }\nfun f(h: Heap<E>): Int = when (h) { E.A -> 0; E.B(_) -> 1 }\n"),
            None
        );
        assert_eq!(
            missing("enum E { A, B(Heap<E>) }\nfun f(h: Heap<E>): Int = when (h) { E.A -> 0; E.B(E.A) -> 1 }\n"),
            Some("E.B(E.B(_))".to_string())
        );
    }

    #[test]
    fn nested_enums() {
        assert_eq!(
            missing(&format!(
                "{}fun f(o: Outer): Int = when (o) {{ Outer.Wrap(Inner.A) -> 1; Outer.Wrap(Inner.B) -> 2; Outer.Empty -> 3 }}\n",
                NESTED
            )),
            None
        );
        assert_eq!(
            missing(&format!(
                "{}fun f(o: Outer): Int = when (o) {{ Outer.Wrap(Inner.A) -> 1; Outer.Empty -> 3 }}\n",
                NESTED
            )),
            Some("Outer.Wrap(Inner.B)".to_string())
        );
    }

    #[test]
    fn structs_with_literals() {
        let point = "struct Point : Copy { val x: Int, val y: Int }\n";
        assert_eq!(
            missing(&format!(
                "{}fun f(p: Point): Int = when (p) {{ Point(0, val y) -> y; Point(y = 0) -> 1; Point(val x, _) -> x }}\n",
                point
            )),
            None
        );
        assert_eq!(
            missing(&format!(
                "{}fun f(p: Point): Int = when (p) {{ Point(0, val y) -> y; Point(y = 0) -> 1 }}\n",
                point
            )),
            Some("Point(_, _)".to_string())
        );
    }

    #[test]
    fn booleans_and_nullables() {
        assert_eq!(
            missing("fun f(b: Bool?): Int = when (b) { true -> 1; false -> 2; null -> 3 }\n"),
            None
        );
        assert_eq!(
            missing("fun f(b: Bool?): Int = when (b) { true -> 1; null -> 3 }\n"),
            Some("false".to_string())
        );
        assert_eq!(
            missing("fun f(b: Bool?): Int = when (b) { true -> 1; false -> 3 }\n"),
            Some("null".to_string())
        );
        assert_eq!(
            missing("enum Flag { On(Bool), Off }\nfun f(x: Flag): Int = when (x) { Flag.On(true) -> 1; Flag.On(false) -> 2; Flag.Off -> 3 }\n"),
            None
        );
        assert_eq!(
            missing("enum Shape { Circle(Double), Square(Double) }\nfun f(s: Shape?): Double = when (s) { null -> 0.0; Shape.Circle(val r) -> r; Shape.Square(val d) -> d }\n"),
            None
        );
        assert_eq!(
            missing("enum Slot { Of(Int?) }\nfun f(s: Slot): Int = when (s) { Slot.Of(null) -> 0; Slot.Of(1) -> 1; Slot.Of(val n) -> 2 }\n"),
            None
        );
    }

    #[test]
    fn open_types_guards_and_alternatives() {
        assert_eq!(
            missing("fun f(x: Int): Int = when (x) { 0 -> 1; 1 -> 2 }\n"),
            Some("_".to_string())
        );
        assert_eq!(
            missing("enum Box { Of(Int), Empty }\nfun f(b: Box): Int = when (b) { Box.Of(0) -> 1; Box.Empty -> 2 }\n"),
            Some("Box.Of(_)".to_string())
        );
        assert_eq!(
            missing("enum Box { Of(Int), Empty }\nfun f(b: Box): Int = when (b) { Box.Of(val n) if n > 0 -> 1; Box.Empty -> 2 }\n"),
            Some("Box.Of(_)".to_string())
        );
        assert_eq!(
            missing("enum Shape { Circle(Double), Square(Double), Empty }\nfun f(s: Shape): Double = when (s) { Shape.Circle(val d), Shape.Square(val d) -> d }\n"),
            Some("Shape.Empty".to_string())
        );
        assert_eq!(
            missing("enum Never {}\nfun f(n: Never): Int = when (n) { }\n"),
            Some("_".to_string())
        );
    }

    #[test]
    fn useless_patterns_are_found() {
        assert_eq!(
            useless("enum Box { Of(Int), Empty }\nfun f(b: Box): Int = when (b) { Box.Of(val n) -> n; Box.Of(_) -> 0; Box.Empty -> 1; else -> 2 }\n"),
            vec![(1, 0), (3, 0)]
        );
        assert_eq!(
            useless("fun f(b: Bool): Int = when (b) { true -> 1; false -> 2; else -> 3 }\n"),
            vec![(2, 0)]
        );
        // A guarded branch covers nothing, so the branch after it is useful.
        assert_eq!(
            useless("enum Box { Of(Int), Empty }\nfun f(b: Box): Int = when (b) { Box.Of(val n) if n > 0 -> n; Box.Of(_) -> 0; Box.Empty -> 1 }\n"),
            Vec::<(usize, usize)>::new()
        );
    }
}

//! The stable text form of a file's MIR, printed by `prism dump-mir` (D15 of `mir-lowering`).
//!
//! After the `module` line, elaborated MIR prints `phase elaborated`. Bodies are printed in
//! order, each with a header naming its owner, its arguments and its return type, then its
//! locals (a drop flag as `(drop flag)`), then its blocks, one statement or terminator per line with the
//! line and column of the source it comes from. Places print as `_1.name` for a field (named
//! from the type table), `(*_1)` for a dereference, `(_1 as payload)` for a nullable's payload
//! and `_1[_2]` for an index; operands as `move _1`, `copy _1` or `const …`. In elaborated MIR,
//! a closure value's captures print with how the closure borrows them: `[&mut _1, &_2]`.

use crate::mir::{
    AggregateKind, Body, BodyId, BodyOwner, Callee, Capture, CaptureMode, Constant, LocalKind,
    MirFile, MirPhase, Operand, Place, Projection, Rvalue, StatementKind, TemplatePart,
    TerminatorKind,
};
use crate::types::projection_type;
use prism_ast::Span;
use prism_typed_tree::{NominalId, Type, TypeParameter, TypeTable};
use std::fmt::Write;

impl MirFile {
    /// Renders the MIR as deterministic text.
    pub fn dump(&self) -> String {
        let mut out = String::new();
        writeln!(out, "module {}", self.module).unwrap();
        match self.phase {
            MirPhase::Built => {}
            MirPhase::Elaborated => writeln!(out, "phase elaborated").unwrap(),
            MirPhase::Composed => writeln!(out, "phase composed").unwrap(),
        }
        for (id, body) in self.bodies.iter() {
            self.dump_body(&mut out, id, body);
        }
        out
    }

    /// Whether body `id` is a skippable composable: one with a restart function (D10 of
    /// `compose-transform`).
    pub fn is_skippable(&self, id: BodyId) -> bool {
        self.bodies.iter().any(
            |(_, body)| matches!(body.owner, BodyOwner::Restart { function } if function == id),
        )
    }

    /// Renders the bodies alone, without the `module` and `phase` lines, as a program's
    /// instances are printed one unit after another (D11 of `monomorphization`).
    pub fn dump_bodies(&self) -> String {
        let mut out = String::new();
        for (id, body) in self.bodies.iter() {
            self.dump_body(&mut out, id, body);
        }
        out
    }

    /// The name of body `id`: its declaration's path, `<function> <parameter>` for a default
    /// generator, `<parent>#closure<index>` for a closure. An instance's path is followed by its
    /// type arguments (`lib/box::keep<Int>`, `main::pad<Int> y`).
    pub fn body_name(&self, id: BodyId) -> String {
        match &self.bodies[id].owner {
            BodyOwner::Function(path) | BodyOwner::Value(path) => format!(
                "{}{}",
                path,
                render_type_arguments(&self.bodies[id].type_arguments)
            ),
            BodyOwner::Default { function, name, .. } => format!(
                "{}{} {}",
                function,
                render_type_arguments(&self.bodies[id].type_arguments),
                name
            ),
            BodyOwner::Closure { parent, index } => {
                format!("{}#closure{}", self.body_name(*parent), index)
            }
            BodyOwner::Restart { function } => format!("{}#restart", self.body_name(*function)),
            BodyOwner::RootContent { parent } => {
                format!("{}#root-content", self.body_name(*parent))
            }
        }
    }

    fn dump_body(&self, out: &mut String, id: BodyId, body: &Body) {
        let arguments: Vec<String> = body
            .arguments()
            .map(|local| format!("_{}: {}", local.0, body.locals[local].ty))
            .collect();
        // Composed MIR marks its composable bodies, and the skippable ones (D19 of
        // `compose-transform`).
        let compose = match (body.is_compose, self.phase) {
            (false, _) => String::new(),
            (true, MirPhase::Composed) if self.is_skippable(id) => {
                "compose composed skippable ".to_string()
            }
            (true, MirPhase::Composed) => "compose composed ".to_string(),
            (true, _) => "compose ".to_string(),
        };
        let at = position(body.span);
        match &body.owner {
            BodyOwner::Value(path) => {
                writeln!(out, "val {}: {} {}", path, body.return_type(), at).unwrap()
            }
            BodyOwner::Function(_)
            | BodyOwner::Default { .. }
            | BodyOwner::Closure { .. }
            | BodyOwner::Restart { .. }
            | BodyOwner::RootContent { .. } => {
                let keyword = match body.owner {
                    BodyOwner::Function(_) => "fun",
                    BodyOwner::Default { .. } => "default",
                    _ => "closure",
                };
                // A generic body shows its type parameters, as its function declares them.
                let type_parameters = render_type_parameters(&body.type_parameters);
                // A composable closure says where it is written (D5 of `compose-transform`).
                let origin = match &body.origin {
                    Some(origin) => format!(
                        " /* {}{} */",
                        origin
                            .region
                            .map_or_else(String::new, |region| format!("in g{}, ", region.0)),
                        origin.step.describe()
                    ),
                    None => String::new(),
                };
                writeln!(
                    out,
                    "{}{} {}{}({}): {} {}{}",
                    compose,
                    keyword,
                    type_parameters,
                    self.body_name(id),
                    arguments.join(", "),
                    body.return_type(),
                    at,
                    origin
                )
                .unwrap();
            }
        }
        for (local, declaration) in body.locals.iter() {
            let name = match &declaration.name {
                Some(name) => format!(" {}", name),
                None => String::new(),
            };
            let kind = match declaration.kind {
                LocalKind::Return => "return",
                LocalKind::Argument => "argument",
                LocalKind::Capture => "capture",
                LocalKind::User => "user",
                LocalKind::Temporary => "temporary",
                LocalKind::DropFlag => "drop flag",
            };
            let var = if declaration.mutable { ", var" } else { "" };
            writeln!(
                out,
                "  local _{}{}: {} ({}{})",
                local.0, name, declaration.ty, kind, var
            )
            .unwrap();
        }
        let printer = Printer {
            file: self,
            body,
            types: &self.types,
        };
        for (block, data) in body.blocks.iter() {
            writeln!(out, "  bb{}:", block.0).unwrap();
            for statement in &data.statements {
                let text = match &statement.kind {
                    StatementKind::Assign(place, rvalue) => {
                        format!("{} = {}", printer.place(place), printer.rvalue(rvalue))
                    }
                    StatementKind::StorageLive(local) => format!("storage_live _{}", local.0),
                    StatementKind::StorageDead(local) => format!("storage_dead _{}", local.0),
                    StatementKind::Read(place) => format!("read {}", printer.place(place)),
                    StatementKind::Destructure {
                        source,
                        variant,
                        parts,
                    } => printer.destructure(*source, *variant, parts),
                    StatementKind::GroupStart(group) => format!(
                        "group_start g{} /* {} */",
                        group.0,
                        body.groups.get(*group).map_or_else(
                            || "?".to_string(),
                            |region| if region.continued {
                                format!("{}, continued", region.step.describe())
                            } else {
                                region.step.describe()
                            }
                        )
                    ),
                    StatementKind::GroupEnd(group) => format!("group_end g{}", group.0),
                };
                writeln!(out, "    {} {}", text, position(statement.span)).unwrap();
            }
            let terminator = &data.terminator;
            let text = printer.terminator(&terminator.kind);
            writeln!(out, "    {} {}", text, position(terminator.span)).unwrap();
        }
    }
}

struct Printer<'a> {
    file: &'a MirFile,
    body: &'a Body,
    types: &'a TypeTable,
}

impl Printer<'_> {
    /// `destructure move _1 as Pair => (0: _4, 1: _5)`, or `destructure move _1 => (...)` for a
    /// struct.
    fn destructure(
        &self,
        source: crate::LocalId,
        variant: Option<usize>,
        parts: &[(usize, crate::LocalId)],
    ) -> String {
        let variant = variant.map(|variant| {
            let name = match self.body.locals.get(source).map(|l| &l.ty) {
                Some(Type::Enum { id, .. }) => self
                    .types
                    .enum_variants(id)
                    .and_then(|variants| variants.get(variant))
                    .map(|(name, _)| name.clone()),
                _ => None,
            };
            format!(" as {}", name.unwrap_or_else(|| variant.to_string()))
        });
        let parts: Vec<String> = parts
            .iter()
            .map(|(index, target)| format!("{}: _{}", index, target.0))
            .collect();
        format!(
            "destructure move _{}{} => ({})",
            source.0,
            variant.unwrap_or_default(),
            parts.join(", ")
        )
    }

    fn place(&self, place: &Place) -> String {
        let mut text = format!("_{}", place.local.0);
        let mut ty = self.body.locals.get(place.local).map(|l| l.ty.clone());
        for projection in &place.projections {
            text = match projection {
                Projection::Deref => format!("(*{})", text),
                Projection::Field(index) => {
                    let name = match &ty {
                        Some(Type::Struct { id, .. }) => self
                            .types
                            .struct_fields(id)
                            .and_then(|fields| fields.get(*index))
                            .map(|(name, _, _)| name.clone()),
                        _ => None,
                    };
                    match name {
                        Some(name) => format!("{}.{}", text, name),
                        None => format!("{}.{}", text, index),
                    }
                }
                Projection::Payload => format!("({} as payload)", text),
                Projection::VariantField { variant, field } => {
                    let name = match &ty {
                        Some(Type::Enum { id, .. }) => self
                            .types
                            .enum_variants(id)
                            .and_then(|variants| variants.get(*variant))
                            .map(|(name, _)| name.clone()),
                        _ => None,
                    };
                    let name = name.unwrap_or_else(|| variant.to_string());
                    format!("({} as {}).{}", text, name, field)
                }
                Projection::Index(local) => format!("{}[_{}]", text, local.0),
            };
            ty = ty.and_then(|ty| projection_type(&ty, *projection, self.types));
        }
        text
    }

    fn operand(&self, operand: &Operand) -> String {
        match operand {
            Operand::Copy(place, _) => format!("copy {}", self.place(place)),
            Operand::Move(place, _) => format!("move {}", self.place(place)),
            Operand::Constant(constant) => format!("const {}", self.constant(constant)),
        }
    }

    fn operands(&self, operands: &[Operand]) -> String {
        operands
            .iter()
            .map(|o| self.operand(o))
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn constant(&self, constant: &Constant) -> String {
        match constant {
            Constant::Int(v) => v.to_string(),
            Constant::Float(v) => format!("{:?}", v),
            Constant::Bool(v) => v.to_string(),
            Constant::Char(v) => format!("{:?}", v),
            Constant::String(v) => format!("{:?}", v),
            Constant::Unit => "()".to_string(),
            Constant::Null(_) => "null".to_string(),
            Constant::TopLevelValue(path) => format!("&{}", path),
        }
    }

    fn variant(&self, enum_id: &NominalId, index: usize) -> String {
        let name = self
            .types
            .enum_variants(enum_id)
            .and_then(|variants| variants.get(index))
            .map_or_else(|| index.to_string(), |(name, _)| name.clone());
        format!("{}::{}.{}", enum_id.module, enum_id.name, name)
    }

    fn rvalue(&self, rvalue: &Rvalue) -> String {
        match rvalue {
            Rvalue::Use(operand) => self.operand(operand),
            Rvalue::Borrow {
                mutable,
                origin,
                place,
            } => format!(
                "&{}{} ({})",
                if *mutable { "mut " } else { "" },
                self.place(place),
                origin.as_str()
            ),
            Rvalue::Binary(operator, left, right) => format!(
                "{} {} {}",
                self.operand(left),
                operator.as_str(),
                self.operand(right)
            ),
            Rvalue::Unary(operator, operand) => {
                format!("{}{}", operator.as_str(), self.operand(operand))
            }
            Rvalue::Bits(operator, left, right) => format!(
                "bits {} {} {}",
                self.operand(left),
                operator.as_str(),
                self.operand(right)
            ),
            Rvalue::Discriminant(place) => format!("discriminant {}", self.place(place)),
            Rvalue::Aggregate(kind, operands) => {
                let name = match kind {
                    AggregateKind::Struct { .. } => format!("struct {}", kind.ty().qualified()),
                    AggregateKind::Variant {
                        enum_id,
                        index,
                        arguments,
                    } => format!(
                        "variant {}{}",
                        self.variant(enum_id, *index),
                        render_type_arguments(arguments)
                    ),
                };
                format!("{}({})", name, self.operands(operands))
            }
            Rvalue::Template(parts) => {
                let parts: Vec<String> = parts
                    .iter()
                    .map(|part| match part {
                        TemplatePart::Text(text) => format!("{:?}", text),
                        TemplatePart::Value(operand) => self.operand(operand),
                    })
                    .collect();
                format!("template({})", parts.join(", "))
            }
            Rvalue::WrapNullable(operand) => format!("wrap {}", self.operand(operand)),
            Rvalue::Subtype(operand) => format!("subtype {}", self.operand(operand)),
            Rvalue::Closure { body, captures } => {
                // An owned capture is an operand, `move _1` or `copy _2`; elaborated MIR prints
                // how the closure borrows each other capture (D10 of `borrowck-exclusivity`).
                let modes = match self.file.phase {
                    MirPhase::Elaborated | MirPhase::Composed => self
                        .file
                        .bodies
                        .get(*body)
                        .map(|closure| closure.capture_modes.as_slice())
                        .unwrap_or_default(),
                    MirPhase::Built => &[],
                };
                let captures: Vec<String> = captures
                    .iter()
                    .enumerate()
                    .map(|(index, capture)| match (capture, modes.get(index)) {
                        (Capture::Owned(operand), _) => self.operand(operand),
                        (Capture::Borrowed(place), Some(CaptureMode::Shared)) => {
                            format!("&{}", self.place(place))
                        }
                        (Capture::Borrowed(place), Some(CaptureMode::Mutable)) => {
                            format!("&mut {}", self.place(place))
                        }
                        (Capture::Borrowed(place), _) => self.place(place),
                    })
                    .collect();
                format!(
                    "closure {} [{}]",
                    self.file.body_name(*body),
                    captures.join(", ")
                )
            }
        }
    }

    fn terminator(&self, kind: &TerminatorKind) -> String {
        match kind {
            TerminatorKind::Goto(target) => format!("goto -> bb{}", target.0),
            TerminatorKind::SwitchInt {
                value,
                targets,
                otherwise,
            } => {
                let mut arms: Vec<String> = targets
                    .iter()
                    .map(|(value, target)| format!("{} -> bb{}", value, target.0))
                    .collect();
                arms.push(format!("otherwise -> bb{}", otherwise.0));
                format!("switch {} [{}]", self.operand(value), arms.join(", "))
            }
            TerminatorKind::Call {
                callee,
                arguments,
                type_arguments,
                destination,
                target,
                is_compose,
                ..
            } => {
                let callee = match callee {
                    Callee::Function(path) => {
                        format!("{}{}", path, render_type_arguments(type_arguments))
                    }
                    Callee::External(path) => format!("external {}", path),
                    Callee::Builtin(builtin) => format!(
                        "builtin {}{}",
                        builtin.name(),
                        render_type_arguments(type_arguments)
                    ),
                    Callee::Value(place, _) => self.place(place),
                    Callee::Default { function, name, .. } => format!(
                        "default {}{} {}",
                        function,
                        render_type_arguments(type_arguments),
                        name
                    ),
                    Callee::Composer(operation) => composer_operation(operation),
                    Callee::State(operation) => format!(
                        "state {}{}",
                        operation.name(),
                        render_type_arguments(type_arguments)
                    ),
                };
                let target = match target {
                    Some(target) => format!(" -> bb{}", target.0),
                    None => String::new(),
                };
                format!(
                    "{} = call {}{}({}){}",
                    self.place(destination),
                    if *is_compose { "compose " } else { "" },
                    callee,
                    self.operands(arguments),
                    target
                )
            }
            TerminatorKind::Drop { place, target } => {
                format!("drop {} -> bb{}", self.place(place), target.0)
            }
            TerminatorKind::Return => "return".to_string(),
            TerminatorKind::Unreachable => "unreachable".to_string(),
            TerminatorKind::Remembered {
                composer,
                slot_type,
                destination,
                found,
                missing,
            } => format!(
                "{} = remembered<{}>({}) -> [found: bb{}, missing: bb{}]",
                self.place(destination),
                slot_type.qualified(),
                self.operand(composer),
                found.0,
                missing.0
            ),
        }
    }
}

/// A composer operation as a callee: `composer start_group key 0x3f9a0c12d4e5b678 /* if 0
/// then */`, `composer changed<Int>`, `composer kept_parameter 0<Int>`.
fn composer_operation(operation: &crate::mir::ComposerOperation) -> String {
    use crate::mir::ComposerOperation as Operation;
    let detail = match operation {
        Operation::StartRestartGroup { key }
        | Operation::StartGroup { key }
        | Operation::StartMovableGroup { key }
        | Operation::StartNodeGroup { key } => {
            format!(" key 0x{:016x} /* {} */", key.hash, key.label)
        }
        Operation::Changed(ty) | Operation::SetSlot(ty) | Operation::Keep(ty) => {
            format!("<{}>", ty.qualified())
        }
        Operation::KeptParameter { index, ty } => format!(" {}<{}>", index, ty.qualified()),
        _ => String::new(),
    };
    format!("composer {}{}", operation.name(), detail)
}

/// Renders a body's type parameters as declared, followed by a space (`<T : Copy> `), or nothing
/// for a body that is not generic.
fn render_type_parameters(parameters: &[TypeParameter]) -> String {
    if parameters.is_empty() {
        return String::new();
    }
    format!("{} ", TypeParameter::render_list(parameters))
}

/// Renders type arguments as written after a name (`<Int, String>`), qualified by module, or
/// nothing when there are none.
fn render_type_arguments(arguments: &[Type]) -> String {
    if arguments.is_empty() {
        return String::new();
    }
    let rendered: Vec<String> = arguments.iter().map(Type::qualified).collect();
    format!("<{}>", rendered.join(", "))
}

fn position(span: Span) -> String {
    format!("@{}:{}", span.line, span.column)
}

#[cfg(test)]
pub(crate) mod tests {
    use crate::mir::*;
    use crate::types::mir_type_table;
    use prism_ast::Span;
    use prism_typed_tree::{Arena, DeclarationPath, NominalId, Type, TypeDef, TypeTable};

    pub(crate) fn span(line: u32, column: u32) -> Span {
        Span::new(0, 0, line, column)
    }

    pub(crate) fn local(ty: Type, kind: LocalKind, name: Option<&str>) -> LocalDeclaration {
        LocalDeclaration {
            ty,
            kind,
            mutable: false,
            name: name.map(str::to_string),
            binding: None,
            escaping: false,
            span: span(1, 1),
        }
    }

    pub(crate) fn statement(kind: StatementKind) -> Statement {
        Statement {
            kind,
            span: span(2, 5),
        }
    }

    pub(crate) fn terminator(kind: TerminatorKind) -> Terminator {
        Terminator {
            kind,
            span: span(2, 5),
        }
    }

    pub(crate) fn types() -> TypeTable {
        let mut types = mir_type_table(&TypeTable::with_builtins());
        types.insert_definition(
            NominalId::new("main", "User"),
            TypeDef::Struct {
                parameters: Vec::new(),
                fields: vec![("name".to_string(), false, Type::String)],
            },
        );
        types.insert_marker(NominalId::new("main", "User"), None, Vec::new());
        types
    }

    /// `fun f(u: &User): String { val n = "x"; return n }`, lowered by hand:
    ///
    /// ```text
    /// bb0: storage_live _2; _2 = const "x"; _0 = move _2; drop _2 -> bb1
    /// bb1: storage_dead _2; return
    /// ```
    pub(crate) fn small_file() -> MirFile {
        let user = Type::plain_struct(NominalId::new("main", "User"));
        let mut locals = Arena::new();
        locals.alloc(local(Type::String, LocalKind::Return, None));
        locals.alloc(local(
            Type::Reference {
                mutable: false,
                inner: Box::new(user),
            },
            LocalKind::Argument,
            Some("u"),
        ));
        locals.alloc(local(Type::String, LocalKind::User, Some("n")));
        let n = LocalId(2);
        let mut blocks = Arena::new();
        blocks.alloc(BasicBlock {
            statements: vec![
                statement(StatementKind::StorageLive(n)),
                statement(StatementKind::Assign(
                    Place::local(n),
                    Box::new(Rvalue::Use(Operand::Constant(Constant::String(
                        "x".to_string(),
                    )))),
                )),
                statement(StatementKind::Read(
                    Place::local(LocalId(1))
                        .project(Projection::Deref)
                        .project(Projection::Field(0)),
                )),
                statement(StatementKind::Assign(
                    Place::return_place(),
                    Box::new(Rvalue::Use(Operand::Move(Place::local(n), Span::default()))),
                )),
            ],
            terminator: terminator(TerminatorKind::Drop {
                place: Place::local(n),
                target: BlockId(1),
            }),
        });
        blocks.alloc(BasicBlock {
            statements: vec![statement(StatementKind::StorageDead(n))],
            terminator: terminator(TerminatorKind::Return),
        });
        let body = Body {
            owner: BodyOwner::Function(DeclarationPath::new("main", "f")),
            is_compose: false,
            type_parameters: Vec::new(),
            type_arguments: Vec::new(),
            locals,
            argument_count: 1,
            capture_count: 0,
            capture_modes: Vec::new(),
            ownership_reason: None,
            blocks,
            groups: Arena::new(),
            origin: None,
            body_key: None,
            span: span(1, 1),
        };
        let mut bodies = Arena::new();
        bodies.alloc(body);
        MirFile {
            module: "main".to_string(),
            phase: MirPhase::Built,
            types: types(),
            bodies,
            externals: Vec::new(),
        }
    }

    #[test]
    fn dumps_a_small_body() {
        let file = small_file();
        let expected = "\
module main
fun main::f(_1: &User): String @1:1
  local _0: String (return)
  local _1 u: &User (argument)
  local _2 n: String (user)
  bb0:
    storage_live _2 @2:5
    _2 = const \"x\" @2:5
    read (*_1).name @2:5
    _0 = move _2 @2:5
    drop _2 -> bb1 @2:5
  bb1:
    storage_dead _2 @2:5
    return @2:5
";
        assert_eq!(file.dump(), expected);
        assert_eq!(file.dump(), file.dump());
    }

    /// `small_file`, elaborated by hand: the drop of `n` is guarded by a drop flag.
    pub(crate) fn elaborated_file() -> MirFile {
        let mut file = small_file();
        file.phase = MirPhase::Elaborated;
        let body = &mut file.bodies[BodyId(0)];
        let flag = body
            .locals
            .alloc(local(Type::Bool, LocalKind::DropFlag, None));
        let set = |value| {
            statement(StatementKind::assign(
                Place::local(flag),
                Rvalue::Use(Operand::Constant(Constant::Bool(value))),
            ))
        };
        let entry = &mut body.blocks[BlockId(0)].statements;
        entry.insert(0, set(false));
        entry.insert(3, set(true));
        entry.insert(5, set(false));
        let drop = body.blocks[BlockId(0)].terminator.clone();
        body.blocks[BlockId(0)].terminator = terminator(TerminatorKind::SwitchInt {
            value: Operand::Copy(Place::local(flag), Span::default()),
            targets: vec![(0, BlockId(1))],
            otherwise: BlockId(2),
        });
        body.blocks.alloc(BasicBlock {
            statements: vec![],
            terminator: drop,
        });
        file
    }

    #[test]
    fn dumps_an_elaborated_body() {
        let expected = "\
module main
phase elaborated
fun main::f(_1: &User): String @1:1
  local _0: String (return)
  local _1 u: &User (argument)
  local _2 n: String (user)
  local _3: Bool (drop flag)
  bb0:
    _3 = const false @2:5
    storage_live _2 @2:5
    _2 = const \"x\" @2:5
    _3 = const true @2:5
    read (*_1).name @2:5
    _3 = const false @2:5
    _0 = move _2 @2:5
    switch copy _3 [0 -> bb1, otherwise -> bb2] @2:5
  bb1:
    storage_dead _2 @2:5
    return @2:5
  bb2:
    drop _2 -> bb1 @2:5
";
        assert_eq!(elaborated_file().dump(), expected);
    }

    #[test]
    fn goto_chains_are_merged() {
        let mut file = small_file();
        let body = &mut file.bodies[BodyId(0)];
        // bb0 drops into bb1, which now jumps to a new bb2, which jumps to a new bb3 that
        // returns: bb1, bb2 and bb3 merge into one block.
        let ret = body.blocks[BlockId(1)].terminator.clone();
        body.blocks[BlockId(1)].terminator = terminator(TerminatorKind::Goto(BlockId(2)));
        body.blocks.alloc(BasicBlock {
            statements: vec![statement(StatementKind::Read(Place::local(LocalId(1))))],
            terminator: terminator(TerminatorKind::Goto(BlockId(3))),
        });
        body.blocks.alloc(BasicBlock {
            statements: vec![],
            terminator: ret,
        });
        crate::merge_goto_blocks(body);
        assert_eq!(body.blocks.len(), 2);
        let last = &body.blocks[BlockId(1)];
        assert_eq!(last.statements.len(), 2);
        assert_eq!(last.terminator.kind, TerminatorKind::Return);
        assert_eq!(file.dump(), file.dump());
        assert!(file.validate().is_empty(), "{:?}", file.validate());
    }
}

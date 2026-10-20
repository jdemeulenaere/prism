//! Restart functions and root content adapters (D10 and D15 of `compose-transform`).
//!
//! A skippable composable `F` gets `F#restart`, a closure capturing nothing of type
//! `(Int) -> Unit`: it reads each kept argument from the slots of its scope's group, in
//! parameter order, and calls `F` with them, the composer it was given and change words with
//! the force bit and every parameter the same. Ending a restart group with it allocates
//! nothing. A body calling `composition` or `setContent` gets `<symbol>#root-content`, a closure
//! owning the content, which it calls with the composer and the force bit.

use crate::rewrite::{bit_position, word_count, FORCE, SAME};
use prism_ast::Span;
use prism_mir::{
    BasicBlock, Body, BodyId, BodyOwner, Callee, CaptureMode, ComposerOperation, Constant,
    LocalDeclaration, LocalId, LocalKind, Operand, Place, Rvalue, Statement, StatementKind,
    Terminator, TerminatorKind,
};
use prism_typed_tree::ParameterKind;
use prism_typed_tree::{Arena, Type};

fn local(ty: Type, kind: LocalKind, name: Option<String>, span: Span) -> LocalDeclaration {
    LocalDeclaration {
        ty,
        kind,
        mutable: false,
        name,
        binding: None,
        parameter_kind: ParameterKind::Plain,
        span,
    }
}

fn block(statements: Vec<Statement>, kind: TerminatorKind, span: Span) -> BasicBlock {
    BasicBlock {
        statements,
        terminator: Terminator { kind, span },
    }
}

/// The restart function of the skippable composable `function`, whose body is `original` as
/// elaborated, with `declared` parameters.
pub(crate) fn restart_body(
    original: &Body,
    function: BodyId,
    composed: &Body,
    declared: usize,
    versioned: &[bool],
    types: &prism_typed_tree::TypeTable,
    span: Span,
) -> Body {
    let BodyOwner::Function(path) = &original.owner else {
        panic!("internal error: a restart function of a body that is not a function");
    };
    let mut locals = Arena::new();
    locals.alloc(local(Type::Unit, LocalKind::Return, None, span));
    let composer = locals.alloc(local(
        Type::Int,
        LocalKind::Argument,
        Some("$composer".to_string()),
        span,
    ));
    let parameter_types: Vec<Type> = (1..=declared)
        .map(|index| composed.locals[LocalId(index as u32)].ty.clone())
        .collect();
    // A borrowed state is kept as its weak handle, which the call borrows viewed as the state
    // object (D13 of `owned-state`).
    let kept_types: Vec<Type> = parameter_types
        .iter()
        .map(|ty| crate::stability::kept_type(ty, types))
        .collect();
    let kept: Vec<LocalId> = kept_types
        .iter()
        .enumerate()
        .map(|(index, ty)| {
            let name = composed.locals[LocalId((index + 1) as u32)].name.clone();
            locals.alloc(local(ty.clone(), LocalKind::Temporary, name, span))
        })
        .collect();
    // A kept identity's version, read beside its reference (D7 of `strong-skipping`). A
    // borrowed state object takes a version word it does not use, passed as `0` (D21).
    let versions: Vec<Option<LocalId>> = versioned
        .iter()
        .zip(&parameter_types)
        .map(|(versioned, ty)| {
            (*versioned && !crate::stability::is_borrowed_state(ty, types))
                .then(|| locals.alloc(local(Type::Int, LocalKind::Temporary, None, span)))
        })
        .collect();
    let viewed: Vec<Option<LocalId>> = parameter_types
        .iter()
        .map(|ty| {
            crate::stability::is_borrowed_state(ty, types)
                .then(|| locals.alloc(local(ty.clone(), LocalKind::Temporary, None, span)))
        })
        .collect();
    let copy = |local: LocalId| Operand::Copy(Place::local(local), span);
    let mut blocks = Arena::new();
    let lives: Vec<Statement> = kept
        .iter()
        .chain(versions.iter().flatten())
        .chain(viewed.iter().flatten())
        .map(|local| Statement {
            kind: StatementKind::StorageLive(*local),
            span,
        })
        .collect();
    // One block per kept argument, read in parameter order, and one per kept version.
    let mut reads: Vec<(ComposerOperation, LocalId)> = Vec::new();
    for (index, (local, ty)) in kept.iter().zip(&kept_types).enumerate() {
        reads.push((
            ComposerOperation::KeptParameter {
                index: index as u32,
                ty: ty.clone(),
            },
            *local,
        ));
        if let Some(version) = versions[index] {
            reads.push((
                ComposerOperation::KeptVersion {
                    index: index as u32,
                    ty: ty.clone(),
                },
                version,
            ));
        }
    }
    for (position, (operation, destination)) in reads.into_iter().enumerate() {
        let statements = if position == 0 {
            lives.clone()
        } else {
            Vec::new()
        };
        blocks.alloc(block(
            statements,
            TerminatorKind::Call {
                callee: Callee::Composer(operation),
                arguments: vec![copy(composer)],
                parameters: vec![Type::Int],
                parameter_kinds: vec![ParameterKind::Plain],
                type_arguments: Vec::new(),
                destination: Place::local(destination),
                target: Some(prism_mir::BlockId((position + 1) as u32)),
                is_compose: false,
            },
            span,
        ));
    }
    // The call of the composable: the kept arguments, the composer, and the force bit with
    // every parameter the same.
    let words = word_count(declared);
    let mut constants = vec![0i64; words];
    constants[0] |= FORCE;
    for index in 0..declared {
        let (word, shift) = bit_position(index);
        constants[word] |= SAME << shift;
    }
    let mut borrows = Vec::new();
    let mut arguments: Vec<Operand> = kept
        .iter()
        .zip(&parameter_types)
        .zip(&viewed)
        .map(|((local, ty), viewed)| {
            if let Some(viewed) = viewed {
                borrows.push(Statement {
                    kind: StatementKind::assign(
                        Place::local(*viewed),
                        Rvalue::Borrow {
                            mutable: false,
                            origin: prism_mir::BorrowOrigin::Argument,
                            place: Place::local(*local).project(prism_mir::Projection::View),
                        },
                    ),
                    span,
                });
                copy(*viewed)
            } else if crate::stability::is_function_parameter(ty) {
                Operand::Move(Place::local(*local), span)
            } else {
                copy(*local)
            }
        })
        .collect();
    arguments.push(copy(composer));
    arguments.extend(
        constants
            .into_iter()
            .map(|word| Operand::Constant(Constant::Int(word))),
    );
    let version_words = versioned.iter().filter(|versioned| **versioned).count();
    arguments.extend(
        versioned
            .iter()
            .zip(&versions)
            .filter(|(versioned, _)| **versioned)
            .map(|(_, version)| match version {
                Some(version) => copy(*version),
                None => Operand::Constant(Constant::Int(0)),
            }),
    );
    let added = 1 + words + version_words;
    let mut parameters = original
        .arguments()
        .map(|local| original.locals[local].ty.clone())
        .collect::<Vec<_>>();
    parameters.extend(std::iter::repeat_n(Type::Int, added));
    let parameter_kinds = original
        .arguments()
        .map(|local| original.locals[local].parameter_kind)
        .chain(std::iter::repeat_n(ParameterKind::Plain, added))
        .collect();
    let type_arguments = original
        .type_parameters
        .iter()
        .map(|parameter| parameter.as_type())
        .collect();
    let call_block = blocks.len();
    let mut statements = if declared == 0 { lives } else { Vec::new() };
    statements.extend(borrows);
    blocks.alloc(block(
        statements,
        TerminatorKind::Call {
            callee: Callee::Function(path.clone()),
            arguments,
            parameters,
            parameter_kinds,
            type_arguments,
            destination: Place::return_place(),
            target: Some(prism_mir::BlockId((call_block + 1) as u32)),
            is_compose: true,
        },
        span,
    ));
    let deads = kept
        .iter()
        .chain(versions.iter().flatten())
        .chain(viewed.iter().flatten())
        .rev()
        .map(|local| Statement {
            kind: StatementKind::StorageDead(*local),
            span,
        })
        .collect();
    blocks.alloc(block(deads, TerminatorKind::Return, span));
    Body {
        owner: BodyOwner::Restart { function },
        is_compose: false,
        type_parameters: original.type_parameters.clone(),
        type_arguments: Vec::new(),
        locals,
        argument_count: 1,
        capture_count: 0,
        capture_modes: Vec::new(),
        ownership_reason: None,
        capture_widenings: Vec::new(),
        blocks,
        groups: Arena::new(),
        origin: None,
        body_key: None,
        span,
    }
}

/// The root content adapter of the body `parent`, whose content has type `content_type`.
pub(crate) fn root_content_body(
    parent_body: &Body,
    parent: BodyId,
    content_type: Type,
    span: Span,
) -> Body {
    let mut locals = Arena::new();
    locals.alloc(local(Type::Unit, LocalKind::Return, None, span));
    let composer = locals.alloc(local(
        Type::Int,
        LocalKind::Argument,
        Some("$composer".to_string()),
        span,
    ));
    let content = locals.alloc(local(
        content_type,
        LocalKind::Capture,
        Some("content".to_string()),
        span,
    ));
    let mut blocks = Arena::new();
    blocks.alloc(block(
        Vec::new(),
        TerminatorKind::Call {
            callee: Callee::Value(Place::local(content), span),
            arguments: vec![
                Operand::Copy(Place::local(composer), span),
                Operand::Constant(Constant::Int(FORCE)),
            ],
            parameters: vec![Type::Int, Type::Int],
            parameter_kinds: vec![ParameterKind::Plain, ParameterKind::Plain],
            type_arguments: Vec::new(),
            destination: Place::return_place(),
            target: Some(prism_mir::BlockId(1)),
            is_compose: true,
        },
        span,
    ));
    blocks.alloc(block(Vec::new(), TerminatorKind::Return, span));
    Body {
        owner: BodyOwner::RootContent { parent },
        is_compose: false,
        type_parameters: parent_body.type_parameters.clone(),
        type_arguments: Vec::new(),
        locals,
        argument_count: 1,
        capture_count: 1,
        capture_modes: vec![CaptureMode::Owned],
        ownership_reason: None,
        capture_widenings: Vec::new(),
        blocks,
        groups: Arena::new(),
        origin: None,
        body_key: None,
        span,
    }
}

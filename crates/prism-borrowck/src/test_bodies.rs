//! Bodies built by hand for the unit tests of the framework.

use prism_ast::Span;
use prism_mir::{
    BasicBlock, BlockId, Body, BodyOwner, Constant, LocalDeclaration, LocalKind, Operand,
    Terminator, TerminatorKind,
};
use prism_typed_tree::{Arena, DeclarationPath, Type};

/// A body whose block `i` jumps to `successors[i]`: nowhere (a return), one block (a goto) or
/// several (a switch on a constant).
pub(crate) fn body_with_edges(successors: &[&[u32]]) -> Body {
    let mut locals = Arena::new();
    locals.alloc(LocalDeclaration {
        ty: Type::Unit,
        kind: LocalKind::Return,
        mutable: false,
        name: None,
        binding: None,
        span: Span::dummy(),
    });
    let mut blocks = Arena::new();
    for targets in successors {
        let kind = match targets {
            [] => TerminatorKind::Return,
            [target] => TerminatorKind::Goto(BlockId(*target)),
            [first, rest @ ..] => TerminatorKind::SwitchInt {
                value: Operand::Constant(Constant::Bool(true)),
                targets: rest
                    .iter()
                    .enumerate()
                    .map(|(value, target)| (value as u64, BlockId(*target)))
                    .collect(),
                otherwise: BlockId(*first),
            },
        };
        blocks.alloc(BasicBlock {
            statements: Vec::new(),
            terminator: Terminator {
                kind,
                span: Span::dummy(),
            },
        });
    }
    Body {
        owner: BodyOwner::Function(DeclarationPath::new("main", "f")),
        is_compose: false,
        locals,
        argument_count: 0,
        capture_count: 0,
        capture_modes: Vec::new(),
        blocks,
        span: Span::dummy(),
    }
}

/// The MIR of `source`, a file `main` that must typecheck.
pub(crate) fn lower(source: &str) -> prism_mir::MirFile {
    let file = prism_parser::parse_source(source).expect("the source parses");
    let mut checker = prism_typechecker::TypeChecker::new(None, "main");
    let tree = checker.check_module(&file);
    assert!(
        checker.errors.is_empty(),
        "type errors: {:?}",
        checker.errors
    );
    prism_mir::lower_file(&tree)
}

/// The body of the function `name` of `file`.
pub(crate) fn body<'a>(file: &'a prism_mir::MirFile, name: &str) -> &'a Body {
    let (_, body) = file
        .bodies
        .iter()
        .find(|(id, _)| file.body_name(*id) == format!("main::{}", name))
        .unwrap_or_else(|| panic!("no body main::{} in\n{}", name, file.dump()));
    body
}

/// The elaborated MIR of `source`, a file `main` that must typecheck and pass the move check,
/// with its capture modes, before any exclusivity check.
pub(crate) fn elaborate(source: &str) -> prism_mir::MirFile {
    crate::check_moves_and_elaborate(&lower(source))
        .unwrap_or_else(|errors| panic!("move errors: {:?}", errors))
        .0
}

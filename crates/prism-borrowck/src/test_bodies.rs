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
        blocks,
        span: Span::dummy(),
    }
}

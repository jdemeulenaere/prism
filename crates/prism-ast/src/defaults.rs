//! What a parameter's default value reads of the inputs its generator may take: the context
//! parameter, the receiver and the parameters before it, in that order (D26 of `ui-toolkit`).
//!
//! The answer is syntactic, so that Phase 1 can put it in a file's ABI and a caller in another
//! file borrows exactly what the generator takes: an earlier parameter or the context is read
//! when its name appears; the receiver and the context are read when `this` or any other name
//! appears, since a name may be a member of the implicit receiver or a call needing the
//! context. Constructs that bind names or hold statements (blocks, closures, `when`, jumps)
//! read every input. The answer may only be larger than what the type checker resolves, never
//! smaller, since a generator binds only the inputs it takes.

use crate::expr::{Expr, ExprKind, TemplatePart};

/// The inputs a default value reads, by position among its generator's possible inputs: bit
/// `i` for position `i`. [`DefaultInputs::ALL`] reads every input, at any position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DefaultInputs(pub u64);

impl DefaultInputs {
    /// Every input: what a default that is not analysed reads.
    pub const ALL: DefaultInputs = DefaultInputs(u64::MAX);

    /// No input: a constant default.
    pub const NONE: DefaultInputs = DefaultInputs(0);

    /// Whether the input at `position` is read.
    pub fn reads(self, position: usize) -> bool {
        if position < 64 {
            self.0 & (1 << position) != 0
        } else {
            self == DefaultInputs::ALL
        }
    }
}

/// The inputs `default` reads, for a function with a context parameter named `context` (if
/// any) and a receiver (if `receiver`), whose parameters before this one are `earlier`.
pub fn default_inputs(
    default: &Expr,
    context: Option<&str>,
    receiver: bool,
    earlier: &[String],
) -> DefaultInputs {
    let leading = usize::from(context.is_some()) + usize::from(receiver);
    if leading + earlier.len() >= 64 {
        return DefaultInputs::ALL;
    }
    let mut reader = Reader {
        context,
        receiver,
        earlier,
        bits: 0,
    };
    if reader.expression(default) {
        DefaultInputs(reader.bits)
    } else {
        DefaultInputs::ALL
    }
}

struct Reader<'a> {
    context: Option<&'a str>,
    receiver: bool,
    earlier: &'a [String],
    bits: u64,
}

impl Reader<'_> {
    /// Records what `expression` reads; `false` when it holds a construct that is not
    /// analysed.
    fn expression(&mut self, expression: &Expr) -> bool {
        match &expression.kind {
            ExprKind::Literal(_) => true,
            ExprKind::Ident(name) => {
                self.name(name);
                true
            }
            ExprKind::This => {
                self.implicit_receiver();
                true
            }
            ExprKind::Binary { left, right, .. } => self.expression(left) && self.expression(right),
            ExprKind::Unary { expr, .. } => self.expression(expr),
            ExprKind::Call { callee, args, .. } => {
                self.expression(callee) && args.iter().all(|arg| self.expression(&arg.value))
            }
            ExprKind::MemberAccess { target, .. } => self.expression(target),
            ExprKind::If {
                cond,
                then_branch,
                else_branch,
            } => {
                self.expression(cond)
                    && self.expression(then_branch)
                    && else_branch
                        .as_ref()
                        .is_none_or(|branch| self.expression(branch))
            }
            ExprKind::Index { target, indices } => {
                self.expression(target) && indices.iter().all(|index| self.expression(index))
            }
            ExprKind::Template(parts) => parts.iter().all(|part| match part {
                TemplatePart::Text(_) => true,
                TemplatePart::Expression(part) => self.expression(part),
            }),
            ExprKind::Block(_)
            | ExprKind::When { .. }
            | ExprKind::Return(_)
            | ExprKind::Break
            | ExprKind::Continue
            | ExprKind::Closure { .. } => false,
        }
    }

    /// Records a name: an earlier parameter or the context by its name, and otherwise what a
    /// member of the implicit receiver or a call needing the context reads.
    fn name(&mut self, name: &str) {
        let leading = usize::from(self.context.is_some()) + usize::from(self.receiver);
        if let Some(index) = self.earlier.iter().position(|earlier| earlier == name) {
            self.bits |= 1 << (leading + index);
        } else if self.context == Some(name) {
            self.bits |= 1;
        } else {
            self.implicit_receiver();
        }
    }

    /// Records the context and the receiver, which an implicit member or call may read.
    fn implicit_receiver(&mut self) {
        if self.context.is_some() {
            self.bits |= 1;
        }
        if self.receiver {
            self.bits |= 1 << usize::from(self.context.is_some());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::span::Span;

    fn ident(name: &str) -> Expr {
        Expr::new(ExprKind::Ident(name.to_string()), Span::dummy())
    }

    fn member(target: Expr, name: &str) -> Expr {
        Expr::new(
            ExprKind::MemberAccess {
                target: Box::new(target),
                member: name.to_string(),
                is_safe_nav: false,
            },
            Span::dummy(),
        )
    }

    fn int(value: i64) -> Expr {
        Expr::new(
            ExprKind::Literal(crate::expr::Literal::Int(value)),
            Span::dummy(),
        )
    }

    fn names(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    #[test]
    fn constants_read_nothing() {
        let dp = member(int(0), "dp");
        assert_eq!(
            default_inputs(&dp, None, true, &names(&["all"])),
            DefaultInputs::NONE
        );
        let theme = member(member(ident("MaterialTheme"), "typography"), "bodyLarge");
        assert_eq!(
            default_inputs(&theme, None, false, &names(&["text"])),
            DefaultInputs::NONE
        );
    }

    #[test]
    fn earlier_parameters_are_read_by_name() {
        let inputs = default_inputs(&ident("all"), None, true, &names(&["all", "horizontal"]));
        assert_eq!(inputs, DefaultInputs(0b10));
        assert!(inputs.reads(1) && !inputs.reads(0) && !inputs.reads(2));
    }

    #[test]
    fn other_names_read_the_receiver_and_the_context() {
        let theme = member(ident("MaterialTheme"), "colorScheme");
        assert_eq!(
            default_inputs(&theme, Some("scope"), true, &names(&["a"])),
            DefaultInputs(0b11)
        );
        assert_eq!(
            default_inputs(&ident("scope"), Some("scope"), true, &names(&["a"])),
            DefaultInputs(0b1)
        );
    }

    #[test]
    fn blocks_read_everything() {
        let block = Expr::new(
            ExprKind::Block(crate::Block {
                statements: Vec::new(),
                tail: None,
                span: Span::dummy(),
                close_brace: Span::dummy(),
            }),
            Span::dummy(),
        );
        assert_eq!(
            default_inputs(&block, None, false, &names(&["a"])),
            DefaultInputs::ALL
        );
        assert!(DefaultInputs::ALL.reads(70));
    }
}

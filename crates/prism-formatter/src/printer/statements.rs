//! Blocks, statements, loops and the right side of `=`.

use super::{CommaList, Printer};
use crate::document::*;
use prism_ast::*;
use prism_lexer::TokenKind;

/// An item of a block: a statement or the tail expression.
enum BlockItem<'a> {
    Statement(&'a Statement),
    Tail(&'a Expr),
}

impl<'a> Printer<'a> {
    /// Whether `block` was written with braces (an unbraced branch or loop body was not).
    pub(crate) fn is_braced(&self, block: &Block) -> bool {
        self.kind_at(block.span.start) == Some(&TokenKind::LBrace)
            && block.close_brace != block.span
    }

    /// A braced block, always broken over lines, or `{}`.
    pub(crate) fn block(&mut self, block: &Block) -> Document {
        if !self.is_braced(block) {
            let leading = self.comments_before(block.span.start);
            return concat(vec![leading, self.unbraced_body(block)]);
        }
        // Comments before the `{` stay before it only when they are all inline; one that ends
        // its line would put the `{` on a line of its own, so they all move inside the block.
        let leading = if self.comments_end_a_line_before(block.span.start) {
            Document::Empty
        } else {
            self.comments_before(block.span.start)
        };
        let items = self.block_items(block, block.close_brace.start);
        concat(vec![leading, self.braced(items)])
    }

    /// The items of a block or closure body, one per line, with their comments.
    pub(crate) fn block_items(&mut self, block: &Block, close_offset: usize) -> Option<Document> {
        let mut items: Vec<BlockItem> = block.statements.iter().map(BlockItem::Statement).collect();
        if let Some(tail) = &block.tail {
            items.push(BlockItem::Tail(tail));
        }
        self.vertical_list(
            &items,
            |item| match item {
                BlockItem::Statement(statement) => statement.span,
                BlockItem::Tail(tail) => tail.span,
            },
            |printer, item| match item {
                BlockItem::Statement(statement) => printer.statement(statement),
                BlockItem::Tail(tail) => printer.expression_statement(tail),
            },
            close_offset,
            |_| false,
        )
    }

    /// The single item of an unbraced branch or loop body.
    pub(crate) fn unbraced_body(&mut self, block: &Block) -> Document {
        let mut parts = Vec::new();
        for statement in &block.statements {
            parts.push(self.statement(statement));
        }
        if let Some(tail) = &block.tail {
            parts.push(self.expression_statement(tail));
        }
        concat(parts)
    }

    /// An expression in statement position: a scope block, or any expression.
    fn expression_statement(&mut self, expression: &Expr) -> Document {
        match &expression.kind {
            ExprKind::Block(block) if self.is_braced(block) => self.block(block),
            _ => self.expression(expression),
        }
    }

    pub(crate) fn statement(&mut self, statement: &Statement) -> Document {
        match &statement.kind {
            StatementKind::Local {
                is_var,
                name,
                ty,
                initializer,
            } => {
                let keyword = if *is_var { "var" } else { "val" };
                let mut parts = vec![text(format!("{} {}", keyword, name))];
                if let Some(ty) = ty {
                    parts.push(text(": "));
                    parts.push(self.ty(ty));
                }
                parts.push(text(" ="));
                parts.push(self.right_side(initializer));
                concat(parts)
            }
            StatementKind::Destructure {
                is_var,
                entries,
                initializer,
            } => {
                let keyword = if *is_var { "var" } else { "val" };
                let close_offset = self
                    .token_before(initializer.span.start)
                    .and_then(|equals| self.token_before(equals.span.start))
                    .map_or(initializer.span.start, |close| close.span.start);
                let list = self.comma_list(
                    entries,
                    |entry| entry.field_span.merge(entry.name_span),
                    |_, entry| text(entry.to_string()),
                    CommaList {
                        open: "(",
                        close: ")",
                        close_offset,
                        trailing_comma: true,
                    },
                );
                concat(vec![
                    text(format!("{} ", keyword)),
                    list,
                    text(" ="),
                    self.right_side(initializer),
                ])
            }
            StatementKind::Assign { target, value } => concat(vec![
                self.expression(target),
                self.comments_before_token_after(target.span.end),
                text(" ="),
                self.right_side(value),
            ]),
            StatementKind::CompoundAssign {
                operator,
                target,
                value,
            } => concat(vec![
                self.expression(target),
                self.comments_before_token_after(target.span.end),
                text(format!(" {}=", operator.as_str())),
                self.right_side(value),
            ]),
            StatementKind::Expression(expression) => self.expression_statement(expression),
            StatementKind::While { condition, body } => concat(vec![
                text("while ("),
                self.expression(condition),
                text(")"),
                self.loop_body(body),
            ]),
            StatementKind::DoWhile { body, condition } => concat(vec![
                text("do "),
                self.block(body),
                text(" while ("),
                self.expression(condition),
                text(")"),
            ]),
            StatementKind::For {
                variable,
                iterable,
                body,
                ..
            } => concat(vec![
                text(format!("for ({} in ", variable)),
                self.expression(iterable),
                text(")"),
                self.loop_body(body),
            ]),
        }
    }

    /// A loop body after its header: ` { ... }`, or an unbraced item on the line when it fits,
    /// otherwise on the next line, indented.
    fn loop_body(&mut self, body: &Block) -> Document {
        if self.is_braced(body) {
            return concat(vec![text(" "), self.block(body)]);
        }
        let item = self.unbraced_body(body);
        group(indent(concat(vec![line(), item])))
    }

    /// The right side of `=`, `op=` or a `when` branch's `->`, after the operator: on the same
    /// line for a call, closure, block, `when` or `if` with a braced branch, which break inside
    /// themselves; anything else moves to the next line, indented, when it does not fit (D6).
    pub(crate) fn right_side(&mut self, value: &Expr) -> Document {
        if self.stays_on_operator_line(value) && !self.has_comment_before(value.span.start) {
            concat(vec![text(" "), self.expression(value)])
        } else {
            group(indent(concat(vec![line(), self.expression(value)])))
        }
    }

    fn stays_on_operator_line(&self, value: &Expr) -> bool {
        if self.is_parenthesized(value) {
            return false;
        }
        match &value.kind {
            ExprKind::Call { .. }
            | ExprKind::Closure { .. }
            | ExprKind::Block(_)
            | ExprKind::When { .. } => true,
            ExprKind::If { then_branch, .. } => {
                matches!(&then_branch.kind, ExprKind::Block(block) if self.is_braced(block))
            }
            _ => false,
        }
    }
}

//! Expressions: operators, calls and chains, control flow, closures and literals.

use super::{CommaList, Printer};
use crate::document::*;
use prism_ast::*;
use prism_lexer::TokenKind;

/// The binding strength of a binary operator, as in the parser's `Precedence`.
fn precedence(operator: BinaryOp) -> u8 {
    match operator {
        BinaryOp::Or => 2,
        BinaryOp::And => 3,
        BinaryOp::Eq | BinaryOp::NotEq => 4,
        BinaryOp::Lt | BinaryOp::LtEq | BinaryOp::Gt | BinaryOp::GtEq => 5,
        BinaryOp::Elvis => 6,
        BinaryOp::Range | BinaryOp::HalfOpenRange => 7,
        BinaryOp::Add | BinaryOp::Sub => 8,
        BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod => 9,
    }
}

/// Which operand of a binary operator an expression is.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Left,
    Right,
}

/// One postfix step of a call chain, in source order.
enum Link<'e> {
    Member {
        name: &'e str,
        is_safe_navigation: bool,
        /// Where the member access's target ends; its `.` or `?.` follows.
        target_end: usize,
    },
    Call(&'e Expr),
    Index(&'e Expr),
}

/// Whether an expression extends as far to the right as it can (an `if` whose last branch is
/// unbraced, a `return` with a value), so that an operator after it would be absorbed.
fn is_open_ended(expression: &Expr) -> bool {
    match &expression.kind {
        ExprKind::If {
            then_branch,
            else_branch,
            ..
        } => match else_branch {
            Some(else_branch) => !matches!(else_branch.kind, ExprKind::Block(_)),
            None => !matches!(then_branch.kind, ExprKind::Block(_)),
        },
        ExprKind::Return(value) => value.is_some(),
        ExprKind::Unary { expr, .. } => is_open_ended(expr),
        ExprKind::Binary { right, .. } => is_open_ended(right),
        _ => false,
    }
}

impl<'a> Printer<'a> {
    /// An expression, after the comments that precede it, in the parentheses it was written in
    /// (D5).
    pub(crate) fn expression(&mut self, expression: &Expr) -> Document {
        let leading = self.comments_before(expression.span.start);
        let printed = self.expression_kind(expression);
        if self.is_parenthesized(expression) {
            concat(vec![leading, text("("), printed, text(")")])
        } else {
            concat(vec![leading, printed])
        }
    }

    /// An expression in parentheses if `needed` and it was not written in them.
    fn expression_in_parentheses_if(&mut self, expression: &Expr, needed: bool) -> Document {
        if needed && !self.is_parenthesized(expression) {
            let leading = self.comments_before(expression.span.start);
            let printed = self.expression_kind(expression);
            concat(vec![leading, text("("), printed, text(")")])
        } else {
            self.expression(expression)
        }
    }

    /// Whether `expression` was written in parentheses: its span then starts before its left
    /// operand, or at a `(` for an expression without one.
    pub(crate) fn is_parenthesized(&self, expression: &Expr) -> bool {
        let leftmost = match &expression.kind {
            ExprKind::Binary { left, .. } => Some(left),
            ExprKind::Call { callee, .. } => Some(callee),
            ExprKind::MemberAccess { target, .. } | ExprKind::Index { target, .. } => Some(target),
            _ => None,
        };
        match leftmost {
            Some(child) => expression.span.start < child.span.start,
            None => self.kind_at(expression.span.start) == Some(&TokenKind::LParen),
        }
    }

    fn expression_kind(&mut self, expression: &Expr) -> Document {
        match &expression.kind {
            ExprKind::Literal(_) => text(self.literal_text(expression)),
            ExprKind::Ident(name) => text(name.clone()),
            ExprKind::This => text("this"),
            ExprKind::Break => text("break"),
            ExprKind::Continue => text("continue"),
            ExprKind::Binary { .. } => self.binary(expression),
            ExprKind::Unary { op, expr } => {
                let needs_parentheses = matches!(expr.kind, ExprKind::Binary { .. });
                let starts_with_minus = *op == UnaryOp::Neg && self.starts_with_minus(expr);
                let operator = if starts_with_minus { "- " } else { op.as_str() };
                concat(vec![
                    text(operator),
                    self.expression_in_parentheses_if(expr, needs_parentheses),
                ])
            }
            ExprKind::Call { .. } | ExprKind::MemberAccess { .. } | ExprKind::Index { .. } => {
                self.chain(expression)
            }
            ExprKind::Block(block) => self.block(block),
            ExprKind::If {
                cond,
                then_branch,
                else_branch,
            } => self.if_expression(cond, then_branch, else_branch.as_deref()),
            ExprKind::When { target, branches } => {
                self.when(target.as_deref(), branches, expression.span)
            }
            ExprKind::Return(value) => match value {
                Some(value) => concat(vec![text("return "), self.expression(value)]),
                None => text("return"),
            },
            ExprKind::Closure { params, body } => self.closure(params, body, expression.span),
        }
    }

    /// Whether an operand printed after a prefix `-` starts with `-`, which would join into
    /// `--`.
    fn starts_with_minus(&self, operand: &Expr) -> bool {
        if self.is_parenthesized(operand) {
            return false;
        }
        match &operand.kind {
            ExprKind::Unary {
                op: UnaryOp::Neg, ..
            } => true,
            ExprKind::Literal(_) => self.literal_text(operand).starts_with('-'),
            ExprKind::Binary { left, .. } => self.starts_with_minus(left),
            _ => false,
        }
    }

    /// A literal's source text (D5), past the parentheses around it; the `i64::MIN` literal is
    /// its `-` and its digits.
    fn literal_text(&self, literal: &Expr) -> String {
        let mut index = self
            .tokens
            .partition_point(|t| t.span.start < literal.span.start);
        while self.tokens[index].kind == TokenKind::LParen {
            index += 1;
        }
        let token = &self.tokens[index];
        if token.kind == TokenKind::Minus {
            format!("-{}", self.slice(self.tokens[index + 1].span))
        } else {
            self.slice(token.span).to_string()
        }
    }

    // --- Operators ---

    /// A binary expression and the operators of its precedence chained to its left, as one
    /// group breaking after each operator, or before `?:` (D6). Ranges never break.
    fn binary(&mut self, expression: &Expr) -> Document {
        let ExprKind::Binary { op, .. } = &expression.kind else {
            unreachable!("binary is only called on binary expressions");
        };
        let level = precedence(*op);
        let mut rest = Vec::new();
        let mut first = expression;
        while let ExprKind::Binary { op, left, right } = &first.kind {
            let chains = std::ptr::eq(first, expression)
                || precedence(*op) == level && !self.is_parenthesized(first);
            if !chains {
                break;
            }
            rest.push((*op, right.as_ref()));
            first = left;
        }
        rest.reverse();
        if std::ptr::eq(first, expression) {
            unreachable!("a binary expression has a left operand");
        }
        let first_document = self.operand(first, level, Side::Left);
        let mut tail = Vec::new();
        let is_range = matches!(op, BinaryOp::Range | BinaryOp::HalfOpenRange);
        let mut previous_end = first.span.end;
        for (operator, right) in rest {
            if is_range {
                tail.push(text(operator.as_str()));
                tail.push(self.operand(right, level, Side::Right));
            } else if operator == BinaryOp::Elvis {
                tail.push(self.comments_before_token_after(previous_end));
                tail.push(line());
                tail.push(text("?: "));
                tail.push(self.operand(right, level, Side::Right));
            } else {
                // A comment between the operand and the operator stays before the operator.
                tail.push(self.comments_before_token_after(previous_end));
                tail.push(text(format!(" {}", operator.as_str())));
                // A comment after the operator, on its line, stays there.
                tail.push(self.trailing_comments(previous_end, right.span.start));
                tail.push(line());
                tail.push(self.operand(right, level, Side::Right));
            }
            previous_end = right.span.end;
        }
        if is_range {
            let mut parts = vec![first_document];
            parts.extend(tail);
            return concat(parts);
        }
        group(concat(vec![first_document, indent(concat(tail))]))
    }

    /// An operand of a binary operator of precedence `level`, in parentheses where the
    /// grammar needs them.
    fn operand(&mut self, operand: &Expr, level: u8, side: Side) -> Document {
        let needed = match &operand.kind {
            ExprKind::Binary { op, .. } => {
                let inner = precedence(*op);
                inner < level || inner == level && (side == Side::Right || level == 5)
            }
            _ => side == Side::Left && is_open_ended(operand),
        };
        self.expression_in_parentheses_if(operand, needed)
    }

    // --- Calls and chains ---

    /// A chain of member accesses, calls and indexes. With two or more calls it is a group
    /// that breaks before each `.` or `?.` starting a call, keeping the receiver and its field
    /// accesses on the first line (D6); otherwise it is printed in one piece.
    fn chain(&mut self, expression: &Expr) -> Document {
        let mut links = Vec::new();
        let mut root = expression;
        loop {
            let next = match &root.kind {
                ExprKind::MemberAccess {
                    target,
                    member,
                    is_safe_nav,
                } => {
                    links.push(Link::Member {
                        name: member,
                        is_safe_navigation: *is_safe_nav,
                        target_end: target.span.end,
                    });
                    target
                }
                ExprKind::Call { callee, .. } => {
                    links.push(Link::Call(root));
                    callee
                }
                ExprKind::Index { target, .. } => {
                    links.push(Link::Index(root));
                    target
                }
                _ => break,
            };
            root = next;
            if self.is_parenthesized(root) {
                break;
            }
        }
        links.reverse();

        let needs_parentheses = matches!(
            root.kind,
            ExprKind::Binary { .. }
                | ExprKind::Unary { .. }
                | ExprKind::If { .. }
                | ExprKind::Return(_)
        ) || matches!(root.kind, ExprKind::Literal(_))
            && self.literal_text(root).starts_with('-');
        let root_document = self.expression_in_parentheses_if(root, needs_parentheses);

        // Segments: the links before the first member access belong to the head; each member
        // access starts a segment holding the calls and indexes after it.
        let mut head = vec![root_document];
        let mut segments: Vec<(bool, Vec<Document>)> = Vec::new();
        // A comment before a `.` breaks the chain there, so it stays before that member.
        let mut has_comments = false;
        for link in &links {
            match link {
                Link::Member {
                    name,
                    is_safe_navigation,
                    target_end,
                } => {
                    let dot_offset = self.tokens[self.token_index_at(*target_end)..]
                        .iter()
                        .find(|t| matches!(t.kind, TokenKind::Dot | TokenKind::QuestionDot))
                        .map_or(*target_end, |t| t.span.start);
                    has_comments |= self.has_comment_before(dot_offset);
                    let leading = self.comments_before(dot_offset);
                    let dot = if *is_safe_navigation { "?." } else { "." };
                    segments.push((false, vec![leading, text(format!("{}{}", dot, name))]));
                }
                Link::Call(call) => {
                    let suffix = self.call_suffix(call);
                    match segments.last_mut() {
                        Some((has_call, parts)) => {
                            *has_call = true;
                            parts.push(suffix);
                        }
                        None => head.push(suffix),
                    }
                }
                Link::Index(index_expression) => {
                    let ExprKind::Index { index, .. } = &index_expression.kind else {
                        unreachable!("an index link is an index expression");
                    };
                    let suffix = concat(vec![text("["), self.expression(index), text("]")]);
                    match segments.last_mut() {
                        Some((_, parts)) => parts.push(suffix),
                        None => head.push(suffix),
                    }
                }
            }
        }
        let calls = segments.iter().filter(|(has_call, _)| *has_call).count();
        if calls < 2 && !has_comments {
            for (_, parts) in segments {
                head.extend(parts);
            }
            return concat(head);
        }
        let mut groups: Vec<Vec<Document>> = Vec::new();
        if has_comments {
            groups.extend(segments.into_iter().map(|(_, parts)| parts));
            let mut broken = Vec::new();
            for parts in groups {
                broken.push(soft_line());
                broken.push(concat(parts));
            }
            return group(concat(vec![concat(head), indent(concat(broken))]));
        }
        // The field accesses before the first call stay in the head; after it, each run of
        // field accesses joins the call that ends it.
        let mut pending: Vec<Document> = Vec::new();
        let mut seen_call = false;
        for (has_call, parts) in segments {
            if !seen_call && !has_call {
                head.extend(parts);
                continue;
            }
            pending.extend(parts);
            if has_call {
                seen_call = true;
                groups.push(std::mem::take(&mut pending));
            }
        }
        if !pending.is_empty() {
            match groups.last_mut() {
                Some(last) => last.extend(pending),
                None => groups.push(pending),
            }
        }
        let mut broken = Vec::new();
        for parts in groups {
            broken.push(soft_line());
            broken.push(concat(parts));
        }
        group(concat(vec![concat(head), indent(concat(broken))]))
    }

    /// A call's type arguments, argument list and trailing lambda. An empty argument list
    /// before a trailing lambda is dropped (D5).
    fn call_suffix(&mut self, call: &Expr) -> Document {
        let ExprKind::Call {
            type_arguments,
            args,
            ..
        } = &call.kind
        else {
            unreachable!("a call link is a call expression");
        };
        let mut parts = Vec::new();
        if !type_arguments.is_empty() {
            parts.push(self.type_arguments(type_arguments));
        }
        let trailing = args.last().filter(|argument| {
            argument.name.is_none()
                && matches!(argument.value.kind, ExprKind::Closure { .. })
                && !self.is_parenthesized(&argument.value)
                && !matches!(
                    self.token_before(argument.value.span.start)
                        .map(|t| &t.kind),
                    Some(TokenKind::LParen | TokenKind::Comma)
                )
        });
        let regular = match trailing {
            Some(_) => &args[..args.len() - 1],
            None => &args[..],
        };
        if trailing.is_none() || !regular.is_empty() {
            let close_offset = match trailing {
                Some(lambda) => self
                    .token_before(lambda.value.span.start)
                    .map_or(lambda.value.span.start, |t| t.span.start),
                None => call.span.end.saturating_sub(1),
            };
            parts.push(self.comma_list(
                regular,
                |argument| argument.span,
                |printer, argument| printer.argument(argument),
                CommaList {
                    open: "(",
                    close: ")",
                    close_offset,
                    trailing_comma: true,
                },
            ));
        }
        if let Some(lambda) = trailing {
            parts.push(text(" "));
            parts.push(self.expression(&lambda.value));
        }
        concat(parts)
    }

    fn argument(&mut self, argument: &CallArg) -> Document {
        match &argument.name {
            Some(name) => concat(vec![
                text(format!("{} = ", name)),
                self.expression(&argument.value),
            ]),
            None => self.expression(&argument.value),
        }
    }

    // --- Control flow ---

    /// The body of an `if` branch: a braced block, or the item of an unbraced one.
    fn branch_body(&mut self, branch: &Expr) -> Document {
        match &branch.kind {
            ExprKind::Block(block) if !self.is_braced(block) && !self.is_parenthesized(branch) => {
                self.unbraced_body(block)
            }
            _ => self.expression(branch),
        }
    }

    fn is_braced_branch(&self, branch: &Expr) -> bool {
        matches!(&branch.kind, ExprKind::Block(block) if self.is_braced(block))
            && !self.is_parenthesized(branch)
    }

    /// `if (c) { ... } else { ... }`, or with unbraced branches on one line when they fit,
    /// otherwise each on its own indented line after `if (c)` and `else` (D6).
    fn if_expression(
        &mut self,
        condition: &Expr,
        then_branch: &Expr,
        else_branch: Option<&Expr>,
    ) -> Document {
        let head = concat(vec![text("if ("), self.expression(condition), text(")")]);
        if self.is_braced_branch(then_branch) {
            let mut parts = vec![head, text(" "), self.expression(then_branch)];
            if let Some(else_branch) = else_branch {
                if self.is_braced_branch(else_branch)
                    || matches!(else_branch.kind, ExprKind::If { .. })
                        && !self.is_parenthesized(else_branch)
                {
                    parts.push(text(" else "));
                    parts.push(self.expression(else_branch));
                } else {
                    parts.push(text(" else"));
                    let body = self.branch_body(else_branch);
                    parts.push(group(indent(concat(vec![line(), body]))));
                }
            }
            return concat(parts);
        }
        let then_document = self.branch_body(then_branch);
        let mut parts = vec![head, indent(concat(vec![line(), then_document]))];
        let Some(else_branch) = else_branch else {
            return group(concat(parts));
        };
        parts.push(line());
        if self.is_braced_branch(else_branch) {
            parts.push(text("else"));
            let header = group(concat(parts));
            return concat(vec![header, text(" "), self.expression(else_branch)]);
        }
        if matches!(else_branch.kind, ExprKind::If { .. }) && !self.is_parenthesized(else_branch) {
            parts.push(text("else "));
            parts.push(self.expression(else_branch));
        } else {
            parts.push(text("else"));
            let body = self.branch_body(else_branch);
            parts.push(indent(concat(vec![line(), body])));
        }
        group(concat(parts))
    }

    /// `when (subject) { ... }` with one branch per line (D6).
    fn when(&mut self, subject: Option<&Expr>, branches: &[WhenBranch], span: Span) -> Document {
        let mut parts = vec![text("when ")];
        if let Some(subject) = subject {
            parts.push(text("("));
            parts.push(self.expression(subject));
            parts.push(text(") "));
        }
        let close_offset = span.end.saturating_sub(1);
        let list = self.vertical_list(
            branches,
            |branch| branch.span,
            |printer, branch| printer.when_branch(branch),
            close_offset,
            |_| false,
        );
        parts.push(self.braced(list));
        concat(parts)
    }

    fn when_branch(&mut self, branch: &WhenBranch) -> Document {
        let mut parts = Vec::new();
        for (index, pattern) in branch.patterns.iter().enumerate() {
            if index > 0 {
                parts.push(text(", "));
            }
            match pattern {
                WhenPattern::Expr(expression) => parts.push(self.expression(expression)),
                WhenPattern::InRange {
                    start,
                    end,
                    end_inclusive,
                } => {
                    let level = precedence(BinaryOp::Range);
                    parts.push(text("in "));
                    parts.push(self.operand(start, level, Side::Left));
                    parts.push(text(if *end_inclusive { ".." } else { "..<" }));
                    parts.push(self.operand(end, level, Side::Right));
                }
                WhenPattern::Else => parts.push(text("else")),
            }
        }
        parts.push(text(" ->"));
        let body = &branch.body;
        match &body.kind {
            ExprKind::Block(block) if !self.is_parenthesized(body) => {
                parts.push(text(" "));
                if self.is_braced(block) {
                    parts.push(self.expression(body));
                } else {
                    parts.push(self.unbraced_body(block));
                }
            }
            _ => parts.push(self.right_side(body)),
        }
        concat(parts)
    }

    /// `{ a, b -> body }`: on one line when its body is a single statement or expression, it
    /// was written on one line and it fits; otherwise broken, with the parameters after the `{`
    /// (D9).
    fn closure(&mut self, parameters: &[ClosureParameter], body: &Block, span: Span) -> Document {
        let header = if parameters.is_empty() {
            String::new()
        } else {
            let names: Vec<&str> = parameters.iter().map(|p| p.name.as_str()).collect();
            format!(" {} ->", names.join(", "))
        };
        let single_item = body.statements.len() + usize::from(body.tail.is_some()) == 1;
        let written_on_one_line = self.same_line(span.start, span.end);
        let items = self.block_items(body, body.close_brace.start);
        match items {
            None if parameters.is_empty() => text("{}"),
            None => text(format!("{{{} }}", header)),
            Some(items) if single_item && written_on_one_line => group(concat(vec![
                text(format!("{{{}", header)),
                indent(concat(vec![line(), items])),
                line(),
                text("}"),
            ])),
            Some(items) => concat(vec![
                text(format!("{{{}", header)),
                indent(concat(vec![hard_line(), items])),
                hard_line(),
                text("}"),
            ]),
        }
    }
}

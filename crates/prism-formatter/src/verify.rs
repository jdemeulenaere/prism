//! The meaning check run on every formatting (D4 of `prism-fmt`): the output must parse to the
//! input's tree with every span erased, and hold the input's comments in order.

use crate::Parsed;
use prism_ast::*;
use prism_lexer::{Comment, CommentKind};

/// `Ok` when `output` keeps the meaning and comments of `source`, otherwise why not.
pub(crate) fn same_meaning(
    source: &str,
    input: &Parsed,
    output: &str,
    reparsed: &Parsed,
) -> Result<(), String> {
    let mut before = input.file.clone();
    let mut after = reparsed.file.clone();
    erase_file(&mut before);
    erase_file(&mut after);
    if before != after {
        return Err(
            "the formatted source parses to a different syntax tree; the input was left unchanged"
                .to_string(),
        );
    }
    let comments_before = comment_texts(source, &input.comments);
    let comments_after = comment_texts(output, &reparsed.comments);
    if comments_before != comments_after {
        return Err(
            "the formatted source does not hold the same comments; the input was left unchanged"
                .to_string(),
        );
    }
    Ok(())
}

/// The text of each comment, line comments without trailing whitespace (which the formatter
/// removes).
pub(crate) fn comment_texts<'a>(source: &'a str, comments: &[Comment]) -> Vec<&'a str> {
    comments
        .iter()
        .map(|comment| {
            let text = &source[comment.span.start..comment.span.end];
            match comment.kind {
                CommentKind::Line => text.trim_end(),
                CommentKind::Block => text,
            }
        })
        .collect()
}

fn erase(span: &mut Span) {
    *span = Span::dummy();
}

pub(crate) fn erase_file(file: &mut SourceFile) {
    erase(&mut file.span);
    if let Some(package) = &mut file.package {
        erase(&mut package.span);
    }
    for import in &mut file.imports {
        erase(&mut import.span);
    }
    for declaration in &mut file.declarations {
        erase_declaration(declaration);
    }
}

fn erase_declaration(declaration: &mut Declaration) {
    match declaration {
        Declaration::Function(function) => {
            erase(&mut function.span);
            if let Some(receiver) = &mut function.receiver {
                erase(&mut receiver.span);
                erase_type(&mut receiver.ty);
            }
            erase_type_parameters(&mut function.type_parameters);
            for parameter in &mut function.params {
                erase(&mut parameter.span);
                erase_type(&mut parameter.ty);
                if let Some(default) = &mut parameter.default_value {
                    erase_expression(default);
                }
            }
            if let Some(return_type) = &mut function.return_type {
                erase_type(return_type);
            }
            match &mut function.body {
                Some(FunctionBody::Block(block)) => erase_block(block),
                Some(FunctionBody::Expr(expression)) => erase_expression(expression),
                None => {}
            }
        }
        Declaration::Struct(structure) => {
            erase(&mut structure.span);
            erase(&mut structure.name_span);
            erase_type_parameters(&mut structure.type_parameters);
            for field in &mut structure.fields {
                erase(&mut field.span);
                erase_type(&mut field.ty);
            }
        }
        Declaration::Enum(enumeration) => {
            erase(&mut enumeration.span);
            erase(&mut enumeration.name_span);
            erase_type_parameters(&mut enumeration.type_parameters);
            for variant in &mut enumeration.variants {
                erase(&mut variant.span);
                for field in &mut variant.fields {
                    erase_type(field);
                }
            }
        }
        Declaration::Var(variable) => {
            erase(&mut variable.span);
            if let Some(ty) = &mut variable.ty {
                erase_type(ty);
            }
            erase_expression(&mut variable.initializer);
        }
    }
}

fn erase_type_parameters(parameters: &mut [TypeParameter]) {
    for parameter in parameters {
        erase(&mut parameter.span);
    }
}

fn erase_type(ty: &mut Type) {
    erase(&mut ty.span);
    match &mut ty.kind {
        TypeKind::Named { type_args, .. } => {
            for argument in type_args {
                erase_type(argument);
            }
        }
        TypeKind::Nullable(inner) | TypeKind::Reference { inner, .. } => erase_type(inner),
        TypeKind::Unit => {}
        TypeKind::Function {
            params,
            return_type,
            ..
        } => {
            for parameter in params {
                erase_type(parameter);
            }
            erase_type(return_type);
        }
    }
}

fn erase_block(block: &mut Block) {
    erase(&mut block.span);
    erase(&mut block.close_brace);
    for statement in &mut block.statements {
        erase_statement(statement);
    }
    if let Some(tail) = &mut block.tail {
        erase_expression(tail);
    }
}

fn erase_statement(statement: &mut Statement) {
    erase(&mut statement.span);
    match &mut statement.kind {
        StatementKind::Local {
            ty, initializer, ..
        } => {
            if let Some(ty) = ty {
                erase_type(ty);
            }
            erase_expression(initializer);
        }
        StatementKind::Destructure {
            entries,
            initializer,
            ..
        } => {
            for entry in entries {
                erase(&mut entry.field_span);
                erase(&mut entry.name_span);
            }
            erase_expression(initializer);
        }
        StatementKind::Assign { target, value }
        | StatementKind::CompoundAssign { target, value, .. } => {
            erase_expression(target);
            erase_expression(value);
        }
        StatementKind::Expression(expression) => erase_expression(expression),
        StatementKind::While { condition, body } | StatementKind::DoWhile { body, condition } => {
            erase_expression(condition);
            erase_block(body);
        }
        StatementKind::For {
            variable_span,
            iterable,
            body,
            ..
        } => {
            erase(variable_span);
            erase_expression(iterable);
            erase_block(body);
        }
    }
}

fn erase_pattern(pattern: &mut Pattern) {
    erase(&mut pattern.span);
    match &mut pattern.kind {
        PatternKind::Wildcard | PatternKind::Binding { .. } => {}
        PatternKind::Value(expression) => erase_expression(expression),
        PatternKind::InRange { start, end, .. } => {
            erase_expression(start);
            erase_expression(end);
        }
        PatternKind::Constructor { callee, arguments } => {
            erase_expression(callee);
            for argument in arguments {
                erase(&mut argument.span);
                erase_pattern(&mut argument.pattern);
            }
        }
    }
}

fn erase_expression(expression: &mut Expr) {
    erase(&mut expression.span);
    match &mut expression.kind {
        ExprKind::Literal(_)
        | ExprKind::Ident(_)
        | ExprKind::This
        | ExprKind::Break
        | ExprKind::Continue => {}
        ExprKind::Binary { left, right, .. } => {
            erase_expression(left);
            erase_expression(right);
        }
        ExprKind::Unary { expr, .. } => erase_expression(expr),
        ExprKind::Call {
            callee,
            type_arguments,
            args,
        } => {
            erase_expression(callee);
            for argument in type_arguments {
                erase_type(argument);
            }
            for argument in args {
                erase(&mut argument.span);
                erase_expression(&mut argument.value);
            }
        }
        ExprKind::MemberAccess { target, .. } => erase_expression(target),
        ExprKind::Block(block) => erase_block(block),
        ExprKind::If {
            cond,
            then_branch,
            else_branch,
        } => {
            erase_expression(cond);
            erase_expression(then_branch);
            if let Some(else_branch) = else_branch {
                erase_expression(else_branch);
            }
        }
        ExprKind::When { target, branches } => {
            if let Some(target) = target {
                erase_expression(target);
            }
            for branch in branches {
                erase(&mut branch.span);
                for pattern in &mut branch.patterns {
                    if let WhenPattern::Pattern(pattern) = pattern {
                        erase_pattern(pattern);
                    }
                }
                if let Some(guard) = &mut branch.guard {
                    erase_expression(guard);
                }
                erase_expression(&mut branch.body);
            }
        }
        ExprKind::Return(value) => {
            if let Some(value) = value {
                erase_expression(value);
            }
        }
        ExprKind::Closure { params, body } => {
            for parameter in params {
                erase(&mut parameter.span);
            }
            erase_block(body);
        }
        ExprKind::Index { target, index } => {
            erase_expression(target);
            erase_expression(index);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(source: &str) -> Parsed {
        crate::parse(source).expect("parses")
    }

    #[test]
    fn layout_differences_are_the_same_meaning() {
        let before = "fun f( a:Int ) { val x = (a+1) ; x }";
        let after = "fun f(a: Int) {\n    val x = (a + 1)\n    x\n}\n";
        assert_eq!(
            same_meaning(before, &parsed(before), after, &parsed(after)),
            Ok(())
        );
    }

    #[test]
    fn a_changed_tree_is_detected() {
        let before = "fun f() { return\n1 }";
        let after = "fun f() { return 1 }";
        assert!(same_meaning(before, &parsed(before), after, &parsed(after)).is_err());
    }

    #[test]
    fn a_lost_or_changed_comment_is_detected() {
        let before = "val x = 1 // one\n/* two */";
        for after in ["val x = 1 // one\n", "val x = 1 /* two */ // one\n"] {
            assert!(same_meaning(before, &parsed(before), after, &parsed(after)).is_err());
        }
        let trimmed = "val x = 1 // one\n/* two */\n";
        let spaced = "val x = 1 // one   \n/* two */";
        assert_eq!(
            same_meaning(spaced, &parsed(spaced), trimmed, &parsed(trimmed)),
            Ok(())
        );
    }
}

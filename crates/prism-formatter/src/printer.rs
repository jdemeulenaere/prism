//! The printer: turns a parsed file into a [`Document`], reading back from the tokens what the
//! tree does not record (D5 of `prism-fmt`) and placing comments (D3).
//!
//! Comments are taken from a cursor that only moves forward, in the order the printer reaches
//! source positions, so the output always holds them in source order. Lists (declarations,
//! members, statements, branches, parameters, arguments) place the comments before, after and
//! between their items; any other comment is printed before the next node that starts after it.

mod declarations;
mod expressions;
mod statements;

use crate::document::{self, *};
use prism_ast::{SourceFile, Span};
use prism_lexer::{Comment, CommentKind, Token, TokenKind};

/// Prints `file`, parsed from `source` into `tokens` and `comments`, as formatted source.
pub(crate) fn print(
    source: &str,
    tokens: &[Token],
    comments: &[Comment],
    file: &SourceFile,
) -> String {
    let mut printer = Printer {
        source,
        tokens,
        comments,
        next_comment: 0,
        consumed_end: 0,
    };
    let document = printer.source_file(file);
    let rendered = render(&document);
    let trimmed = rendered.trim_end_matches('\n');
    if trimmed.is_empty() {
        String::new()
    } else {
        format!("{}\n", trimmed)
    }
}

pub(crate) struct Printer<'a> {
    source: &'a str,
    /// The lexed tokens in source order, ending with `Eof`.
    tokens: &'a [Token],
    comments: &'a [Comment],
    /// The first comment not printed yet.
    next_comment: usize,
    /// The end of the last comment printed, for counting blank lines after it.
    consumed_end: usize,
}

/// Where a list's items are printed relative to each other.
pub(crate) struct CommaList<'d> {
    pub open: &'d str,
    pub close: &'d str,
    /// The source offset of the closing delimiter; comments before it belong to the list.
    pub close_offset: usize,
    /// Whether a broken list ends with a trailing comma (D7).
    pub trailing_comma: bool,
}

impl<'a> Printer<'a> {
    // --- Source and tokens ---

    /// The index of the first token starting at or after `offset`.
    pub(crate) fn token_index_at(&self, offset: usize) -> usize {
        self.tokens
            .partition_point(|token| token.span.start < offset)
    }

    /// The first token starting at or after `offset`.
    pub(crate) fn token_at(&self, offset: usize) -> Option<&'a Token> {
        self.tokens
            .get(self.token_index_at(offset))
            .filter(|token| token.kind != TokenKind::Eof)
    }

    /// The kind of the first token starting at or after `offset`.
    pub(crate) fn kind_at(&self, offset: usize) -> Option<&'a TokenKind> {
        self.token_at(offset).map(|token| &token.kind)
    }

    /// The last token ending at or before `offset`.
    pub(crate) fn token_before(&self, offset: usize) -> Option<&'a Token> {
        let count = self
            .tokens
            .partition_point(|token| token.span.end <= offset);
        count.checked_sub(1).map(|index| &self.tokens[index])
    }

    /// The first token of `kind` starting at or after `offset`.
    pub(crate) fn find_token(&self, offset: usize, kind: &TokenKind) -> Option<&'a Token> {
        self.tokens[self.token_index_at(offset)..]
            .iter()
            .find(|token| &token.kind == kind)
    }

    /// The source text of `span`.
    pub(crate) fn slice(&self, span: Span) -> &'a str {
        &self.source[span.start.min(self.source.len())..span.end.min(self.source.len())]
    }

    /// Whether `start..end` holds no newline.
    pub(crate) fn same_line(&self, start: usize, end: usize) -> bool {
        start >= end || !self.source[start..end.min(self.source.len())].contains('\n')
    }

    /// Whether a line holding only whitespace lies between `start` and `end`. A line holding a
    /// token, such as a `{` that a moved comment preceded, is not blank.
    fn has_blank_line_between(&self, start: usize, end: usize) -> bool {
        if start >= end {
            return false;
        }
        let lines: Vec<&str> = self.source[start..end.min(self.source.len())]
            .split('\n')
            .collect();
        lines.len() > 2
            && lines[1..lines.len() - 1]
                .iter()
                .any(|line| line.trim().is_empty())
    }

    // --- Comments ---

    fn peek_comment(&self) -> Option<&'a Comment> {
        self.comments.get(self.next_comment)
    }

    /// The next comment if it starts before `offset`.
    fn comment_before(&self, offset: usize) -> Option<&'a Comment> {
        self.peek_comment()
            .filter(|comment| comment.span.start < offset)
    }

    /// Whether a comment starts before `offset` that has not been printed.
    pub(crate) fn has_comment_before(&self, offset: usize) -> bool {
        self.comment_before(offset).is_some()
    }

    fn take_comment(&mut self) -> &'a Comment {
        let comment = &self.comments[self.next_comment];
        self.next_comment += 1;
        self.consumed_end = self.consumed_end.max(comment.span.end);
        comment
    }

    fn comment_text(&self, comment: &Comment) -> String {
        let text = self.slice(comment.span);
        match comment.kind {
            CommentKind::Line => text.trim_end().to_string(),
            CommentKind::Block => text.to_string(),
        }
    }

    /// Whether only whitespace precedes `comment` on its line.
    fn starts_line(&self, comment: &Comment) -> bool {
        let line_start = self.source[..comment.span.start]
            .rfind('\n')
            .map_or(0, |index| index + 1);
        self.source[line_start..comment.span.start]
            .trim()
            .is_empty()
    }

    /// Whether nothing but whitespace follows `comment` on its line.
    fn ends_line(&self, comment: &Comment) -> bool {
        if comment.kind == CommentKind::Line {
            return true;
        }
        let rest = &self.source[comment.span.end..];
        let line = rest.split('\n').next().unwrap_or_default();
        line.trim().is_empty()
    }

    /// Whether only comments and whitespace follow `comment` on its line.
    fn only_comments_follow(&self, comment: &Comment) -> bool {
        let rest = &self.source[comment.span.end..];
        let line = rest.split('\n').next().unwrap_or_default().trim_start();
        line.is_empty() || line.starts_with("//") || line.starts_with("/*")
    }

    /// Whether one of the comments starting before `offset` ends its line.
    pub(crate) fn comments_end_a_line_before(&self, offset: usize) -> bool {
        self.comments[self.next_comment..]
            .iter()
            .take_while(|comment| comment.span.start < offset)
            .any(|comment| self.ends_line(comment))
    }

    /// The comments starting before `offset`, each where it stands relative to the code: an
    /// own-line comment on its own line, a block comment inline and followed by a space, a line
    /// comment ending the line. Used before a node.
    pub(crate) fn comments_before(&mut self, offset: usize) -> Document {
        let mut parts = Vec::new();
        while let Some(comment) = self.comment_before(offset) {
            self.take_comment();
            let ends_line = self.ends_line(comment);
            parts.push(document::comment(
                self.comment_text(comment),
                self.starts_line(comment),
                ends_line,
            ));
            if !ends_line {
                parts.push(text(" "));
            }
        }
        concat(parts)
    }

    /// The comments starting before `offset`, printed after what precedes them: inline after a
    /// space, or on their own line when they were. Used at the end of an item, for comments its
    /// printing did not place.
    pub(crate) fn comments_at_end(&mut self, offset: usize) -> Document {
        let mut parts = Vec::new();
        while let Some(comment) = self.comment_before(offset) {
            self.take_comment();
            let starts_line = self.starts_line(comment);
            if !starts_line {
                parts.push(text(" "));
            }
            parts.push(document::comment(
                self.comment_text(comment),
                starts_line,
                self.ends_line(comment),
            ));
        }
        concat(parts)
    }

    /// The comments between `offset` and the first token after it (such as an operator after
    /// its left operand), printed inline after what precedes them.
    pub(crate) fn comments_before_token_after(&mut self, offset: usize) -> Document {
        match self.token_at(offset) {
            Some(token) => self.comments_at_end(token.span.start),
            None => Document::Empty,
        }
    }

    /// The comments starting before `offset` that sit before a closing delimiter with no item
    /// after them, separated by spaces, or on their own lines when they were.
    /// With `after_item`, an inline comment is also separated from the item before it.
    pub(crate) fn comments_dangling(&mut self, offset: usize, after_item: bool) -> Document {
        let mut parts = Vec::new();
        while let Some(comment) = self.comment_before(offset) {
            self.take_comment();
            let starts_line = self.starts_line(comment);
            if (after_item || !parts.is_empty()) && !starts_line {
                parts.push(text(" "));
            }
            parts.push(document::comment(
                self.comment_text(comment),
                starts_line,
                self.ends_line(comment),
            ));
        }
        concat(parts)
    }

    /// The comments on the line where an item ends (at `end`), after it: a line comment, or
    /// block comments followed only by comments or the end of the line.
    /// Only comments starting before `limit` (the next item) are taken.
    pub(crate) fn trailing_comments(&mut self, end: usize, limit: usize) -> Document {
        let mut parts = Vec::new();
        while let Some(comment) = self.peek_comment() {
            if comment.span.start < end
                || comment.span.start >= limit
                || !self.same_line(end, comment.span.start)
                || !self.only_comments_follow(comment) && comment.kind == CommentKind::Block
            {
                break;
            }
            self.take_comment();
            parts.push(text(" "));
            parts.push(document::comment(
                self.comment_text(comment),
                false,
                self.ends_line(comment),
            ));
        }
        concat(parts)
    }

    /// The comments that start on the line where an item ends (at `end`) and before `limit`,
    /// whatever follows them, each after a space.
    fn comments_on_line(&mut self, end: usize, limit: usize) -> Document {
        let mut parts = Vec::new();
        while let Some(comment) = self.comment_before(limit) {
            if !self.same_line(end, comment.span.start) {
                break;
            }
            self.take_comment();
            parts.push(text(" "));
            parts.push(document::comment(
                self.comment_text(comment),
                false,
                self.ends_line(comment),
            ));
        }
        concat(parts)
    }

    // --- Lists ---

    /// Items one per line (declarations, members, statements, branches), with their leading,
    /// trailing and dangling comments and at most one blank line between them (D8). Returns
    /// `None` when there is neither an item nor a comment before `close_offset`.
    /// `blank_after(i)` forces a blank line after item `i`.
    pub(crate) fn vertical_list<T>(
        &mut self,
        items: &[T],
        span_of: impl Fn(&T) -> Span,
        mut print_item: impl FnMut(&mut Self, &T) -> Document,
        close_offset: usize,
        blank_after: impl Fn(usize) -> bool,
    ) -> Option<Document> {
        let mut list = VerticalList::default();
        for (index, item) in items.iter().enumerate() {
            let span = span_of(item);
            while let Some(comment) = self.comment_before(span.start) {
                self.take_comment();
                list.separate(self, comment.span.start);
                let ends_line = self.ends_line(comment);
                list.parts.push(document::comment(
                    self.comment_text(comment),
                    false,
                    ends_line,
                ));
                list.inline = !ends_line;
                if list.inline {
                    list.parts.push(text(" "));
                }
                list.previous_end = comment.span.end;
            }
            list.separate(self, span.start);
            list.inline = false;
            let document = print_item(self, item);
            list.parts.push(document);
            let leftovers = self.comments_at_end(span.end);
            list.parts.push(leftovers);
            let limit = items
                .get(index + 1)
                .map_or(close_offset, |next| span_of(next).start);
            let trailing = self.trailing_comments(span.end, limit);
            list.parts.push(trailing);
            list.previous_end = span.end.max(self.consumed_end);
            list.force_blank = blank_after(index);
        }
        while let Some(comment) = self.comment_before(close_offset) {
            self.take_comment();
            list.separate(self, comment.span.start);
            list.parts.push(document::comment(
                self.comment_text(comment),
                false,
                self.ends_line(comment),
            ));
            list.inline = false;
            list.previous_end = comment.span.end;
        }
        if list.parts.is_empty() {
            None
        } else {
            Some(concat(list.parts))
        }
    }

    /// A delimited, comma-separated list (parameters, arguments, shorthand fields, payloads):
    /// flat when it fits, otherwise one item per line, with comments kept beside their items.
    pub(crate) fn comma_list<T>(
        &mut self,
        items: &[T],
        span_of: impl Fn(&T) -> Span,
        mut print_item: impl FnMut(&mut Self, &T) -> Document,
        list: CommaList,
    ) -> Document {
        let mut parts = Vec::new();
        for (index, item) in items.iter().enumerate() {
            let span = span_of(item);
            let is_last = index + 1 == items.len();
            let leading = self.comments_before(span.start);
            parts.push(leading);
            let document = print_item(self, item);
            parts.push(document);
            if !is_last {
                parts.push(text(","));
            } else if list.trailing_comma {
                parts.push(if_break(text(","), Document::Empty));
            }
            // Comments inside the item that its printing did not place, and comments after it
            // on its line, follow the separator, where a second formatting finds them again.
            let leftovers = self.comments_at_end(span.end);
            parts.push(leftovers);
            let limit = items
                .get(index + 1)
                .map_or(list.close_offset, |next| span_of(next).start);
            let trailing = self.comments_on_line(span.end, limit);
            parts.push(trailing);
            if !is_last {
                parts.push(line());
            }
        }
        let dangling = self.comments_dangling(list.close_offset, !items.is_empty());
        if items.is_empty() && dangling.is_empty() {
            return text(format!("{}{}", list.open, list.close));
        }
        parts.push(dangling);
        group(concat(vec![
            text(list.open),
            indent(concat(vec![soft_line(), concat(parts)])),
            soft_line(),
            text(list.close),
        ]))
    }

    /// `{` and `}` around a vertical list, or `{}` when it is empty.
    pub(crate) fn braced(&mut self, contents: Option<Document>) -> Document {
        match contents {
            Some(contents) => concat(vec![
                text("{"),
                indent(concat(vec![hard_line(), contents])),
                hard_line(),
                text("}"),
            ]),
            None => text("{}"),
        }
    }
}

/// The state of [`Printer::vertical_list`] between items.
#[derive(Default)]
struct VerticalList {
    parts: Vec<Document>,
    /// The end of the last item or comment printed.
    previous_end: usize,
    /// The last thing printed is a block comment that the next item follows on its line.
    inline: bool,
    force_blank: bool,
}

impl VerticalList {
    /// The line break before something starting at `next_start`: none for the first thing or
    /// after an inline comment, otherwise a newline, and a blank line where the source has one.
    fn separate(&mut self, printer: &Printer, next_start: usize) {
        if self.inline || self.parts.is_empty() {
            return;
        }
        self.parts.push(hard_line());
        if self.force_blank || printer.has_blank_line_between(self.previous_end, next_start) {
            self.parts.push(hard_line());
        }
        self.force_blank = false;
    }
}

//! The document algebra the printer builds and the width-aware renderer that lays it out
//! (Wadler's "prettier printer", with Prettier's `fits` rule).
//!
//! A [`Document`] describes every layout the printer allows: a [`Document::Group`] is printed
//! flat (each [`Document::Line`] a space, each [`Document::SoftLine`] nothing) when it fits in
//! the rest of the line, and broken (each line a newline) otherwise. A group that contains a
//! hard line or a comment that ends its line is always broken.

/// Spaces per indentation level.
pub const INDENT_WIDTH: usize = 4;

/// The line width the renderer fills, in characters.
pub const LINE_WIDTH: usize = 100;

/// A layout description. Build it with the helper functions of this module, which keep the
/// broken flag of each group up to date.
#[derive(Debug, Clone, PartialEq)]
pub enum Document {
    /// Nothing.
    Empty,
    /// Text without newlines.
    Text(String),
    /// A space when flat, a newline when broken.
    Line,
    /// Nothing when flat, a newline when broken.
    SoftLine,
    /// Always a newline.
    HardLine,
    /// The contents, one indentation level deeper after each newline.
    Indent(Box<Document>),
    /// Laid out flat when it fits, otherwise broken. `broken` is set when the contents force a
    /// break (a hard line, a comment ending its line, a broken group).
    Group {
        contents: Box<Document>,
        broken: bool,
    },
    /// `broken` in a broken group, `flat` in a flat one.
    IfBreak {
        broken: Box<Document>,
        flat: Box<Document>,
    },
    /// A sequence.
    Concat(Vec<Document>),
    /// The contents laid out flat whatever the width, for what is never broken across lines
    /// (the expression parts of a string template, D14 of `string-templates`).
    Flat(Box<Document>),
    /// Source text printed exactly as written: its later lines are not indented, and a newline
    /// in it breaks its enclosing groups (a string template kept as written).
    Verbatim(String),
    /// A comment, printed verbatim (a block comment may span lines). With `newline_before`, it
    /// starts a new line unless the current line is still empty; with `newline_after`, it ends
    /// its line: whatever is printed next starts on a new line.
    Comment {
        text: String,
        newline_before: bool,
        newline_after: bool,
    },
}

pub fn text(value: impl Into<String>) -> Document {
    Document::Text(value.into())
}

pub fn line() -> Document {
    Document::Line
}

pub fn soft_line() -> Document {
    Document::SoftLine
}

pub fn hard_line() -> Document {
    Document::HardLine
}

pub fn indent(contents: Document) -> Document {
    Document::Indent(Box::new(contents))
}

pub fn group(contents: Document) -> Document {
    let broken = contents.forces_break();
    Document::Group {
        contents: Box::new(contents),
        broken,
    }
}

pub fn if_break(broken: Document, flat: Document) -> Document {
    Document::IfBreak {
        broken: Box::new(broken),
        flat: Box::new(flat),
    }
}

pub fn concat(parts: Vec<Document>) -> Document {
    Document::Concat(parts)
}

pub fn flat(contents: Document) -> Document {
    Document::Flat(Box::new(contents))
}

pub fn verbatim(value: impl Into<String>) -> Document {
    Document::Verbatim(value.into())
}

pub fn comment(text: impl Into<String>, newline_before: bool, newline_after: bool) -> Document {
    Document::Comment {
        text: text.into(),
        newline_before,
        newline_after,
    }
}

impl Document {
    /// Whether this document prints nothing at all.
    pub fn is_empty(&self) -> bool {
        match self {
            Document::Empty => true,
            Document::Text(value) => value.is_empty(),
            Document::Concat(parts) => parts.iter().all(Document::is_empty),
            _ => false,
        }
    }

    /// Whether laying this document out always breaks its enclosing groups. Nested groups
    /// answer from their cached flag, so building a tree bottom-up stays linear.
    pub(crate) fn forces_break(&self) -> bool {
        match self {
            Document::Empty | Document::Text(_) | Document::Line | Document::SoftLine => false,
            Document::HardLine => true,
            Document::Indent(contents) | Document::Flat(contents) => contents.forces_break(),
            Document::Verbatim(value) => value.contains('\n'),
            Document::Group { broken, .. } => *broken,
            Document::IfBreak { broken, flat } => broken.forces_break() || flat.forces_break(),
            Document::Concat(parts) => parts.iter().any(Document::forces_break),
            Document::Comment {
                newline_before,
                newline_after,
                ..
            } => *newline_before || *newline_after,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Flat,
    Break,
}

/// One pending piece of work for the renderer: a document at an indentation, in the mode of
/// its enclosing group.
#[derive(Clone, Copy)]
struct Command<'a> {
    indent: usize,
    mode: Mode,
    document: &'a Document,
}

/// Lays `document` out at [`LINE_WIDTH`]. Lines never end in whitespace.
pub fn render(document: &Document) -> String {
    let mut output = Output::default();
    let mut commands = vec![Command {
        indent: 0,
        mode: Mode::Break,
        document,
    }];
    while let Some(command) = commands.pop() {
        match command.document {
            Document::Empty => {}
            Document::Text(value) | Document::Verbatim(value) => {
                output.write(value, command.indent)
            }
            Document::Flat(contents) => commands.push(Command {
                mode: Mode::Flat,
                document: contents,
                ..command
            }),
            Document::Line | Document::SoftLine | Document::HardLine => {
                let breaks = command.mode == Mode::Break
                    || matches!(command.document, Document::HardLine)
                    || output.pending_newline;
                if breaks {
                    output.newline();
                } else if matches!(command.document, Document::Line) {
                    output.write(" ", command.indent);
                }
            }
            Document::Indent(contents) => commands.push(Command {
                indent: command.indent + 1,
                document: contents,
                ..command
            }),
            Document::Group { contents, broken } => {
                let flat = Command {
                    mode: Mode::Flat,
                    document: contents,
                    ..command
                };
                // At the start of a line (indentation is written lazily), or after a comment
                // that ends its line, the group starts at its indentation.
                let column = if output.pending_newline || output.line_is_blank() {
                    command.indent * INDENT_WIDTH
                } else {
                    output.column
                };
                // Inside a flat group, a group that is not forced to break is flat too.
                let is_flat = !broken
                    && (command.mode == Mode::Flat
                        || fits(flat, &commands, LINE_WIDTH.saturating_sub(column)));
                let mode = if is_flat { Mode::Flat } else { Mode::Break };
                commands.push(Command { mode, ..flat });
            }
            Document::IfBreak { broken, flat } => commands.push(Command {
                document: if command.mode == Mode::Break {
                    broken
                } else {
                    flat
                },
                ..command
            }),
            Document::Concat(parts) => {
                for part in parts.iter().rev() {
                    commands.push(Command {
                        document: part,
                        ..command
                    });
                }
            }
            Document::Comment {
                text,
                newline_before,
                newline_after,
            } => {
                if *newline_before && !output.line_is_blank() {
                    output.newline();
                } else if text.starts_with("//")
                    && !output.pending_newline
                    && !output.line_is_blank()
                    && !output.text.ends_with(' ')
                {
                    // A line comment after code is separated from it by one space.
                    output.write(" ", command.indent);
                }
                let after_code = !output.pending_newline && !output.line_is_blank();
                let line_indent = output.line_indent();
                output.write(text, command.indent);
                if *newline_after {
                    output.pending_newline = true;
                    if after_code {
                        // What the comment pushes to the next line continues the line the
                        // comment ends, so it is indented one level deeper than that line.
                        output.continuation_indent = line_indent + 1;
                    }
                }
            }
        }
    }
    output.finish()
}

/// Whether `next`, laid out flat, and then the `rest` of the commands up to their first line
/// break, fit in `width` characters (Prettier's rule).
fn fits(next: Command, rest: &[Command], mut width: usize) -> bool {
    let mut stack = vec![next];
    let mut rest_index = rest.len();
    loop {
        let command = match stack.pop() {
            Some(command) => command,
            None if rest_index == 0 => return true,
            None => {
                rest_index -= 1;
                rest[rest_index]
            }
        };
        match command.document {
            Document::Empty => {}
            Document::Text(value) => match width.checked_sub(value.chars().count()) {
                Some(left) => width = left,
                None => return false,
            },
            Document::Verbatim(value) => {
                let first_line = value.split('\n').next().unwrap_or_default();
                match width.checked_sub(first_line.chars().count()) {
                    Some(left) => width = left,
                    None => return false,
                }
                if value.contains('\n') {
                    return true;
                }
            }
            Document::Flat(contents) => stack.push(Command {
                mode: Mode::Flat,
                document: contents,
                ..command
            }),
            Document::Line | Document::SoftLine | Document::HardLine => {
                if command.mode == Mode::Break || matches!(command.document, Document::HardLine) {
                    return true;
                }
                if matches!(command.document, Document::Line) {
                    match width.checked_sub(1) {
                        Some(left) => width = left,
                        None => return false,
                    }
                }
            }
            Document::Indent(contents) => stack.push(Command {
                document: contents,
                ..command
            }),
            Document::Group { contents, broken } => stack.push(Command {
                mode: if *broken { Mode::Break } else { command.mode },
                document: contents,
                ..command
            }),
            Document::IfBreak { broken, flat } => stack.push(Command {
                document: if command.mode == Mode::Break {
                    broken
                } else {
                    flat
                },
                ..command
            }),
            Document::Concat(parts) => {
                for part in parts.iter().rev() {
                    stack.push(Command {
                        document: part,
                        ..command
                    });
                }
            }
            Document::Comment {
                text,
                newline_before,
                newline_after,
            } => {
                if *newline_before {
                    return true;
                }
                let first_line = text.split('\n').next().unwrap_or_default();
                match width.checked_sub(first_line.chars().count()) {
                    Some(left) => width = left,
                    None => return false,
                }
                if *newline_after || text.contains('\n') {
                    return true;
                }
            }
        }
    }
}

/// The text being rendered, with the state the layout rules need.
#[derive(Default)]
struct Output {
    text: String,
    /// Characters on the current line.
    column: usize,
    /// Set by a comment that ends its line: the next text starts on a new line.
    pending_newline: bool,
    /// The least indentation of the line that a pending newline starts, when the comment that
    /// set it followed code.
    continuation_indent: usize,
}

impl Output {
    fn write(&mut self, value: &str, mut indent: usize) {
        if value.is_empty() {
            return;
        }
        if self.pending_newline {
            indent = indent.max(self.continuation_indent);
            self.newline();
        }
        if self.column == 0 && self.text.ends_with('\n') || self.text.is_empty() {
            // Indentation is written lazily, so a blank line holds no spaces.
            self.text.push_str(&" ".repeat(indent * INDENT_WIDTH));
            self.column = indent * INDENT_WIDTH;
        }
        self.text.push_str(value);
        match value.rfind('\n') {
            Some(index) => self.column = value[index + 1..].chars().count(),
            None => self.column += value.chars().count(),
        }
    }

    fn newline(&mut self) {
        let trimmed = self.text.trim_end_matches(' ').len();
        self.text.truncate(trimmed);
        self.text.push('\n');
        self.column = 0;
        self.pending_newline = false;
        self.continuation_indent = 0;
    }

    /// The indentation level of the current line.
    fn line_indent(&self) -> usize {
        let line_start = self.text.rfind('\n').map_or(0, |index| index + 1);
        let line = &self.text[line_start..];
        (line.len() - line.trim_start_matches(' ').len()) / INDENT_WIDTH
    }

    /// Whether nothing but indentation has been written on the current line.
    fn line_is_blank(&self) -> bool {
        self.text.is_empty() || self.text.ends_with('\n')
    }

    fn finish(mut self) -> String {
        let trimmed = self.text.trim_end_matches(' ').len();
        self.text.truncate(trimmed);
        self.text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(arguments: &[&str]) -> Document {
        let mut items = Vec::new();
        for (index, argument) in arguments.iter().enumerate() {
            if index > 0 {
                items.push(text(","));
                items.push(line());
            }
            items.push(text(*argument));
        }
        items.push(if_break(text(","), Document::Empty));
        group(concat(vec![
            text("f("),
            indent(concat(vec![soft_line(), concat(items)])),
            soft_line(),
            text(")"),
        ]))
    }

    #[test]
    fn a_group_that_fits_stays_flat() {
        assert_eq!(render(&call(&["a", "b"])), "f(a, b)");
    }

    #[test]
    fn a_group_that_does_not_fit_breaks_with_its_if_break() {
        let long = "x".repeat(60);
        assert_eq!(
            render(&call(&[&long, &long])),
            format!("f(\n    {},\n    {},\n)", long, long)
        );
    }

    #[test]
    fn the_rest_of_the_line_counts_towards_fitting() {
        let argument = "x".repeat(94);
        let flat = concat(vec![call(&[&argument]), text(" {")]);
        // `f(` + 94 + `)` is 97 characters, and ` {` makes 99: it fits.
        assert_eq!(render(&flat), format!("f({}) {{", argument));
        let too_long = concat(vec![call(&[&argument]), text(" {}}")]);
        assert!(render(&too_long).starts_with("f(\n"));
    }

    #[test]
    fn a_line_comment_ends_its_line_and_breaks_its_groups() {
        let document = group(concat(vec![
            text("a"),
            line(),
            comment("// note", false, true),
            text("b"),
        ]));
        assert_eq!(render(&document), "a\n// note\nb");
    }

    #[test]
    fn an_own_line_comment_starts_a_line_unless_the_line_is_empty() {
        let document = concat(vec![
            text("a"),
            comment("/* x */", true, false),
            text(" b"),
            hard_line(),
            comment("/* y */", true, false),
        ]);
        assert_eq!(render(&document), "a\n/* x */ b\n/* y */");
    }

    #[test]
    fn a_flat_document_stays_on_one_line_whatever_the_width() {
        let long = "x".repeat(60);
        let document = concat(vec![text("\"${"), flat(call(&[&long, &long])), text("}\"")]);
        assert_eq!(render(&document), format!("\"${{f({}, {})}}\"", long, long));
        // Its width still counts towards the fitting of an enclosing group.
        let enclosing = group(concat(vec![
            text("g("),
            indent(concat(vec![soft_line(), document])),
            soft_line(),
            text(")"),
        ]));
        assert!(render(&enclosing).starts_with("g(\n    \"${f("));
    }

    #[test]
    fn a_verbatim_text_keeps_its_lines_and_breaks_its_groups() {
        let source = "\"a ${when (n) {\n    1 -> \"one\"\n}} b\"";
        let document = indent(concat(vec![
            hard_line(),
            group(concat(vec![text("val s ="), line(), verbatim(source)])),
        ]));
        assert_eq!(render(&document), format!("\n    val s =\n    {}", source));
        assert!(verbatim(source).forces_break());
        assert!(!verbatim("\"one line\"").forces_break());
    }

    #[test]
    fn blank_lines_and_line_ends_hold_no_spaces() {
        let document = indent(concat(vec![
            hard_line(),
            text("a "),
            hard_line(),
            hard_line(),
            text("b"),
        ]));
        assert_eq!(render(&document), "\n    a\n\n    b");
    }
}

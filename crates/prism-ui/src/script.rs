//! Event scripts (D16 of `ui-toolkit`): the commands of a headless run, parsed before the
//! program runs so that a syntax error is reported before any of its output.
//!
//! One command per line; blank lines are allowed and `#` starts a comment outside strings.
//! Strings are double-quoted, with `\"`, `\\` and `\n` escapes. The commands:
//!
//! - `click "<text>" [<n>]` and `click <x> <y>`;
//! - `press` and `release`, with the same targets;
//! - `type "<text>"`;
//! - `key backspace` and `key enter`;
//! - `resize <width> <height>`;
//! - `frame`;
//! - `dump`, `dump tree` and `dump draw`;
//! - under `--watch` only, `edit <target> <replacement>` and `wait reload` (D12 of
//!   `compose-hot-reload`). An `edit` makes the session read `<target>` as the content of
//!   `<replacement>`, both relative to the script's directory; the replacement is read when the
//!   script is parsed, and consecutive `edit` lines form one command, applied as one reload.

use crate::input::Key;
use prism_runtime::reload::ScriptedEdit;
use std::fmt;
use std::path::{Path, PathBuf};

/// What `dump` prints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DumpKind {
    /// The laid-out tree (`dump` and `dump tree`).
    Tree,
    /// The display list (`dump draw`).
    Draw,
}

/// Where a pointer command acts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptTarget {
    /// The centre of the `occurrence`-th node, from 1 in tree order, showing exactly `text`.
    Text { text: String, occurrence: usize },
    /// A point of the window.
    Point(i64, i64),
}

/// One command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptCommand {
    Click(ScriptTarget),
    Press(ScriptTarget),
    Release(ScriptTarget),
    Type(String),
    Key(Key),
    Resize(i64, i64),
    Frame,
    Dump(DumpKind),
    /// One group of consecutive `edit` lines, applied as one reload.
    Edit(Vec<ScriptedEdit>),
    /// `wait reload`: waits for a reload from the session's file watcher.
    WaitReload,
}

/// What parsing a script needs beyond its text: whether the run watches, which allows `edit`
/// and `wait`, the script's directory, which `edit` paths are relative to, and how to read a
/// replacement file.
pub struct ScriptContext<'a> {
    pub watching: bool,
    pub directory: PathBuf,
    pub read: &'a dyn Fn(&Path) -> Result<String, String>,
}

/// A command and the line it is on, from 1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptLine {
    pub line: usize,
    pub command: ScriptCommand,
}

/// A syntax error of a script, on `line`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptError {
    pub line: usize,
    pub message: String,
}

impl fmt::Display for ScriptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.line, self.message)
    }
}

/// A token of a line.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Word(String),
    Text(String),
}

impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Token::Word(word) => f.write_str(word),
            Token::Text(_) => f.write_str("a string"),
        }
    }
}

/// The tokens of one line, up to its comment.
fn tokens(line: &str) -> Result<Vec<Token>, String> {
    let mut tokens = Vec::new();
    let mut characters = line.chars().peekable();
    while let Some(&character) = characters.peek() {
        if character.is_whitespace() {
            characters.next();
        } else if character == '#' {
            break;
        } else if character == '"' {
            characters.next();
            let mut text = String::new();
            loop {
                match characters.next() {
                    None => return Err("unterminated string".to_string()),
                    Some('"') => break,
                    Some('\\') => match characters.next() {
                        Some('"') => text.push('"'),
                        Some('\\') => text.push('\\'),
                        Some('n') => text.push('\n'),
                        Some(other) => return Err(format!("unknown escape '\\{}'", other)),
                        None => return Err("unterminated string".to_string()),
                    },
                    Some(other) => text.push(other),
                }
            }
            tokens.push(Token::Text(text));
        } else {
            let mut word = String::new();
            while let Some(&character) = characters.peek() {
                if character.is_whitespace() || character == '"' || character == '#' {
                    break;
                }
                word.push(character);
                characters.next();
            }
            tokens.push(Token::Word(word));
        }
    }
    Ok(tokens)
}

fn number(token: &Token) -> Option<i64> {
    match token {
        Token::Word(word) => word.parse().ok(),
        Token::Text(_) => None,
    }
}

/// Parses one line's tokens into a command.
fn command(tokens: &[Token], context: &ScriptContext) -> Result<ScriptCommand, String> {
    let Token::Word(name) = &tokens[0] else {
        return Err("expected a command".to_string());
    };
    let arguments = &tokens[1..];
    let at_most = |count: usize| match arguments.get(count) {
        Some(extra) => Err(format!("unexpected {} after '{}'", quoted(extra), name)),
        None => Ok(()),
    };
    let target = || -> Result<ScriptTarget, String> {
        let expected = || format!("expected a string or two numbers after '{}'", name);
        match arguments.first() {
            Some(Token::Text(text)) => {
                let occurrence = match arguments.get(1) {
                    None => 1,
                    Some(token) => match number(token) {
                        Some(occurrence) if occurrence >= 1 => occurrence as usize,
                        Some(_) => return Err("an occurrence counts from 1".to_string()),
                        None => {
                            return Err(format!(
                                "expected an occurrence after the string of '{}'",
                                name
                            ))
                        }
                    },
                };
                at_most(2)?;
                Ok(ScriptTarget::Text {
                    text: text.clone(),
                    occurrence,
                })
            }
            Some(first) => {
                let (Some(x), Some(y)) = (number(first), arguments.get(1).and_then(number)) else {
                    return Err(expected());
                };
                at_most(2)?;
                Ok(ScriptTarget::Point(x, y))
            }
            None => Err(expected()),
        }
    };
    Ok(match name.as_str() {
        "click" => ScriptCommand::Click(target()?),
        "press" => ScriptCommand::Press(target()?),
        "release" => ScriptCommand::Release(target()?),
        "type" => match arguments.first() {
            Some(Token::Text(text)) => {
                at_most(1)?;
                ScriptCommand::Type(text.clone())
            }
            _ => return Err("expected a string after 'type'".to_string()),
        },
        "key" => {
            let key = match arguments.first() {
                Some(Token::Word(word)) if word == "backspace" => Key::Backspace,
                Some(Token::Word(word)) if word == "enter" => Key::Enter,
                _ => return Err("expected 'backspace' or 'enter' after 'key'".to_string()),
            };
            at_most(1)?;
            ScriptCommand::Key(key)
        }
        "resize" => {
            let (Some(width), Some(height)) = (
                arguments.first().and_then(number),
                arguments.get(1).and_then(number),
            ) else {
                return Err("expected two numbers after 'resize'".to_string());
            };
            if width < 0 || height < 0 {
                return Err("a window size cannot be negative".to_string());
            }
            at_most(2)?;
            ScriptCommand::Resize(width, height)
        }
        "frame" => {
            at_most(0)?;
            ScriptCommand::Frame
        }
        "dump" => {
            let kind = match arguments.first() {
                None => DumpKind::Tree,
                Some(Token::Word(word)) if word == "tree" => DumpKind::Tree,
                Some(Token::Word(word)) if word == "draw" => DumpKind::Draw,
                Some(_) => return Err("expected 'tree' or 'draw' after 'dump'".to_string()),
            };
            at_most(1)?;
            ScriptCommand::Dump(kind)
        }
        "edit" => {
            if !context.watching {
                return Err("the edit command needs --watch".to_string());
            }
            let (Some(Token::Word(target)), Some(Token::Word(replacement))) =
                (arguments.first(), arguments.get(1))
            else {
                return Err("expected a target and a replacement after 'edit'".to_string());
            };
            at_most(2)?;
            let replacement = context.directory.join(replacement);
            let content = (context.read)(&replacement)
                .map_err(|error| format!("cannot read {}: {}", replacement.display(), error))?;
            ScriptCommand::Edit(vec![ScriptedEdit {
                target: context.directory.join(target),
                content,
            }])
        }
        "wait" => {
            if !context.watching {
                return Err("the wait command needs --watch".to_string());
            }
            match arguments.first() {
                Some(Token::Word(word)) if word == "reload" => {}
                _ => return Err("expected 'reload' after 'wait'".to_string()),
            }
            at_most(1)?;
            ScriptCommand::WaitReload
        }
        other => return Err(format!("unknown command '{}'", other)),
    })
}

fn quoted(token: &Token) -> String {
    match token {
        Token::Word(word) => format!("'{}'", word),
        Token::Text(_) => "a string".to_string(),
    }
}

/// Parses a whole script of a run that does not watch, in which `edit` and `wait` are errors.
pub fn parse_script(text: &str) -> Result<Vec<ScriptLine>, ScriptError> {
    parse_script_with(
        text,
        &ScriptContext {
            watching: false,
            directory: PathBuf::new(),
            read: &|_| Err("no file is read".to_string()),
        },
    )
}

/// Parses a whole script in `context`.
pub fn parse_script_with(
    text: &str,
    context: &ScriptContext,
) -> Result<Vec<ScriptLine>, ScriptError> {
    let mut lines: Vec<ScriptLine> = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let error = |message| ScriptError {
            line: index + 1,
            message,
        };
        let tokens = tokens(line).map_err(error)?;
        if tokens.is_empty() {
            continue;
        }
        let command = command(&tokens, context).map_err(error)?;
        // Consecutive edits are one reload.
        if let (
            ScriptCommand::Edit(edits),
            Some(ScriptLine {
                command: ScriptCommand::Edit(group),
                ..
            }),
        ) = (&command, lines.last_mut())
        {
            group.extend(edits.iter().cloned());
            continue;
        }
        lines.push(ScriptLine {
            line: index + 1,
            command,
        });
    }
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(text: &str) -> Vec<ScriptCommand> {
        parse_script(text)
            .unwrap()
            .into_iter()
            .map(|line| line.command)
            .collect()
    }

    fn error(text: &str) -> String {
        parse_script(text).unwrap_err().to_string()
    }

    fn text(text: &str, occurrence: usize) -> ScriptTarget {
        ScriptTarget::Text {
            text: text.to_string(),
            occurrence,
        }
    }

    #[test]
    fn every_command() {
        assert_eq!(
            parsed(
                "click \"+\"\nclick \"item\" 2\nclick 10 -5\npress \"a\"\npress 1 2\n\
                 release \"a\" 3\nrelease 3 4\ntype \"ab\"\nkey backspace\nkey enter\n\
                 resize 200 100\nframe\ndump\ndump tree\ndump draw\n"
            ),
            [
                ScriptCommand::Click(text("+", 1)),
                ScriptCommand::Click(text("item", 2)),
                ScriptCommand::Click(ScriptTarget::Point(10, -5)),
                ScriptCommand::Press(text("a", 1)),
                ScriptCommand::Press(ScriptTarget::Point(1, 2)),
                ScriptCommand::Release(text("a", 3)),
                ScriptCommand::Release(ScriptTarget::Point(3, 4)),
                ScriptCommand::Type("ab".to_string()),
                ScriptCommand::Key(Key::Backspace),
                ScriptCommand::Key(Key::Enter),
                ScriptCommand::Resize(200, 100),
                ScriptCommand::Frame,
                ScriptCommand::Dump(DumpKind::Tree),
                ScriptCommand::Dump(DumpKind::Tree),
                ScriptCommand::Dump(DumpKind::Draw),
            ]
        );
    }

    #[test]
    fn comments_blank_lines_escapes_and_line_numbers() {
        let lines = parse_script(
            "# two clicks\n\n  click \"+\"  # the button\nclick \"a \\\"b\\\" \\\\ \\n#\"\n",
        )
        .unwrap();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].line, 3);
        assert_eq!(lines[0].command, ScriptCommand::Click(text("+", 1)));
        assert_eq!(lines[1].line, 4);
        assert_eq!(
            lines[1].command,
            ScriptCommand::Click(text("a \"b\" \\ \n#", 1))
        );
        assert!(parse_script("").unwrap().is_empty());
    }

    #[test]
    fn every_error() {
        assert_eq!(error("clik \"+\""), "1: unknown command 'clik'");
        assert_eq!(
            error("frame\nclick"),
            "2: expected a string or two numbers after 'click'"
        );
        assert_eq!(
            error("click 10"),
            "1: expected a string or two numbers after 'click'"
        );
        assert_eq!(
            error("press x y"),
            "1: expected a string or two numbers after 'press'"
        );
        assert_eq!(error("click \"+"), "1: unterminated string");
        assert_eq!(error("click \"\\q\""), "1: unknown escape '\\q'");
        assert_eq!(error("click \"+\" 0"), "1: an occurrence counts from 1");
        assert_eq!(
            error("release \"+\" two"),
            "1: expected an occurrence after the string of 'release'"
        );
        assert_eq!(error("click 1 2 3"), "1: unexpected '3' after 'click'");
        assert_eq!(
            error("click \"a\" 1 \"b\""),
            "1: unexpected a string after 'click'"
        );
        assert_eq!(error("type ab"), "1: expected a string after 'type'");
        assert_eq!(error("type"), "1: expected a string after 'type'");
        assert_eq!(
            error("key space"),
            "1: expected 'backspace' or 'enter' after 'key'"
        );
        assert_eq!(
            error("resize 200"),
            "1: expected two numbers after 'resize'"
        );
        assert_eq!(error("resize -1 5"), "1: a window size cannot be negative");
        assert_eq!(error("frame now"), "1: unexpected 'now' after 'frame'");
        assert_eq!(
            error("dump layout"),
            "1: expected 'tree' or 'draw' after 'dump'"
        );
        assert_eq!(
            error("edit main.prism main.prism.v2"),
            "1: the edit command needs --watch"
        );
        assert_eq!(error("wait reload"), "1: the wait command needs --watch");
        assert_eq!(error("\"+\""), "1: expected a command");
    }

    fn watching(text: &str) -> Result<Vec<ScriptLine>, ScriptError> {
        parse_script_with(
            text,
            &ScriptContext {
                watching: true,
                directory: PathBuf::from("/case"),
                read: &|path: &Path| Ok(format!("content of {}", path.display())),
            },
        )
    }

    /// D12 of `compose-hot-reload`: consecutive edits form one command, read relative to the
    /// script's directory; `wait reload` waits.
    #[test]
    fn edits_and_waits_under_watch() {
        let lines = watching(
            "edit lib/labels.prism lib/labels.prism.v2\n\
             edit app/main.prism app/main.prism.v2\n\
             dump\nwait reload\nedit main.prism main.prism.v3\n",
        )
        .unwrap();
        let edit = |target: &str, replacement: &str| ScriptedEdit {
            target: PathBuf::from(format!("/case/{}", target)),
            content: format!("content of /case/{}", replacement),
        };
        let commands: Vec<(usize, ScriptCommand)> = lines
            .into_iter()
            .map(|line| (line.line, line.command))
            .collect();
        assert_eq!(
            commands,
            [
                (
                    1,
                    ScriptCommand::Edit(vec![
                        edit("lib/labels.prism", "lib/labels.prism.v2"),
                        edit("app/main.prism", "app/main.prism.v2")
                    ])
                ),
                (3, ScriptCommand::Dump(DumpKind::Tree)),
                (4, ScriptCommand::WaitReload),
                (
                    5,
                    ScriptCommand::Edit(vec![edit("main.prism", "main.prism.v3")])
                ),
            ]
        );
        let error = |text: &str| watching(text).unwrap_err().to_string();
        assert_eq!(
            error("edit main.prism"),
            "1: expected a target and a replacement after 'edit'"
        );
        assert_eq!(error("wait"), "1: expected 'reload' after 'wait'");
        assert_eq!(error("wait reload now"), "1: unexpected 'now' after 'wait'");
        let missing = parse_script_with(
            "edit a b\n",
            &ScriptContext {
                watching: true,
                directory: PathBuf::from("/case"),
                read: &|_| Err("no such file".to_string()),
            },
        )
        .unwrap_err()
        .to_string();
        assert_eq!(missing, "1: cannot read /case/b: no such file");
    }
}

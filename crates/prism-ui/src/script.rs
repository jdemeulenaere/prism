//! Event scripts (D16 of `ui-toolkit`): the commands of a headless run, parsed before the
//! program runs so that a syntax error is reported before any of its output.
//!
//! One command per line; blank lines are allowed and `#` starts a comment outside strings.
//! Strings are double-quoted, with `\"`, `\\` and `\n` escapes. The commands:
//!
//! - `click "<text>" [<n>]`, `click <kind> [<n>]` and `click <x> <y>`;
//! - `press` and `release`, with the same targets;
//! - `type "<text>"`;
//! - `key backspace` and `key enter`;
//! - `resize <width> <height>`;
//! - `frame`;
//! - `pause <milliseconds>`, which sleeps outside every frame, to pace inputs as a person does
//!   (D15 of `strong-skipping`);
//! - `dump`, `dump tree`, `dump draw` and `dump recompositions`, the last printing how many
//!   times each composable function of the program ran and was skipped since the previous one
//!   (D14 of `strong-skipping`);
//! - under `--watch` only, `edit <target> <replacement>` and `wait reload` (D12 of
//!   `compose-hot-reload`). An `edit` makes the session read `<target>` as the content of
//!   `<replacement>`, both relative to the script's directory; the replacement is read when the
//!   script is parsed, and consecutive `edit` lines form one command, applied as one reload.
//!
//! A kind target (D5 of `reactive-app-verification`) names the n-th node, from 1 in tree order,
//! of a kind the dump prints (`column`, `row`, `box`, `text`, `spacer`, `textfield`, `layout`)
//! or whose `clickable` or `toggleable` element has a role (`button`, `checkbox`), for nodes
//! that show no text: `click checkbox 2`, `click textfield`. The three target forms do not
//! overlap: a text target is quoted, a point is two integers and a kind is a word; another word
//! is `unknown target '<word>'`.

use crate::input::Key;
use crate::node::NodeKind;
use crate::properties::Role;
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
    /// The runs and skips of each composable function of the program since the run started or
    /// the previous `dump recompositions` (D14 of `strong-skipping`).
    Recompositions,
}

/// What a kind target names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetKind {
    /// Nodes of a kind, as the dump names them.
    Node(NodeKind),
    /// Nodes whose `clickable` or `toggleable` element has a role.
    Role(Role),
}

impl TargetKind {
    /// The kind a target word names: a node kind but the window, or `button` or `checkbox`.
    pub fn from_word(word: &str) -> Option<TargetKind> {
        Some(match word {
            "column" => TargetKind::Node(NodeKind::Column),
            "row" => TargetKind::Node(NodeKind::Row),
            "box" => TargetKind::Node(NodeKind::Box),
            "text" => TargetKind::Node(NodeKind::Text),
            "spacer" => TargetKind::Node(NodeKind::Spacer),
            "textfield" => TargetKind::Node(NodeKind::TextField),
            "layout" => TargetKind::Node(NodeKind::Layout),
            "button" => TargetKind::Role(Role::Button),
            "checkbox" => TargetKind::Role(Role::Checkbox),
            _ => return None,
        })
    }

    /// The word of the kind in scripts and errors.
    pub fn word(self) -> &'static str {
        match self {
            TargetKind::Node(kind) => kind.name(),
            TargetKind::Role(Role::Button) => "button",
            TargetKind::Role(Role::Checkbox) => "checkbox",
            TargetKind::Role(_) => "role",
        }
    }
}

/// Where a pointer command acts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptTarget {
    /// The centre of the `occurrence`-th node, from 1 in tree order, showing exactly `text`.
    Text { text: String, occurrence: usize },
    /// The centre of the `occurrence`-th node, from 1 in tree order, of `kind`.
    Kind { kind: TargetKind, occurrence: usize },
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
    /// `pause <milliseconds>`: the program thread sleeps, outside every frame (D15 of
    /// `strong-skipping`).
    Pause(u64),
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
            Some(Token::Word(word)) if number(&Token::Word(word.clone())).is_none() => {
                let Some(kind) = TargetKind::from_word(word) else {
                    return Err(format!("unknown target '{}'", word));
                };
                let occurrence = match arguments.get(1) {
                    None => 1,
                    Some(token) => match number(token) {
                        Some(occurrence) if occurrence >= 1 => occurrence as usize,
                        Some(_) => return Err("an occurrence counts from 1".to_string()),
                        None => {
                            return Err(format!(
                                "expected an occurrence after the kind of '{}'",
                                name
                            ))
                        }
                    },
                };
                at_most(2)?;
                Ok(ScriptTarget::Kind { kind, occurrence })
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
        "pause" => {
            let milliseconds = match arguments.first().and_then(number) {
                Some(milliseconds) if milliseconds >= 0 => milliseconds as u64,
                _ => return Err("expected a number of milliseconds after 'pause'".to_string()),
            };
            at_most(1)?;
            ScriptCommand::Pause(milliseconds)
        }
        "dump" => {
            let kind = match arguments.first() {
                None => DumpKind::Tree,
                Some(Token::Word(word)) if word == "tree" => DumpKind::Tree,
                Some(Token::Word(word)) if word == "draw" => DumpKind::Draw,
                Some(Token::Word(word)) if word == "recompositions" => DumpKind::Recompositions,
                Some(_) => {
                    return Err(
                        "expected 'tree', 'draw' or 'recompositions' after 'dump'".to_string()
                    )
                }
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
                 resize 200 100\nframe\ndump\ndump tree\ndump draw\npause 100\n\
                 dump recompositions\n"
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
                ScriptCommand::Pause(100),
                ScriptCommand::Dump(DumpKind::Recompositions),
            ]
        );
    }

    /// D5 of `reactive-app-verification`: kind targets, with an occurrence or without.
    #[test]
    fn kind_targets() {
        let kind = |word: &str, occurrence: usize| ScriptTarget::Kind {
            kind: TargetKind::from_word(word).unwrap(),
            occurrence,
        };
        assert_eq!(
            parsed(
                "click checkbox 2\nclick textfield\npress button 1\nrelease text 3\n\
                 click column\nclick row\nclick box\nclick spacer\n"
            ),
            [
                ScriptCommand::Click(kind("checkbox", 2)),
                ScriptCommand::Click(kind("textfield", 1)),
                ScriptCommand::Press(kind("button", 1)),
                ScriptCommand::Release(kind("text", 3)),
                ScriptCommand::Click(kind("column", 1)),
                ScriptCommand::Click(kind("row", 1)),
                ScriptCommand::Click(kind("box", 1)),
                ScriptCommand::Click(kind("spacer", 1)),
            ]
        );
        assert_eq!(error("click slider"), "1: unknown target 'slider'");
        assert_eq!(error("click window"), "1: unknown target 'window'");
        assert_eq!(error("click checkbox 0"), "1: an occurrence counts from 1");
        assert_eq!(
            error("click checkbox two"),
            "1: expected an occurrence after the kind of 'click'"
        );
        assert_eq!(
            error("click checkbox 1 2"),
            "1: unexpected '2' after 'click'"
        );
        for word in [
            "column",
            "row",
            "box",
            "text",
            "spacer",
            "textfield",
            "button",
            "checkbox",
        ] {
            assert_eq!(TargetKind::from_word(word).unwrap().word(), word);
        }
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
            error("press 1 y"),
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
            "1: expected 'tree', 'draw' or 'recompositions' after 'dump'"
        );
        assert_eq!(
            error("pause"),
            "1: expected a number of milliseconds after 'pause'"
        );
        assert_eq!(
            error("pause -5"),
            "1: expected a number of milliseconds after 'pause'"
        );
        assert_eq!(error("pause 5 6"), "1: unexpected '6' after 'pause'");
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

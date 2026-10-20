//! The file structure, declarations and types.

use super::{CommaList, Printer};
use crate::document::*;
use prism_ast::*;
use prism_lexer::TokenKind;

/// An item of the top-level list.
enum TopLevelItem<'a> {
    Package(&'a PackageDecl),
    /// An import, printed at the place (the span) of the import that sorts there.
    Import(&'a ImportDecl, Span),
    Declaration(&'a Declaration),
}

impl TopLevelItem<'_> {
    fn span(&self) -> Span {
        match self {
            TopLevelItem::Package(package) => package.span,
            TopLevelItem::Import(_, place) => *place,
            TopLevelItem::Declaration(declaration) => declaration.span(),
        }
    }
}

/// The text an import is printed and sorted by: its path, with a leading `/` when it is
/// root-anchored, so root-anchored imports sort before sibling ones (D9 of
/// `explicit-library`).
pub(crate) fn import_text(import: &ImportDecl) -> String {
    format!(
        "{}{}",
        if import.is_root { "/" } else { "" },
        import.path.join("/")
    )
}

impl<'a> Printer<'a> {
    pub(crate) fn source_file(&mut self, file: &SourceFile) -> Document {
        let mut items = Vec::new();
        if let Some(package) = &file.package {
            items.push(TopLevelItem::Package(package));
        }
        // The imports sorted by path, each printed at the place of the import that sorts there,
        // unless a comment lies among them, which keeps them as written so that no comment
        // moves relative to code (D9 of `explicit-library`).
        let mut imports: Vec<&ImportDecl> = file.imports.iter().collect();
        if !self.comment_among_imports(&file.imports) {
            imports.sort_by_cached_key(|import| import_text(import));
        }
        items.extend(
            imports
                .into_iter()
                .zip(&file.imports)
                .map(|(import, place)| TopLevelItem::Import(import, place.span)),
        );
        items.extend(file.declarations.iter().map(TopLevelItem::Declaration));
        // One blank line after the package declaration, after the last import, and between
        // two declarations unless both are top-level `val`s (D8).
        let blank_after: Vec<bool> = (0..items.len())
            .map(|index| match (&items[index], items.get(index + 1)) {
                (_, None) => false,
                (TopLevelItem::Package(_), _) => true,
                (TopLevelItem::Import(..), Some(TopLevelItem::Import(..))) => false,
                (TopLevelItem::Import(..), _) => true,
                (
                    TopLevelItem::Declaration(Declaration::Var(_)),
                    Some(TopLevelItem::Declaration(Declaration::Var(_))),
                ) => false,
                (TopLevelItem::Declaration(_), _) => true,
            })
            .collect();
        let source_length = self.source.len();
        self.vertical_list(
            &items,
            TopLevelItem::span,
            |printer, item| match item {
                TopLevelItem::Package(package) => {
                    text(format!("package {}", package.path.join(".")))
                }
                TopLevelItem::Import(import, _) => text(format!("import {}", import_text(import))),
                TopLevelItem::Declaration(declaration) => printer.declaration(declaration),
            },
            source_length,
            |index| blank_after[index],
        )
        .unwrap_or(Document::Empty)
    }

    /// Whether a comment starts between the first import's start and the end of the last
    /// import's line.
    fn comment_among_imports(&self, imports: &[ImportDecl]) -> bool {
        let (Some(first), Some(last)) = (imports.first(), imports.last()) else {
            return false;
        };
        let line_end = self.source[last.span.end..]
            .find('\n')
            .map_or(self.source.len(), |offset| last.span.end + offset);
        self.comments
            .iter()
            .any(|comment| comment.span.start >= first.span.start && comment.span.start < line_end)
    }

    fn declaration(&mut self, declaration: &Declaration) -> Document {
        match declaration {
            Declaration::Function(function) => self.function(function),
            Declaration::Struct(structure) => self.structure(structure),
            Declaration::Enum(enumeration) => self.enumeration(enumeration),
            Declaration::Var(variable) => {
                let mut parts = vec![self.visibility(variable.visibility, variable.span)];
                parts.push(text(format!("val {}", variable.name)));
                if let Some(ty) = &variable.ty {
                    parts.push(text(": "));
                    parts.push(self.ty(ty));
                }
                parts.push(text(" ="));
                parts.push(self.right_side(&variable.initializer));
                concat(parts)
            }
            Declaration::ExtensionProperty(property) => {
                let mut parts = vec![self.visibility(property.visibility, property.span)];
                parts.push(text(format!(
                    "{} {} ",
                    property.library.kind.as_str(),
                    property.keyword()
                )));
                if !property.type_parameters.is_empty() {
                    parts.push(self.type_parameters(&property.type_parameters));
                    parts.push(text(" "));
                }
                parts.push(self.ty(&property.receiver));
                parts.push(text(format!(".{}: ", property.name)));
                parts.push(self.ty(&property.ty));
                concat(parts)
            }
        }
    }

    /// The visibility modifier and its space: `public` and `package` always, `private` only
    /// when written (D5), since it is the default.
    fn visibility(&self, visibility: Visibility, span: Span) -> Document {
        match visibility {
            Visibility::Public => text("public "),
            Visibility::Package => text("package "),
            Visibility::Private if self.kind_at(span.start) == Some(&TokenKind::Private) => {
                text("private ")
            }
            Visibility::Private => Document::Empty,
        }
    }

    fn type_parameters(&mut self, parameters: &[TypeParameter]) -> Document {
        if parameters.is_empty() {
            return Document::Empty;
        }
        let mut parts = vec![text("<")];
        for (index, parameter) in parameters.iter().enumerate() {
            if index > 0 {
                parts.push(text(", "));
            }
            parts.push(self.comments_before(parameter.span.start));
            parts.push(text(parameter.name.clone()));
            if let Some(bound) = parameter.bound {
                parts.push(text(format!(" : {}", bound.as_str())));
            }
        }
        parts.push(text(">"));
        concat(parts)
    }

    fn function(&mut self, function: &FunctionDecl) -> Document {
        let mut parts = Vec::new();
        // The context parameter goes on its own line above the rest of the declaration (D11 of
        // `receiver-lambdas`); the visibility is read after it.
        let mut modifiers = function.span;
        if let Some(context) = &function.context {
            parts.push(text(format!("context({}: ", context.name)));
            parts.push(self.ty(&context.ty));
            parts.push(text(")"));
            parts.push(hard_line());
            if let Some(token) = self.token_at(context.span.end) {
                let start = token.span.start;
                parts.push(self.comments_before(start));
            }
            modifiers.start = context.span.end;
        }
        parts.push(self.visibility(function.visibility, modifiers));
        if let Some(library) = function.library {
            parts.push(text(format!("{} ", library.kind.as_str())));
        }
        if function.is_operator {
            parts.push(text("operator "));
        }
        if function.is_capture {
            parts.push(text("capture "));
        }
        if function.is_compose {
            parts.push(text("compose "));
        }
        if let Some(receiver) = &function.receiver {
            if receiver.mode != ReceiverMode::Shared {
                parts.push(text(format!("{} ", receiver.mode.as_str())));
            }
        }
        parts.push(text("fun "));
        if !function.type_parameters.is_empty() {
            parts.push(self.type_parameters(&function.type_parameters));
            parts.push(text(" "));
        }
        if let Some(receiver) = &function.receiver {
            parts.push(self.ty(&receiver.ty));
            parts.push(text("."));
        }
        parts.push(text(function.name.clone()));

        // The parameter list starts at the first `(` after the receiver and type parameters.
        let header_end = function
            .receiver
            .as_ref()
            .map(|receiver| receiver.span.end)
            .into_iter()
            .chain(function.type_parameters.iter().map(|p| p.span.end))
            .max()
            .unwrap_or(function.span.start);
        let open = self.find_token(header_end, &TokenKind::LParen);
        let close_search = function
            .params
            .last()
            .map_or_else(|| open.map_or(header_end, |t| t.span.end), |p| p.span.end);
        let close_offset = self
            .find_token(close_search, &TokenKind::RParen)
            .map_or(close_search, |t| t.span.start);
        parts.push(self.comma_list(
            &function.params,
            |parameter| parameter.span,
            |printer, parameter| printer.parameter(parameter),
            CommaList {
                open: "(",
                close: ")",
                close_offset,
                trailing_comma: true,
            },
        ));
        if let Some(return_type) = &function.return_type {
            parts.push(text(": "));
            parts.push(self.ty(return_type));
        }
        match &function.body {
            Some(FunctionBody::Block(block)) => {
                parts.push(text(" "));
                parts.push(self.block(block));
            }
            Some(FunctionBody::Expr(body)) => {
                parts.push(text(" ="));
                parts.push(self.right_side(body));
            }
            None => {}
        }
        concat(parts)
    }

    fn parameter(&mut self, parameter: &Param) -> Document {
        let mut parts = vec![text(format!("{}: ", parameter.name))];
        parts.push(text(parameter.kind.to_string()));
        parts.push(self.ty(&parameter.ty));
        if let Some(default) = &parameter.default_value {
            parts.push(text(" = "));
            parts.push(self.expression(default));
        }
        concat(parts)
    }

    fn marker(marker: Option<TypeMarker>) -> Document {
        match marker {
            Some(marker) => text(format!(" : {}", marker.as_str())),
            None => Document::Empty,
        }
    }

    fn structure(&mut self, structure: &StructDecl) -> Document {
        // An intrinsic struct is one line: its header and marker, without fields (D16 of
        // `compose-transform`).
        if structure.is_intrinsic() {
            return concat(vec![
                self.visibility(structure.visibility, structure.span),
                text(format!("intrinsic struct {}", structure.name)),
                self.type_parameters(&structure.type_parameters),
                Self::marker(structure.marker),
            ]);
        }
        let mut parts = vec![
            self.visibility(structure.visibility, structure.span),
            text(format!("struct {}", structure.name)),
            self.type_parameters(&structure.type_parameters),
        ];
        let is_block_form = self.slice(structure.span).ends_with('}');
        if is_block_form {
            parts.push(Self::marker(structure.marker));
            parts.push(text(" "));
            let open = self
                .find_token(structure.span.start, &TokenKind::LBrace)
                .map_or(structure.span.start, |t| t.span.start);
            let close = structure.span.end - 1;
            parts.push(self.member_body(
                &structure.fields,
                |field| field.span,
                |printer, field| printer.field(field, true),
                open,
                close,
            ));
        } else {
            let search = structure
                .type_parameters
                .last()
                .map_or(structure.span.start, |p| p.span.end);
            let open = self
                .find_token(search, &TokenKind::LParen)
                .map_or(search, |t| t.span.end);
            let close_search = structure.fields.last().map_or(open, |field| field.span.end);
            let close_offset = self
                .find_token(close_search, &TokenKind::RParen)
                .map_or(close_search, |t| t.span.start);
            parts.push(self.comma_list(
                &structure.fields,
                |field| field.span,
                |printer, field| printer.field(field, false),
                CommaList {
                    open: "(",
                    close: ")",
                    close_offset,
                    trailing_comma: true,
                },
            ));
            parts.push(Self::marker(structure.marker));
        }
        concat(parts)
    }

    /// A struct field; `val` is printed in the block form, and in the shorthand form when
    /// written (D5).
    fn field(&mut self, field: &StructField, is_block_form: bool) -> Document {
        let keyword = if field.is_var {
            "var "
        } else if is_block_form || self.kind_at(field.span.start) == Some(&TokenKind::Val) {
            "val "
        } else {
            ""
        };
        concat(vec![
            text(format!("{}{}: ", keyword, field.name)),
            self.ty(&field.ty),
        ])
    }

    fn enumeration(&mut self, enumeration: &EnumDecl) -> Document {
        let open = self
            .find_token(enumeration.span.start, &TokenKind::LBrace)
            .map_or(enumeration.span.start, |t| t.span.start);
        let close = enumeration.span.end - 1;
        let mut parts = vec![
            self.visibility(enumeration.visibility, enumeration.span),
            text(format!("enum {}", enumeration.name)),
            self.type_parameters(&enumeration.type_parameters),
            Self::marker(enumeration.marker),
            text(" "),
        ];
        parts.push(self.member_body(
            &enumeration.variants,
            |variant| variant.span,
            |printer, variant| printer.variant(variant),
            open,
            close,
        ));
        concat(parts)
    }

    fn variant(&mut self, variant: &EnumVariant) -> Document {
        if variant.fields.is_empty() {
            return text(variant.name.clone());
        }
        let close_offset = variant.span.end - 1;
        let payload = self.comma_list(
            &variant.fields,
            |field| field.span,
            |printer, field| printer.ty(field),
            CommaList {
                open: "(",
                close: ")",
                close_offset,
                trailing_comma: true,
            },
        );
        concat(vec![text(variant.name.clone()), payload])
    }

    /// A struct or enum body between `open` (`{`) and `close` (`}`): kept on one line when it
    /// was written on one line without comments and still fits, otherwise one member per line
    /// (D9).
    fn member_body<T>(
        &mut self,
        members: &[T],
        span_of: impl Fn(&T) -> Span,
        mut print_member: impl FnMut(&mut Self, &T) -> Document,
        open: usize,
        close: usize,
    ) -> Document {
        let one_line =
            !members.is_empty() && self.same_line(open, close) && !self.has_comment_before(close);
        if !one_line {
            let list = self.vertical_list(members, span_of, print_member, close, |_| false);
            return self.braced(list);
        }
        let mut items = Vec::new();
        for (index, member) in members.iter().enumerate() {
            if index > 0 {
                items.push(if_break(Document::Empty, text(",")));
                items.push(line());
            }
            items.push(print_member(self, member));
        }
        group(concat(vec![
            text("{"),
            indent(concat(vec![line(), concat(items)])),
            line(),
            text("}"),
        ]))
    }

    // --- Types ---

    /// A type, never broken, with the parentheses its meaning needs (D5): around a function
    /// type, a reference or a nullable type made nullable.
    pub(crate) fn ty(&mut self, ty: &Type) -> Document {
        let leading = self.comments_before(ty.span.start);
        let printed = match &ty.kind {
            TypeKind::Named { name, type_args } => {
                let mut parts = vec![text(name.clone())];
                if !type_args.is_empty() {
                    parts.push(self.type_arguments(type_args));
                }
                concat(parts)
            }
            TypeKind::Nullable(inner) => {
                let needs_parentheses = matches!(
                    inner.kind,
                    TypeKind::Function { .. } | TypeKind::Reference { .. } | TypeKind::Nullable(_)
                );
                let inner = self.ty(inner);
                if needs_parentheses {
                    concat(vec![text("("), inner, text(")?")])
                } else {
                    concat(vec![inner, text("?")])
                }
            }
            TypeKind::Reference { mutable, inner } => concat(vec![
                text(if *mutable { "&mut " } else { "&" }),
                self.ty(inner),
            ]),
            TypeKind::Unit => text("()"),
            TypeKind::Function {
                is_compose,
                receiver,
                params,
                parameter_kinds,
                return_type,
            } => {
                let mut parts = Vec::new();
                if *is_compose {
                    parts.push(text("compose "));
                }
                if let Some(receiver) = receiver {
                    if receiver.mutable {
                        parts.push(text("mut "));
                    }
                    parts.push(self.ty(&receiver.ty));
                    parts.push(text("."));
                }
                parts.push(text("("));
                for (index, parameter) in params.iter().enumerate() {
                    if index > 0 {
                        parts.push(text(", "));
                    }
                    let kind = parameter_kinds.get(index).copied().unwrap_or_default();
                    parts.push(text(kind.to_string()));
                    parts.push(self.ty(parameter));
                }
                parts.push(text(") -> "));
                parts.push(self.ty(return_type));
                concat(parts)
            }
        };
        concat(vec![leading, printed])
    }

    /// `<A, B>` after a type name or a callee.
    pub(crate) fn type_arguments(&mut self, arguments: &[Type]) -> Document {
        let mut parts = vec![text("<")];
        for (index, argument) in arguments.iter().enumerate() {
            if index > 0 {
                parts.push(text(", "));
            }
            parts.push(self.ty(argument));
        }
        parts.push(text(">"));
        concat(parts)
    }
}

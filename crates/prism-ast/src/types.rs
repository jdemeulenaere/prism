//! Type representation in Prism AST.

use crate::span::Span;
use std::fmt;

/// The structural kind of a type expression.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TypeKind {
    /// A named type, optionally with generic type arguments, e.g. `Int`, `String`, `Heap<Expr>`, `Vector<String>`.
    Named { name: String, type_args: Vec<Type> },
    /// A nullable type suffix `T?`, e.g. `String?` or `&String?`.
    Nullable(Box<Type>),
    /// A stack-scoped borrowed reference: `&T` or `&mut T`.
    Reference { mutable: bool, inner: Box<Type> },
    /// The unit type `()`.
    Unit,
    /// A function type, e.g. `(Int, String) -> Bool`.
    Function {
        params: Vec<Type>,
        return_type: Box<Type>,
    },
}

/// A resolved or parsed type with its source span.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Type {
    pub kind: TypeKind,
    pub span: Span,
}

impl Type {
    pub fn new(kind: TypeKind, span: Span) -> Self {
        Self { kind, span }
    }

    /// Helper for a simple named type without arguments (e.g., `Int`).
    pub fn named(name: impl Into<String>, span: Span) -> Self {
        Self {
            kind: TypeKind::Named {
                name: name.into(),
                type_args: Vec::new(),
            },
            span,
        }
    }

    /// Helper for a generic named type (e.g., `Heap<T>`).
    pub fn generic(name: impl Into<String>, type_args: Vec<Type>, span: Span) -> Self {
        Self {
            kind: TypeKind::Named {
                name: name.into(),
                type_args,
            },
            span,
        }
    }

    /// Helper for a nullable type `T?`.
    pub fn nullable(inner: Type, span: Span) -> Self {
        Self {
            kind: TypeKind::Nullable(Box::new(inner)),
            span,
        }
    }

    /// Helper for an immutable reference `&T`.
    pub fn reference(inner: Type, span: Span) -> Self {
        Self {
            kind: TypeKind::Reference {
                mutable: false,
                inner: Box::new(inner),
            },
            span,
        }
    }

    /// Helper for a mutable reference `&mut T`.
    pub fn reference_mut(inner: Type, span: Span) -> Self {
        Self {
            kind: TypeKind::Reference {
                mutable: true,
                inner: Box::new(inner),
            },
            span,
        }
    }

    /// Helper for unit `()`.
    pub fn unit(span: Span) -> Self {
        Self {
            kind: TypeKind::Unit,
            span,
        }
    }
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            TypeKind::Named { name, type_args } => {
                write!(f, "{}", name)?;
                if !type_args.is_empty() {
                    write!(f, "<")?;
                    for (i, arg) in type_args.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{}", arg)?;
                    }
                    write!(f, ">")?;
                }
                Ok(())
            }
            TypeKind::Nullable(inner) => write!(f, "{}?", inner),
            TypeKind::Reference { mutable, inner } => {
                if *mutable {
                    write!(f, "&mut {}", inner)
                } else {
                    write!(f, "&{}", inner)
                }
            }
            TypeKind::Unit => write!(f, "()"),
            TypeKind::Function {
                params,
                return_type,
            } => {
                write!(f, "(")?;
                for (i, p) in params.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", p)?;
                }
                write!(f, ") -> {}", return_type)
            }
        }
    }
}

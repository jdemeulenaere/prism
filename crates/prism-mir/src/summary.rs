//! File summaries: what planning a program's instances needs from each file, without its MIR
//! (D5 and D7 of `instance-cache`).
//!
//! A file's summary records whether it declares a `main` the backend accepts, the instances its
//! compiled bodies request (its **roots**), and for each of its generic functions and extensions
//! its Monomorphization Hash and its **generic calls**: the calls with type arguments of its
//! body, its default value generators and its closures, whose type arguments may mention its own
//! type parameters. Substituting an instance's type arguments into its generic's calls gives
//! exactly the requests instantiating its body would find, so the instances of a whole program,
//! and its cross-file polymorphic recursion (`E6004`), follow from the summaries alone. A summary
//! is one pass over the file's calls, and is small: a few dozen bytes per call.

use crate::codec::{Codec, Decoder, Encoder};
use crate::mir::{Body, Callee, MirFile, TerminatorKind};
use crate::record::GenericRecord;
use prism_ast::Span;
use prism_typed_tree::{DeclarationPath, Type, TypeParameter};
use std::collections::HashSet;

/// What planning needs from one file.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FileSummary {
    /// The file's `main`, when it declares one the backend accepts (`find_main`).
    pub entry_point: Option<DeclarationPath>,
    /// The instances the file's compiled bodies request, each once, in order of first call.
    pub roots: Vec<InstanceRequest>,
    /// The file's generic functions and extensions, private ones included, in body order.
    pub generics: Vec<GenericSummary>,
}

/// A call of a generic function or of one of its default value generators, with its type
/// arguments.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct InstanceRequest {
    /// The generic function or extension called, or whose default value is generated.
    pub callee: DeclarationPath,
    /// For a default value's generator, the parameter it generates.
    pub default: Option<DefaultParameter>,
    pub type_arguments: Vec<Type>,
    /// The call's source, in the calling file.
    pub span: Span,
}

/// The declared parameter, counted without the receiver, whose default a generator computes.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DefaultParameter {
    pub parameter: usize,
    pub name: String,
}

/// One generic function or extension of a file.
#[derive(Debug, Clone, PartialEq)]
pub struct GenericSummary {
    pub path: DeclarationPath,
    pub type_parameters: Vec<TypeParameter>,
    /// Its Monomorphization Hash: the hash of its record's encoding (D2).
    pub record_hash: [u8; 32],
    /// The calls with type arguments of its body, defaults and closures, in body order.
    pub calls: Vec<InstanceRequest>,
}

impl InstanceRequest {
    /// The request a call of `callee` with `type_arguments` makes, or `None` for a call without
    /// type arguments or of a built-in or a function value, whose type arguments are lowered
    /// directly.
    pub fn of_call(callee: &Callee, type_arguments: &[Type], span: Span) -> Option<Self> {
        if type_arguments.is_empty() {
            return None;
        }
        let (callee, default) = match callee {
            Callee::Function(path) => (path.clone(), None),
            Callee::Default {
                function,
                parameter,
                name,
            } => (
                function.clone(),
                Some(DefaultParameter {
                    parameter: *parameter,
                    name: name.clone(),
                }),
            ),
            Callee::Builtin(_) | Callee::Value(..) => return None,
        };
        Some(Self {
            callee,
            default,
            type_arguments: type_arguments.to_vec(),
            span,
        })
    }

    /// The call this request makes.
    pub fn callee(&self) -> Callee {
        match &self.default {
            None => Callee::Function(self.callee.clone()),
            Some(default) => Callee::Default {
                function: self.callee.clone(),
                parameter: default.parameter,
                name: default.name.clone(),
            },
        }
    }

    /// The requests of `body`'s calls, in block order.
    pub fn of_body(body: &Body) -> impl Iterator<Item = InstanceRequest> + '_ {
        body.blocks.iter().filter_map(|(_, block)| {
            let TerminatorKind::Call {
                callee,
                type_arguments,
                ..
            } = &block.terminator.kind
            else {
                return None;
            };
            Self::of_call(callee, type_arguments, block.terminator.span)
        })
    }
}

impl FileSummary {
    /// The summary of `file`, whose generic records are `records` ([`GenericRecord::of_file`]),
    /// each hashed with `hash` over its encoding, and whose accepted `main` is `entry_point`.
    pub fn of(
        file: &MirFile,
        records: &[GenericRecord],
        hash: impl Fn(&[u8]) -> [u8; 32],
        entry_point: Option<DeclarationPath>,
    ) -> FileSummary {
        let mut roots = Vec::new();
        let mut seen: HashSet<(DeclarationPath, Option<DefaultParameter>, Vec<Type>)> =
            HashSet::new();
        for (_, body) in file.bodies.iter() {
            // A compiled body: generic bodies and their closures are compiled as instances.
            if !body.type_parameters.is_empty() {
                continue;
            }
            for request in InstanceRequest::of_body(body) {
                let key = (
                    request.callee.clone(),
                    request.default.clone(),
                    request.type_arguments.clone(),
                );
                if seen.insert(key) {
                    roots.push(request);
                }
            }
        }
        let generics = records
            .iter()
            .map(|record| GenericSummary {
                path: record.path.clone(),
                type_parameters: record.function.type_parameters.clone(),
                record_hash: hash(&record.encode()),
                calls: std::iter::once(&record.function)
                    .chain(&record.defaults)
                    .chain(&record.closures)
                    .flat_map(InstanceRequest::of_body)
                    .collect(),
            })
            .collect();
        FileSummary {
            entry_point,
            roots,
            generics,
        }
    }

    /// The summary's canonical encoding (D4 of `instance-cache`).
    pub fn encode(&self) -> Vec<u8> {
        crate::codec::encode(self)
    }

    /// The summary `bytes` encode, or `None` when they are malformed.
    pub fn decode(bytes: &[u8]) -> Option<FileSummary> {
        crate::codec::decode(bytes)
    }
}

impl Codec for DefaultParameter {
    fn encode(&self, encoder: &mut Encoder) {
        let DefaultParameter { parameter, name } = self;
        parameter.encode(encoder);
        name.encode(encoder);
    }
    fn decode(decoder: &mut Decoder) -> Option<Self> {
        Some(DefaultParameter {
            parameter: decoder.decode()?,
            name: decoder.decode()?,
        })
    }
}

impl Codec for InstanceRequest {
    fn encode(&self, encoder: &mut Encoder) {
        let InstanceRequest {
            callee,
            default,
            type_arguments,
            span,
        } = self;
        callee.encode(encoder);
        default.encode(encoder);
        type_arguments.encode(encoder);
        span.encode(encoder);
    }
    fn decode(decoder: &mut Decoder) -> Option<Self> {
        let request = InstanceRequest {
            callee: decoder.decode()?,
            default: decoder.decode()?,
            type_arguments: decoder.decode()?,
            span: decoder.decode()?,
        };
        (!request.type_arguments.is_empty()).then_some(request)
    }
}

impl Codec for GenericSummary {
    fn encode(&self, encoder: &mut Encoder) {
        let GenericSummary {
            path,
            type_parameters,
            record_hash,
            calls,
        } = self;
        path.encode(encoder);
        type_parameters.encode(encoder);
        record_hash.encode(encoder);
        calls.encode(encoder);
    }
    fn decode(decoder: &mut Decoder) -> Option<Self> {
        Some(GenericSummary {
            path: decoder.decode()?,
            type_parameters: decoder.decode()?,
            record_hash: decoder.decode()?,
            calls: decoder.decode()?,
        })
    }
}

impl Codec for FileSummary {
    fn encode(&self, encoder: &mut Encoder) {
        let FileSummary {
            entry_point,
            roots,
            generics,
        } = self;
        entry_point.encode(encoder);
        roots.encode(encoder);
        generics.encode(encoder);
    }
    fn decode(decoder: &mut Decoder) -> Option<Self> {
        Some(FileSummary {
            entry_point: decoder.decode()?,
            roots: decoder.decode()?,
            generics: decoder.decode()?,
        })
    }
}

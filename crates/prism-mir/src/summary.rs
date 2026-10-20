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
//!
//! Hooks of generic types are instances too (D11 of `drop-and-clone-hooks`): a body that holds a
//! value of a generic struct or enum with a drop or clone hook, in a local or in a part of one,
//! or passes one as a type argument, requests the hook's instance as it would a call. Roots and
//! generics' calls therefore also hold, once per distinct type reached, a request of
//! `<Type>.drop` for each generic type with a drop hook that a body's local types and call type
//! arguments reach through fields, payloads, nullables, boxes and lists, and of `<Type>.clone`
//! for each such type with a clone hook that is `Clone` at its arguments. A generic's requests
//! keep its type parameters, which planning substitutes. A file whose types have no generic
//! hook walks nothing.

use crate::codec::{Codec, Decoder, Encoder};
use crate::mir::{Body, BodyId, BodyOwner, Callee, MirFile, TerminatorKind};
use crate::record::GenericRecord;
use prism_ast::Span;
use prism_typed_tree::{DeclarationPath, Type, TypeParameter, TypeTable};
use std::collections::{HashMap, HashSet};

/// What planning needs from one file.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FileSummary {
    /// The file's `main`, when it declares one the backend accepts (`find_main`).
    pub entry_point: Option<DeclarationPath>,
    /// The instances the file's compiled bodies request, each once, in order of first call.
    pub roots: Vec<InstanceRequest>,
    /// The file's generic functions and extensions, private ones included, in body order.
    pub generics: Vec<GenericSummary>,
    /// The body keys of the file's composable functions and closures that are not part of a
    /// generic, in body order: what a reload of the file, or of a file it imports, names for
    /// recomposition (D4 and D5 of `compose-hot-reload`).
    pub composable_bodies: Vec<u64>,
    /// The body key of each composable function of the file, generic ones included, with its
    /// declaration path as `prism dump-mir` names its body, in body order: what
    /// `dump recompositions` names its counts by (D14 of `strong-skipping`). Composable closures
    /// have no name and are not listed.
    pub composable_functions: Vec<(u64, String)>,
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
    /// The calls with type arguments of its body and of the closures lowered inside it, in
    /// body order: what an instance of the function requests.
    pub calls: Vec<InstanceRequest>,
    /// Its default value generators, in parameter order, each with its own calls.
    pub defaults: Vec<DefaultSummary>,
    /// The body keys of the generic, when it is composable, and of its composable closures, in
    /// body order: what a reload of one of its instances names (D5 of `compose-hot-reload`).
    pub composable_bodies: Vec<u64>,
}

/// One default value generator of a generic.
#[derive(Debug, Clone, PartialEq)]
pub struct DefaultSummary {
    /// The declared parameter it generates, counted without the receiver.
    pub parameter: usize,
    /// The calls with type arguments of its body and of the closures lowered inside it.
    pub calls: Vec<InstanceRequest>,
}

impl GenericSummary {
    /// What an instance of the function (`None`) or of the generator of `default` requests,
    /// before substitution; `None` for a default the generic does not have.
    pub fn calls_of(&self, default: Option<&DefaultParameter>) -> Option<&[InstanceRequest]> {
        match default {
            None => Some(&self.calls),
            Some(default) => self
                .defaults
                .iter()
                .find(|summary| summary.parameter == default.parameter)
                .map(|summary| &summary.calls[..]),
        }
    }

    /// Every call of the generic's bodies, its defaults' included.
    pub fn all_calls(&self) -> impl Iterator<Item = &InstanceRequest> {
        self.calls
            .iter()
            .chain(self.defaults.iter().flat_map(|default| &default.calls))
    }
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
            Callee::External(_)
            | Callee::Builtin(_)
            | Callee::Value(..)
            | Callee::Composer(_)
            | Callee::State(_) => return None,
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
    /// The summary of `file`, whose generic records are `records` ([`GenericRecord::of_file`])
    /// with the Monomorphization Hashes `record_hashes`, in the same order, and whose accepted
    /// `main` is `entry_point`.
    pub fn of(
        file: &MirFile,
        records: &[GenericRecord],
        record_hashes: &[[u8; 32]],
        entry_point: Option<DeclarationPath>,
    ) -> FileSummary {
        let mut summary = Self::for_instances(file, records, record_hashes);
        summary.entry_point = entry_point;
        for (_, body) in file.bodies.iter() {
            if let (BodyOwner::Function(path), Some(key)) = (&body.owner, body.body_key) {
                summary
                    .composable_functions
                    .push((key, declaration_name(path)));
            }
            // A compiled body: generic bodies and their closures are compiled as instances.
            if body.type_parameters.is_empty() {
                summary.composable_bodies.extend(body.body_key);
            }
        }
        summary
    }

    /// The parts of [`FileSummary::of`] that instance planning (`prism_codegen::plan_instances`)
    /// reads: the
    /// roots and the generics, without an entry point or composable bodies. Its cost is one
    /// visit of each body's header, the blocks of the bodies that request instances
    /// ([`Body::requests_instances`]) and the records, so planning a large file's instances
    /// never reads its other blocks or names.
    pub fn for_instances(
        file: &MirFile,
        records: &[GenericRecord],
        record_hashes: &[[u8; 32]],
    ) -> FileSummary {
        let mut roots = Vec::new();
        let mut seen: HashSet<(DeclarationPath, Option<DefaultParameter>, Vec<Type>)> =
            HashSet::new();
        for (_, body) in file.bodies.iter() {
            // A compiled body: generic bodies and their closures are compiled as instances.
            // Visiting only the bodies that request instances keeps the summary from reading
            // every block of a large file, whose blocks outgrow the caches.
            if !body.type_parameters.is_empty() || !body.requests_instances {
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
        // The hook instances the compiled bodies' values need (D11 of `drop-and-clone-hooks`),
        // each type walked once per file.
        if let Some(mut walk) = HookWalk::new(&file.types) {
            let mut requests = Vec::new();
            for (_, body) in file.bodies.iter() {
                if body.type_parameters.is_empty() {
                    walk.body(body, &mut requests);
                }
            }
            for request in requests {
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
            .zip(record_hashes)
            .map(|(record, hash)| generic_summary(record, *hash))
            .collect();
        FileSummary {
            entry_point: None,
            roots,
            generics,
            composable_bodies: Vec::new(),
            composable_functions: Vec::new(),
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

/// `path` as its `Display` writes it, built without the formatting machinery for a plain
/// declaration, the common case: one allocation of the exact size and two copies.
fn declaration_name(path: &DeclarationPath) -> String {
    if path.receiver.is_some() {
        return path.to_string();
    }
    let mut name = String::with_capacity(path.module.len() + 2 + path.name.len());
    name.push_str(&path.module);
    name.push_str("::");
    name.push_str(&path.name);
    name
}

/// The summary of `record`, whose Monomorphization Hash is `record_hash`: each closure's calls
/// count for the function or the default its outermost parent is.
fn generic_summary(record: &GenericRecord, record_hash: [u8; 32]) -> GenericSummary {
    // The body each body of the record belongs to: `None` for the function, `Some(i)` for the
    // `i`-th default. Closures follow their parents in body order.
    let mut roots: HashMap<BodyId, Option<usize>> = HashMap::new();
    roots.insert(record.function_id, None);
    for (index, id) in record.default_ids.iter().enumerate() {
        roots.insert(*id, Some(index));
    }
    let mut calls: Vec<InstanceRequest> = InstanceRequest::of_body(&record.function).collect();
    let mut defaults: Vec<DefaultSummary> = record
        .defaults
        .iter()
        .map(|body| DefaultSummary {
            parameter: match &body.owner {
                BodyOwner::Default { parameter, .. } => *parameter,
                _ => 0,
            },
            calls: InstanceRequest::of_body(body).collect(),
        })
        .collect();
    let mut closure_roots = Vec::with_capacity(record.closures.len());
    for (closure, id) in record.closures.iter().zip(&record.closure_ids) {
        let Some(parent) = crate::record::parent_of(&closure.owner) else {
            continue;
        };
        let Some(root) = roots.get(&parent).copied() else {
            continue;
        };
        roots.insert(*id, root);
        let target = match root {
            None => &mut calls,
            Some(index) => &mut defaults[index].calls,
        };
        target.extend(InstanceRequest::of_body(closure));
        closure_roots.push((closure, root));
    }
    // The hook instances of the generic types its bodies hold, with its type parameters left
    // as they are (D11 of `drop-and-clone-hooks`): one walk for the function and its closures,
    // and one for each default and its closures, since each is instantiated alone.
    if HookWalk::new(&record.types).is_some() {
        let targets = std::iter::once((None, &record.function)).chain(
            record
                .defaults
                .iter()
                .enumerate()
                .map(|(i, body)| (Some(i), body)),
        );
        for (root, body) in targets {
            let mut walk = HookWalk::new(&record.types).expect("a generic type has a hook");
            let target = match root {
                None => &mut calls,
                Some(index) => &mut defaults[index].calls,
            };
            walk.body(body, target);
            for (closure, _) in closure_roots.iter().filter(|(_, r)| *r == root) {
                walk.body(closure, target);
            }
        }
    }
    let composable_bodies = record
        .function
        .body_key
        .into_iter()
        .chain(
            record
                .closures
                .iter()
                .filter_map(|closure| closure.body_key),
        )
        .collect();
    GenericSummary {
        path: record.path.clone(),
        type_parameters: record.function.type_parameters.clone(),
        record_hash,
        calls,
        defaults,
        composable_bodies,
    }
}

/// The walk of the types a file's bodies hold for the generic types with hooks they reach (D11
/// of `drop-and-clone-hooks`): each type is visited once, so the walk is linear in the distinct
/// types of the bodies walked and of their definitions.
struct HookWalk<'t> {
    types: &'t TypeTable,
    visited: HashSet<Type>,
    /// The requests already made, so that each is made once.
    requested: HashSet<(String, Vec<Type>)>,
}

impl<'t> HookWalk<'t> {
    /// The walk over `types`, or `None` when no generic struct or enum of `types` has a hook,
    /// so that nothing can be requested.
    fn new(types: &'t TypeTable) -> Option<Self> {
        let generic_hook = types
            .drop_hook_types()
            .chain(types.clone_hook_types())
            .any(|id| !types.parameters(id).is_empty());
        generic_hook.then(|| HookWalk {
            types,
            visited: HashSet::new(),
            requested: HashSet::new(),
        })
    }

    /// Requests into `out` the hook instances the local types of `body` and the type arguments
    /// of its calls reach.
    fn body(&mut self, body: &Body, out: &mut Vec<InstanceRequest>) {
        for (_, local) in body.locals.iter() {
            self.walk(&local.ty, local.span, out);
        }
        if !body.requests_instances {
            return;
        }
        for (_, block) in body.blocks.iter() {
            if let TerminatorKind::Call { type_arguments, .. } = &block.terminator.kind {
                for argument in type_arguments {
                    self.walk(argument, block.terminator.span, out);
                }
            }
        }
    }

    /// Requests the hook instances of the generic types with hooks that `ty` reaches through
    /// fields, payloads, nullables, boxes and lists. A reference or a function value is not
    /// walked: a borrow drops and clones nothing, and a closure's captures are its own body's
    /// locals.
    fn walk(&mut self, ty: &Type, span: Span, out: &mut Vec<InstanceRequest>) {
        let mut pending = vec![ty.clone()];
        while let Some(ty) = pending.pop() {
            if !self.visited.insert(ty.clone()) {
                continue;
            }
            match &ty {
                Type::Nullable(inner) | Type::Heap(inner) | Type::List(inner) => {
                    pending.push(inner.as_ref().clone())
                }
                Type::Struct { id, arguments } | Type::Enum { id, arguments } => {
                    if !arguments.is_empty() {
                        if self.types.has_drop_hook(id) {
                            self.request(id, arguments, "drop", span, out);
                        }
                        if self.types.has_clone_hook(id) && self.types.is_clone(&ty) {
                            self.request(id, arguments, "clone", span, out);
                        }
                    }
                    if let Some(fields) = self.types.instantiated_fields(id, arguments) {
                        pending.extend(fields.into_iter().map(|(_, _, field)| field));
                    }
                    if let Some(variants) = self.types.instantiated_variants(id, arguments) {
                        pending.extend(variants.into_iter().flat_map(|(_, payloads)| payloads));
                    }
                }
                _ => {}
            }
        }
    }

    /// Requests the instance of the hook `name` of the generic type `id` for `arguments`.
    fn request(
        &mut self,
        id: &prism_typed_tree::NominalId,
        arguments: &[Type],
        name: &str,
        span: Span,
        out: &mut Vec<InstanceRequest>,
    ) {
        if !self.requested.insert((
            format!("{}::{}.{}", id.module, id.name, name),
            arguments.to_vec(),
        )) {
            return;
        }
        let parameters: Vec<Type> = self
            .types
            .parameters(id)
            .iter()
            .map(TypeParameter::as_type)
            .collect();
        let receiver = if self.types.enum_variants(id).is_some() {
            Type::Enum {
                id: id.clone(),
                arguments: parameters,
            }
        } else {
            Type::Struct {
                id: id.clone(),
                arguments: parameters,
            }
        };
        out.push(InstanceRequest {
            callee: DeclarationPath::extension(id.module.clone(), receiver, name),
            default: None,
            type_arguments: arguments.to_vec(),
            span,
        });
    }
}

impl Codec for DefaultSummary {
    fn encode(&self, encoder: &mut Encoder) {
        let DefaultSummary { parameter, calls } = self;
        parameter.encode(encoder);
        calls.encode(encoder);
    }
    fn decode(decoder: &mut Decoder) -> Option<Self> {
        Some(DefaultSummary {
            parameter: decoder.decode()?,
            calls: decoder.decode()?,
        })
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
            defaults,
            composable_bodies,
        } = self;
        path.encode(encoder);
        type_parameters.encode(encoder);
        record_hash.encode(encoder);
        calls.encode(encoder);
        defaults.encode(encoder);
        composable_bodies.encode(encoder);
    }
    fn decode(decoder: &mut Decoder) -> Option<Self> {
        Some(GenericSummary {
            path: decoder.decode()?,
            type_parameters: decoder.decode()?,
            record_hash: decoder.decode()?,
            calls: decoder.decode()?,
            defaults: decoder.decode()?,
            composable_bodies: decoder.decode()?,
        })
    }
}

impl Codec for FileSummary {
    fn encode(&self, encoder: &mut Encoder) {
        let FileSummary {
            entry_point,
            roots,
            generics,
            composable_bodies,
            composable_functions,
        } = self;
        entry_point.encode(encoder);
        roots.encode(encoder);
        generics.encode(encoder);
        composable_bodies.encode(encoder);
        composable_functions.encode(encoder);
    }
    fn decode(decoder: &mut Decoder) -> Option<Self> {
        Some(FileSummary {
            entry_point: decoder.decode()?,
            roots: decoder.decode()?,
            generics: decoder.decode()?,
            composable_bodies: decoder.decode()?,
            composable_functions: decoder.decode()?,
        })
    }
}

//! Generic records: what a file ships for each of its generic functions so that any file of the
//! program can instantiate it without its source (D2 of `monomorphization`).
//!
//! A record holds the elaborated polymorphic MIR of a generic function or extension, of its
//! default value generators and of the closures lowered inside them, and the definitions of the
//! structs and enums those bodies mention, closed over their fields and payloads. It holds no
//! other body: the non-generic functions a generic body calls are called by symbol, and the
//! generics it calls are instances of their own records. An instance of the function or of a
//! default is built with the closures lowered inside it ([`GenericRecord::instantiate`], D11 of
//! `closures`).

use crate::instantiate::instantiate;
use crate::mir::{
    AggregateKind, Body, BodyId, BodyOwner, Callee, Constant, MirFile, Operand, Rvalue,
    StatementKind, TerminatorKind,
};
use prism_typed_tree::{ArenaId, DeclarationPath, NominalId, Type, TypeTable};
use std::collections::HashMap;

/// The record of one generic function or extension.
#[derive(Debug, Clone, PartialEq)]
pub struct GenericRecord {
    /// The generic's declaration path.
    pub path: DeclarationPath,
    /// Its body.
    pub function: Body,
    /// The generators of its default values, in parameter order.
    pub defaults: Vec<Body>,
    /// The closures lowered in its body and its defaults, in body order.
    pub closures: Vec<Body>,
    /// The definitions and markers of the structs and enums the bodies mention.
    pub types: TypeTable,
    /// The id in the generic's file of the function, of each default and of each closure, in
    /// the order of those fields, which closures name their parents by.
    pub function_id: BodyId,
    pub default_ids: Vec<BodyId>,
    pub closure_ids: Vec<BodyId>,
}

impl GenericRecord {
    /// The record of every generic function and extension of `file`, private ones included, in
    /// body order.
    pub fn of_file(file: &MirFile) -> Vec<GenericRecord> {
        let mut records: Vec<GenericRecord> = Vec::new();
        // The record each body belongs to, by body index, for closures to find their parent's.
        let mut owners: Vec<Option<usize>> = Vec::with_capacity(file.bodies.len());
        let mut by_path: std::collections::HashMap<&DeclarationPath, usize> =
            std::collections::HashMap::new();
        for (id, body) in file.bodies.iter() {
            let owner = match &body.owner {
                BodyOwner::Function(path) if !body.type_parameters.is_empty() => {
                    records.push(GenericRecord {
                        path: path.clone(),
                        function: body.clone(),
                        defaults: Vec::new(),
                        closures: Vec::new(),
                        types: TypeTable::new(),
                        function_id: id,
                        default_ids: Vec::new(),
                        closure_ids: Vec::new(),
                    });
                    by_path.insert(path, records.len() - 1);
                    Some(records.len() - 1)
                }
                BodyOwner::Default { function, .. } => {
                    let record = by_path.get(function).copied();
                    if let Some(record) = record {
                        records[record].defaults.push(body.clone());
                        records[record].default_ids.push(id);
                    }
                    record
                }
                BodyOwner::Closure { parent, .. } => {
                    let record = owners.get(parent.index()).copied().flatten();
                    if let Some(record) = record {
                        records[record].closures.push(body.clone());
                        records[record].closure_ids.push(id);
                    }
                    record
                }
                BodyOwner::Function(_) | BodyOwner::Value(_) => None,
            };
            owners.push(owner);
        }
        for record in &mut records {
            let mut ids = Vec::new();
            for body in std::iter::once(&record.function)
                .chain(&record.defaults)
                .chain(&record.closures)
            {
                mentioned_ids(body, &mut ids);
            }
            record.types = file.types.restricted_to_ids(ids);
        }
        records
    }

    /// The body an instance of `callee` runs, with its id in the generic's file: the function
    /// for a call of it, the generator of the default value of `parameter` for a default.
    pub fn body_for(&self, callee: &Callee) -> Option<(BodyId, &Body)> {
        match callee {
            Callee::Function(_) => Some((self.function_id, &self.function)),
            Callee::Default { parameter, .. } => self
                .defaults
                .iter()
                .zip(&self.default_ids)
                .find(|(body, _)| {
                    matches!(&body.owner, BodyOwner::Default { parameter: p, .. } if p == parameter)
                })
                .map(|(body, id)| (*id, body)),
            Callee::Builtin(_) | Callee::Value(..) => None,
        }
    }

    /// The instance of `callee`'s body for `type_arguments` over `types` ([`instantiate`]),
    /// followed by the instances of the closures lowered inside it, nested ones included, in
    /// body order (D11 of `closures`): body 0 is the instance, and each closure's parent and
    /// each closure value built name the bodies of the returned list.
    pub fn instantiate(
        &self,
        callee: &Callee,
        type_arguments: &[Type],
        types: &TypeTable,
    ) -> Option<Vec<Body>> {
        let (id, body) = self.body_for(callee)?;
        // Closures follow their parents in body order, so one pass finds every descendant.
        let mut numbers: HashMap<BodyId, BodyId> = HashMap::from([(id, BodyId(0))]);
        let mut bodies = vec![instantiate(body, type_arguments, types)];
        for (closure, closure_id) in self.closures.iter().zip(&self.closure_ids) {
            let BodyOwner::Closure { parent, .. } = closure.owner else {
                continue;
            };
            if numbers.contains_key(&parent) {
                numbers.insert(*closure_id, BodyId(bodies.len() as u32));
                bodies.push(instantiate(closure, type_arguments, types));
            }
        }
        for body in &mut bodies {
            renumber_bodies(body, &numbers);
        }
        Some(bodies)
    }
}

/// Renames the bodies `body` names, its closure parent and the closures it builds, by
/// `numbers`.
fn renumber_bodies(body: &mut Body, numbers: &HashMap<BodyId, BodyId>) {
    if let BodyOwner::Closure { parent, .. } = &mut body.owner {
        *parent = numbers[parent];
    }
    for (_, block) in body.blocks.iter_mut() {
        for statement in &mut block.statements {
            if let StatementKind::Assign(_, rvalue) = &mut statement.kind {
                if let Rvalue::Closure { body, .. } = rvalue.as_mut() {
                    *body = numbers[body];
                }
            }
        }
    }
}

/// Pushes the id of every struct and enum `body` mentions: in its locals, aggregates, `null`
/// constants, and its calls' parameters, type arguments and receivers.
pub fn mentioned_ids(body: &Body, ids: &mut Vec<NominalId>) {
    for (_, local) in body.locals.iter() {
        local.ty.collect_nominal_ids(ids);
    }
    let operand = |operand: &Operand, ids: &mut Vec<NominalId>| {
        if let Operand::Constant(Constant::Null(ty)) = operand {
            ty.collect_nominal_ids(ids);
        }
    };
    for (_, block) in body.blocks.iter() {
        for statement in &block.statements {
            let StatementKind::Assign(_, rvalue) = &statement.kind else {
                continue;
            };
            match rvalue.as_ref() {
                Rvalue::Use(o)
                | Rvalue::Unary(_, o)
                | Rvalue::WrapNullable(o)
                | Rvalue::Subtype(o) => operand(o, ids),
                Rvalue::Binary(_, left, right) => {
                    operand(left, ids);
                    operand(right, ids);
                }
                Rvalue::Aggregate(kind, operands) => {
                    match kind {
                        AggregateKind::Struct { id, arguments } => {
                            ids.push(id.clone());
                            arguments.iter().for_each(|ty| ty.collect_nominal_ids(ids));
                        }
                        AggregateKind::Variant {
                            enum_id, arguments, ..
                        } => {
                            ids.push(enum_id.clone());
                            arguments.iter().for_each(|ty| ty.collect_nominal_ids(ids));
                        }
                    }
                    operands.iter().for_each(|o| operand(o, ids));
                }
                Rvalue::Borrow { .. }
                | Rvalue::Discriminant(_)
                | Rvalue::Closure { .. }
                | Rvalue::Template(_) => {}
            }
        }
        if let TerminatorKind::Call {
            callee,
            arguments,
            parameters,
            type_arguments,
            ..
        } = &block.terminator.kind
        {
            parameters
                .iter()
                .chain(type_arguments)
                .for_each(|ty| ty.collect_nominal_ids(ids));
            if let Callee::Function(path) | Callee::Default { function: path, .. } = callee {
                if let Some(receiver) = &path.receiver {
                    receiver.collect_nominal_ids(ids);
                }
            }
            arguments.iter().for_each(|o| operand(o, ids));
        }
    }
}

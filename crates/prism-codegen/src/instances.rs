//! Instance collection over a whole program (D5, D8 and D10 of `monomorphization`).
//!
//! [`collect_instances`] builds the generic record of every generic function and extension of
//! the program's files, rejects polymorphic recursion through generics of several files
//! (`E6004`), and then runs a worklist from the calls with type arguments in every compiled body:
//! each request names an instance by its symbol, and an instance not yet built is substituted
//! from its generic's record ([`prism_mir::instantiate`]) into a unit of its own, whose calls
//! with type arguments become requests in turn. The work is linear in the program's distinct
//! instances and their sizes, plus a hash lookup per call site of a generic: a request for a
//! symbol already built is dropped before its type definitions are gathered. The absence of a
//! growing cycle bounds the depth of every type argument, so the worklist ends.

use crate::error::{CodegenError, CodegenErrorKind};
use crate::supported::{check_file, is_compiled};
use crate::symbols::callee_symbol;
use prism_ast::Span;
use prism_mir::{instantiate, Body, Callee, GenericRecord, MirFile, MirPhase, TerminatorKind};
use prism_typed_tree::instantiation::{edges_of, generic_name, growing_cycles};
use prism_typed_tree::{Arena, Type, TypeTable};
use std::collections::{HashMap, HashSet, VecDeque};

/// The code unit of one instance: the generic's module, the instance body alone, and the
/// definitions of the record's and the type arguments' types.
#[derive(Debug, Clone)]
pub struct InstanceUnit {
    /// The instance's symbol (`lib/box::keep<Int>`), which also names the unit.
    pub symbol: String,
    pub file: MirFile,
}

/// A diagnostic located in the file of `module`: an instance's error is reported at its
/// generic's source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstanceError {
    pub module: String,
    pub error: CodegenError,
}

/// One instance asked for: the generic's name, the call naming it (its function or one of its
/// defaults), the type arguments and the definitions of the types they mention.
struct Request {
    symbol: String,
    generic: String,
    callee: Callee,
    type_arguments: Vec<Type>,
    types: TypeTable,
}

/// The instance units the program of `files`, each elaborated, needs, sorted by symbol, or the
/// `E6004` errors of its cross-file polymorphic recursions.
pub fn collect_instances(files: &[&MirFile]) -> Result<Vec<InstanceUnit>, Vec<InstanceError>> {
    let mut records: HashMap<String, GenericRecord> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    for file in files {
        for record in GenericRecord::of_file(file) {
            let name = generic_name(&record.path);
            order.push(name.clone());
            records.insert(name, record);
        }
    }
    check_cross_file_recursion(&records, &order)?;

    let mut seen: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<Request> = VecDeque::new();
    for file in files {
        for (_, body) in file.bodies.iter() {
            if is_compiled(body) {
                request_calls(body, &file.types, &records, &mut seen, &mut queue);
            }
        }
    }
    let mut units = Vec::new();
    while let Some(request) = queue.pop_front() {
        let record = &records[&request.generic];
        let Some(body) = record.body_for(&request.callee) else {
            continue;
        };
        let mut types = record.types.clone();
        types.extend_from(&request.types);
        let instance = instantiate(body, &request.type_arguments, &types);
        request_calls(&instance, &types, &records, &mut seen, &mut queue);
        let mut bodies = Arena::new();
        bodies.alloc(instance);
        let file = MirFile {
            module: record.path.module.clone(),
            phase: MirPhase::Elaborated,
            types,
            bodies,
        };
        // An instance holding a closure is rejected (`E6001`) before it is compiled; its
        // closure bodies are not part of the unit (D12).
        #[cfg(debug_assertions)]
        if !holds_closure(&file.bodies) {
            let violations = file.validate();
            assert!(
                violations.is_empty(),
                "instance {} is ill-formed:\n{}\n{}",
                request.symbol,
                violations.join("\n"),
                file.dump()
            );
        }
        units.push(InstanceUnit {
            symbol: request.symbol,
            file,
        });
    }
    units.sort_by(|a, b| a.symbol.cmp(&b.symbol));
    Ok(units)
}

/// Queues an instance for each call with type arguments in `body` whose symbol is new, its type
/// arguments' definitions taken from `types`.
fn request_calls(
    body: &Body,
    types: &TypeTable,
    records: &HashMap<String, GenericRecord>,
    seen: &mut HashSet<String>,
    queue: &mut VecDeque<Request>,
) {
    for (_, block) in body.blocks.iter() {
        let TerminatorKind::Call {
            callee,
            type_arguments,
            ..
        } = &block.terminator.kind
        else {
            continue;
        };
        if type_arguments.is_empty() {
            continue;
        }
        let path = match callee {
            Callee::Function(path) | Callee::Default { function: path, .. } => path,
            // A built-in's type argument (`Heap<T>(...)`) is lowered directly.
            Callee::Builtin(_) | Callee::Value(..) => continue,
        };
        let generic = generic_name(path);
        if !records.contains_key(&generic) {
            continue;
        }
        let Some(symbol) = callee_symbol(callee, type_arguments) else {
            continue;
        };
        if !seen.insert(symbol.clone()) {
            continue;
        }
        queue.push_back(Request {
            symbol,
            generic,
            callee: callee.clone(),
            type_arguments: type_arguments.clone(),
            types: types.restricted_to(type_arguments),
        });
    }
}

/// Whether a body of `bodies` builds a closure.
#[cfg(debug_assertions)]
fn holds_closure(bodies: &Arena<prism_mir::BodyId, Body>) -> bool {
    bodies.iter().any(|(_, body)| {
        body.blocks.iter().any(|(_, block)| {
            block.statements.iter().any(|statement| {
                matches!(&statement.kind, prism_mir::StatementKind::Assign(_, rvalue)
                    if matches!(rvalue.as_ref(), prism_mir::Rvalue::Closure { .. }))
            })
        })
    })
}

/// Reports `E6004` for each growing cycle of the calls between the records' generics (D8): one
/// that passes through several files, since the type checker rejects those within one file
/// (`E4027`).
fn check_cross_file_recursion(
    records: &HashMap<String, GenericRecord>,
    order: &[String],
) -> Result<(), Vec<InstanceError>> {
    let mut sites: Vec<(String, Span)> = Vec::new();
    let mut edges = Vec::new();
    for name in order {
        let record = &records[name];
        for body in std::iter::once(&record.function)
            .chain(&record.defaults)
            .chain(&record.closures)
        {
            for (_, block) in body.blocks.iter() {
                let TerminatorKind::Call {
                    callee,
                    type_arguments,
                    ..
                } = &block.terminator.kind
                else {
                    continue;
                };
                let (Callee::Function(path) | Callee::Default { function: path, .. }) = callee
                else {
                    continue;
                };
                let callee = generic_name(path);
                if type_arguments.is_empty() || !records.contains_key(&callee) {
                    continue;
                }
                sites.push((record.path.module.clone(), block.terminator.span));
                edges.extend(edges_of(name, &callee, type_arguments, sites.len() - 1));
            }
        }
    }
    let errors: Vec<InstanceError> = growing_cycles(&edges)
        .into_iter()
        .map(|cycle| {
            let growing = &cycle[0];
            let names: Vec<String> = cycle.iter().map(|edge| edge.from.generic.clone()).collect();
            let parameter = records[&growing.to.generic]
                .function
                .type_parameters
                .get(growing.to.parameter as usize)
                .map_or_else(|| "?".to_string(), |parameter| parameter.name.clone());
            let quoted: Vec<String> = names.iter().map(|name| format!("'{}'", name)).collect();
            let mut steps = quoted.clone();
            steps.push(quoted[0].clone());
            let (module, span) = sites[growing.site].clone();
            InstanceError {
                module,
                error: CodegenError {
                    kind: CodegenErrorKind::PolymorphicRecursionAcrossFiles,
                    message: format!(
                        "polymorphic recursion across files: {} instantiates itself through {} with '{}' for '{}'",
                        quoted[0],
                        quoted[1..].join(", "),
                        growing.argument,
                        parameter
                    ),
                    span,
                    notes: vec![format!(
                        "the cycle {} would need infinitely many instances",
                        steps.join(" -> ")
                    )],
                    help: Some(
                        "pass the same type arguments on every recursive call, or wrap the value in a non-generic type"
                            .to_string(),
                    ),
                },
            }
        })
        .collect();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Runs the backend's support check on every instance unit (D10), reporting each unsupported
/// construct once per location in a generic's source, however many instances share it.
pub fn check_instances(units: &[InstanceUnit]) -> Result<(), Vec<InstanceError>> {
    let mut reported: HashSet<(String, Span, String)> = HashSet::new();
    let mut errors = Vec::new();
    for unit in units {
        if let Err(unit_errors) = check_file(&unit.file) {
            for error in unit_errors {
                let key = (unit.file.module.clone(), error.span, error.message.clone());
                if reported.insert(key) {
                    errors.push(InstanceError {
                        module: unit.file.module.clone(),
                        error,
                    });
                }
            }
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::lower_files;

    fn collect(files: &[MirFile]) -> Result<Vec<InstanceUnit>, Vec<InstanceError>> {
        collect_instances(&files.iter().collect::<Vec<_>>())
    }

    fn symbols(files: &[MirFile]) -> Vec<String> {
        collect(files)
            .unwrap_or_else(|errors| panic!("{:?}", errors))
            .into_iter()
            .map(|unit| unit.symbol)
            .collect()
    }

    #[test]
    fn a_transitive_instance_through_a_module_the_caller_does_not_import() {
        let files = lower_files(&[
            (
                "lib/util",
                "package lib\n\
                 public struct Pair<A, B>(val a: A, val b: B)\n\
                 public fun <T : Copy> twin(x: T): Pair<T, T> = Pair(x, x)\n",
            ),
            (
                "lib/box",
                "package lib\nimport /lib/util\n\
                 public fun <T : Copy> first(x: T): T = twin(x).a\n",
            ),
            (
                "app/main",
                "package app\nimport /lib/box\nfun main() { println(first(7)) }\n",
            ),
        ]);
        assert_eq!(
            symbols(&files),
            ["lib/box::first<Int>", "lib/util::twin<Int>"]
        );
        let units = collect(&files).unwrap();
        // The instance of `twin` knows `Pair`, which `app/main` never names.
        let pair = prism_typed_tree::NominalId::new("lib/util", "Pair");
        assert!(units[1].file.types.definition(&pair).is_some());
        assert_eq!(units[1].file.module, "lib/util");
    }

    #[test]
    fn two_callers_share_one_instance() {
        let files = lower_files(&[
            ("lib/box", "package lib\npublic fun <T> keep(x: T): T = x\n"),
            (
                "app/other",
                "package app\nimport /lib/box\npublic fun other(): Int = keep(2)\n",
            ),
            (
                "app/main",
                "package app\nimport /lib/box\nimport /app/other\n\
                 fun main() { println(keep(1)); println(other()) }\n",
            ),
        ]);
        assert_eq!(symbols(&files), ["lib/box::keep<Int>"]);
    }

    #[test]
    fn a_default_generator_instance() {
        let files = lower_files(&[(
            "main",
            "fun <T : Copy> pad(x: T, y: T = x): T = y\nfun main() { println(pad(1)) }\n",
        )]);
        assert_eq!(
            symbols(&files),
            ["main::pad<Int>", "main::pad<Int>#default(y)"]
        );
    }

    #[test]
    fn a_cross_file_growing_cycle() {
        let files = lower_files(&[
            (
                "a/x",
                "package a\nimport /a/y\n\
                 public struct Box<T>(val v: T)\n\
                 public fun <T> ping(x: T, n: Int): Int = if (n == 0) 0 else pong(Box(x), n - 1)\n",
            ),
            (
                "a/y",
                "package a\nimport /a/x\npublic fun <T> pong(x: T, n: Int): Int = ping(x, n)\n",
            ),
        ]);
        let errors = collect(&files).unwrap_err();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].module, "a/x");
        assert_eq!(errors[0].error.code(), "E6004");
        assert!(
            errors[0].error.message.contains("'a/x::ping'")
                && errors[0].error.message.contains("'a/y::pong'"),
            "{}",
            errors[0].error.message
        );
    }

    #[test]
    fn an_unsupported_construct_is_reported_once_for_every_instance() {
        let files = lower_files(&[(
            "main",
            "fun <T> later(x: T): Int { val f = { 1 }; return 1 }\n\
             fun main() { println(later(1)); println(later(true)) }\n",
        )]);
        let units = collect(&files).unwrap();
        assert_eq!(units.len(), 2);
        let errors = check_instances(&units).unwrap_err();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].error.code(), "E6001");
    }
}

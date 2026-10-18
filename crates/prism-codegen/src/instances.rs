//! Instance planning and building over a whole program (D5, D8 and D10 of `monomorphization`,
//! D7 and D13 of `instance-cache`).
//!
//! [`plan_instances`] lists the instances a program needs from its files' summaries alone
//! ([`FileSummary`]), without any MIR: it rejects polymorphic recursion through generics of
//! several files (`E6004`) over every generic's calls, then runs a worklist from every file's
//! roots, in which an instance's requests are its generic's calls with its type arguments
//! substituted. Each request names an instance by its symbol, and a symbol already planned is
//! dropped before anything else is computed, so planning is linear in the program's distinct
//! instances and their generic calls, plus a hash lookup per call. The absence of a growing
//! cycle bounds the depth of every type argument, so the worklist ends.
//!
//! [`build_instance_unit`] then substitutes one planned instance from its generic's record
//! ([`prism_mir::GenericRecord::instantiate`]) into a unit of its own, with the closures
//! lowered inside it (D11 of `closures`), over the record's type table and the program's
//! definitions of its type arguments. [`collect_instances`] is both steps over elaborated
//! files, for the dumps and [`crate::Program::compile`].

use crate::error::{CodegenError, CodegenErrorKind};
use crate::supported::check_file;
use crate::symbols::callee_symbol;
use prism_ast::Span;
use prism_mir::{
    Callee, DefaultParameter, FileSummary, GenericRecord, GenericSummary, InstanceRequest, MirFile,
    MirPhase,
};
use prism_typed_tree::instantiation::{cycle_generics, edges_of, generic_name, growing_cycles};
use prism_typed_tree::{Arena, DeclarationPath, Type, TypeTable};
use std::collections::{HashMap, HashSet, VecDeque};

/// The code unit of one instance: the generic's module, the instance body followed by its
/// closures, and the definitions of the record's and the type arguments' types.
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

/// One instance a program needs, planned from summaries (D7 of `instance-cache`).
#[derive(Debug, Clone, PartialEq)]
pub struct PlannedInstance {
    /// The instance's symbol, which names its unit and its slot.
    pub symbol: String,
    /// The generic function or extension.
    pub path: DeclarationPath,
    /// For an instance of a default value's generator, the parameter it generates.
    pub default: Option<DefaultParameter>,
    /// The concrete type arguments, one per type parameter of the generic.
    pub type_arguments: Vec<Type>,
    /// The generic's Monomorphization Hash.
    pub record_hash: [u8; 32],
}

impl PlannedInstance {
    /// The call that names the instance.
    pub fn callee(&self) -> Callee {
        match &self.default {
            None => Callee::Function(self.path.clone()),
            Some(default) => Callee::Default {
                function: self.path.clone(),
                parameter: default.parameter,
                name: default.name.clone(),
            },
        }
    }
}

/// The instances the program of `summaries`, one per file, needs, sorted by symbol, or the
/// `E6004` errors of its cross-file polymorphic recursions. Reads no MIR.
pub fn plan_instances(
    summaries: &[&FileSummary],
) -> Result<Vec<PlannedInstance>, Vec<InstanceError>> {
    let mut generics: HashMap<String, &GenericSummary> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    for summary in summaries {
        for generic in &summary.generics {
            let name = generic_name(&generic.path);
            order.push(name.clone());
            generics.insert(name, generic);
        }
    }
    check_cross_file_recursion(&generics, &order)?;

    let mut seen: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<PlannedInstance> = VecDeque::new();
    let mut request = |request: &InstanceRequest,
                       arguments: Option<&[Type]>,
                       queue: &mut VecDeque<PlannedInstance>| {
        let Some(generic) = generics.get(&generic_name(&request.callee)) else {
            return;
        };
        let type_arguments: Vec<Type> = match arguments {
            Some(arguments) => request
                .type_arguments
                .iter()
                .map(|ty| ty.substitute(arguments))
                .collect(),
            None => request.type_arguments.clone(),
        };
        let Some(symbol) = callee_symbol(&request.callee(), &type_arguments) else {
            return;
        };
        if !seen.insert(symbol.clone()) {
            return;
        }
        queue.push_back(PlannedInstance {
            symbol,
            path: request.callee.clone(),
            default: request.default.clone(),
            type_arguments,
            record_hash: generic.record_hash,
        });
    };
    for summary in summaries {
        for root in &summary.roots {
            request(root, None, &mut queue);
        }
    }
    let mut planned = Vec::new();
    while let Some(instance) = queue.pop_front() {
        let generic = generics[&generic_name(&instance.path)];
        if let Some(calls) = generic.calls_of(instance.default.as_ref()) {
            for call in calls {
                request(call, Some(&instance.type_arguments), &mut queue);
            }
        }
        planned.push(instance);
    }
    planned.sort_by(|a, b| a.symbol.cmp(&b.symbol));
    Ok(planned)
}

/// The unit of the instance `planned` of the generic whose record is `record`, over the
/// record's own table and `types`, which defines every struct and enum the type arguments
/// reach: the program's definitions (D13 of `instance-cache`). `None` when the record has no
/// such body.
pub fn build_instance_unit(
    record: &GenericRecord,
    planned: &PlannedInstance,
    types: &TypeTable,
) -> Option<InstanceUnit> {
    let mut table = record.types.clone();
    table.extend_from(&types.restricted_to(&planned.type_arguments));
    let instances = record.instantiate(&planned.callee(), &planned.type_arguments, &table)?;
    let mut bodies = Arena::new();
    for instance in instances {
        bodies.alloc(instance);
    }
    let file = MirFile {
        module: record.path.module.clone(),
        phase: MirPhase::Elaborated,
        types: table,
        bodies,
        externals: Vec::new(),
    };
    #[cfg(debug_assertions)]
    {
        let violations = file.validate();
        assert!(
            violations.is_empty(),
            "instance {} is ill-formed:\n{}\n{}",
            planned.symbol,
            violations.join("\n"),
            file.dump()
        );
    }
    Some(InstanceUnit {
        symbol: planned.symbol.clone(),
        file,
    })
}

/// The instance units the program of `files`, each elaborated, needs, sorted by symbol, or the
/// `E6004` errors of its cross-file polymorphic recursions: [`plan_instances`] over the files'
/// summaries, then [`build_instance_unit`] for every planned instance over the union of the
/// files' type tables.
pub fn collect_instances(files: &[&MirFile]) -> Result<Vec<InstanceUnit>, Vec<InstanceError>> {
    let mut records: HashMap<String, GenericRecord> = HashMap::new();
    let mut summaries = Vec::new();
    let mut types = TypeTable::new();
    for file in files {
        let file_records = GenericRecord::of_file(file);
        // No record is stored or keyed, so no hash is needed.
        let hashes = vec![[0; 32]; file_records.len()];
        summaries.push(FileSummary::of(file, &file_records, &hashes, None));
        for record in file_records {
            records.insert(generic_name(&record.path), record);
        }
        types.extend_from(&file.types);
    }
    let planned = plan_instances(&summaries.iter().collect::<Vec<_>>())?;
    Ok(planned
        .iter()
        .filter_map(|instance| {
            build_instance_unit(&records[&generic_name(&instance.path)], instance, &types)
        })
        .collect())
}

/// Reports `E6004` for each growing cycle of the calls between the generics of `generics` (D8
/// of `monomorphization`): one that passes through several files, since the type checker
/// rejects those within one file (`E4027`).
fn check_cross_file_recursion(
    generics: &HashMap<String, &GenericSummary>,
    order: &[String],
) -> Result<(), Vec<InstanceError>> {
    let mut sites: Vec<(String, Span)> = Vec::new();
    let mut edges = Vec::new();
    for name in order {
        let generic = generics[name];
        for call in generic.all_calls() {
            let callee = generic_name(&call.callee);
            if !generics.contains_key(&callee) {
                continue;
            }
            sites.push((generic.path.module.clone(), call.span));
            edges.extend(edges_of(
                name,
                &callee,
                &call.type_arguments,
                sites.len() - 1,
            ));
        }
    }
    let errors: Vec<InstanceError> = growing_cycles(&edges)
        .into_iter()
        .map(|cycle| {
            let growing = &cycle[0];
            let names = cycle_generics(&cycle);
            let parameter = generics[&growing.to.generic]
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
    fn an_instance_holds_its_closures() {
        let files = lower_files(&[(
            "main",
            "fun run(g: () -> Int): Int = g()\n\
             fun <T : Copy> count(x: T, f: (T) -> Int): Int = run { run { f(x) } + 1 }\n\
             fun main() { println(count(2) { n -> n * 10 }) }\n",
        )]);
        let units = collect(&files).unwrap();
        assert_eq!(units.len(), 1);
        let symbols: Vec<String> = units[0]
            .file
            .bodies
            .iter()
            .map(|(_, body)| crate::symbols::symbol_of(body, &units[0].file))
            .collect();
        assert_eq!(
            symbols,
            [
                "main::count<Int>",
                "main::count<Int>#closure0",
                "main::count<Int>#closure0#closure0"
            ]
        );
        assert!(check_instances(&units).is_ok());
    }

    #[test]
    fn an_unsupported_construct_is_reported_once_for_every_instance() {
        let files = lower_files(&[(
            "main",
            "fun <T> later(x: T): Int { val f: compose () -> Unit = {}; return 1 }\n\
             fun main() { println(later(1)); println(later(true)) }\n",
        )]);
        let units = collect(&files).unwrap();
        assert_eq!(units.len(), 2);
        let errors = check_instances(&units).unwrap_err();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].error.code(), "E6001");
    }
}

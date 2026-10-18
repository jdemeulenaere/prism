//! Polymorphic recursion and non-regular generic types (`E4027`, D8 and D9 of
//! `monomorphization`).
//!
//! A generic function that reaches a call of itself with a type argument nesting its own type
//! parameter (`fun <T> nest(x: T) { nest(Box(x)) }`), directly or through other generic
//! functions of the file, would need infinitely many instances, and so would a generic struct or
//! enum that reaches itself through its fields with such an argument
//! (`struct Nest<T>(val next: Heap<Nest<Box<T>>>?)`), whose glue would need infinitely many
//! types. Both are growing cycles of the instantiation graph
//! ([`prism_typed_tree::instantiation`]), found once per file, whether or not anything
//! instantiates the generic. The search is linear in the number of type-parameter occurrences in
//! the file's generic calls and generic field types.

use crate::error::TypeError;
use prism_ast::decl::Declaration;
use prism_ast::module::SourceFile;
use prism_ast::Span;
use prism_typed_tree::instantiation::{edges_of, generic_name, growing_cycles, InstantiationEdge};
use prism_typed_tree::{
    Callee, ExpressionKind, FunctionItem, Item, NominalId, Type, TypeDef, TypeParameter, TypedFile,
};
use std::collections::{HashMap, HashSet};

/// Where an edge of the graph comes from.
struct Site {
    span: Span,
    /// For a field or payload, the generic type it holds; `None` for a call.
    application: Option<Type>,
    /// Whether the site is in a declaration of the file itself.
    own: bool,
}

/// The `E4027` errors of `file`, checked from `source_file`. Types already reported in
/// `errors` as infinitely sized (`E4023`) are skipped, so one mistake gives one error.
pub fn check_polymorphic_recursion(
    source_file: &SourceFile,
    file: &TypedFile,
    errors: &[TypeError],
) -> Vec<TypeError> {
    let mut found = function_cycles(file);
    found.extend(type_cycles(source_file, file, errors));
    found
}

/// How a generic function or extension is named in a diagnostic: `nest`, or `Box<T>.get`.
fn display_name(item: &FunctionItem) -> String {
    match &item.path.receiver {
        Some(receiver) => format!("{}.{}", receiver, item.path.name),
        None => item.path.name.clone(),
    }
}

fn function_cycles(file: &TypedFile) -> Vec<TypeError> {
    let generics: HashMap<String, &FunctionItem> = file
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Function(function) if !function.type_parameters.is_empty() => {
                Some((generic_name(&function.path), function))
            }
            _ => None,
        })
        .collect();
    if generics.is_empty() {
        return Vec::new();
    }
    let mut sites = Vec::new();
    let mut edges = Vec::new();
    for item in &file.items {
        let Item::Function(function) = item else {
            continue;
        };
        if function.type_parameters.is_empty() {
            continue;
        }
        let caller = generic_name(&function.path);
        // The body's arena holds its default values and closures too.
        for (_, expression) in file.bodies[function.body].expressions.iter() {
            let ExpressionKind::Call(call) = &expression.kind else {
                continue;
            };
            let Callee::Function(path) = &call.callee else {
                continue;
            };
            if call.type_arguments.is_empty() || path.module != file.module {
                continue;
            }
            let callee = generic_name(path);
            if !generics.contains_key(&callee) {
                continue;
            }
            sites.push(Site {
                span: expression.span,
                application: None,
                own: true,
            });
            edges.extend(edges_of(
                &caller,
                &callee,
                &call.type_arguments,
                sites.len() - 1,
            ));
        }
    }
    growing_cycles(&edges)
        .into_iter()
        .map(|cycle| {
            let growing = &cycle[0];
            let names: Vec<String> = cycle
                .iter()
                .map(|edge| display_name(generics[&edge.from.generic]))
                .collect();
            let callee = generics[&growing.to.generic];
            let parameter = parameter_name(&callee.type_parameters, growing.to.parameter);
            let message = if cycle.len() == 1 {
                format!(
                    "'{}' instantiates itself with '{}' for '{}'",
                    names[0], growing.argument, parameter
                )
            } else {
                format!(
                    "'{}' instantiates itself through {} with '{}' for '{}'",
                    names[0],
                    quoted_list(&names[1..]),
                    growing.argument,
                    parameter
                )
            };
            TypeError::PolymorphicRecursion {
                message,
                cycle: cycle_text(&names),
                is_type: false,
                span: sites[growing.site].span,
            }
        })
        .collect()
}

fn type_cycles(source_file: &SourceFile, file: &TypedFile, errors: &[TypeError]) -> Vec<TypeError> {
    let infinite: HashSet<&str> = errors
        .iter()
        .filter_map(|error| match error {
            TypeError::InfinitelySizedType { type_name, .. } => Some(type_name.as_str()),
            _ => None,
        })
        .collect();
    // The spans of the own declarations' members: per type, one per field or variant.
    let mut member_spans: HashMap<NominalId, Vec<Span>> = HashMap::new();
    for declaration in &source_file.declarations {
        let (name, spans): (&String, Vec<Span>) = match declaration {
            Declaration::Struct(s) if !s.type_parameters.is_empty() => {
                (&s.name, s.fields.iter().map(|field| field.span).collect())
            }
            Declaration::Enum(e) if !e.type_parameters.is_empty() => (
                &e.name,
                e.variants.iter().map(|variant| variant.span).collect(),
            ),
            _ => continue,
        };
        if infinite.contains(name.as_str()) {
            continue;
        }
        member_spans.insert(NominalId::new(file.module.clone(), name.clone()), spans);
    }
    if member_spans.is_empty() {
        return Vec::new();
    }
    let mut sites = Vec::new();
    let mut edges: Vec<InstantiationEdge> = Vec::new();
    let mut names: HashMap<String, (String, Vec<TypeParameter>)> = HashMap::new();
    for id in file.types.ids() {
        let Some(definition) = file.types.definition(id) else {
            continue;
        };
        if definition.parameters().is_empty() {
            continue;
        }
        let own = id.module == file.module;
        // An own generic type without member spans is one `E4023` reported.
        let spans = member_spans.get(id);
        if own && spans.is_none() {
            continue;
        }
        let key = format!("{}::{}", id.module, id.name);
        names.insert(
            key.clone(),
            (id.name.clone(), definition.parameters().to_vec()),
        );
        let members: Vec<Vec<&Type>> = match definition {
            TypeDef::Struct { fields, .. } => fields.iter().map(|(_, _, ty)| vec![ty]).collect(),
            TypeDef::Enum { variants, .. } => variants
                .iter()
                .map(|(_, payloads)| payloads.iter().collect())
                .collect(),
        };
        for (index, member) in members.into_iter().enumerate() {
            let span = spans
                .and_then(|spans| spans.get(index).copied())
                .unwrap_or_else(Span::dummy);
            let mut applications = Vec::new();
            for ty in member {
                generic_applications(ty, &mut applications);
            }
            for application in applications {
                let (target, arguments) = match &application {
                    Type::Struct { id, arguments } | Type::Enum { id, arguments } => {
                        (format!("{}::{}", id.module, id.name), arguments.clone())
                    }
                    _ => continue,
                };
                sites.push(Site {
                    span,
                    application: Some(application.clone()),
                    own,
                });
                edges.extend(edges_of(&key, &target, &arguments, sites.len() - 1));
            }
        }
    }
    growing_cycles(&edges)
        .into_iter()
        .filter_map(|cycle| {
            // Reported at the growing field when it is the file's own, else at the first own
            // field on the cycle; a cycle of other modules' types alone is theirs to report.
            let reported = if sites[cycle[0].site].own {
                &cycle[0]
            } else {
                cycle.iter().find(|edge| sites[edge.site].own)?
            };
            let growing = &cycle[0];
            let type_names: Vec<String> = cycle
                .iter()
                .map(|edge| {
                    names
                        .get(&edge.from.generic)
                        .map_or_else(|| edge.from.generic.clone(), |(name, _)| name.clone())
                })
                .collect();
            let application = sites[growing.site]
                .application
                .as_ref()
                .map_or_else(String::new, Type::to_string);
            let message = if cycle.len() == 1 {
                format!("'{}' contains '{}'", type_names[0], application)
            } else {
                format!(
                    "'{}' contains '{}', which contains '{}' again through {}",
                    type_names[0],
                    application,
                    type_names[0],
                    quoted_list(&type_names[1..])
                )
            };
            Some(TypeError::PolymorphicRecursion {
                message,
                cycle: cycle_text(&type_names),
                is_type: true,
                span: sites[reported.site].span,
            })
        })
        .collect()
}

/// Every application of a generic struct or enum in `ty`, outer ones first, looking through
/// boxes, nullables, references and other generic types' arguments but not into function types,
/// whose values hold none of these inline (D9).
fn generic_applications(ty: &Type, out: &mut Vec<Type>) {
    match ty {
        Type::Struct { arguments, .. } | Type::Enum { arguments, .. } => {
            if !arguments.is_empty() {
                out.push(ty.clone());
            }
            for argument in arguments {
                generic_applications(argument, out);
            }
        }
        Type::Reference { inner, .. } | Type::Nullable(inner) | Type::Heap(inner) => {
            generic_applications(inner, out)
        }
        _ => {}
    }
}

fn parameter_name(parameters: &[TypeParameter], index: u32) -> String {
    parameters
        .get(index as usize)
        .map_or_else(|| format!("#{}", index), |parameter| parameter.name.clone())
}

/// `'a'`, `'a' and 'b'`, `'a', 'b' and 'c'`.
fn quoted_list(names: &[String]) -> String {
    let quoted: Vec<String> = names.iter().map(|name| format!("'{}'", name)).collect();
    match quoted.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} and {}", rest.join(", "), last),
        Some((last, _)) => last.clone(),
        None => String::new(),
    }
}

/// The cycle as a note writes it: `'ping' -> 'pong' -> 'ping'`.
fn cycle_text(names: &[String]) -> String {
    let mut steps: Vec<String> = names.iter().map(|name| format!("'{}'", name)).collect();
    if let Some(first) = names.first() {
        steps.push(format!("'{}'", first));
    }
    steps.join(" -> ")
}

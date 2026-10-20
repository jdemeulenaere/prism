//! Type fingerprints and slot contracts (D5 and D6 of `hot-reload`).
//!
//! A **type fingerprint** is a structural BLAKE3 hash of a type: scalars, `String` and `Range`
//! by tag, references, nullables, boxes and lists by tag and contents, function types by their
//! parameters, result and compose flag, and structs and enums by qualified name, marker, type
//! arguments, and their fields' or variants' names and fingerprints in declaration order, with
//! type arguments substituted. A definition that reaches itself (through a box or a list) is hashed with a
//! back-reference to its position on the stack of definitions being hashed, so cycles end. Two
//! types with the same fingerprint have the same layout and the same drop, clone and equality
//! behaviour, and every [`prism_typed_tree::NominalId`] has one definition per program, so the
//! same type has the same fingerprint in every unit.
//!
//! A **slot contract** is what a slot's code expects of its callers: its kind, its parameter
//! and result fingerprints, and for closure code its captures with their modes, fingerprints
//! and paths (`t.id`), and how its environment is held. A reload repoints a slot only to code of the same contract;
//! code of a different contract gets a new slot ([`crate::link::FunctionTable::slot_for`]).
//!
//! Fingerprints are memoized per unit, so a unit's contracts cost time linear in the
//! definitions its signatures reach.

use crate::layout::EnvironmentKind;
use prism_abi::{ContentHash, DeterministicHasher};
use prism_mir::{Body, BodyOwner, CaptureMode, LocalId};
use prism_typed_tree::{NominalId, Type, TypeDef, TypeTable};
use std::collections::HashMap;

/// The fingerprints of the types of one unit, over its type table, memoized.
///
/// A fingerprint depends only on the type, never on which types were fingerprinted before it,
/// so the same type has the same fingerprint in every unit and in every version of a unit.
/// The two memos therefore hold different things: `fingerprints` the fingerprint of each type
/// asked for, and `definitions` the hash of each definition, which a fingerprint embeds.
pub struct TypeFingerprints<'t> {
    types: &'t TypeTable,
    /// The fingerprint of each type [`TypeFingerprints::fingerprint`] computed.
    fingerprints: HashMap<Type, ContentHash>,
    /// The hash of each definition computed so far that does not depend on where it is reached
    /// from, with the definitions on the cycles it reaches.
    definitions: HashMap<Frame, DefinitionHash>,
}

/// A struct or enum with its type arguments: an entry of the stack of definitions being
/// hashed, and the key of a definition's hash.
type Frame = (NominalId, Vec<Type>);

/// The memoized hash of a definition.
struct DefinitionHash {
    hash: ContentHash,
    /// The definitions on the cycles the definition reaches, itself included when it reaches
    /// itself. The hash holds where none of them is being hashed: reached from one of them, the
    /// definition would refer back to it instead of hashing it again.
    cycles: Vec<Frame>,
}

impl<'t> TypeFingerprints<'t> {
    pub fn new(types: &'t TypeTable) -> Self {
        Self {
            types,
            fingerprints: HashMap::new(),
            definitions: HashMap::new(),
        }
    }

    /// The fingerprint of `ty`.
    pub fn fingerprint(&mut self, ty: &Type) -> ContentHash {
        if let Some(hash) = self.fingerprints.get(ty) {
            return *hash;
        }
        let mut hasher = DeterministicHasher::new();
        hasher.write_str("type");
        let mut stack: Vec<Frame> = Vec::new();
        let mut cycles: Vec<Frame> = Vec::new();
        self.hash_type(ty, &mut stack, &mut cycles, &mut hasher);
        let hash = hasher.finish();
        self.fingerprints.insert(ty.clone(), hash);
        hash
    }

    /// Writes `ty` into `hasher`, and returns the lowest position of `stack` it refers back to,
    /// or `usize::MAX` when it refers to no definition being hashed. Adds to `cycles` the
    /// definitions on the cycles it reaches.
    fn hash_type(
        &mut self,
        ty: &Type,
        stack: &mut Vec<Frame>,
        cycles: &mut Vec<Frame>,
        hasher: &mut DeterministicHasher,
    ) -> usize {
        let tag = |hasher: &mut DeterministicHasher, tag: u8| {
            hasher.write_u8(tag);
            usize::MAX
        };
        match ty {
            Type::Int => tag(hasher, 1),
            Type::Float => tag(hasher, 2),
            Type::Bool => tag(hasher, 3),
            Type::Char => tag(hasher, 4),
            Type::String => tag(hasher, 5),
            Type::Unit => tag(hasher, 6),
            Type::Nothing => tag(hasher, 7),
            Type::Reference { mutable, inner } => {
                hasher.write_u8(8);
                hasher.write_u8(u8::from(*mutable));
                self.hash_type(inner, stack, cycles, hasher)
            }
            Type::Nullable(inner) => {
                hasher.write_u8(9);
                self.hash_type(inner, stack, cycles, hasher)
            }
            Type::Heap(inner) => {
                hasher.write_u8(10);
                self.hash_type(inner, stack, cycles, hasher)
            }
            Type::List(inner) => {
                hasher.write_u8(11);
                self.hash_type(inner, stack, cycles, hasher)
            }
            // A receiver does not change the representation (D2 of `receiver-lambdas`).
            Type::Fn {
                is_compose,
                params,
                kinds,
                ret,
                ..
            } => {
                hasher.write_u8(12);
                hasher.write_u8(u8::from(*is_compose));
                hasher.write_u64(params.len() as u64);
                let mut lowest = usize::MAX;
                for (param, kind) in params.iter().zip(kinds) {
                    hasher.write_u8(match kind {
                        prism_typed_tree::ParameterKind::Plain => 0,
                        prism_typed_tree::ParameterKind::Escaping => 1,
                        prism_typed_tree::ParameterKind::Attached => 2,
                    });
                    lowest = lowest.min(self.hash_type(param, stack, cycles, hasher));
                }
                lowest.min(self.hash_type(ret, stack, cycles, hasher))
            }
            Type::Struct { id, arguments } | Type::Enum { id, arguments } => {
                hasher.write_u8(13);
                self.hash_nominal(id, arguments, stack, cycles, hasher)
            }
            Type::Opaque(name) => {
                hasher.write_u8(14);
                hasher.write_str(name);
                usize::MAX
            }
            Type::Parameter(parameter) => {
                hasher.write_u8(15);
                hasher.write_u64(u64::from(parameter.index));
                hasher.write_str(&parameter.name);
                usize::MAX
            }
            Type::Error => tag(hasher, 16),
        }
    }

    /// Writes the struct or enum `id` with `arguments` into `hasher`: a back-reference when it is
    /// being hashed already, otherwise the hash of its definition, memoized when it refers to no
    /// outer definition.
    fn hash_nominal(
        &mut self,
        id: &NominalId,
        arguments: &[Type],
        stack: &mut Vec<Frame>,
        cycles: &mut Vec<Frame>,
        hasher: &mut DeterministicHasher,
    ) -> usize {
        if let Some(position) = stack
            .iter()
            .position(|(other, other_arguments)| other == id && other_arguments == arguments)
        {
            hasher.write_u8(0);
            hasher.write_u64((stack.len() - position) as u64);
            for frame in &stack[position..] {
                if !cycles.contains(frame) {
                    cycles.push(frame.clone());
                }
            }
            return position;
        }
        let frame: Frame = (id.clone(), arguments.to_vec());
        if let Some(memoized) = self.definitions.get(&frame) {
            if memoized.cycles.iter().all(|member| !stack.contains(member)) {
                hasher.write_u8(1);
                hasher.write_hash(&memoized.hash);
                for member in &memoized.cycles {
                    if !cycles.contains(member) {
                        cycles.push(member.clone());
                    }
                }
                return usize::MAX;
            }
        }
        let own = stack.len();
        let first_cycle = cycles.len();
        stack.push(frame.clone());
        let mut inner = DeterministicHasher::new();
        inner.write_str(&id.module);
        inner.write_str(&id.name);
        inner.write_u8(match self.types.marker(id) {
            None => 0,
            Some(prism_ast::TypeMarker::Copy) => 1,
            Some(prism_ast::TypeMarker::Clone) => 2,
        });
        // The type arguments are hashed whatever the definition says: a struct without fields,
        // such as an intrinsic state handle, whose one word names a cell of its argument's
        // type, is a different type for each argument (`State<Int>` is not `State<String>`).
        let mut lowest = usize::MAX;
        inner.write_u64(arguments.len() as u64);
        for argument in arguments {
            lowest = lowest.min(self.hash_type(argument, stack, cycles, &mut inner));
        }
        match self.types.definition(id) {
            Some(TypeDef::Struct { fields, .. }) => {
                inner.write_u8(1);
                inner.write_u64(fields.len() as u64);
                for (name, is_var, field) in fields {
                    inner.write_str(name);
                    inner.write_u8(u8::from(*is_var));
                    let field = field.substitute(arguments);
                    lowest = lowest.min(self.hash_type(&field, stack, cycles, &mut inner));
                }
            }
            Some(TypeDef::Enum { variants, .. }) => {
                inner.write_u8(2);
                inner.write_u64(variants.len() as u64);
                for (name, payloads) in variants {
                    inner.write_str(name);
                    inner.write_u64(payloads.len() as u64);
                    for payload in payloads {
                        let payload = payload.substitute(arguments);
                        lowest = lowest.min(self.hash_type(&payload, stack, cycles, &mut inner));
                    }
                }
            }
            None => inner.write_u8(0),
        }
        stack.pop();
        let hash = inner.finish();
        // A definition that refers back only to itself, or to nothing on the stack, hashes the
        // same wherever it is reached from, as long as no definition on its cycles is being
        // hashed there.
        if lowest >= own {
            let cycles = cycles[first_cycle..].to_vec();
            self.definitions
                .insert(frame, DefinitionHash { hash, cycles });
        }
        hasher.write_u8(1);
        hasher.write_hash(&hash);
        if lowest >= own {
            usize::MAX
        } else {
            lowest
        }
    }
}

/// What a slot's code is the code of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyKind {
    Function,
    Default,
    Initializer,
    Accessor,
    Closure,
    /// A restart function, whose contract also covers its composable's (D8 of
    /// `compose-hot-reload`, adopted by `compose-transform`).
    Restart,
    /// A root content adapter.
    RootContent,
}

impl BodyKind {
    /// The kind of `body`.
    pub fn of(body: &Body) -> BodyKind {
        match body.owner {
            BodyOwner::Function(_) => BodyKind::Function,
            BodyOwner::Value(_) => BodyKind::Initializer,
            BodyOwner::Default { .. } => BodyKind::Default,
            BodyOwner::Closure { .. } => BodyKind::Closure,
            BodyOwner::Restart { .. } => BodyKind::Restart,
            BodyOwner::RootContent { .. } => BodyKind::RootContent,
        }
    }

    fn tag(self) -> u8 {
        match self {
            BodyKind::Function => 1,
            BodyKind::Default => 2,
            BodyKind::Initializer => 3,
            BodyKind::Accessor => 4,
            BodyKind::Closure => 5,
            BodyKind::Restart => 6,
            BodyKind::RootContent => 7,
        }
    }
}

/// The contract of the code of `body`: its kind, compose flag, parameter and result
/// fingerprints, and for a closure its captures in order, each with its mode, fingerprint and,
/// for a closure written in source, its path, and its environment's kind (`environment`, `None`
/// outside closures). Parameter names are not part of it.
pub fn slot_contract(
    body: &Body,
    environment: Option<EnvironmentKind>,
    restarted: Option<&Body>,
    fingerprints: &mut TypeFingerprints,
) -> ContentHash {
    let mut hasher = DeterministicHasher::new();
    hasher.write_str("contract");
    // A restart function's own type never changes, so its contract covers the composable it
    // restarts: the two split together when a kept parameter's type changes (D8 of
    // `compose-hot-reload`, adopted by `compose-transform`).
    if let Some(restarted) = restarted {
        let restarted = slot_contract(restarted, None, None, fingerprints);
        hasher.write_str("restarts");
        hasher.write_hash(&restarted);
    }
    hasher.write_u8(BodyKind::of(body).tag());
    hasher.write_u8(u8::from(body.is_compose));
    hasher.write_u64(body.argument_count as u64);
    for local in body.arguments() {
        hasher.write_hash(&fingerprints.fingerprint(&body.locals[local].ty));
    }
    hasher.write_hash(&fingerprints.fingerprint(body.return_type()));
    if body.owner.is_closure() {
        hasher.write_u64(body.capture_count as u64);
        for index in 0..body.capture_count {
            let local = LocalId((body.argument_count + 1 + index) as u32);
            hasher.write_u8(match body.capture_modes.get(index) {
                Some(CaptureMode::Shared) => 1,
                Some(CaptureMode::Mutable) => 2,
                Some(CaptureMode::Owned) => 3,
                Some(CaptureMode::Weak) => 4,
                None => 0,
            });
            hasher.write_hash(&fingerprints.fingerprint(&body.locals[local].ty));
            // A closure written in source takes each capture by its path, the binding's name
            // and its fields' names (`t.id`), which says what the environment's value is: two
            // paths of one type (`t.id`, `t.count`) are different captures, so a value made
            // before a reload never runs code that reads its copy of one path as the other.
            // A restart function's or root content adapter's captures are its composable's
            // arguments, matched by position as a function's parameters are.
            if matches!(body.owner, BodyOwner::Closure { .. }) {
                hasher.write_str(body.locals[local].name.as_deref().unwrap_or(""));
            }
        }
        hasher.write_u8(match environment {
            None => 0,
            Some(EnvironmentKind::None) => 1,
            Some(EnvironmentKind::Frame) => 2,
            Some(EnvironmentKind::Inline) => 3,
            Some(EnvironmentKind::Allocated) => 4,
        });
    }
    hasher.finish()
}

/// The contract of the accessor of a top-level value of type `ty` (D8 of `hot-reload`): it
/// takes nothing and returns the address of storage laid out for `ty`.
pub fn accessor_contract(ty: &Type, fingerprints: &mut TypeFingerprints) -> ContentHash {
    let mut hasher = DeterministicHasher::new();
    hasher.write_str("contract");
    hasher.write_u8(BodyKind::Accessor.tag());
    hasher.write_hash(&fingerprints.fingerprint(ty));
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{lower_files, lower_source};
    use prism_mir::MirFile;

    /// The fingerprint of the struct or enum `name` declared in `file`.
    fn nominal(file: &MirFile, name: &str, is_enum: bool) -> ContentHash {
        let id = NominalId::new(file.module.clone(), name);
        let ty = if is_enum {
            Type::plain_enum(id)
        } else {
            Type::plain_struct(id)
        };
        TypeFingerprints::new(&file.types).fingerprint(&ty)
    }

    fn function_contract(file: &MirFile, name: &str) -> ContentHash {
        let (_, body) = file
            .bodies
            .iter()
            .find(|(_, body)| matches!(&body.owner, BodyOwner::Function(path) if path.name == name))
            .unwrap_or_else(|| panic!("no body {}", name));
        slot_contract(body, None, None, &mut TypeFingerprints::new(&file.types))
    }

    fn closure_contract(file: &MirFile) -> ContentHash {
        let (_, body) = file
            .bodies
            .iter()
            .find(|(_, body)| matches!(body.owner, BodyOwner::Closure { .. }))
            .expect("a closure");
        slot_contract(body, None, None, &mut TypeFingerprints::new(&file.types))
    }

    /// The contract of the restart function of `name`, which covers its composable's.
    fn restart_contract(file: &MirFile, name: &str) -> ContentHash {
        let (function, _) = file
            .bodies
            .iter()
            .find(|(_, body)| matches!(&body.owner, BodyOwner::Function(path) if path.name == name))
            .unwrap_or_else(|| panic!("no body {}", name));
        let (_, body) = file
            .bodies
            .iter()
            .find(|(_, body)| matches!(body.owner, BodyOwner::Restart { function: f } if f == function))
            .expect("a restart function");
        slot_contract(
            body,
            None,
            Some(&file.bodies[function]),
            &mut TypeFingerprints::new(&file.types),
        )
    }

    /// D8 of `compose-hot-reload`, adopted by `compose-transform`: a restart function splits
    /// with its composable when a kept parameter's type changes, and not for a body edit.
    #[test]
    fn a_restart_contract_covers_its_composable() {
        let int = lower_source("compose fun Badge(n: Int) { println(n) }\n");
        let edited = lower_source("compose fun Badge(n: Int) { println(n + 1) }\n");
        let bool = lower_source("compose fun Badge(n: Bool) { println(n) }\n");
        assert_eq!(
            restart_contract(&int, "Badge"),
            restart_contract(&edited, "Badge")
        );
        assert_ne!(
            restart_contract(&int, "Badge"),
            restart_contract(&bool, "Badge")
        );
    }

    /// compose-hot-reload, hot-reload: "A changed parameter type splits the restart function".
    #[test]
    fn a_changed_parameter_type_splits_the_restart_function() {
        let int = lower_source("compose fun Label(n: Int) { println(\"a\") }\n");
        let double = lower_source("compose fun Label(n: Double) { println(\"a\") }\n");
        let edited = lower_source("compose fun Label(n: Int) { println(\"b\") }\n");
        assert_ne!(
            restart_contract(&int, "Label"),
            restart_contract(&double, "Label")
        );
        assert_eq!(
            restart_contract(&int, "Label"),
            restart_contract(&edited, "Label")
        );
    }

    #[test]
    fn int_and_double_fields_differ() {
        let int = lower_source("struct P { val x: Int }\n");
        let double = lower_source("struct P { val x: Double }\n");
        assert_ne!(nominal(&int, "P", false), nominal(&double, "P", false));
        assert_eq!(
            nominal(&int, "P", false),
            nominal(
                &lower_source("struct P { val x: Int }\nfun f() {}\n"),
                "P",
                false
            )
        );
    }

    /// `state-handles`: a handle's one word names a cell of its type argument, so handles of
    /// two value types never share a fingerprint, nor do instances of a struct whose fields do
    /// not mention its parameter.
    #[test]
    fn type_arguments_of_a_struct_without_fields_differ() {
        let file = lower_source("struct Tag<T> { }\nfun f(a: Tag<Int>, b: Tag<Bool>) {}\n");
        let tag = |argument: Type| Type::Struct {
            id: NominalId::new(file.module.clone(), "Tag"),
            arguments: vec![argument],
        };
        let mut fingerprints = TypeFingerprints::new(&file.types);
        assert_ne!(
            fingerprints.fingerprint(&tag(Type::Int)),
            fingerprints.fingerprint(&tag(Type::Bool))
        );
    }

    #[test]
    fn adding_a_field_changes_a_struct() {
        let one = lower_source("struct P { val x: Int }\n");
        let two = lower_source("struct P { val x: Int, val y: Int }\n");
        assert_ne!(nominal(&one, "P", false), nominal(&two, "P", false));
    }

    #[test]
    fn a_recursive_enum_through_a_box_terminates() {
        let file = lower_source("enum Chain { End, Link(Int, Heap<Chain>) }\n");
        let first = nominal(&file, "Chain", true);
        assert_eq!(first, nominal(&file, "Chain", true));
        let other = lower_source("enum Chain { End, Link(Bool, Heap<Chain>) }\n");
        assert_ne!(first, nominal(&other, "Chain", true));
    }

    /// A fingerprint does not depend on which types were fingerprinted before it: reordering
    /// declarations must not change a contract.
    #[test]
    fn fingerprints_do_not_depend_on_the_order_they_are_computed_in() {
        let file = lower_source("struct P { val x: Int }\n");
        let p = Type::plain_struct(NominalId::new(file.module.clone(), "P"));
        let reference = Type::Reference {
            mutable: false,
            inner: Box::new(p.clone()),
        };
        let mut first = TypeFingerprints::new(&file.types);
        let in_order = (first.fingerprint(&p), first.fingerprint(&reference));
        let mut second = TypeFingerprints::new(&file.types);
        let reference_first = second.fingerprint(&reference);
        assert_eq!(in_order, (second.fingerprint(&p), reference_first));
    }

    #[test]
    fn mutually_recursive_definitions_do_not_depend_on_the_order() {
        let file = lower_source("enum A { End, Next(Heap<B>) }\nenum B { Stop, Then(Heap<A>) }\n");
        let a = Type::plain_enum(NominalId::new(file.module.clone(), "A"));
        let b = Type::plain_enum(NominalId::new(file.module.clone(), "B"));
        let boxed_b = Type::Heap(Box::new(b.clone()));
        let mut first = TypeFingerprints::new(&file.types);
        let a_first = (
            first.fingerprint(&a),
            first.fingerprint(&b),
            first.fingerprint(&boxed_b),
        );
        let mut second = TypeFingerprints::new(&file.types);
        let boxed_b_second = second.fingerprint(&boxed_b);
        let b_second = second.fingerprint(&b);
        let a_second = second.fingerprint(&a);
        assert_eq!(a_first, (a_second, b_second, boxed_b_second));
        assert_ne!(a_first.0, a_first.1);
    }

    #[test]
    fn boxes_of_different_contents_differ() {
        let file = lower_source("fun f() {}\n");
        let mut fingerprints = TypeFingerprints::new(&file.types);
        let boxed = |inner: Type| Type::Heap(Box::new(inner));
        assert_ne!(
            fingerprints.fingerprint(&boxed(Type::Int)),
            fingerprints.fingerprint(&boxed(Type::Nullable(Box::new(Type::Int))))
        );
    }

    #[test]
    fn the_same_type_in_two_units_has_the_same_fingerprint() {
        let files = lower_files(&[
            (
                "lib/shapes",
                "package lib\npublic struct P { val x: Int }\n",
            ),
            (
                "app/main",
                "package app\nimport /lib/shapes\nfun show(p: &P): Int = p.x\n",
            ),
        ]);
        let id = NominalId::new("lib/shapes", "P");
        let in_lib =
            TypeFingerprints::new(&files[0].types).fingerprint(&Type::plain_struct(id.clone()));
        let in_app = TypeFingerprints::new(&files[1].types).fingerprint(&Type::plain_struct(id));
        assert_eq!(in_lib, in_app);
    }

    #[test]
    fn a_renamed_parameter_keeps_the_contract_and_an_added_one_changes_it() {
        let base = function_contract(&lower_source("fun step(i: Int): Int = i\n"), "step");
        let renamed = function_contract(&lower_source("fun step(j: Int): Int = j * 2\n"), "step");
        assert_eq!(base, renamed);
        let added = function_contract(
            &lower_source("fun step(i: Int, k: Int): Int = i * k\n"),
            "step",
        );
        assert_ne!(base, added);
        let result = function_contract(&lower_source("fun step(i: Int): Bool = true\n"), "step");
        assert_ne!(base, result);
    }

    #[test]
    fn a_changed_capture_changes_a_closures_contract() {
        let one = closure_contract(&lower_source(
            "fun main() { val k = 1; val f: (Int) -> Int = { n -> n + k }; println(f(1)) }\n",
        ));
        let same = closure_contract(&lower_source(
            "fun main() { val k = 1; val f: (Int) -> Int = { n -> n * k }; println(f(1)) }\n",
        ));
        assert_eq!(one, same);
        let two = closure_contract(&lower_source(
            "fun main() { val k = 1; val j = 5; val f: (Int) -> Int = { n -> n + k + j }; println(f(1)) }\n",
        ));
        assert_ne!(one, two);
    }

    #[test]
    fn a_captured_path_swapped_for_one_of_the_same_type_changes_a_closures_contract() {
        let closure = |path: &str| {
            closure_contract(&lower_source(&format!(
                "struct T {{ val id: Int, val count: Int }}\n\
                 fun make(t: &T): () -> Int = {{ {} }}\n",
                path
            )))
        };
        assert_eq!(closure("t.id"), closure("t.id + 1"));
        assert_ne!(closure("t.id"), closure("t.count"));
    }
}

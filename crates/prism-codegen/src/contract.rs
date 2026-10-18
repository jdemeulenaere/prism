//! Type fingerprints and slot contracts (D5 and D6 of `hot-reload`).
//!
//! A **type fingerprint** is a structural BLAKE3 hash of a type: scalars, `String` and `Range`
//! by tag, references, nullables, boxes and lists by tag and contents, function types by their
//! parameters, result and compose flag, and structs and enums by qualified name, marker, and
//! their fields' or variants' names and fingerprints in declaration order, with type arguments
//! substituted. A definition that reaches itself (through a box or a list) is hashed with a
//! back-reference to its position on the stack of definitions being hashed, so cycles end. Two
//! types with the same fingerprint have the same layout and the same drop, clone and equality
//! behaviour, and every [`prism_typed_tree::NominalId`] has one definition per program, so the
//! same type has the same fingerprint in every unit.
//!
//! A **slot contract** is what a slot's code expects of its callers: its kind, its parameter
//! and result fingerprints, and for closure code its captures with their modes and fingerprints
//! and how its environment is held. A reload repoints a slot only to code of the same contract;
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
pub struct TypeFingerprints<'t> {
    types: &'t TypeTable,
    memo: HashMap<Type, ContentHash>,
}

/// A struct or enum on the stack of definitions being hashed.
type Frame = (NominalId, Vec<Type>);

impl<'t> TypeFingerprints<'t> {
    pub fn new(types: &'t TypeTable) -> Self {
        Self {
            types,
            memo: HashMap::new(),
        }
    }

    /// The fingerprint of `ty`.
    pub fn fingerprint(&mut self, ty: &Type) -> ContentHash {
        if let Some(hash) = self.memo.get(ty) {
            return *hash;
        }
        let mut hasher = DeterministicHasher::new();
        hasher.write_str("type");
        let mut stack: Vec<Frame> = Vec::new();
        self.hash_type(ty, &mut stack, &mut hasher);
        let hash = hasher.finish();
        self.memo.insert(ty.clone(), hash);
        hash
    }

    /// Writes `ty` into `hasher`, and returns the lowest position of `stack` it refers back to,
    /// or `usize::MAX` when it refers to no definition being hashed.
    fn hash_type(
        &mut self,
        ty: &Type,
        stack: &mut Vec<Frame>,
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
                self.hash_type(inner, stack, hasher)
            }
            Type::Nullable(inner) => {
                hasher.write_u8(9);
                self.hash_type(inner, stack, hasher)
            }
            Type::Heap(inner) => {
                hasher.write_u8(10);
                self.hash_type(inner, stack, hasher)
            }
            Type::List(inner) => {
                hasher.write_u8(11);
                self.hash_type(inner, stack, hasher)
            }
            Type::Fn {
                is_compose,
                params,
                escaping,
                ret,
            } => {
                hasher.write_u8(12);
                hasher.write_u8(u8::from(*is_compose));
                hasher.write_u64(params.len() as u64);
                let mut lowest = usize::MAX;
                for (param, escaping) in params.iter().zip(escaping) {
                    hasher.write_u8(u8::from(*escaping));
                    lowest = lowest.min(self.hash_type(param, stack, hasher));
                }
                lowest.min(self.hash_type(ret, stack, hasher))
            }
            Type::Struct { id, arguments } | Type::Enum { id, arguments } => {
                hasher.write_u8(13);
                self.hash_nominal(id, arguments, stack, hasher)
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
        hasher: &mut DeterministicHasher,
    ) -> usize {
        if let Some(position) = stack
            .iter()
            .position(|(other, other_arguments)| other == id && other_arguments == arguments)
        {
            hasher.write_u8(0);
            hasher.write_u64((stack.len() - position) as u64);
            return position;
        }
        let ty = Type::Struct {
            id: id.clone(),
            arguments: arguments.to_vec(),
        };
        if let Some(hash) = self.memo.get(&ty) {
            hasher.write_u8(1);
            hasher.write_hash(hash);
            return usize::MAX;
        }
        let own = stack.len();
        stack.push((id.clone(), arguments.to_vec()));
        let mut inner = DeterministicHasher::new();
        inner.write_str(&id.module);
        inner.write_str(&id.name);
        inner.write_u8(match self.types.marker(id) {
            None => 0,
            Some(prism_ast::TypeMarker::Copy) => 1,
            Some(prism_ast::TypeMarker::Clone) => 2,
        });
        let mut lowest = usize::MAX;
        match self.types.definition(id) {
            Some(TypeDef::Struct { fields, .. }) => {
                inner.write_u8(1);
                inner.write_u64(fields.len() as u64);
                for (name, is_var, field) in fields {
                    inner.write_str(name);
                    inner.write_u8(u8::from(*is_var));
                    let field = field.substitute(arguments);
                    lowest = lowest.min(self.hash_type(&field, stack, &mut inner));
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
                        lowest = lowest.min(self.hash_type(&payload, stack, &mut inner));
                    }
                }
            }
            None => {
                inner.write_u8(0);
                inner.write_u64(arguments.len() as u64);
                for argument in arguments {
                    lowest = lowest.min(self.hash_type(argument, stack, &mut inner));
                }
            }
        }
        stack.pop();
        let hash = inner.finish();
        // A definition that refers back only to itself, or to nothing on the stack, hashes the
        // same wherever it is reached from.
        if lowest >= own {
            self.memo.insert(ty, hash);
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
}

impl BodyKind {
    /// The kind of `body`.
    pub fn of(body: &Body) -> BodyKind {
        match body.owner {
            BodyOwner::Function(_) => BodyKind::Function,
            BodyOwner::Value(_) => BodyKind::Initializer,
            BodyOwner::Default { .. } => BodyKind::Default,
            BodyOwner::Closure { .. } => BodyKind::Closure,
        }
    }

    fn tag(self) -> u8 {
        match self {
            BodyKind::Function => 1,
            BodyKind::Default => 2,
            BodyKind::Initializer => 3,
            BodyKind::Accessor => 4,
            BodyKind::Closure => 5,
        }
    }
}

/// The contract of the code of `body`: its kind, compose flag, parameter and result
/// fingerprints, and for a closure its captures in order, each with its mode and fingerprint,
/// and its environment's kind (`environment`, `None` outside closures). Parameter names are not
/// part of it.
pub fn slot_contract(
    body: &Body,
    environment: Option<EnvironmentKind>,
    fingerprints: &mut TypeFingerprints,
) -> ContentHash {
    let mut hasher = DeterministicHasher::new();
    hasher.write_str("contract");
    hasher.write_u8(BodyKind::of(body).tag());
    hasher.write_u8(u8::from(body.is_compose));
    hasher.write_u64(body.argument_count as u64);
    for local in body.arguments() {
        hasher.write_hash(&fingerprints.fingerprint(&body.locals[local].ty));
    }
    hasher.write_hash(&fingerprints.fingerprint(body.return_type()));
    if let BodyOwner::Closure { .. } = body.owner {
        hasher.write_u64(body.capture_count as u64);
        for index in 0..body.capture_count {
            let local = LocalId((body.argument_count + 1 + index) as u32);
            hasher.write_u8(match body.capture_modes.get(index) {
                Some(CaptureMode::Shared) => 1,
                Some(CaptureMode::Mutable) => 2,
                Some(CaptureMode::Owned) => 3,
                None => 0,
            });
            hasher.write_hash(&fingerprints.fingerprint(&body.locals[local].ty));
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
        slot_contract(body, None, &mut TypeFingerprints::new(&file.types))
    }

    fn closure_contract(file: &MirFile) -> ContentHash {
        let (_, body) = file
            .bodies
            .iter()
            .find(|(_, body)| matches!(body.owner, BodyOwner::Closure { .. }))
            .expect("a closure");
        slot_contract(body, None, &mut TypeFingerprints::new(&file.types))
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
}

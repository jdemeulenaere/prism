//! How values are represented, and the signatures of generated functions (D6 of
//! `codegen-core`, D9 and D10 of `codegen-aggregates`).
//!
//! An `Int` is an `i64`, a `Double` an `f64`, a `Bool` an `i8` holding 0 or 1, a `Char` an
//! `i32` holding a Unicode scalar value, and a reference, a `Heap<T>` and a nullable of either a
//! pointer (0 for `null`). A one-word intrinsic struct (a state handle or a weak reference) is
//! an `i64` holding its handle. A `String`, a struct, an enum, a function value (a descriptor address
//! and an environment address, D2 of `closures`) and any other nullable live in memory, laid out
//! as [`crate::layout`] says. `Unit` and `Nothing` have no representation: they are left out of
//! signatures. A value in memory is passed as its address, and returned through an address the
//! caller passes before the arguments, so a machine signature depends on no layout. A closure's
//! code takes its environment's address after the result address and before the arguments (D4
//! of `closures`). Every function uses the host's C calling convention.

use cranelift_codegen::ir::{types, AbiParam, Signature, Type as MachineType};
use cranelift_codegen::isa::CallConv;
use prism_mir::Body;
use prism_typed_tree::Type;

/// How a value of some type is held.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Representation {
    /// `Unit` and `Nothing`: no value at all.
    Empty,
    /// One machine value.
    Scalar(MachineType),
    /// Bytes in memory, laid out by the type's [`crate::layout::Layout`]: a `String`, a list, a
    /// struct, an enum, a function value, or a nullable that is not a nullable pointer.
    Memory,
}

/// The representation of `ty`, or `None` for a type with none (type parameters and unknown
/// types). It follows from the type's form alone, never from a
/// layout, so callers and bodies agree on it.
pub fn representation(ty: &Type, pointer: MachineType) -> Option<Representation> {
    match ty {
        Type::Unit | Type::Nothing => Some(Representation::Empty),
        // A one-word intrinsic struct is its handle word (D16 of `compose-transform`, the
        // `code-generation` delta of `ui-toolkit`).
        Type::Struct { id, .. } if id.intrinsic().is_some() => {
            Some(Representation::Scalar(types::I64))
        }
        Type::String
        | Type::List(_)
        | Type::Struct { .. }
        | Type::Enum { .. }
        | Type::Fn { .. } => Some(Representation::Memory),
        Type::Nullable(inner) if !is_pointer(inner) => Some(Representation::Memory),
        _ => machine_type(ty, pointer).map(Representation::Scalar),
    }
}

/// Whether values of `ty` are a pointer that is never null: a reference or a `Heap<T>`, so a
/// nullable of `ty` is a pointer whose `null` is 0 (D7 of `codegen-aggregates`).
pub fn is_pointer(ty: &Type) -> bool {
    matches!(ty, Type::Reference { .. } | Type::Heap(_))
}

/// The machine type of a value of `ty` held in one machine value: a scalar, a reference, a
/// `Heap<T>` or a nullable of a reference or a `Heap<T>`.
pub fn machine_type(ty: &Type, pointer: MachineType) -> Option<MachineType> {
    match ty {
        Type::Int => Some(types::I64),
        Type::Float => Some(types::F64),
        Type::Bool => Some(types::I8),
        Type::Char => Some(types::I32),
        Type::Reference { .. } | Type::Heap(_) => Some(pointer),
        Type::Nullable(inner) if is_pointer(inner) => Some(pointer),
        Type::Struct { id, .. } if id.intrinsic().is_some() => Some(types::I64),
        _ => None,
    }
}

/// A parameter or result of machine type `ty`. A `Bool` is zero-extended at calls, as the C
/// calling conventions of some hosts (Apple's arm64) require of the caller.
pub fn abi_param(ty: MachineType) -> AbiParam {
    if ty == types::I8 {
        AbiParam::new(ty).uext()
    } else {
        AbiParam::new(ty)
    }
}

/// The signature of a function taking `parameters` and returning `result`: a scalar is one
/// machine value; a value in memory is its address, and a result in memory is written through
/// an address passed first, before the arguments, with nothing returned (D10 of
/// `codegen-aggregates`). Parameters and results without a representation are left out.
pub fn signature(
    parameters: &[Type],
    result: &Type,
    call_conv: CallConv,
    pointer: MachineType,
) -> Signature {
    let mut signature = Signature::new(call_conv);
    match representation(result, pointer) {
        Some(Representation::Memory) => signature.params.push(AbiParam::new(pointer)),
        Some(Representation::Scalar(machine)) => signature.returns.push(abi_param(machine)),
        Some(Representation::Empty) | None => {}
    }
    for parameter in parameters {
        match representation(parameter, pointer) {
            Some(Representation::Memory) => signature.params.push(AbiParam::new(pointer)),
            Some(Representation::Scalar(machine)) => signature.params.push(abi_param(machine)),
            Some(Representation::Empty) | None => {}
        }
    }
    signature
}

/// The signature of `body`: its argument types and its return type.
pub fn body_signature(body: &Body, call_conv: CallConv, pointer: MachineType) -> Signature {
    let parameters: Vec<Type> = body
        .arguments()
        .map(|local| body.locals[local].ty.clone())
        .collect();
    signature(&parameters, body.return_type(), call_conv, pointer)
}

/// The signature of the code of the closure `body`: its body's, with the environment's address
/// after the result address, if any, and before the arguments (D4 of `closures`).
pub fn closure_signature(body: &Body, call_conv: CallConv, pointer: MachineType) -> Signature {
    let parameters: Vec<Type> = body
        .arguments()
        .map(|local| body.locals[local].ty.clone())
        .collect();
    function_value_signature(&parameters, body.return_type(), call_conv, pointer)
}

/// Whether `ty` is a function type or a nullable of one.
pub fn is_function_type(ty: &Type) -> bool {
    match ty {
        Type::Fn { .. } => true,
        Type::Nullable(inner) => matches!(**inner, Type::Fn { .. }),
        _ => false,
    }
}

/// The signature with which a function value taking `parameters` and returning `result` is
/// called: the closure signature of D4 of `closures`, built from the call's parameter types
/// and destination, so that it agrees with [`closure_signature`] of every closure the value
/// may hold.
pub fn function_value_signature(
    parameters: &[Type],
    result: &Type,
    call_conv: CallConv,
    pointer: MachineType,
) -> Signature {
    let mut signature = signature(parameters, result, call_conv, pointer);
    let position = usize::from(representation(result, pointer) == Some(Representation::Memory));
    signature.params.insert(position, AbiParam::new(pointer));
    signature
}

/// Whether a kept value of `ty` is a borrowed argument's identity, its address and its
/// version in two words (D7 of `strong-skipping`): a reference or a nullable of one, which a
/// slot never holds otherwise.
pub fn is_identity_type(ty: &Type) -> bool {
    match ty {
        Type::Reference { .. } => true,
        Type::Nullable(inner) => matches!(**inner, Type::Reference { .. }),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbols::{callee_symbol, symbol_of};
    use crate::test_support::lower_files;
    use prism_mir::TerminatorKind;
    use prism_typed_tree::NominalId;
    use std::collections::HashMap;

    const CALL_CONV: CallConv = CallConv::SystemV;

    #[test]
    fn scalar_types_and_references() {
        let pointer = types::I64;
        let reference = |inner: Type| Type::Reference {
            mutable: false,
            inner: Box::new(inner),
        };
        assert_eq!(machine_type(&Type::Int, pointer), Some(types::I64));
        assert_eq!(machine_type(&Type::Float, pointer), Some(types::F64));
        assert_eq!(machine_type(&Type::Bool, pointer), Some(types::I8));
        assert_eq!(machine_type(&Type::Char, pointer), Some(types::I32));
        assert_eq!(machine_type(&reference(Type::Int), pointer), Some(pointer));
        assert_eq!(
            machine_type(&reference(reference(Type::Bool)), pointer),
            Some(pointer)
        );
        assert_eq!(machine_type(&Type::String, pointer), None);
        assert_eq!(
            representation(&Type::Unit, pointer),
            Some(Representation::Empty)
        );
        let point = Type::plain_struct(NominalId::new("main", "Point"));
        let range = Type::plain_struct(NominalId::builtin_range());
        for ty in [
            point.clone(),
            range.clone(),
            Type::plain_enum(NominalId::new("main", "Color")),
            Type::Int.nullable(),
            point.clone().nullable(),
        ] {
            assert_eq!(
                representation(&ty, pointer),
                Some(Representation::Memory),
                "{}",
                ty
            );
        }
        for ty in [
            reference(point.clone()),
            reference(range),
            reference(point.clone()).nullable(),
            Type::Heap(Box::new(Type::Int)).nullable(),
        ] {
            assert_eq!(machine_type(&ty, pointer), Some(pointer), "{}", ty);
        }
        assert_eq!(
            representation(&Type::String, pointer),
            Some(Representation::Memory)
        );
        let function = Type::function(false, vec![Type::Int], Type::Unit);
        assert_eq!(
            representation(&function, pointer),
            Some(Representation::Memory)
        );
        assert_eq!(
            representation(&function.nullable(), pointer),
            Some(Representation::Memory)
        );
    }

    #[test]
    fn signatures_of_closures_and_function_values() {
        let files = lower_files(&[(
            "main",
            "fun run(f: () -> Int): Int = f()
             fun point(f: (Int, Bool) -> String): Int = 1
             fun main() { val a = 1; println(run { a + 1 }); point { n, b -> \"x\" } }
",
        )]);
        let pointer = types::I64;
        let closures: Vec<String> = files[0]
            .bodies
            .iter()
            .filter(|(_, body)| matches!(body.owner, prism_mir::BodyOwner::Closure { .. }))
            .map(|(_, body)| closure_signature(body, CALL_CONV, pointer).to_string())
            .collect();
        // The environment, then the arguments; a result in memory is written through an
        // address passed before the environment.
        assert_eq!(
            closures,
            ["(i64) -> i64 system_v", "(i64, i64, i64, i8 uext) system_v"]
        );
        let value = function_value_signature(&[Type::Int], &Type::Int, CALL_CONV, pointer);
        assert_eq!(value.to_string(), "(i64, i64) -> i64 system_v");
        let value = function_value_signature(&[], &Type::String, CALL_CONV, pointer);
        assert_eq!(value.to_string(), "(i64, i64) system_v");
    }

    #[test]
    fn signatures_of_scalar_functions() {
        // The scenario "Signatures of scalar functions" of the `code-generation` spec.
        let files = lower_files(&[(
            "main",
            "fun f(n: Int, b: Bool, c: Char, d: Double, r: &Int): Double = d\nfun g(n: Int) {}\n",
        )]);
        let pointer = types::I64;
        let signatures: Vec<String> = files[0]
            .bodies
            .iter()
            .map(|(_, body)| body_signature(body, CALL_CONV, pointer).to_string())
            .collect();
        assert_eq!(
            signatures,
            [
                "(i64, i8 uext, i32, f64, i64) -> f64 system_v",
                "(i64) system_v"
            ]
        );
    }

    /// The signatures of the bodies of `files`, by symbol, and the number of calls of them, each
    /// of whose signature is checked against its callee's.
    fn check_calls_agree(files: &[prism_mir::MirFile]) -> (HashMap<String, Signature>, usize) {
        let pointer = types::I64;
        let mut bodies = HashMap::new();
        for file in files {
            for (_, body) in file.bodies.iter() {
                let symbol = symbol_of(body, file);
                bodies.insert(symbol, body_signature(body, CALL_CONV, pointer));
            }
        }
        let mut calls = 0;
        for file in files {
            for (_, body) in file.bodies.iter() {
                for (_, block) in body.blocks.iter() {
                    let TerminatorKind::Call {
                        callee,
                        parameters,
                        type_arguments,
                        destination,
                        ..
                    } = &block.terminator.kind
                    else {
                        continue;
                    };
                    let Some(symbol) = callee_symbol(callee, type_arguments) else {
                        continue;
                    };
                    let result = destination.ty(body, &file.types);
                    let call = signature(parameters, &result, CALL_CONV, pointer);
                    assert_eq!(Some(&call), bodies.get(&symbol), "call of {}", symbol);
                    calls += 1;
                }
            }
        }
        (bodies, calls)
    }

    #[test]
    fn signatures_over_aggregates_agree_with_their_calls() {
        let files = lower_files(&[
            (
                "lib/geo",
                "package lib\n\
                 public struct Point : Copy { val x: Int, val y: Int }\n\
                 public enum Shape : Copy { Dot(Point), Empty }\n\
                 public fun mirror(p: Point, n: Int): Point = Point(p.y, p.x)\n\
                 public fun wrap(p: Point): Shape = Shape.Dot(p)\n\
                 public fun maybe(n: Int?, r: (&Point)?): Int? = n\n\
                 public fun local(): Point = mirror(Point(1, 2), 3)\n",
            ),
            (
                "app/main",
                "package app\nimport /lib/geo\n\
                 fun f(): Int { val p = mirror(Point(1, 2), 3); val s = wrap(p); val m = maybe(null, &p); return 1 }\n",
            ),
        ]);
        let (bodies, calls) = check_calls_agree(&files);
        assert_eq!(calls, 4);
        let text = |symbol: &str| bodies[symbol].to_string();
        // The result's address, then the address of `p` and `n`.
        assert_eq!(text("lib/geo::mirror"), "(i64, i64, i64) system_v");
        assert_eq!(text("lib/geo::wrap"), "(i64, i64) system_v");
        // `Int?` is in memory, `(&Point)?` a pointer.
        assert_eq!(text("lib/geo::maybe"), "(i64, i64, i64) system_v");
    }

    #[test]
    fn a_body_and_its_calls_agree_on_the_signature() {
        let files = lower_files(&[
            (
                "lib/math",
                "package lib\n\
                 public fun mix(a: Int, b: &Int, c: Bool, d: Double = 1.0): Double = d\n\
                 public fun Int.twice(): Int = this * 2\n\
                 public fun quiet(): Unit {}\n\
                 public fun local(): Double = mix(1, &2, true) + mix(1, &2, false, 2.0)\n\
                 public compose fun Show(s: &String, n: Int): Unit {}\n",
            ),
            (
                "app/main",
                "package app\nimport /lib/math\n\
                 fun f(): Double { quiet(); val n = 3.twice(); return mix(n, &n, false) }\n\
                 compose fun Screen(t: &String) { Show(t, 1) }\n",
            ),
        ]);
        let pointer = types::I64;
        let mut bodies = HashMap::new();
        for file in &files {
            for (_, body) in file.bodies.iter() {
                let symbol = symbol_of(body, file);
                bodies.insert(symbol, body_signature(body, CALL_CONV, pointer));
            }
        }
        let mut calls = 0;
        for file in &files {
            for (_, body) in file.bodies.iter() {
                for (_, block) in body.blocks.iter() {
                    let TerminatorKind::Call {
                        callee,
                        parameters,
                        type_arguments,
                        destination,
                        ..
                    } = &block.terminator.kind
                    else {
                        continue;
                    };
                    let Some(symbol) = callee_symbol(callee, type_arguments) else {
                        continue;
                    };
                    let result = destination.ty(body, &file.types);
                    let call = signature(parameters, &result, CALL_CONV, pointer);
                    assert_eq!(Some(&call), bodies.get(&symbol), "call of {}", symbol);
                    calls += 1;
                }
            }
        }
        // Seven plain calls, and the composable calls with their version words (D4 of
        // `strong-skipping`): `Screen`'s of `Show`, and each restart function's.
        assert_eq!(calls, 10);
    }
}

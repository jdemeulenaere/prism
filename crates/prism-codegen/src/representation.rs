//! How values are represented, and the signatures of generated functions (D6 of
//! `codegen-core`).
//!
//! An `Int` is an `i64`, a `Double` an `f64`, a `Bool` an `i8` holding 0 or 1, a `Char` an
//! `i32` holding a Unicode scalar value, and a reference to a supported referent a pointer.
//! `Unit` and `Nothing` have no representation: they are left out of signatures. A local of the
//! built-in `Range` is three variables (D7); a `Range` exists nowhere else yet. Every function
//! uses the host's C calling convention.

use cranelift_codegen::ir::{types, AbiParam, Signature, Type as MachineType};
use cranelift_codegen::isa::CallConv;
use prism_mir::Body;
use prism_typed_tree::{NominalId, Type};

/// How a value of some type is held.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Representation {
    /// `Unit` and `Nothing`: no value at all.
    Empty,
    /// One machine value.
    Scalar(MachineType),
    /// A built-in `Range` local: `start`, `end` and `endInclusive`.
    Range,
}

/// The representation of `ty`, or `None` when the backend does not support it.
pub fn representation(ty: &Type, pointer: MachineType) -> Option<Representation> {
    match ty {
        Type::Unit | Type::Nothing => Some(Representation::Empty),
        Type::Struct { id, .. } if *id == NominalId::builtin_range() => Some(Representation::Range),
        _ => machine_type(ty, pointer).map(Representation::Scalar),
    }
}

/// The machine type of a value of `ty` held in one machine value: a scalar or a reference to a
/// supported referent.
pub fn machine_type(ty: &Type, pointer: MachineType) -> Option<MachineType> {
    match ty {
        Type::Int => Some(types::I64),
        Type::Float => Some(types::F64),
        Type::Bool => Some(types::I8),
        Type::Char => Some(types::I32),
        Type::Reference { inner, .. } if is_supported_referent(inner, pointer) => Some(pointer),
        _ => None,
    }
}

/// Whether a reference to `ty` is supported: a reference to a scalar, to `Unit`, or to another
/// supported reference (D8). References to a `Range` or to any aggregate are `codegen-aggregates`'.
pub fn is_supported_referent(ty: &Type, pointer: MachineType) -> bool {
    matches!(ty, Type::Unit) || machine_type(ty, pointer).is_some()
}

/// The size and alignment in bytes of a value of `ty` in memory: in a stack slot or a top-level
/// value's storage. `Unit` takes one byte, so that its storage has an address.
pub fn memory_layout(ty: &Type, pointer: MachineType) -> (u32, u32) {
    match machine_type(ty, pointer) {
        Some(machine) => (machine.bytes(), machine.bytes()),
        None => (1, 1),
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

/// The signature of a function taking `parameters` and returning `result`. Parameters and
/// results without a representation are left out.
pub fn signature(
    parameters: &[Type],
    result: &Type,
    call_conv: CallConv,
    pointer: MachineType,
) -> Signature {
    let mut signature = Signature::new(call_conv);
    for parameter in parameters {
        if let Some(machine) = machine_type(parameter, pointer) {
            signature.params.push(abi_param(machine));
        }
    }
    if let Some(machine) = machine_type(result, pointer) {
        signature.returns.push(abi_param(machine));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbols::{callee_symbol, symbol_of};
    use crate::test_support::lower_files;
    use prism_mir::TerminatorKind;
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
        assert_eq!(machine_type(&reference(Type::String), pointer), None);
        assert_eq!(machine_type(&Type::String, pointer), None);
        assert_eq!(
            representation(&Type::Unit, pointer),
            Some(Representation::Empty)
        );
        let range = Type::plain_struct(NominalId::builtin_range());
        assert_eq!(representation(&range, pointer), Some(Representation::Range));
        assert_eq!(machine_type(&reference(range), pointer), None);
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

    #[test]
    fn a_body_and_its_calls_agree_on_the_signature() {
        let files = lower_files(&[
            (
                "lib/math",
                "package lib\n\
                 public fun mix(a: Int, b: &Int, c: Bool, d: Double = 1.0): Double = d\n\
                 public fun Int.twice(): Int = this * 2\n\
                 public fun quiet(): Unit {}\n\
                 public fun local(): Double = mix(1, &2, true) + mix(1, &2, false, 2.0)\n",
            ),
            (
                "app/main",
                "package app\nimport /lib/math\n\
                 fun f(): Double { quiet(); val n = 3.twice(); return mix(n, &n, false) }\n",
            ),
        ]);
        let pointer = types::I64;
        let mut bodies = HashMap::new();
        for (_, body) in files[0].bodies.iter() {
            let symbol = symbol_of(&body.owner, &files[0]);
            bodies.insert(symbol, body_signature(body, CALL_CONV, pointer));
        }
        let mut calls = 0;
        for file in &files {
            for (_, body) in file.bodies.iter() {
                for (_, block) in body.blocks.iter() {
                    let TerminatorKind::Call {
                        callee,
                        parameters,
                        destination,
                        ..
                    } = &block.terminator.kind
                    else {
                        continue;
                    };
                    let Some(symbol) = callee_symbol(callee) else {
                        continue;
                    };
                    let result = destination.ty(body, &file.types);
                    let call = signature(parameters, &result, CALL_CONV, pointer);
                    assert_eq!(Some(&call), bodies.get(&symbol), "call of {}", symbol);
                    calls += 1;
                }
            }
        }
        assert_eq!(calls, 7);
    }
}

//! The four lifetime elision rules (D2 of `borrowck-elision`): which inputs a returned borrow is
//! tied to, from a signature alone.
//!
//! One function, [`tied_inputs`], decides it from the parameter types and the kind of signature,
//! and both sides use it: the caller, for what a call's result keeps borrowed
//! ([`crate::loan_flow::LoanFlow::call_items`]), and the callee, for what its body may return
//! (`returns`). Since an imported callee's parameter types and receiver come from its ABI, the
//! two agree exactly, in every file, with no lifetime annotation and nothing new in the ABI.
//!
//! | Signature | Rule | Tied to |
//! |---|---|---|
//! | method with a borrowed receiver (`fun`, `mut fun`) | 2 | the receiver only |
//! | other function, `consuming fun` included | 1, 3 or 4 | every input borrow, or nothing |
//! | function value, a type with input borrows | 1 or 3 | every input borrow |
//! | function value, a type without | [`Rule::FunctionValue`] | the function value itself |
//! | default value's generator | [`Rule::Default`] | its slot and each shared reference earlier argument |

use crate::places;
use prism_mir::{Body, BodyOwner, Callee};
use prism_typed_tree::Type;

/// The kind of signature whose rule applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signature {
    /// A top-level function or extension (`has_receiver`), or a top-level `val`'s initializer,
    /// which is a function without parameters.
    Function { has_receiver: bool },
    /// A function type: a closure, or a call of a function value.
    FunctionValue,
    /// A default value's generator, whose parameters are a shared borrow `&P` of each earlier
    /// argument and, for a reference parameter, the `&mut R?` slot it may fill.
    Default,
}

/// The elision rule a signature follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rule {
    /// Rule 4: no input borrow, so a returned borrow can only be of a top-level value.
    NoInput,
    /// Rule 1: one input borrow.
    Single,
    /// Rule 2: a method with a borrowed receiver, tied to the receiver only.
    Receiver,
    /// Rule 3: several input borrows, tied to every one.
    Multiple,
    /// Rule 4 of a function type: no input borrow, so the result is tied to the function value
    /// itself, what a closure captures (D8).
    FunctionValue,
    /// A default value's generator (D10).
    Default,
}

impl Signature {
    /// The signature a call of `callee` follows: the built-in `clone` is a method with a shared
    /// receiver, and `print` and `println` are plain functions, whose `Unit` result borrows
    /// nothing.
    pub fn of_callee(callee: &Callee) -> Self {
        match callee {
            Callee::Function(_) | Callee::Builtin(_) => Signature::Function {
                has_receiver: callee.has_receiver(),
            },
            Callee::Value(..) => Signature::FunctionValue,
            Callee::Default { .. } => Signature::Default,
        }
    }

    /// The signature `body` follows: a top-level `val`'s initializer is a function without
    /// parameters, a closure a function value.
    pub fn of_body(body: &Body) -> Self {
        match &body.owner {
            BodyOwner::Function(_) | BodyOwner::Value(_) => Signature::Function {
                has_receiver: body.owner.has_receiver(),
            },
            BodyOwner::Closure { .. } => Signature::FunctionValue,
            BodyOwner::Default { .. } => Signature::Default,
        }
    }
}

/// The rule `signature` with `parameters` follows, and the positions of the parameters a
/// returned borrow is tied to, in order.
pub fn tied_inputs(signature: Signature, parameters: &[Type]) -> (Rule, Vec<usize>) {
    let input_borrows = || -> Vec<usize> {
        parameters
            .iter()
            .enumerate()
            .filter(|(_, ty)| places::is_input_borrow(ty))
            .map(|(position, _)| position)
            .collect()
    };
    match signature {
        Signature::Function { has_receiver: true }
            if parameters.first().is_some_and(places::is_input_borrow) =>
        {
            (Rule::Receiver, vec![0])
        }
        Signature::Function { .. } => {
            let tied = input_borrows();
            let rule = match tied.len() {
                0 => Rule::NoInput,
                1 => Rule::Single,
                _ => Rule::Multiple,
            };
            (rule, tied)
        }
        Signature::FunctionValue => {
            let tied = input_borrows();
            let rule = match tied.len() {
                0 => Rule::FunctionValue,
                1 => Rule::Single,
                _ => Rule::Multiple,
            };
            (rule, tied)
        }
        Signature::Default => {
            let tied = parameters
                .iter()
                .enumerate()
                .filter(|(_, ty)| match ty {
                    // The slot.
                    Type::Reference { mutable: true, .. } => true,
                    // An earlier argument `&P` whose `P` is a shared input borrow.
                    Type::Reference {
                        mutable: false,
                        inner,
                    } => places::is_shared_input_borrow(inner),
                    _ => false,
                })
                .map(|(position, _)| position)
                .collect();
            (Rule::Default, tied)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prism_typed_tree::NominalId;

    fn reference(ty: Type, mutable: bool) -> Type {
        Type::Reference {
            mutable,
            inner: Box::new(ty),
        }
    }

    fn shared(ty: Type) -> Type {
        reference(ty, false)
    }

    fn team() -> Type {
        Type::plain_struct(NominalId::new("main", "Team"))
    }

    fn method() -> Signature {
        Signature::Function { has_receiver: true }
    }

    fn function() -> Signature {
        Signature::Function {
            has_receiver: false,
        }
    }

    #[test]
    fn a_method_with_a_borrowed_receiver_is_tied_to_its_receiver() {
        // `fun Team.captain(o: &String)` and `mut fun Team.leader(o: &String)`.
        let parameters = [shared(team()), shared(Type::String)];
        assert_eq!(
            tied_inputs(method(), &parameters),
            (Rule::Receiver, vec![0])
        );
        let parameters = [reference(team(), true), shared(Type::String)];
        assert_eq!(
            tied_inputs(method(), &parameters),
            (Rule::Receiver, vec![0])
        );
    }

    #[test]
    fn a_consuming_method_follows_rules_1_3_and_4() {
        // `consuming fun Team.pick(o: &User)`.
        let user = Type::plain_struct(NominalId::new("main", "User"));
        assert_eq!(
            tied_inputs(method(), &[team(), shared(user)]),
            (Rule::Single, vec![1])
        );
        assert_eq!(tied_inputs(method(), &[team()]), (Rule::NoInput, vec![]));
    }

    #[test]
    fn functions_follow_rules_1_3_and_4() {
        assert_eq!(tied_inputs(function(), &[]), (Rule::NoInput, vec![]));
        // `fun first(a: &String, b: &String)`.
        let parameters = [shared(Type::String), shared(Type::String)];
        assert_eq!(
            tied_inputs(function(), &parameters),
            (Rule::Multiple, vec![0, 1])
        );
        // A function-typed parameter is not an input borrow; a `(&T)?` one is.
        let function_type = Type::Fn {
            params: vec![shared(Type::Int)],
            ret: Box::new(shared(Type::Int)),
            is_compose: false,
        };
        let parameters = [function_type, shared(Type::Int).nullable(), Type::Int];
        assert_eq!(
            tied_inputs(function(), &parameters),
            (Rule::Single, vec![1])
        );
    }

    #[test]
    fn function_values_follow_rules_1_3_and_4() {
        // `(&Int, Int, &Int) -> &Int`.
        let parameters = [shared(Type::Int), Type::Int, shared(Type::Int)];
        assert_eq!(
            tied_inputs(Signature::FunctionValue, &parameters),
            (Rule::Multiple, vec![0, 2])
        );
        // `(Int) -> &Int`: tied to the function value itself.
        assert_eq!(
            tied_inputs(Signature::FunctionValue, &[Type::Int]),
            (Rule::FunctionValue, vec![])
        );
    }

    #[test]
    fn a_default_is_tied_to_its_slot_and_shared_reference_arguments() {
        // `(&String, &&User, &&mut User, &mut String?)`: a by-value argument, a shared and a
        // mutable reference argument, and the slot.
        let user = Type::plain_struct(NominalId::new("main", "User"));
        let parameters = [
            shared(Type::String),
            shared(shared(user.clone())),
            shared(reference(user, true)),
            reference(Type::String.nullable(), true),
        ];
        assert_eq!(
            tied_inputs(Signature::Default, &parameters),
            (Rule::Default, vec![1, 3])
        );
    }
}

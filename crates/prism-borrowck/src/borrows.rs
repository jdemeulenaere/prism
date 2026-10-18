//! The loans of a body (D2 and D3 of `borrowck-exclusivity`): one per borrow its MIR makes,
//! written or implied, and one per capture of each closure value it builds, plus the inputs
//! that stand for what the caller passed to each loan-carrying argument or capture (D4 of
//! `borrowck-elision`).
//!
//! The analyses track loans per loan-carrying local ([`places::carries_loans`]): the items a
//! local may hold are the loans, numbered first, then the inputs. An input serves each role by
//! its local's type: a function input for an escaping closure (a type containing a function
//! type), a referent input for writes through a `&mut R` argument whose `R` carries loans, and
//! the origin of a returned borrow for any argument or capture.

use crate::location::Location;
use crate::places;
use prism_ast::Span;
use prism_mir::{
    BodyId, BorrowOrigin, CaptureMode, LocalId, LocalKind, MirFile, Place, Rvalue, StatementKind,
};
use prism_typed_tree::{define_id, Arena, ArenaId};
use std::collections::HashMap;

define_id!(
    /// A loan of a body.
    LoanId
);

/// What made a loan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoanKind {
    /// An `Rvalue::Borrow`, with its origin.
    Borrow(BorrowOrigin),
    /// Capture `index` of a value of the closure `closure`.
    Capture { closure: BodyId, index: usize },
}

/// One borrow made in a body.
#[derive(Debug, Clone, PartialEq)]
pub struct Loan {
    /// The borrowed place.
    pub place: Place,
    pub mutable: bool,
    /// The statement that makes it.
    pub location: Location,
    /// The span of that statement: the borrow, or the closure.
    pub span: Span,
    pub kind: LoanKind,
    /// Whether the borrowed place is itself a reference (its type is `&T`, `(&T)?`, ...): only
    /// a value whose type can borrow a reference holds the loan
    /// ([`places::can_borrow_reference`]).
    pub of_reference: bool,
}

/// An item a loan-carrying local may hold: a loan, or an input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Item {
    Loan(LoanId),
    /// The input held at entry by the loan-carrying argument or capture local: whatever the
    /// caller passed, so a function input for a local whose type contains a function type, the
    /// caller's referent for an argument through which loans can be written (`&mut R` whose
    /// `R` carries loans), and what a returned borrow derived from the argument borrows.
    Input(LocalId),
}

/// The loans, inputs and loan-carrying locals of one body.
#[derive(Debug, Clone)]
pub struct BorrowSet {
    pub loans: Arena<LoanId, Loan>,
    /// The argument and capture locals whose type carries loans, each holding its own input at
    /// entry (D4 of `borrowck-elision`): so an escaping function-typed parameter is caught, a
    /// write through any copy of a `&mut R` argument reaches the argument, which the caller
    /// sees afterwards, and a returned borrow names the input it derives from.
    pub inputs: Vec<LocalId>,
    /// The locals whose type carries loans, in local order.
    pub carriers: Vec<LocalId>,
    /// The position of each local in `carriers`.
    carrier_index: Vec<Option<usize>>,
    /// The loans each statement makes.
    by_location: HashMap<Location, Vec<LoanId>>,
}

impl BorrowSet {
    /// Gathers the loans of `body`, a body of the elaborated `file` whose closure bodies record
    /// their capture modes.
    pub fn gather(file: &MirFile, body: &prism_mir::Body) -> Self {
        let mut carriers = Vec::new();
        let mut carrier_index = vec![None; body.locals.len()];
        let mut inputs = Vec::new();
        for (local, declaration) in body.locals.iter() {
            if places::carries_loans(&declaration.ty) {
                carrier_index[local.index()] = Some(carriers.len());
                carriers.push(local);
            }
            if matches!(declaration.kind, LocalKind::Argument | LocalKind::Capture)
                && places::carries_loans(&declaration.ty)
            {
                inputs.push(local);
            }
        }
        let is_reference = |place: &Place| places::is_input_borrow(&place.ty(body, &file.types));
        let mut loans = Arena::new();
        let mut by_location: HashMap<Location, Vec<LoanId>> = HashMap::new();
        for (block, data) in body.blocks.iter() {
            for (index, statement) in data.statements.iter().enumerate() {
                let location = Location {
                    block,
                    statement: index,
                };
                let StatementKind::Assign(_, rvalue) = &statement.kind else {
                    continue;
                };
                match rvalue.as_ref() {
                    Rvalue::Borrow {
                        mutable,
                        origin,
                        place,
                    } => {
                        let id = loans.alloc(Loan {
                            place: place.clone(),
                            mutable: *mutable,
                            location,
                            span: statement.span,
                            kind: LoanKind::Borrow(*origin),
                            of_reference: is_reference(place),
                        });
                        by_location.entry(location).or_default().push(id);
                    }
                    Rvalue::Closure {
                        body: closure,
                        captures,
                    } => {
                        let modes = &file.bodies[*closure].capture_modes;
                        for (capture, place) in captures.iter().enumerate() {
                            let mutable = modes.get(capture) == Some(&CaptureMode::Mutable);
                            let id = loans.alloc(Loan {
                                place: place.clone(),
                                mutable,
                                location,
                                span: statement.span,
                                kind: LoanKind::Capture {
                                    closure: *closure,
                                    index: capture,
                                },
                                of_reference: is_reference(place),
                            });
                            by_location.entry(location).or_default().push(id);
                        }
                    }
                    _ => {}
                }
            }
        }
        Self {
            loans,
            inputs,
            carriers,
            carrier_index,
            by_location,
        }
    }

    /// The position of `local` among the loan-carrying locals, if it carries loans.
    pub fn carrier(&self, local: LocalId) -> Option<usize> {
        self.carrier_index.get(local.index()).copied().flatten()
    }

    /// The number of items a local may hold: the loans, then the inputs.
    pub fn item_count(&self) -> usize {
        self.loans.len() + self.inputs.len()
    }

    /// The index of `item` among the items.
    pub fn item_index(&self, item: Item) -> usize {
        match item {
            Item::Loan(loan) => loan.index(),
            Item::Input(local) => {
                self.loans.len()
                    + self
                        .inputs
                        .iter()
                        .position(|input| *input == local)
                        .expect("an input of the body")
            }
        }
    }

    /// The item at `index`.
    pub fn item(&self, index: usize) -> Item {
        if index < self.loans.len() {
            Item::Loan(LoanId(index as u32))
        } else {
            Item::Input(self.inputs[index - self.loans.len()])
        }
    }

    /// The loans the statement at `location` makes, in order.
    pub fn loans_at(&self, location: Location) -> &[LoanId] {
        self.by_location
            .get(&location)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_bodies::{body, elaborate};

    const DECLARATIONS: &str = "\
struct User { var name: String }
struct Profile { var nick: String? }
struct Stack { var count: Int }
mut fun Stack.push(n: Int) {}
fun Stack.size(): Int = count
fun bump(r: &mut Int) {}
fun show(n: (&String)?) {}
fun run(f: () -> Int): Int = f()
";

    /// The loans of `f` in `source`, as `(kind, mutable)`, and the borrow set.
    fn loans(source: &str) -> (Vec<(LoanKind, bool)>, BorrowSet) {
        let file = elaborate(&format!("{}{}", DECLARATIONS, source));
        let set = BorrowSet::gather(&file, body(&file, "f"));
        let loans = set
            .loans
            .iter()
            .map(|(_, loan)| (loan.kind, loan.mutable))
            .collect();
        (loans, set)
    }

    fn borrow(origin: BorrowOrigin, mutable: bool) -> (LoanKind, bool) {
        (LoanKind::Borrow(origin), mutable)
    }

    #[test]
    fn written_and_argument_borrows() {
        let (found, set) = loans("fun f() { var x = 1; val r = &x; bump(&mut x) }\n");
        assert_eq!(
            found,
            vec![
                borrow(BorrowOrigin::Written, false),
                borrow(BorrowOrigin::Written, true)
            ]
        );
        // `r` and the argument's temporary carry loans; `x` does not.
        assert!(set.carrier(LocalId(1)).is_none());
        assert_eq!(set.carriers.len(), 2);
    }

    #[test]
    fn receiver_borrows() {
        let (found, _) = loans("fun f() { var v = Stack(0); v.push(v.size()) }\n");
        assert_eq!(
            found,
            vec![
                borrow(BorrowOrigin::Receiver, false),
                borrow(BorrowOrigin::Receiver, true)
            ]
        );
    }

    #[test]
    fn conversion_borrows() {
        // `show(p.nick)` borrows the payload as an argument; a `(&String)?` view of a written
        // borrow converts it.
        let (found, _) = loans("fun f(p: &Profile) { show(p.nick) }\n");
        assert_eq!(found, vec![borrow(BorrowOrigin::Argument, false)]);
        let (found, _) = loans("fun f(p: Profile) { val a: (&String)? = &p.nick; show(a) }\n");
        assert_eq!(
            found,
            vec![
                borrow(BorrowOrigin::Written, false),
                borrow(BorrowOrigin::Conversion, false)
            ]
        );
    }

    #[test]
    fn closure_captures_and_function_inputs() {
        let (found, set) =
            loans("fun f(g: () -> Int) { var u = User(\"a\"); run { u.name = \"z\"; 1 } }\n");
        let closure = match found[0].0 {
            LoanKind::Capture { closure, index: 0 } => closure,
            other => panic!("not a capture: {:?}", other),
        };
        assert_eq!(found, vec![(LoanKind::Capture { closure, index: 0 }, true)]);
        assert_eq!(set.inputs, vec![LocalId(1)]);
        assert_eq!(set.item_count(), 2);
        assert_eq!(set.item(1), Item::Input(LocalId(1)));
        assert_eq!(set.item_index(Item::Input(LocalId(1))), 1);
    }

    #[test]
    fn every_loan_carrying_argument_holds_an_input() {
        let (_, set) = loans("fun f(a: &Int, s: &mut &Int, g: () -> Int, n: Int) {}\n");
        assert_eq!(set.inputs, vec![LocalId(1), LocalId(2), LocalId(3)]);
    }
}

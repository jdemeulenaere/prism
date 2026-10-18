//! Loan flow (D4 and D9 of `borrowck-exclusivity`): a forward analysis of which loans, and
//! function inputs, each loan-carrying local may hold at each point.
//!
//! Its bits are pairs of a loan-carrying local and an item ([`Item`]). A borrow puts its loan
//! in its destination, with every loan the borrowed place's root local holds (a reborrow, a
//! field through a reference and a borrow of a reference keep the source's loans); a copy, a
//! move, a wrap or a subtyping conversion passes on the loans of its operand's root local; a
//! closure value holds its captures' loans and the loans of the captured locals, and a variable
//! it captures mutably receives every loan the closure holds. A whole-local assignment first
//! removes what the local held (re-pointing) and kills every loan of a place through a
//! dereference of it; a write through a dereference gives the written value's loans to the
//! reference and to the root locals of the loans it holds. A call's result, when its type
//! contains a reference, holds the loans of the call's input borrows, of a function-value
//! callee, but not the shared borrows a default value's generator takes of earlier by-value
//! arguments; and the locals a call may write through a `&mut` argument or a closure receive
//! the input borrows' loans. A move of a whole local, and the start and end of its storage,
//! empty it. A local never holds a loan of a place rooted at itself, since a reference cannot
//! point into itself.

use crate::borrows::{BorrowSet, Item};
use crate::dataflow::{Analysis, BitSet};
use crate::location::Location;
use crate::places;
use prism_mir::{
    Body, Callee, LocalId, MirFile, Operand, Place, Projection, Rvalue, Statement, StatementKind,
    Terminator, TerminatorKind,
};
use prism_typed_tree::{ArenaId, Type};

/// Which items each loan-carrying local may hold.
pub struct LoanFlow<'a> {
    pub file: &'a MirFile,
    pub body: &'a Body,
    pub borrows: &'a BorrowSet,
}

impl<'a> LoanFlow<'a> {
    pub fn new(file: &'a MirFile, body: &'a Body, borrows: &'a BorrowSet) -> Self {
        Self {
            file,
            body,
            borrows,
        }
    }

    fn bit(&self, carrier: usize, item: usize) -> usize {
        carrier * self.borrows.item_count() + item
    }

    /// The indices of the items `local` may hold in `state`.
    fn held(&self, state: &BitSet, local: LocalId) -> Vec<usize> {
        let Some(carrier) = self.borrows.carrier(local) else {
            return Vec::new();
        };
        (0..self.borrows.item_count())
            .filter(|item| state.contains(self.bit(carrier, *item)))
            .collect()
    }

    /// The items `local` may hold in `state`.
    pub fn holds(&self, state: &BitSet, local: LocalId) -> Vec<Item> {
        self.held(state, local)
            .into_iter()
            .map(|item| self.borrows.item(item))
            .collect()
    }

    /// Whether `local` may hold `item` in `state`.
    pub fn holds_item(&self, state: &BitSet, local: LocalId, item: Item) -> bool {
        self.borrows
            .carrier(local)
            .is_some_and(|carrier| state.contains(self.bit(carrier, self.borrows.item_index(item))))
    }

    /// Adds `items` to what `local` holds, except loans of places rooted at `local`.
    fn give(&self, state: &mut BitSet, local: LocalId, items: &[usize]) {
        let Some(carrier) = self.borrows.carrier(local) else {
            return;
        };
        for &item in items {
            if let Item::Loan(loan) = self.borrows.item(item) {
                if self.borrows.loans[loan].place.local == local {
                    continue;
                }
            }
            state.insert(self.bit(carrier, item));
        }
    }

    /// Empties `local`.
    fn clear(&self, state: &mut BitSet, local: LocalId) {
        if let Some(carrier) = self.borrows.carrier(local) {
            for item in 0..self.borrows.item_count() {
                state.remove(self.bit(carrier, item));
            }
        }
    }

    /// Removes from every local the loans of places through a dereference of `local`, which no
    /// longer name what they borrowed once `local` is re-pointed.
    fn kill_through_dereference(&self, state: &mut BitSet, local: LocalId) {
        for (loan, data) in self.borrows.loans.iter() {
            if data.place.local == local
                && data.place.projections.first() == Some(&Projection::Deref)
            {
                let item = loan.index();
                for carrier in 0..self.borrows.carriers.len() {
                    state.remove(self.bit(carrier, item));
                }
            }
        }
    }

    /// The items the value of `operand` carries: those of its place's root local.
    fn operand_items(&self, state: &BitSet, operand: &Operand) -> Vec<usize> {
        match operand.place() {
            Some(place) => self.held(state, place.local),
            None => Vec::new(),
        }
    }

    /// The items the value of `rvalue`, assigned at `location`, carries.
    pub fn rvalue_items(&self, state: &BitSet, rvalue: &Rvalue, location: Location) -> Vec<usize> {
        let mut items = match rvalue {
            Rvalue::Use(operand) | Rvalue::WrapNullable(operand) | Rvalue::Subtype(operand) => {
                self.operand_items(state, operand)
            }
            Rvalue::Borrow { place, .. } => self.held(state, place.local),
            Rvalue::Closure { captures, .. } => captures
                .iter()
                .flat_map(|place| self.held(state, place.local))
                .collect(),
            Rvalue::Binary(..)
            | Rvalue::Unary(..)
            | Rvalue::Discriminant(_)
            | Rvalue::Aggregate(..) => Vec::new(),
        };
        if matches!(rvalue, Rvalue::Borrow { .. } | Rvalue::Closure { .. }) {
            items.extend(
                self.borrows
                    .loans_at(location)
                    .iter()
                    .map(|loan| loan.index()),
            );
        }
        items.sort_unstable();
        items.dedup();
        items
    }

    /// The whole locals that `operands` move, which hold nothing afterwards. A move of a `Copy`
    /// value is a copy.
    fn moved_locals<'o>(&self, operands: impl Iterator<Item = &'o Operand>) -> Vec<LocalId> {
        operands
            .filter_map(|operand| match operand {
                Operand::Move(place, _) => place.as_local(),
                _ => None,
            })
            .filter(|local| !self.file.types.is_copy(&self.body.locals[*local].ty))
            .collect()
    }

    /// Writes a value carrying `items` to `destination`.
    fn write(&self, state: &mut BitSet, destination: &Place, items: &[usize]) {
        let root = destination.local;
        if destination.projections.is_empty() {
            if self.borrows.carrier(root).is_some() {
                self.clear(state, root);
                if matches!(self.body.locals[root].ty, Type::Reference { .. }) {
                    self.kill_through_dereference(state, root);
                }
                self.give(state, root, items);
            }
        } else if destination.projections.contains(&Projection::Deref) {
            if items.is_empty() {
                return;
            }
            // `*r = v`: what `r` points at now holds `v`'s loans.
            let mut receivers = vec![root];
            for item in self.held(state, root) {
                if let Item::Loan(loan) = self.borrows.item(item) {
                    receivers.push(self.borrows.loans[loan].place.local);
                }
            }
            for receiver in receivers {
                self.give(state, receiver, items);
            }
        } else {
            self.give(state, root, items);
        }
    }

    /// The items of the call's input borrows (D9): the arguments whose parameter type contains
    /// a reference, except the shared borrows of by-value earlier arguments a default value's
    /// generator takes; and a function-value callee's items.
    pub fn call_items(
        &self,
        state: &BitSet,
        callee: &Callee,
        arguments: &[Operand],
        parameters: &[Type],
    ) -> Vec<usize> {
        let mut items = Vec::new();
        for (argument, parameter) in arguments.iter().zip(parameters) {
            if !places::is_input_borrow(parameter) {
                continue;
            }
            if let (
                Callee::Default { .. },
                Type::Reference {
                    mutable: false,
                    inner,
                },
            ) = (callee, parameter)
            {
                if !places::carries_loans(inner) {
                    continue;
                }
            }
            items.extend(self.operand_items(state, argument));
        }
        if let Callee::Value(place, _) = callee {
            items.extend(self.held(state, place.local));
        }
        items.sort_unstable();
        items.dedup();
        items
    }

    /// The loan-carrying locals a call may write: the roots of the mutable loans held by a
    /// `&mut R` argument whose `R` carries loans, or by a function-value callee.
    fn call_writes(
        &self,
        state: &BitSet,
        callee: &Callee,
        arguments: &[Operand],
        parameters: &[Type],
    ) -> Vec<LocalId> {
        let mut held = Vec::new();
        for (argument, parameter) in arguments.iter().zip(parameters) {
            if let Type::Reference {
                mutable: true,
                inner,
            } = parameter
            {
                if places::carries_loans(inner) {
                    held.extend(self.operand_items(state, argument));
                }
            }
        }
        if let Callee::Value(place, _) = callee {
            held.extend(self.held(state, place.local));
        }
        let mut writes: Vec<LocalId> = held
            .into_iter()
            .filter_map(|item| match self.borrows.item(item) {
                Item::Loan(loan) if self.borrows.loans[loan].mutable => {
                    Some(self.borrows.loans[loan].place.local)
                }
                _ => None,
            })
            .filter(|local| self.borrows.carrier(*local).is_some())
            .collect();
        writes.sort();
        writes.dedup();
        writes
    }
}

/// The operands an rvalue reads.
fn rvalue_operands(rvalue: &Rvalue) -> Vec<&Operand> {
    match rvalue {
        Rvalue::Use(operand)
        | Rvalue::Unary(_, operand)
        | Rvalue::WrapNullable(operand)
        | Rvalue::Subtype(operand) => vec![operand],
        Rvalue::Binary(_, left, right) => vec![left, right],
        Rvalue::Aggregate(_, operands) => operands.iter().collect(),
        Rvalue::Borrow { .. } | Rvalue::Discriminant(_) | Rvalue::Closure { .. } => Vec::new(),
    }
}

impl Analysis for LoanFlow<'_> {
    fn domain_size(&self) -> usize {
        self.borrows.carriers.len() * self.borrows.item_count()
    }

    fn initialize_boundary(&self, state: &mut BitSet) {
        for input in &self.borrows.inputs {
            let item = self.borrows.item_index(Item::Input(*input));
            self.give(state, *input, &[item]);
        }
    }

    fn apply_statement(&self, state: &mut BitSet, statement: &Statement, location: Location) {
        match &statement.kind {
            StatementKind::Assign(destination, rvalue) => {
                let items = self.rvalue_items(state, rvalue, location);
                for local in self.moved_locals(rvalue_operands(rvalue).into_iter()) {
                    self.clear(state, local);
                }
                self.write(state, destination, &items);
                if let Rvalue::Closure {
                    body: closure,
                    captures,
                } = rvalue.as_ref()
                {
                    // Calling the closure may write any loan it holds into a variable it
                    // captures mutably.
                    let modes = &self.file.bodies[*closure].capture_modes;
                    for (index, place) in captures.iter().enumerate() {
                        if modes.get(index) == Some(&prism_mir::CaptureMode::Mutable) {
                            self.give(state, place.local, &items);
                        }
                    }
                }
            }
            StatementKind::StorageLive(local) | StatementKind::StorageDead(local) => {
                self.clear(state, *local)
            }
            StatementKind::Read(_) => {}
        }
    }

    fn apply_terminator(&self, state: &mut BitSet, terminator: &Terminator, _: Location) {
        if let TerminatorKind::Call {
            callee,
            arguments,
            parameters,
            destination,
            ..
        } = &terminator.kind
        {
            let items = self.call_items(state, callee, arguments, parameters);
            for local in self.call_writes(state, callee, arguments, parameters) {
                self.give(state, local, &items);
            }
            for local in self.moved_locals(arguments.iter()) {
                self.clear(state, local);
            }
            let ty = destination.ty(self.body, &self.file.types);
            let result = if places::is_input_borrow(&ty) {
                items
            } else {
                Vec::new()
            };
            self.write(state, destination, &result);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataflow::solve;
    use crate::location::locations;
    use crate::test_bodies::{body, elaborate};

    /// What the local named `name` of `f` in `source` may hold just before the first location
    /// whose span starts at the first occurrence of `at` in the line of `f`, each item named by
    /// its place (`mut ` for a mutable loan, `_` for a temporary, `input g` for a function
    /// input).
    fn held_before(source: &str, name: &str, at: &str) -> Vec<String> {
        let file = elaborate(source);
        let f = body(&file, "f");
        let borrows = BorrowSet::gather(&file, f);
        let flow = solve(f, LoanFlow::new(&file, f, &borrows));
        let (line, text) = source
            .lines()
            .enumerate()
            .find(|(_, line)| line.starts_with("fun f("))
            .expect("a function f");
        let column = text.find(at).expect("the marker") as u32 + 1;
        let location = locations(f)
            .into_iter()
            .find(|location| {
                let data = &f.blocks[location.block];
                let span = match data.statements.get(location.statement) {
                    Some(statement) => {
                        if matches!(statement.kind, StatementKind::StorageLive(_)) {
                            return false;
                        }
                        statement.span
                    }
                    None => data.terminator.span,
                };
                span.line == line as u32 + 1 && span.column == column
            })
            .unwrap_or_else(|| panic!("no location at {}:{}\n{}", line + 1, column, file.dump()));
        let local = f
            .locals
            .iter()
            .find(|(_, l)| l.name.as_deref() == Some(name))
            .map(|(id, _)| id)
            .unwrap_or_else(|| panic!("no local {}", name));
        let mut cursor = flow.cursor(f);
        let state = cursor.seek(location).clone();
        let mut names: Vec<String> = flow
            .analysis
            .holds(&state, local)
            .into_iter()
            .map(|item| match item {
                Item::Loan(loan) => {
                    let loan = &borrows.loans[loan];
                    let name = places::place_name(f, &file.types, &loan.place)
                        .unwrap_or_else(|| "_".to_string());
                    if loan.mutable {
                        format!("mut {}", name)
                    } else {
                        name
                    }
                }
                Item::Input(local) => {
                    format!("input {}", f.locals[local].name.clone().unwrap_or_default())
                }
            })
            .collect();
        names.sort();
        names
    }

    const DECLARATIONS: &str = "\
struct User { var name: String }
struct Profile { var nick: String? }
struct Team { var lead: String }
fun Team.captain(): &String = &lead
fun print(n: &Int) {}
fun show(n: (&String)?) {}
fun take(s: String) {}
fun greet(s: &String): Int = 1
fun first(a: &String, b: &String): &String = a
fun pick(a: &mut Int): &Int = a
fun run(f: () -> Int): Int = f()
";

    fn held(body: &str, name: &str, at: &str) -> Vec<String> {
        held_before(&format!("{}{}", DECLARATIONS, body), name, at)
    }

    #[test]
    fn a_copied_reference_holds_the_loan() {
        assert_eq!(
            held(
                "fun f() { var x = 1; val r = &x; val r2 = r; x = 2; print(r2) }\n",
                "r2",
                "x = 2"
            ),
            vec!["x"]
        );
    }

    #[test]
    fn re_pointing_replaces_the_loans() {
        assert_eq!(
            held(
                "fun f() { var a = 1; var b = 2; var r: &Int = &a; r = &b; a = 3; print(r) }\n",
                "r",
                "a = 3"
            ),
            vec!["b"]
        );
    }

    #[test]
    fn a_reborrow_holds_its_source_loans() {
        assert_eq!(
            held(
                "fun f() { var u = User(\"a\"); val m = &mut u; val n = &mut m.name; u.name = \"b\"; *n = \"c\" }\n",
                "n",
                "u.name = "
            ),
            vec!["mut m.name", "mut u"]
        );
    }

    #[test]
    fn a_reference_of_a_reference_holds_the_inner_loans() {
        assert_eq!(
            held(
                "fun f() { var x = 1; var r = &x; val rr = &r; x = 2; print(rr) }\n",
                "rr",
                "x = 2"
            ),
            vec!["r", "x"]
        );
    }

    #[test]
    fn writing_through_a_reference_gives_the_referent_the_loans() {
        assert_eq!(
            held(
                "fun f() { var a = 1; var b = 2; var r: &Int = &b; val rr = &mut r; *rr = &a; a = 3; print(r) }\n",
                "r",
                "a = 3"
            ),
            vec!["a", "b"]
        );
    }

    #[test]
    fn an_optional_borrow_holds_the_loan() {
        assert_eq!(
            held(
                "fun f() { var p = Profile(null); val a: (&String)? = &p.nick; p.nick = null; show(a) }\n",
                "a",
                "p.nick = null"
            ),
            vec!["_", "p.nick"]
        );
    }

    #[test]
    fn a_returned_borrow_holds_every_input() {
        assert_eq!(
            held(
                "fun f(x: String, y: String) { val r = first(&x, &y); take(y); greet(r) }\n",
                "r",
                "y); greet"
            ),
            vec!["x", "y"]
        );
        assert_eq!(
            held(
                "fun f() { var x = 1; val r = pick(&mut x); print(x); print(r) }\n",
                "r",
                "x); print(r)"
            ),
            vec!["mut x"]
        );
        assert_eq!(
            held(
                "fun f() { var t = Team(\"a\"); val c = t.captain(); t.lead = \"b\"; greet(c) }\n",
                "c",
                "t.lead"
            ),
            vec!["t"]
        );
    }

    #[test]
    fn a_default_generator_does_not_keep_by_value_arguments_borrowed() {
        let source = "\
fun greet(a: String, n: &String = \"anon\"): Int = 1
fun f(s: String): Int = greet(s)
";
        let file = elaborate(source);
        let f = body(&file, "f");
        let borrows = BorrowSet::gather(&file, f);
        let flow = solve(f, LoanFlow::new(&file, f, &borrows));
        // The call of `greet`, whose second argument is the generator's result.
        let call = locations(f)
            .into_iter()
            .find(|location| {
                location.is_terminator(f)
                    && matches!(
                        &f.blocks[location.block].terminator.kind,
                        TerminatorKind::Call { callee: Callee::Function(path), .. }
                            if path.to_string() == "main::greet"
                    )
            })
            .expect("the call of greet");
        let TerminatorKind::Call { arguments, .. } = &f.blocks[call.block].terminator.kind else {
            unreachable!()
        };
        let result = arguments[1].place().expect("a temporary").local;
        let mut cursor = flow.cursor(f);
        let held = flow.analysis.holds(cursor.seek(call), result);
        assert_eq!(held.len(), 1, "{:?}", held);
        let Item::Loan(loan) = held[0] else {
            panic!("not a loan")
        };
        assert!(borrows.loans[loan].mutable, "the slot's loan");
    }

    #[test]
    fn a_function_typed_argument_holds_its_input() {
        assert_eq!(
            held("fun f(g: () -> Int): Int = run(g)\n", "g", "g)"),
            vec!["input g"]
        );
    }
}

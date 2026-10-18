//! Loan flow (D4 and D9 of `borrowck-exclusivity`): a forward analysis of which loans, and
//! function inputs, each loan-carrying local may hold at each point.
//!
//! Its states are sets of pairs of a loan-carrying local and an item ([`Item`]), kept sparse
//! ([`Holdings`]). A borrow puts its loan in its destination, with every loan the borrowed
//! place's root local holds (a reborrow, a field through a reference and a borrow of a
//! reference keep the source's loans); a copy, a
//! move, a wrap or a subtyping conversion passes on the loans of its operand's root local; a
//! closure value holds its captures' loans and the loans of the captured locals, and a variable
//! it captures mutably, and what that variable points at, receive the loans of its other
//! captures. A whole-local assignment first
//! removes what the local held (re-pointing) and kills every loan of a place through a
//! dereference of it; a write through a dereference gives the written value's loans to the
//! reference and to the root locals of the loans it holds. A call's result, when its type
//! contains a reference, holds the items of the operands its elision rule ties it to
//! ([`crate::elision`]): the receiver of a method with a borrowed receiver, every input borrow
//! of another function, every input-borrow argument of a function value or, when its type has
//! none, the function value's items, and a default value's slot and shared reference earlier
//! arguments, without the generator's own borrows of their temporaries; and what a call may write through (what a `&mut R` argument, a function-typed
//! argument or a function-value callee points at) receives the loans of the call's other
//! operands. An argument through which loans can be written holds its own input at entry, so
//! that a write through a copy of it reaches it. A move of a whole local, and the start and end of its storage,
//! empty it. A local never holds a loan of a place rooted at itself, since a reference cannot
//! point into itself, and a written value holds a loan of a reference itself only when its type
//! can refer to a reference or is a function type ([`places::can_borrow_reference`]): a `&Int`
//! read through a `&&Int`, or returned by a closure that captures a reference, holds what that
//! reference holds, not the borrow of the reference.

use crate::borrows::{BorrowSet, Item, LoanKind};
use crate::dataflow::{Analysis, Domain, PairSet, Results};
use crate::elision::{self, Rule, Signature};
use crate::liveness::Liveness;
use crate::location::Location;
use crate::places;
use prism_mir::{
    Body, BorrowOrigin, Callee, Capture, LocalId, MirFile, Operand, Place, Projection, Rvalue,
    Statement, StatementKind, Terminator, TerminatorKind,
};
use prism_typed_tree::{ArenaId, Type};

/// The state of the loan flow: the items each loan-carrying local may hold, as pairs of a
/// carrier (its position among [`BorrowSet::carriers`]) and an item, sorted, and the same pairs
/// by item, so that both what a carrier holds and who holds an item are found by binary search.
///
/// A state's size is the number of pairs it holds, not the number of carriers times the number
/// of items: most locals hold nothing or one loan, and a local's items are dropped once it is
/// dead ([`LoanFlow::pruned`]), so a body with many borrows has small states.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Holdings {
    /// `(carrier, item)`.
    pairs: PairSet,
    /// `(item, carrier)` for each pair.
    by_item: PairSet,
}

impl Holdings {
    /// The number of pairs.
    pub fn len(&self) -> usize {
        self.pairs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pairs.is_empty()
    }

    /// The items `carrier` holds, in increasing order.
    pub fn items(&self, carrier: usize) -> impl Iterator<Item = usize> + '_ {
        self.pairs
            .with_first(carrier as u32)
            .map(|(_, item)| item as usize)
    }

    /// Whether `carrier` holds `item`.
    pub fn contains(&self, carrier: usize, item: usize) -> bool {
        self.pairs.contains((carrier as u32, item as u32))
    }

    /// The carriers that hold `item`, in increasing order.
    pub fn holders(&self, item: usize) -> impl Iterator<Item = usize> + '_ {
        self.by_item
            .with_first(item as u32)
            .map(|(_, carrier)| carrier as usize)
    }

    /// Every pair as `(item, carrier)`, by item, then carrier.
    pub fn by_item(&self) -> impl Iterator<Item = (usize, usize)> + '_ {
        self.by_item
            .iter()
            .map(|(item, carrier)| (item as usize, carrier as usize))
    }

    /// Adds `items` to what `carrier` holds.
    fn insert(&mut self, carrier: usize, items: impl IntoIterator<Item = usize>) {
        for item in items {
            if self.pairs.insert((carrier as u32, item as u32)) {
                self.by_item.insert((item as u32, carrier as u32));
            }
        }
    }

    /// Removes everything `carrier` holds.
    fn clear(&mut self, carrier: usize) {
        let held: Vec<(u32, u32)> = self.pairs.with_first(carrier as u32).collect();
        for (carrier, item) in held {
            self.pairs.remove((carrier, item));
            self.by_item.remove((item, carrier));
        }
    }

    /// Removes `items`, sorted, from every carrier.
    fn remove_items(&mut self, items: &[usize]) {
        let kept = |item: u32| items.binary_search(&(item as usize)).is_err();
        self.pairs.retain(|(_, item)| kept(*item));
        self.by_item.retain(|(item, _)| kept(*item));
    }
}

impl Domain for Holdings {
    fn join(&mut self, other: &Self) -> bool {
        if !self.pairs.union(&other.pairs) {
            return false;
        }
        self.by_item.union(&other.by_item);
        true
    }
}

/// Which items each loan-carrying local may hold.
pub struct LoanFlow<'a> {
    pub file: &'a MirFile,
    pub body: &'a Body,
    pub borrows: &'a BorrowSet,
    /// The items of the loans of places through a dereference of each local, which no longer
    /// name what they borrowed once the local is re-pointed, sorted.
    through_dereference: Vec<Vec<usize>>,
    /// When pruned, the carriers that each location, per block, uses or defines last: dead
    /// after it, they hold nothing afterwards.
    dying: Option<Vec<Vec<Vec<u32>>>>,
}

impl<'a> LoanFlow<'a> {
    /// The loan flow of `body`, whose loans are `borrows`, which drops what a carrier holds
    /// once `liveness` finds it dead after a location that uses or defines it.
    ///
    /// Every query of a state asks what a local holds where it is used, or which locals in
    /// scope there hold a loan, and a dead local is used again only after a definition, which
    /// replaces what it held: so the answers are those of the full flow, while a state holds
    /// only the items of live locals, and a body whose locals each hold a loan but die one
    /// after the other keeps its states small. The one exception is the diagnostic of an
    /// escape, which names an owned capture by what its operand holds where the closure
    /// escapes, perhaps after the operand is dead: its errors are rebuilt from the full flow
    /// ([`crate::check_file`]).
    pub fn pruned(
        file: &'a MirFile,
        body: &'a Body,
        borrows: &'a BorrowSet,
        liveness: &Results<Liveness>,
    ) -> Self {
        let mut flow = Self::new(file, body, borrows);
        let mut dying = Vec::with_capacity(body.blocks.len());
        for (block, data) in body.blocks.iter() {
            // Backward through the block, from the state after its terminator.
            let mut live = liveness.block_state(block, body);
            let mut block_dying = vec![Vec::new(); data.statements.len() + 1];
            for index in (0..=data.statements.len()).rev() {
                let location = Location {
                    block,
                    statement: index,
                };
                let effects = liveness.analysis.carrier_effects(location);
                if !effects.is_empty() {
                    let mut touched: Vec<u32> = effects
                        .iter()
                        .map(|(carrier, _)| *carrier)
                        .filter(|carrier| !live.contains(*carrier as usize))
                        .collect();
                    touched.sort_unstable();
                    touched.dedup();
                    block_dying[index] = touched;
                }
                match data.statements.get(index) {
                    Some(statement) => liveness
                        .analysis
                        .apply_statement(&mut live, statement, location),
                    None => {
                        liveness
                            .analysis
                            .apply_terminator(&mut live, &data.terminator, location)
                    }
                }
            }
            dying.push(block_dying);
        }
        flow.dying = Some(dying);
        flow
    }

    /// Drops what the locals dead after `location` hold.
    fn prune(&self, state: &mut Holdings, location: Location) {
        if let Some(dying) = &self.dying {
            for carrier in &dying[location.block.index()][location.statement] {
                state.clear(*carrier as usize);
            }
        }
    }

    /// The full loan flow of `body`, whose loans are `borrows`.
    pub fn new(file: &'a MirFile, body: &'a Body, borrows: &'a BorrowSet) -> Self {
        let through_dereference = body
            .locals
            .iter()
            .map(|(local, _)| {
                borrows
                    .loans_of(local)
                    .iter()
                    .filter(|loan| {
                        borrows.loans[**loan].place.projections.first() == Some(&Projection::Deref)
                    })
                    .map(|loan| loan.index())
                    .collect()
            })
            .collect();
        Self {
            file,
            body,
            borrows,
            through_dereference,
            dying: None,
        }
    }

    /// The indices of the items `local` may hold in `state`.
    fn held(&self, state: &Holdings, local: LocalId) -> Vec<usize> {
        match self.borrows.carrier(local) {
            Some(carrier) => state.items(carrier).collect(),
            None => Vec::new(),
        }
    }

    /// The items `local` may hold in `state`.
    pub fn holds(&self, state: &Holdings, local: LocalId) -> Vec<Item> {
        self.held(state, local)
            .into_iter()
            .map(|item| self.borrows.item(item))
            .collect()
    }

    /// Whether `local` may hold `item` in `state`.
    pub fn holds_item(&self, state: &Holdings, local: LocalId, item: Item) -> bool {
        self.borrows
            .carrier(local)
            .is_some_and(|carrier| state.contains(carrier, self.borrows.item_index(item)))
    }

    /// Adds `items` to what `local` holds, except loans of places rooted at `local`.
    fn give(&self, state: &mut Holdings, local: LocalId, items: &[usize]) {
        let Some(carrier) = self.borrows.carrier(local) else {
            return;
        };
        let given = items
            .iter()
            .copied()
            .filter(|item| match self.borrows.item(*item) {
                Item::Loan(loan) => self.borrows.loans[loan].place.local != local,
                Item::Input(_) => true,
            });
        state.insert(carrier, given);
    }

    /// Empties `local`.
    fn clear(&self, state: &mut Holdings, local: LocalId) {
        if let Some(carrier) = self.borrows.carrier(local) {
            state.clear(carrier);
        }
    }

    /// `items` without the loans of references that a value of type `ty` cannot hold: a value
    /// that cannot borrow a reference ([`places::can_borrow_reference`]) points past a
    /// borrowed reference to what it refers to, whose loans the reference's root local holds.
    pub fn fitting(&self, mut items: Vec<usize>, ty: &Type) -> Vec<usize> {
        if !places::can_borrow_reference(ty) {
            items.retain(|item| match self.borrows.item(*item) {
                Item::Loan(loan) => !self.borrows.loans[loan].of_reference,
                Item::Input(_) => true,
            });
        }
        items
    }

    /// The type of the values a closure that captures `place` mutably may write to it: its own
    /// type for a `var`, which the closure may re-point, and what it refers to for a `val`
    /// reference, which it can only write through.
    fn written_type(&self, place: &Place) -> Type {
        let ty = place.ty(self.body, &self.file.types);
        if self.body.locals[place.local].mutable || !place.projections.is_empty() {
            return ty;
        }
        referent(&ty).unwrap_or(ty)
    }

    /// Removes from every local the loans of places through a dereference of `local`, which no
    /// longer name what they borrowed once `local` is re-pointed.
    fn kill_through_dereference(&self, state: &mut Holdings, local: LocalId) {
        let items = &self.through_dereference[local.index()];
        if !items.is_empty() {
            state.remove_items(items);
        }
    }

    /// What building the closure `closure` from `captures` at `location` may later write
    /// (D4): calling it may write, into each variable it captures mutably and into what that
    /// variable points at, the items of its other captures.
    fn closure_writes(
        &self,
        state: &Holdings,
        location: Location,
        closure: prism_mir::BodyId,
        captures: &[Capture],
    ) -> Vec<(LocalId, Vec<usize>)> {
        // The loans are those of the borrowed captures, in capture order; an owned capture
        // carries the items of its operand (D6 of `escaping-closures`).
        let mut loans = self.borrows.loans_at(location).iter();
        let items: Vec<Vec<usize>> = captures
            .iter()
            .map(|capture| {
                let mut items = self.capture_items(state, capture);
                if let Capture::Borrowed(_) = capture {
                    if let Some(loan) = loans.next() {
                        items.push(loan.index());
                    }
                }
                items
            })
            .collect();
        let modes = &self.file.bodies[closure].capture_modes;
        let mut writes = Vec::new();
        for (index, capture) in captures.iter().enumerate() {
            if modes.get(index) != Some(&prism_mir::CaptureMode::Mutable) {
                continue;
            }
            let Capture::Borrowed(place) = capture else {
                continue;
            };
            let mut others: Vec<usize> = items
                .iter()
                .enumerate()
                .filter(|(other, _)| *other != index)
                .flat_map(|(_, items)| items.iter().copied())
                .collect();
            others.sort_unstable();
            others.dedup();
            let others = self.fitting(others, &self.written_type(place));
            let mut receivers = vec![place.local];
            receivers.extend(self.referents(&self.held(state, place.local), true));
            for receiver in receivers {
                writes.push((receiver, others.clone()));
            }
        }
        writes
    }

    /// The items a closure value carries for `capture`: those of the captured place's root
    /// local, except for an owned value whose type carries no loans, such as an `Int` copied
    /// through a reference (D6 of `escaping-closures`).
    pub fn capture_items(&self, state: &Holdings, capture: &Capture) -> Vec<usize> {
        let Some(place) = capture.place() else {
            return Vec::new();
        };
        if capture.is_owned() && !places::carries_loans(&place.ty(self.body, &self.file.types)) {
            return Vec::new();
        }
        self.held(state, place.local)
    }

    /// The items the value of `operand` carries: those of its place's root local.
    fn operand_items(&self, state: &Holdings, operand: &Operand) -> Vec<usize> {
        match operand.place() {
            Some(place) => self.held(state, place.local),
            None => Vec::new(),
        }
    }

    /// The items the value of `rvalue`, assigned at `location`, carries.
    pub fn rvalue_items(
        &self,
        state: &Holdings,
        rvalue: &Rvalue,
        location: Location,
    ) -> Vec<usize> {
        let mut items = match rvalue {
            Rvalue::Use(operand) | Rvalue::WrapNullable(operand) | Rvalue::Subtype(operand) => {
                self.operand_items(state, operand)
            }
            Rvalue::Borrow { place, .. } => self.held(state, place.local),
            Rvalue::Closure { captures, .. } => captures
                .iter()
                .flat_map(|capture| self.capture_items(state, capture))
                .collect(),
            // A template's result is a new string, holding no borrow of its parts.
            Rvalue::Binary(..)
            | Rvalue::Unary(..)
            | Rvalue::Discriminant(_)
            | Rvalue::Aggregate(..)
            | Rvalue::Template(_) => Vec::new(),
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
    fn write(&self, state: &mut Holdings, destination: &Place, items: &[usize]) {
        let items = &self.fitting(items.to_vec(), &destination.ty(self.body, &self.file.types));
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
            receivers.extend(self.referents(&self.held(state, root), false));
            for receiver in receivers {
                self.give(state, receiver, items);
            }
        } else {
            self.give(state, root, items);
        }
    }

    /// The items a call's result holds when its type contains a reference (D2 of
    /// `borrowck-elision`): those of the operands its elision rule ties it to
    /// ([`elision::tied_inputs`]), and, for a function value whose type has no input borrow,
    /// the function value's own items. A default value's result does not hold the generator's
    /// shared borrows of the argument temporaries, only what they borrow (D10).
    pub fn call_items(
        &self,
        state: &Holdings,
        callee: &Callee,
        arguments: &[Operand],
        parameters: &[Type],
    ) -> Vec<usize> {
        let (rule, tied) = elision::tied_inputs(Signature::of_callee(callee), parameters);
        let mut items = Vec::new();
        for position in tied {
            if let Some(argument) = arguments.get(position) {
                items.extend(self.operand_items(state, argument));
            }
        }
        if let (Rule::FunctionValue, Callee::Value(place, _)) = (rule, callee) {
            items.extend(self.held(state, place.local));
        }
        if rule == Rule::Default {
            items.retain(|item| match self.borrows.item(*item) {
                Item::Loan(loan) => {
                    let loan = &self.borrows.loans[loan];
                    loan.mutable || loan.kind != LoanKind::Borrow(BorrowOrigin::Default)
                }
                Item::Input(_) => true,
            });
        }
        items.sort_unstable();
        items.dedup();
        items
    }

    /// The loan-carrying locals that a value holding `items` points at, which a write through
    /// it may give loans to: the root locals of its loans (only of its mutable ones when
    /// `mutable_only`), and the arguments whose referent input it holds.
    fn referents(&self, items: &[usize], mutable_only: bool) -> Vec<LocalId> {
        let mut referents: Vec<LocalId> = items
            .iter()
            .filter_map(|item| match self.borrows.item(*item) {
                Item::Loan(loan) => {
                    let loan = &self.borrows.loans[loan];
                    (loan.mutable || !mutable_only).then_some(loan.place.local)
                }
                Item::Input(local) => {
                    places::writes_loans_through(&self.body.locals[local].ty).then_some(local)
                }
            })
            .filter(|local| self.borrows.carrier(*local).is_some())
            .collect();
        referents.sort();
        referents.dedup();
        referents
    }

    /// What a call may write (D4): for each of its operands through which it can write loans
    /// (a `&mut R` argument whose `R` carries loans, a function-typed argument, which the callee
    /// may call, and a function-value callee), the locals it points at, each receiving the
    /// items of the call's other loan-carrying operands. A written value cannot be a borrow of
    /// the slot it is written to, whose type differs, so an operand's own items never need to
    /// reach what it points at.
    fn call_writes(
        &self,
        state: &Holdings,
        callee: &Callee,
        arguments: &[Operand],
        parameters: &[Type],
    ) -> Vec<(LocalId, Vec<usize>)> {
        // Each loan-carrying operand's items, and whether the call may write through it, with
        // the type of what it writes when that is known: `R` for a `&mut R` argument.
        let mut sources: Vec<(Vec<usize>, Option<Option<Type>>)> = Vec::new();
        for (argument, parameter) in arguments.iter().zip(parameters) {
            if places::carries_loans(parameter) {
                let writes = if places::writes_loans_through(parameter) {
                    Some(referent(parameter))
                } else if places::contains_function(parameter) {
                    Some(None)
                } else {
                    None
                };
                sources.push((self.operand_items(state, argument), writes));
            }
        }
        if let Callee::Value(place, _) = callee {
            sources.push((self.held(state, place.local), Some(None)));
        }
        let mut writes = Vec::new();
        for (index, (items, through)) in sources.iter().enumerate() {
            let Some(written) = through else {
                continue;
            };
            let mut others: Vec<usize> = sources
                .iter()
                .enumerate()
                .filter(|(other, _)| *other != index)
                .flat_map(|(_, (items, _))| items.iter().copied())
                .collect();
            others.sort_unstable();
            others.dedup();
            if let Some(written) = written {
                others = self.fitting(others, written);
            }
            for referent in self.referents(items, true) {
                writes.push((referent, others.clone()));
            }
        }
        writes
    }
}

/// What a reference type refers to, under nullables: `R` for `&mut R` or `(&mut R)?`.
fn referent(ty: &Type) -> Option<Type> {
    match ty {
        Type::Reference { inner, .. } => Some(inner.as_ref().clone()),
        Type::Nullable(inner) => referent(inner),
        _ => None,
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
        Rvalue::Template(_) => rvalue.template_operands().collect(),
        // A closure value moves or copies the captures it owns (D2 of `escaping-closures`).
        Rvalue::Closure { captures, .. } => captures
            .iter()
            .filter_map(|capture| match capture {
                Capture::Owned(operand) => Some(operand),
                Capture::Borrowed(_) => None,
            })
            .collect(),
        Rvalue::Borrow { .. } | Rvalue::Discriminant(_) => Vec::new(),
    }
}

impl Analysis for LoanFlow<'_> {
    type Domain = Holdings;

    fn bottom(&self) -> Holdings {
        Holdings::default()
    }

    fn initialize_boundary(&self, state: &mut Holdings) {
        for input in &self.borrows.inputs {
            let item = self.borrows.item_index(Item::Input(*input));
            self.give(state, *input, &[item]);
        }
    }

    fn apply_statement(&self, state: &mut Holdings, statement: &Statement, location: Location) {
        match &statement.kind {
            StatementKind::Assign(destination, rvalue) => {
                let items = self.rvalue_items(state, rvalue, location);
                let writes = match rvalue.as_ref() {
                    Rvalue::Closure {
                        body: closure,
                        captures,
                    } => self.closure_writes(state, location, *closure, captures),
                    _ => Vec::new(),
                };
                for local in self.moved_locals(rvalue_operands(rvalue).into_iter()) {
                    self.clear(state, local);
                }
                self.write(state, destination, &items);
                for (local, items) in writes {
                    self.give(state, local, &items);
                }
            }
            StatementKind::StorageLive(local) | StatementKind::StorageDead(local) => {
                self.clear(state, *local)
            }
            StatementKind::Read(_) => {}
            // The parts carry the source's loans, fitted to their types; the source moved (D16
            // of `payload-patterns`).
            StatementKind::Destructure { source, parts, .. } => {
                let items = self.held(state, *source);
                self.clear(state, *source);
                for (_, target) in parts {
                    self.write(state, &Place::local(*target), &items);
                }
            }
        }
        self.prune(state, location);
    }

    fn apply_terminator(&self, state: &mut Holdings, terminator: &Terminator, location: Location) {
        self.apply_call(state, terminator);
        self.prune(state, location);
    }
}

impl LoanFlow<'_> {
    /// The effect of a call terminator.
    fn apply_call(&self, state: &mut Holdings, terminator: &Terminator) {
        if let TerminatorKind::Call {
            callee,
            arguments,
            parameters,
            destination,
            ..
        } = &terminator.kind
        {
            let items = self.call_items(state, callee, arguments, parameters);
            for (local, written) in self.call_writes(state, callee, arguments, parameters) {
                self.give(state, local, &written);
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

    #[test]
    fn holdings_are_sorted_pairs() {
        let mut state = Holdings::default();
        state.insert(2, [5, 1, 5]);
        state.insert(0, [3]);
        assert_eq!(state.items(2).collect::<Vec<_>>(), vec![1, 5]);
        assert!(state.contains(0, 3) && !state.contains(1, 3));
        let mut other = Holdings::default();
        other.insert(1, [5]);
        other.insert(2, [1]);
        assert!(state.join(&other));
        assert!(!state.join(&other));
        assert_eq!(state.holders(5).collect::<Vec<_>>(), vec![1, 2]);
        state.remove_items(&[5]);
        state.clear(0);
        assert_eq!(state.pairs.iter().collect::<Vec<_>>(), vec![(2, 1)]);
        assert_eq!(state.by_item.iter().collect::<Vec<_>>(), vec![(1, 2)]);
        // Many items at once are merged in; the index by item stays the same pairs.
        state.insert(1, [9, 8, 7, 6, 1, 0]);
        state.insert(3, [7]);
        assert_eq!(state.holders(7).collect::<Vec<_>>(), vec![1, 3]);
        assert_eq!(state.holders(1).collect::<Vec<_>>(), vec![1, 2]);
        state.clear(1);
        let mut swapped: Vec<(u32, u32)> = state.pairs.iter().map(|(c, i)| (i, c)).collect();
        swapped.sort_unstable();
        assert_eq!(state.by_item.iter().collect::<Vec<_>>(), swapped);
        assert_eq!(state.by_item().collect::<Vec<_>>(), vec![(1, 2), (7, 3)]);
    }

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
    fn a_reference_read_through_another_holds_what_it_refers_to() {
        // `c` is a copy of `r`'s value: it holds the loan of `a`, not the borrow of `r`.
        assert_eq!(
            held(
                "fun f() { var a = 1; var b = 2; var r: &Int = &a; val rr = &r; val c: &Int = rr; r = &b; print(c) }\n",
                "c",
                "print(c)"
            ),
            vec!["a"]
        );
        // So is the result of a closure that captures `r`.
        assert_eq!(
            held(
                "fun f() { var a = 1; var r: &Int = &a; val g = { r }; val q = g(); print(q) }\n",
                "q",
                "print(q)"
            ),
            vec!["a"]
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
    fn a_closure_writes_its_other_captures_through_a_captured_reference() {
        // What `rr` points at receives the loan of `a`, but not the loan of `rr` itself.
        assert_eq!(
            held(
                "fun f() { var a = 1; var b = 2; var r: &Int = &b; val rr = &mut r; run { *rr = &a; 1 }; print(r) }\n",
                "r",
                "print(r)"
            ),
            vec!["a", "b"]
        );
    }

    #[test]
    fn a_call_writes_its_other_arguments_through_a_mutable_reference() {
        let source = "\
fun print(n: &Int) {}
fun set(s: &mut &Int, v: &Int) {}
fun f() { var a = 1; var b = 2; var r: &Int = &b; val rr = &mut r; set(rr, &a); print(r) }
";
        assert_eq!(held_before(source, "r", "print(r)"), vec!["a", "b"]);
        // A moved argument still reaches the caller's referent through its input.
        let source = "\
fun h(s: &mut &Int, v: &Int) {}
fun f(s: &mut &Int) { val n = 1; val t = s; h(t, &n); print(n) }
fun print(n: Int) {}
";
        assert_eq!(held_before(source, "s", "print(n)"), vec!["n"]);
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
    fn a_method_result_holds_its_receiver_only() {
        let source = "\
struct Team { var lead: String }
fun Team.captain(o: &String): &String = &lead
fun greet(s: &String): Int = 1
fun f() { var t = Team(\"a\"); var s = \"x\"; val c = t.captain(&s); s = \"y\"; greet(c) }
";
        assert_eq!(held_before(source, "c", "s = \"y\""), vec!["t"]);
    }

    #[test]
    fn a_function_value_result_follows_its_type() {
        // With an input borrow, the argument's loan only; without, the function value's items.
        let source = "\
fun print(n: &Int) {}
fun f(g: (&Int) -> &Int, h: () -> &Int) { val a = 1; val r = g(&a); val q = h(); print(r) }
";
        assert_eq!(held_before(source, "r", "print(r)"), vec!["a"]);
        assert_eq!(held_before(source, "q", "print(r)"), vec!["input h"]);
        let source = "\
fun print(n: &Int) {}
fun f() { var t = 1; val g = { &t }; val r = g(); print(r) }
";
        assert_eq!(held_before(source, "r", "print(r)"), vec!["t"]);
    }

    #[test]
    fn a_default_result_holds_what_its_shared_arguments_borrow() {
        let source = "\
struct User { var name: String }
fun User.hi(n: &String = name): &String = n
fun greet(s: &String): Int = 1
fun f(): Int { var u = User(\"a\"); val h = u.hi(); return greet(h) }
";
        assert_eq!(held_before(source, "h", "return greet"), vec!["u"]);
        // A function's result holds the default's slot and what `u` borrows, not the
        // generator's borrow of the argument temporary holding `&u`.
        let source = "\
struct User { var name: String }
fun hello(u: &User, n: &String = u.name): &String = n
fun greet(s: &String): Int = 1
fun f(): Int { var u = User(\"a\"); val h = hello(&u); return greet(h) }
";
        assert_eq!(held_before(source, "h", "return greet"), vec!["mut _", "u"]);
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

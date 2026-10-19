//! The program scope of top-level initializers is free while compose is unused (D11 of
//! `state-handles`): `prism_compose_begin_program` pushes nothing until a composition starts,
//! since no pass runs and no creation owner is pushed before one does. This file holds one test,
//! so that no other test of the process has started a composition before it.

use prism_compose::test_support::content;
use prism_compose::{compose_function, Composition, NodeTree};

/// The entry point `name`, as a function of type `F`.
///
/// # Safety
///
/// `F` must be the entry point's C signature.
unsafe fn entry<F: Copy>(name: &str) -> F {
    let function = compose_function(name).unwrap_or_else(|| panic!("no entry point {}", name));
    assert_eq!(std::mem::size_of::<F>(), std::mem::size_of::<*const u8>());
    std::mem::transmute_copy(&function.address.as_ptr())
}

#[test]
fn the_program_scope_opens_only_once_compose_is_in_use() {
    let begin: extern "C" fn() -> u8 = unsafe { entry("prism_compose_begin_program") };
    let end: extern "C" fn(u8) = unsafe { entry("prism_compose_end_program") };
    assert_eq!(begin(), 0, "a program without compositions pushes no owner");
    end(0);
    let composition = Composition::start(Box::new(NodeTree::new()), content(|_| {})).unwrap();
    assert_eq!(
        begin(),
        1,
        "once a composition started, the program is pushed"
    );
    assert_eq!(begin(), 1, "nested initializers push the program again");
    end(1);
    end(1);
    Composition::dispose(composition).unwrap();
}

//! The entry points called through their C signatures, as generated code calls them, and the
//! misuse error that ends the process, checked in a child process (D12 of `compose-runtime`).

use prism_compose::test_support::content;
use prism_compose::{compose_function, Composer};
use prism_runtime::{FunctionValue, PrismString, PANIC_EXIT_STATUS};
use std::process::Command;

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

fn start(content: FunctionValue) -> u64 {
    let mut content = std::mem::ManuallyDrop::new(content);
    unsafe {
        let start: unsafe extern "C" fn(*mut u8) -> u64 = entry("prism_compose_start");
        start((&mut *content as *mut FunctionValue).cast())
    }
}

#[test]
fn a_composition_through_the_entry_points() {
    let composition = start(content(|k| unsafe {
        let Composer(k) = k;
        let start_restart: extern "C" fn(u64, i64) = entry("prism_compose_start_restart_group");
        let has_word: extern "C" fn(u64, i64) -> u8 = entry("prism_compose_has_word");
        let set_word: extern "C" fn(u64, i64, i64) = entry("prism_compose_set_word");
        let word: extern "C" fn(u64) -> i64 = entry("prism_compose_word");
        let start_node: extern "C" fn(u64, i64, i64) -> u64 =
            entry("prism_compose_start_node_group");
        let end_group: extern "C" fn(u64) = entry("prism_compose_end_group");
        let end_restart: unsafe extern "C" fn(u64, *mut u8) =
            entry("prism_compose_end_restart_group");
        start_restart(k, 10);
        if has_word(k, 0) == 0 {
            set_word(k, 0, 3);
        } else {
            assert_eq!(word(k), 3);
        }
        start_node(k, 30, 5);
        end_group(k);
        let mut restart = std::mem::ManuallyDrop::new(content(|_| {}));
        end_restart(k, (&mut *restart as *mut FunctionValue).cast());
    }));
    unsafe {
        let root_scope: extern "C" fn(u64) -> u64 = entry("prism_compose_root_scope");
        let invalidate: extern "C" fn(u64) = entry("prism_compose_invalidate");
        let recompose: extern "C" fn(u64) -> u8 = entry("prism_compose_recompose");
        let dump: unsafe extern "C" fn(*mut PrismString, u64) = entry("prism_compose_dump");
        let dispose: extern "C" fn(u64) = entry("prism_compose_dispose");
        assert_eq!(recompose(composition), 0);
        invalidate(root_scope(composition));
        assert_eq!(recompose(composition), 1);
        let mut text = std::mem::MaybeUninit::<PrismString>::uninit();
        dump(text.as_mut_ptr(), composition);
        let text = text.assume_init();
        assert_eq!(
            text.as_str(),
            "root\n  restart 10\n    word 0 = 3\n    node 30 #1 kind 5\nnodes\n  #1 kind 5\n"
        );
        prism_runtime::prism_rt_string_drop(&text as *const PrismString as *mut PrismString);
        dispose(composition);
    }
}

/// The variable telling a child process which misuse to make.
const CHILD: &str = "PRISM_COMPOSE_MISUSE_CHILD";

/// compose: "Reading a slot that is not there".
#[test]
fn reading_a_slot_that_is_not_there() {
    if std::env::var(CHILD).as_deref() == Ok("no slot") {
        start(content(|k| unsafe {
            let word: extern "C" fn(u64) -> i64 = entry("prism_compose_word");
            word(k.0);
        }));
        unreachable!("the misuse ends the process");
    }
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "reading_a_slot_that_is_not_there",
            "--nocapture",
            "--test-threads",
            "1",
        ])
        .env(CHILD, "no slot")
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("panic: composition misuse: no slot to read\n"),
        "stderr: {}",
        stderr
    );
    assert!(!stderr.contains("  at "), "no location: {}", stderr);
    assert_eq!(output.status.code(), Some(PANIC_EXIT_STATUS));
}

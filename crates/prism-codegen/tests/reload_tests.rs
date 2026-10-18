//! Tests of reloading a running program (`hot-reload`): slots repointed or split by contract,
//! and reloads applied through the stack-limit interrupt or at a frame loop's reload points, on
//! real source compiled through the front end. They run programs and request reloads, which
//! change the runtime's process-wide stack limit, so they run in a process of their own, one at a
//! time.

#[path = "../src/test_support.rs"]
#[allow(dead_code)]
mod test_support;

use prism_codegen::{check_file, Program, SlotChange};
use prism_mir::MirFile;
use prism_runtime::reload::{self, AppliedReload, PendingReload};
use prism_typed_tree::DeclarationPath;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::Mutex;
use std::time::Duration;
use test_support::lower_source;

fn lowered(source: &str) -> MirFile {
    let file = lower_source(source);
    check_file(&file).unwrap_or_else(|errors| panic!("not supported: {:?}", errors));
    file
}

fn compile(source: &str) -> Program {
    Program::compile(&[lowered(source)]).unwrap_or_else(|error| panic!("{}", error))
}

/// The address of the slot `symbol` is mapped to in `program`.
fn slot_address(program: &Program, symbol: &str) -> usize {
    program.table().slot(symbol) as usize
}

/// Requests the reload of `file` into `program`, whose writes `then` follows on the program
/// thread, and returns the symbols whose slots it splits.
fn request_reload(
    program: &mut Program,
    file: &MirFile,
    then: impl FnOnce() + Send + 'static,
) -> Vec<String> {
    let mut linker = program.live().linker().expect("a linker");
    linker.add_file(file).expect("the reload compiles");
    let prepared = linker.prepare().expect("the reload is prepared");
    let splits = prepared
        .splits
        .iter()
        .map(|(symbol, _)| symbol.clone())
        .collect();
    let writes = program.live_mut().hold(prepared);
    let units = vec![file.module.clone()];
    reload::request(PendingReload::new(move || {
        writes.apply();
        then();
        AppliedReload {
            units,
            message: String::new(),
        }
    }));
    splits
}

/// code-generation: "A repointed slot keeps its address".
#[test]
fn a_repointed_slot_keeps_its_address() {
    let _lock = prism_runtime::test_lock();
    let mut program = compile("fun value(): Int = 1\nfun main() { println(value()) }\n");
    let slot = slot_address(&program, "main::value");
    let before = program.function_address("main::value").unwrap();
    let file = lowered("fun value(): Int = 2\nfun main() { println(value()) }\n");
    let splits = request_reload(&mut program, &file, || {});
    assert!(splits.is_empty());
    assert!(reload::apply_now().is_some());
    program.live_mut().settle(true);
    assert_eq!(slot_address(&program, "main::value"), slot);
    let after = program.function_address("main::value").unwrap();
    assert_ne!(after, before, "the slot holds the reload's code");
    // SAFETY: `main::value` takes nothing and returns an `Int`.
    let value: extern "C" fn() -> i64 = unsafe { std::mem::transmute(after) };
    assert_eq!(value(), 2);
    assert_eq!(program.live().module_count(), 2);
    prism_runtime::clear_stack_limit();
}

/// A different contract gets a new slot, which later loads resolve to, while the old slot keeps
/// the old code for the code finalized before (D5 of `hot-reload`).
#[test]
fn a_different_contract_gets_a_new_slot() {
    let _lock = prism_runtime::test_lock();
    let mut program = compile("fun step(i: Int): Int = i\nfun main() { println(step(1)) }\n");
    let old_slot = slot_address(&program, "main::step");
    let old_code = program.function_address("main::step").unwrap();
    let (_, change) = program.table().slot_for("main::step", None);
    assert_eq!(
        change,
        SlotChange::Repointed,
        "code without a contract fits any"
    );
    let file =
        lowered("fun step(i: Int, k: Int): Int = i * k\nfun main() { println(step(1, 10)) }\n");
    let splits = request_reload(&mut program, &file, || {});
    assert_eq!(splits, ["main::step"]);
    assert!(reload::apply_now().is_some());
    program.live_mut().settle(true);
    let new_slot = slot_address(&program, "main::step");
    assert_ne!(new_slot, old_slot);
    // SAFETY: the old slot lives as long as the table.
    let kept = unsafe { (*(old_slot as *const AtomicUsize)).load(Ordering::Acquire) };
    assert_eq!(kept, old_code as usize, "the old slot keeps its code");
    // SAFETY: the new `main::step` takes two `Int`s and returns one.
    let step: extern "C" fn(i64, i64) -> i64 =
        unsafe { std::mem::transmute(program.function_address("main::step").unwrap()) };
    assert_eq!(step(3, 10), 30);
    prism_runtime::clear_stack_limit();
}

/// code-generation: "An interrupt does not end the program".
#[test]
fn an_interrupt_does_not_end_the_program() {
    let _lock = prism_runtime::test_lock();
    let source =
        "fun value(): Int = 1\nfun main() { var n = 0; while (value() == 1) { n += 1 } }\n";
    let mut program = compile(source);
    let (sender, receiver) = mpsc::channel();
    let main = DeclarationPath::new("main", "main");
    let running = program
        .start(&main, Vec::new(), false, move |status| {
            let _ = sender.send(status);
        })
        .expect("the program starts");
    std::thread::sleep(Duration::from_millis(20));
    let file = lowered(&source.replace("Int = 1", "Int = 2"));
    request_reload(&mut program, &file, || {});
    let status = receiver
        .recv_timeout(Duration::from_secs(20))
        .expect("main returns once the reload is applied");
    assert_eq!(status, 0);
    assert_eq!(running.join().unwrap(), 0);
    assert!(reload::withdraw().is_none(), "the reload was applied");
    program.live_mut().settle(true);
}

static TICKS: Mutex<Vec<i64>> = Mutex::new(Vec::new());
static APPLIED: Mutex<Vec<Vec<String>>> = Mutex::new(Vec::new());
type Handshake = (mpsc::Sender<()>, mpsc::Receiver<()>);
static FIRST_TICK: Mutex<Option<Handshake>> = Mutex::new(None);

/// The test's frame loop's reload point, which the program calls between iterations.
extern "C" fn test_frame() {
    if let Some(applied) = reload::reload_point() {
        APPLIED.lock().unwrap().push(applied.units);
    }
}

/// Records what an iteration computed; on the first, lets the test request a reload while the
/// iteration runs.
extern "C" fn test_tick(value: i64) {
    let first = {
        let mut ticks = TICKS.lock().unwrap();
        ticks.push(value);
        ticks.len() == 1
    };
    if first {
        if let Some((started, requested)) = FIRST_TICK.lock().unwrap().as_ref() {
            started.send(()).unwrap();
            requested.recv().unwrap();
        }
    }
}

fn wake() {}

/// Points the slots of `main::frame` and `main::tick` of `table` at the test's functions.
fn install_hooks(table: &prism_codegen::FunctionTable) {
    table.set("main::frame", test_frame as extern "C" fn() as *const u8);
    table.set("main::tick", test_tick as extern "C" fn(i64) as *const u8);
}

/// hot-reload: "A deferred reload is applied at the reload point".
#[test]
fn a_deferred_reload_is_applied_at_the_reload_point() {
    let _lock = prism_runtime::test_lock();
    let source = "fun frame() {}\nfun tick(v: Int) {}\nfun value(): Int = 1\n\
                  fun main() { var i = 0; while (i < 3) { frame(); tick(value()); i += 1 } }\n";
    let mut program = compile(source);
    install_hooks(program.table());
    TICKS.lock().unwrap().clear();
    APPLIED.lock().unwrap().clear();
    let (started_sender, started) = mpsc::channel();
    let (requested, requested_receiver) = mpsc::channel();
    *FIRST_TICK.lock().unwrap() = Some((started_sender, requested_receiver));
    reload::defer_to_reload_points(wake);
    let (sender, receiver) = mpsc::channel();
    let main = DeclarationPath::new("main", "main");
    let running = program
        .start(&main, Vec::new(), false, move |status| {
            let _ = sender.send(status);
        })
        .expect("the program starts");
    started
        .recv_timeout(Duration::from_secs(20))
        .expect("the first iteration runs");
    let file = lowered(&source.replace("Int = 1", "Int = 2"));
    let table = std::sync::Arc::clone(program.live().table());
    // The reload defines the Prism `frame` and `tick` again; the test's own stay in their slots.
    request_reload(&mut program, &file, move || install_hooks(&table));
    requested.send(()).unwrap();
    let status = receiver
        .recv_timeout(Duration::from_secs(20))
        .expect("main returns");
    assert_eq!(status, 0);
    running.join().unwrap();
    reload::apply_at_next_call();
    *FIRST_TICK.lock().unwrap() = None;
    program.live_mut().settle(true);
    // The first iteration finished on the old code; the reload point before the second applied
    // the reload and reported its module.
    assert_eq!(*TICKS.lock().unwrap(), [1, 2, 2]);
    assert_eq!(*APPLIED.lock().unwrap(), [vec!["main".to_string()]]);
}

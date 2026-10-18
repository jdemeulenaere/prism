//! Tests of lowering and linking that compile real source through the front end and call the
//! compiled functions from Rust through their slots.

use crate::test_support::{lower_files, lower_source};
use crate::{check_file, Program};
use prism_mir::MirFile;
use std::sync::atomic::{AtomicUsize, Ordering};

fn compile_files(files: Vec<MirFile>) -> Program {
    for file in &files {
        if let Err(errors) = check_file(file) {
            panic!("{} is not supported: {:?}", file.module, errors);
        }
    }
    Program::compile(&files).unwrap_or_else(|error| panic!("{}", error))
}

fn compile(source: &str) -> Program {
    compile_files(vec![lower_source(source)])
}

/// The compiled function `symbol` of `program`, as a function pointer of type `F`.
fn function<F: Copy>(program: &Program, symbol: &str) -> F {
    let address = program
        .function_address(symbol)
        .unwrap_or_else(|| panic!("{} has no slot", symbol));
    assert_eq!(std::mem::size_of::<F>(), std::mem::size_of::<*const u8>());
    // SAFETY: the tests name each function with the type of its signature.
    unsafe { std::mem::transmute_copy(&address) }
}

type IntToInt = extern "C" fn(i64) -> i64;
type IntsToInt = extern "C" fn(i64, i64) -> i64;

#[test]
fn identity_and_if() {
    let program = compile(
        "fun id(n: Int): Int = n\n\
         fun sign(n: Int): Int = if (n < 0) -1 else if (n == 0) 0 else 1\n",
    );
    let id: IntToInt = function(&program, "main::id");
    assert_eq!(id(42), 42);
    assert_eq!(id(i64::MIN), i64::MIN);
    let sign: IntToInt = function(&program, "main::sign");
    assert_eq!([sign(-5), sign(0), sign(7)], [-1, 0, 1]);
}

#[test]
fn nested_when_over_int() {
    let program = compile(
        "fun f(x: Int, y: Int): Int = when (x) {\n\
             0 -> when (y) { 0 -> 1; in 1..5 -> 2; else -> 3 }\n\
             in 1..<10 -> 10\n\
             else -> 20\n\
         }\n",
    );
    let f: IntsToInt = function(&program, "main::f");
    assert_eq!(
        [f(0, 0), f(0, 3), f(0, 9), f(5, 0), f(10, 0), f(-1, 0)],
        [1, 2, 3, 10, 20, 20]
    );
}

#[test]
fn while_and_do_while() {
    let program = compile(
        "fun count(n: Int): Int { var i = 0; var steps = 0; while (i < n) { i += 1; steps += 1 }; return steps }\n\
         fun once(n: Int): Int { var m = n; do { m += 1 } while (false); return m }\n\
         fun first_multiple(of: Int): Int { var k = 1; while (true) { if (k % of == 0) break; k += 1 }; return k }\n",
    );
    let count: IntToInt = function(&program, "main::count");
    assert_eq!([count(0), count(3), count(-2)], [0, 3, 0]);
    let once: IntToInt = function(&program, "main::once");
    assert_eq!(once(0), 1);
    let first_multiple: IntToInt = function(&program, "main::first_multiple");
    assert_eq!(first_multiple(7), 7);
}

#[test]
fn for_over_both_range_kinds_with_break_and_continue() {
    let program = compile(
        "fun half_open(n: Int): Int { var sum = 0; for (i in 0..<n) { if (i == 2) continue; sum += i }; return sum }\n\
         fun closed(n: Int): Int { var sum = 0; for (i in 1..n) sum += i; return sum }\n\
         fun stored(n: Int): Int { val r = 1..n; var sum = 0; for (i in r) { if (i > 3) break; sum += i }; return sum }\n\
         fun nested(n: Int): Int {\n\
             var count = 0\n\
             for (i in 0..<n) { var j = 0; while (true) { if (j == i) break; j += 1; count += 1 } }\n\
             return count\n\
         }\n\
         fun last_two(): Int { var times = 0; for (i in 9223372036854775806..9223372036854775807) times += 1; return times }\n\
         fun last_one(): Int { var last = 0; for (i in 9223372036854775806..<9223372036854775807) last = i; return last }\n",
    );
    let half_open: IntToInt = function(&program, "main::half_open");
    assert_eq!([half_open(5), half_open(0), half_open(-3)], [8, 0, 0]);
    let closed: IntToInt = function(&program, "main::closed");
    assert_eq!([closed(3), closed(0)], [6, 0]);
    let stored: IntToInt = function(&program, "main::stored");
    assert_eq!(stored(10), 6);
    let nested: IntToInt = function(&program, "main::nested");
    assert_eq!(nested(4), 6);
    let last_two: extern "C" fn() -> i64 = function(&program, "main::last_two");
    assert_eq!(last_two(), 2);
    let last_one: extern "C" fn() -> i64 = function(&program, "main::last_one");
    assert_eq!(last_one(), i64::MAX - 1);
}

#[test]
fn integer_operators_on_boundary_values() {
    let program = compile(
        "fun add(a: Int, b: Int): Int = a + b\n\
         fun sub(a: Int, b: Int): Int = a - b\n\
         fun mul(a: Int, b: Int): Int = a * b\n\
         fun div(a: Int, b: Int): Int = a / b\n\
         fun rem(a: Int, b: Int): Int = a % b\n\
         fun neg(a: Int): Int = -a\n\
         fun compound(a: Int): Int { var n = a; n *= 3; n -= 1; n /= 2; n %= 5; n += 10; return n }\n",
    );
    let add: IntsToInt = function(&program, "main::add");
    assert_eq!(add(i64::MAX - 1, 1), i64::MAX);
    assert_eq!(add(-3, 5), 2);
    let sub: IntsToInt = function(&program, "main::sub");
    assert_eq!(sub(i64::MIN + 1, 1), i64::MIN);
    let mul: IntsToInt = function(&program, "main::mul");
    assert_eq!(mul(-4, 5), -20);
    assert_eq!(mul(i64::MIN, 1), i64::MIN);
    let div: IntsToInt = function(&program, "main::div");
    assert_eq!(
        [div(-7, 2), div(7, -2), div(i64::MIN, 1)],
        [-3, -3, i64::MIN]
    );
    let rem: IntsToInt = function(&program, "main::rem");
    assert_eq!(
        [rem(-7, 2), rem(7, -2), rem(i64::MIN, -1), rem(5, -1)],
        [-1, 1, 0, 0]
    );
    let neg: IntToInt = function(&program, "main::neg");
    assert_eq!([neg(5), neg(i64::MAX)], [-5, -i64::MAX]);
    let compound: IntToInt = function(&program, "main::compound");
    // ((5 * 3 - 1) / 2) % 5 + 10
    assert_eq!(compound(5), 12);
}

#[test]
fn double_operators_and_comparisons() {
    let program = compile(
        "fun rem(a: Double, b: Double): Double = a % b\n\
         fun mix(a: Double, b: Double): Double = -(a * b) / (a - b) + a\n\
         fun lt(a: Double, b: Double): Bool = a < b\n\
         fun eq(a: Double, b: Double): Bool = a == b\n\
         fun ne(a: Double, b: Double): Bool = a != b\n\
         fun ge(a: Double, b: Double): Bool = a >= b\n\
         fun not(b: Bool): Bool = !b\n\
         fun chars(a: Char, b: Char): Bool = a < b && a != b\n",
    );
    let rem: extern "C" fn(f64, f64) -> f64 = function(&program, "main::rem");
    assert_eq!(rem(-7.5, 2.0), -1.5);
    let mix: extern "C" fn(f64, f64) -> f64 = function(&program, "main::mix");
    assert_eq!(mix(3.0, 1.0), -(3.0 * 1.0) / (3.0 - 1.0) + 3.0);
    assert!(mix(1.0, 1.0).is_infinite());
    type Compare = extern "C" fn(f64, f64) -> u8;
    let (lt, eq, ne, ge): (Compare, Compare, Compare, Compare) = (
        function(&program, "main::lt"),
        function(&program, "main::eq"),
        function(&program, "main::ne"),
        function(&program, "main::ge"),
    );
    let nan = f64::NAN;
    assert_eq!(
        [lt(nan, 1.0), eq(nan, nan), ne(nan, nan), ge(nan, nan)],
        [0, 0, 1, 0]
    );
    assert_eq!(
        [lt(1.0, 2.0), eq(2.0, 2.0), ne(2.0, 2.0), ge(2.0, 1.0)],
        [1, 1, 0, 1]
    );
    assert_eq!(eq(0.0, -0.0), 1);
    let not: extern "C" fn(u8) -> u8 = function(&program, "main::not");
    assert_eq!([not(0), not(1)], [1, 0]);
    let chars: extern "C" fn(u32, u32) -> u8 = function(&program, "main::chars");
    assert_eq!(
        [chars('a' as u32, 'b' as u32), chars('é' as u32, 'a' as u32)],
        [1, 0]
    );
}

#[test]
fn calls_through_slots() {
    let program = compile(
        "fun fact(n: Int): Int = if (n <= 1) 1 else n * fact(n - 1)\n\
         fun twice(n: Int): Int = n * 2\n\
         fun quad(n: Int): Int = twice(twice(n))\n\
         fun scale(a: Int, b: Int = a + 1): Int = a * b\n\
         fun scaled(a: Int): Int = scale(a) + scale(a, 10)\n\
         fun bump(r: &mut Int) { *r = r + 1 }\n\
         fun bumped(n: Int): Int { var m = n; bump(&mut m); bump(&mut m); return m }\n\
         fun cloned(n: Int): Int { val c = n.clone(); return c + 1 }\n",
    );
    let fact: IntToInt = function(&program, "main::fact");
    assert_eq!(fact(20), 2432902008176640000);
    let quad: IntToInt = function(&program, "main::quad");
    assert_eq!(quad(3), 12);
    let scaled: IntToInt = function(&program, "main::scaled");
    assert_eq!(scaled(3), 3 * 4 + 3 * 10);
    let bumped: IntToInt = function(&program, "main::bumped");
    assert_eq!(bumped(1), 3);
    let cloned: IntToInt = function(&program, "main::cloned");
    assert_eq!(cloned(4), 5);
}

static INITIALIZATIONS: AtomicUsize = AtomicUsize::new(0);

extern "C" fn counting_initializer() -> i64 {
    INITIALIZATIONS.fetch_add(1, Ordering::SeqCst);
    5
}

#[test]
fn a_top_level_value_initializes_once() {
    let program = compile(
        "fun five(): Int = 5\n\
         val V: Int = five()\n\
         val W: Int = V * 2\n\
         fun read(n: Int): Int = V + V + n\n\
         fun both(): Int = W + V\n",
    );
    let read: IntToInt = function(&program, "main::read");
    assert_eq!(read(1), 11);
    // The accessor calls the initializer through its slot, so pointing the slot at a counting
    // function shows how often it runs; the value is initialized already, so it never does.
    program.table().set(
        "main::V#init",
        counting_initializer as extern "C" fn() -> i64 as *const u8,
    );
    assert_eq!(read(0), 10);
    assert_eq!(INITIALIZATIONS.load(Ordering::SeqCst), 0);

    let fresh = compile("val V: Int = 1\nfun read(): Int = V + V + V\n");
    fresh.table().set(
        "main::V#init",
        counting_initializer as extern "C" fn() -> i64 as *const u8,
    );
    let read: extern "C" fn() -> i64 = function(&fresh, "main::read");
    assert_eq!(read(), 15);
    assert_eq!(read(), 15);
    assert_eq!(INITIALIZATIONS.load(Ordering::SeqCst), 1);
    let both: extern "C" fn() -> i64 = function(&program, "main::both");
    assert_eq!(both(), 15);
}

#[test]
fn files_call_each_other_in_both_directions() {
    let program = compile_files(lower_files(&[
        (
            "lib/even",
            "package lib\nimport /lib/odd\npublic fun isEven(n: Int): Bool = if (n == 0) true else isOdd(n - 1)\n",
        ),
        (
            "lib/odd",
            "package lib\nimport /lib/even\npublic fun isOdd(n: Int): Bool = if (n == 0) false else isEven(n - 1)\n",
        ),
    ]));
    let is_even: extern "C" fn(i64) -> u8 = function(&program, "lib/even::isEven");
    let is_odd: extern "C" fn(i64) -> u8 = function(&program, "lib/odd::isOdd");
    assert_eq!(
        [is_even(10), is_even(7), is_odd(7), is_odd(0)],
        [1, 0, 1, 0]
    );
}

#[test]
fn a_slot_nothing_defines_holds_the_unlinked_stub() {
    // `lib/a` calls `lib/b::g`, whose file is not part of the program.
    let files = lower_files(&[
        ("lib/b", "package lib\npublic fun g(): Int = 1\n"),
        (
            "lib/a",
            "package lib\nimport /lib/b\npublic fun f(): Int = g()\n",
        ),
    ]);
    let program = compile_files(vec![files[1].clone()]);
    let stub = prism_runtime::prism_rt_unlinked_function as extern "C" fn() -> ! as *const u8;
    assert_eq!(program.function_address("lib/b::g"), Some(stub));
    assert_ne!(program.function_address("lib/a::f"), Some(stub));
}

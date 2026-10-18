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

// Aggregates (D9 to D17 of `codegen-aggregates`).

type ToInt = extern "C" fn() -> i64;
type ToBool = extern "C" fn() -> u8;

#[test]
fn fields_are_read_and_written_in_place() {
    let program = compile(
        "struct Point : Copy { var x: Int, var y: Int }\n\
         struct Line : Copy { var from: Point, var to: Point }\n\
         fun bump(r: &mut Int) { *r = r + 1 }\n\
         fun shift(l: &mut Line) { l.to.y = 40; l.from = Point(7, 8) }\n\
         fun replace(r: &mut Point, v: Point) { *r = v }\n\
         fun fields(n: Int): Int {\n\
             var l = Line(Point(1, 2), Point(3, n))\n\
             l.from.x = 10; bump(&mut l.to.x); shift(&mut l); val y = l.to.y; replace(&mut l.to, Point(y, 5))\n\
             return l.from.x * 1000 + l.from.y * 100 + l.to.x + l.to.y\n\
         }\n\
         fun make(x: Int): Point = Point(x, x + 1)\n",
    );
    let fields: IntToInt = function(&program, "main::fields");
    assert_eq!(fields(4), 7000 + 800 + 40 + 5);
    // A result in memory is written through the address passed first.
    let make: extern "C" fn(*mut [i64; 2], i64) = function(&program, "main::make");
    let mut point = [0i64; 2];
    make(&mut point, 41);
    assert_eq!(point, [41, 42]);
}

#[test]
fn enums_and_nullables_of_both_kinds() {
    let program = compile(
        "enum Color { Red, Green, Blue }\n\
         enum Shape : Copy { Circle(Double), Rect(Int, Bool), Empty }\n\
         fun index(c: Color): Int = when (c) { Red -> 0; Green -> 1; Blue -> 2 }\n\
         fun pick(n: Int): Color = if (n == 0) Color.Red else if (n == 1) Color.Green else Color.Blue\n\
         fun shape(n: Int): Shape = if (n == 0) Shape.Circle(1.5) else if (n == 1) Shape.Rect(n, true) else Shape.Empty\n\
         fun same_shape(a: Int, b: Int): Bool = shape(a) == shape(b)\n\
         fun color(n: Int): Int = index(pick(n))\n\
         fun half(n: Int): Int? = if (n % 2 == 0) n / 2 else null\n\
         fun or_minus(n: Int): Int = half(n) ?: -1\n\
         fun view(r: (&Int)?): Int = r ?: &-1\n\
         fun viewed(n: Int): Int { val h = half(n); val a = 5; val r: (&Int)? = if (h == null) null else &a; return view(r) + view(null) }\n",
    );
    let color: IntToInt = function(&program, "main::color");
    assert_eq!([color(0), color(1), color(2)], [0, 1, 2]);
    let same_shape: extern "C" fn(i64, i64) -> u8 = function(&program, "main::same_shape");
    assert_eq!(
        [
            same_shape(0, 0),
            same_shape(1, 1),
            same_shape(2, 2),
            same_shape(0, 2),
            same_shape(1, 0)
        ],
        [1, 1, 1, 0, 0]
    );
    let or_minus: IntToInt = function(&program, "main::or_minus");
    assert_eq!([or_minus(8), or_minus(3)], [4, -1]);
    let viewed: IntToInt = function(&program, "main::viewed");
    assert_eq!([viewed(2), viewed(3)], [4, -2]);
    // `Int?` is a tag byte and the payload at offset 8.
    let half: extern "C" fn(*mut [i64; 2], i64) = function(&program, "main::half");
    let mut value = [-1i64; 2];
    half(&mut value, 10);
    assert_eq!((value[0] & 0xff, value[1]), (1, 5));
    half(&mut value, 3);
    assert_eq!(value[0] & 0xff, 0);
}

#[test]
fn large_structs_are_copied_and_compared() {
    let program = compile(
        "struct Big : Copy { var a: Int, var b: Int, var c: Int, var d: Int, var e: Int, var f: Double }\n\
         struct Two : Copy { var first: Big, var second: Big }\n\
         fun big(n: Int): Big = Big(n, n, n, n, n, 0.5)\n\
         fun copies(n: Int): Bool { val a = big(n); var b = a; val same = a == b; b.e = 0; return same && a != b && a.e == n }\n\
         fun swap(n: Int): Int { var t = Two(big(1), big(n)); t = Two(t.second, t.first); return t.first.a * 10 + t.second.e }\n",
    );
    let copies: extern "C" fn(i64) -> u8 = function(&program, "main::copies");
    assert_eq!(copies(7), 1);
    let swap: IntToInt = function(&program, "main::swap");
    assert_eq!(swap(3), 31);
}

#[test]
fn arguments_and_results_are_values() {
    let program = compile(
        "struct Point : Copy { var x: Int, var y: Int }\n\
         struct Counter { var n: Int }\n\
         fun mirror(p: Point): Point = Point(p.y, p.x)\n\
         fun mirrored(): Int { var p = Point(1, 2); p = mirror(p); return p.x * 10 + p.y }\n\
         fun f(p: Point, r: &mut Point): Int { r.x = 9; return p.x }\n\
         fun aliased(): Int { var a = Point(1, 0); val seen = f(a, &mut a); return seen * 10 + a.x }\n\
         fun take(c: Counter): Int = c.n\n\
         fun moved(flag: Bool): Int { val c = Counter(4); if (flag) return take(c); return 1 }\n",
    );
    let mirrored: ToInt = function(&program, "main::mirrored");
    assert_eq!(mirrored(), 21);
    let aliased: ToInt = function(&program, "main::aliased");
    assert_eq!(aliased(), 19);
    let moved: extern "C" fn(u8) -> i64 = function(&program, "main::moved");
    assert_eq!([moved(1), moved(0)], [4, 1]);
}

#[test]
fn equality_of_structs_enums_and_nullables() {
    let program = compile(
        "struct Point : Copy { val x: Int, val y: Int }\n\
         enum Shape : Copy { Circle(Double), Square(Double), Empty }\n\
         struct D : Copy { val v: Double }\n\
         struct Counter { val n: Int }\n\
         fun points(): Bool = Point(1, 2) == Point(1, 2) && Point(1, 2) != Point(2, 1)\n\
         fun shapes(): Bool = Shape.Circle(1.0) != Shape.Circle(2.0) && Shape.Circle(1.0) != Shape.Square(1.0) && Shape.Empty == Shape.Empty\n\
         fun nullables(): Bool { val a: Int? = null; val b: Int? = 1; val c: Int? = 1; return a == null && a != b && b == c && b != null }\n\
         fun doubles(): Bool = D(0.0 / 0.0) != D(0.0 / 0.0) && D(0.0) == D(-0.0)\n\
         fun same(a: Counter, b: Counter): Bool = a == b\n\
         fun counters(): Bool = same(Counter(1), Counter(1)) && !same(Counter(1), Counter(2))\n\
         fun optional(a: Counter?, b: Counter?): Bool = a == b\n\
         fun optionals(): Bool = optional(null, null) && !optional(Counter(1), null) && !optional(null, Counter(1)) && optional(Counter(3), Counter(3)) && !optional(Counter(3), Counter(4))\n",
    );
    for name in [
        "points",
        "shapes",
        "nullables",
        "doubles",
        "counters",
        "optionals",
    ] {
        let test: ToBool = function(&program, &format!("main::{}", name));
        assert_eq!(test(), 1, "{}", name);
    }
}

static AGGREGATE_INITIALIZATIONS: AtomicUsize = AtomicUsize::new(0);

extern "C" fn counting_point_initializer(result: *mut [i64; 2]) {
    AGGREGATE_INITIALIZATIONS.fetch_add(1, Ordering::SeqCst);
    // SAFETY: the accessor passes the value's storage, 16 bytes aligned to 8.
    unsafe { *result = [3, 4] };
}

#[test]
fn a_top_level_struct_initializes_once() {
    let program = compile(
        "struct Point : Copy { val x: Int, val y: Int }\n\
         val ORIGIN: Point = Point(0, 7)\n\
         fun read(): Int { val p = ORIGIN; return ORIGIN.x * 100 + p.y * 10 + ORIGIN.y }\n",
    );
    program.table().set(
        "main::ORIGIN#init",
        counting_point_initializer as extern "C" fn(*mut [i64; 2]) as *const u8,
    );
    let read: ToInt = function(&program, "main::read");
    assert_eq!(read(), 344);
    assert_eq!(read(), 344);
    assert_eq!(AGGREGATE_INITIALIZATIONS.load(Ordering::SeqCst), 1);
}

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

// Strings (D2 to D7 and D16 of `runtime-strings-and-drops`).

use prism_runtime::{allocation_statistics, prism_rt_string_drop, PrismString};
use std::mem::MaybeUninit;

/// Runs `test` under the allocation check, with the runtime's process-wide state to itself, and
/// returns the number of allocations it made, checking that it freed all of them.
fn checked(test: impl FnOnce()) -> u64 {
    let _lock = prism_runtime::test_lock();
    prism_runtime::start(Vec::new(), true);
    test();
    let statistics = allocation_statistics();
    assert_eq!(prism_runtime::finish(), 0);
    prism_runtime::start(Vec::new(), false);
    assert_eq!(statistics.live_allocations, 0, "{:?}", statistics);
    statistics.allocations_made
}

/// The string a compiled function writes through the result address it is passed.
fn returned(f: impl FnOnce(*mut PrismString)) -> PrismString {
    let mut result = MaybeUninit::<PrismString>::uninit();
    f(result.as_mut_ptr());
    // SAFETY: a function returning a `String` writes its result address.
    unsafe { result.assume_init() }
}

fn drop_string(mut string: PrismString) {
    // SAFETY: the string is not used afterwards.
    unsafe { prism_rt_string_drop(&mut string) };
}

type StringResult = extern "C" fn(*mut PrismString);
type StringPair<R> = extern "C" fn(*const PrismString, *const PrismString) -> R;

#[test]
fn literals_and_their_clones_allocate_nothing() {
    // The scenarios "Literals do not allocate" and "An empty concatenation owns nothing".
    let program = compile(
        "fun literal(): String = \"héllo\"\n\
         fun cloned(): Int { val s = \"hi\"; val t = s.clone(); return t.length + s.length }\n\
         fun empty(): String = \"\" + \"\"\n\
         fun one_sided(): String = \"\" + \"ab\"\n",
    );
    let literal: StringResult = function(&program, "main::literal");
    let cloned: ToInt = function(&program, "main::cloned");
    let empty: StringResult = function(&program, "main::empty");
    let one_sided: StringResult = function(&program, "main::one_sided");
    let made = checked(|| {
        let text = returned(|r| literal(r));
        assert_eq!(
            (
                text.as_str(),
                text.byte_length,
                text.character_count,
                text.capacity
            ),
            ("héllo", 6, 5, 0)
        );
        assert_eq!(cloned(), 4);
        let nothing = returned(|r| empty(r));
        assert_eq!((nothing.byte_length, nothing.capacity), (0, 0));
        assert!(!nothing.pointer.is_null());
        let other = returned(|r| one_sided(r));
        assert_eq!((other.as_str(), other.capacity), ("ab", 0));
        for string in [text, nothing, other] {
            drop_string(string);
        }
    });
    assert_eq!(made, 0);
}

#[test]
fn string_operations_give_their_results() {
    let program = compile(
        "fun join(a: &String, b: &String): String = a + b\n\
         fun same(a: &String, b: &String): Bool = a == b\n\
         fun differ(a: &String, b: &String): Bool = a != b\n\
         fun count(s: &String): Int = s.length\n\
         fun at(s: &String, i: Int): Char = s[i]\n\
         fun int(n: Int): String = n.toString()\n\
         fun double(d: Double): String = d.toString()\n\
         fun bool(b: Bool): String = b.toString()\n\
         fun char(c: Char): String = c.toString()\n\
         fun pick(s: String): Int = when (s) { \"one\" -> 1; \"two\" -> 2; else -> 0 }\n",
    );
    let join: extern "C" fn(*mut PrismString, *const PrismString, *const PrismString) =
        function(&program, "main::join");
    let same: StringPair<u8> = function(&program, "main::same");
    let differ: StringPair<u8> = function(&program, "main::differ");
    let count: extern "C" fn(*const PrismString) -> i64 = function(&program, "main::count");
    let at: extern "C" fn(*const PrismString, i64) -> u32 = function(&program, "main::at");
    let int: extern "C" fn(*mut PrismString, i64) = function(&program, "main::int");
    let double: extern "C" fn(*mut PrismString, f64) = function(&program, "main::double");
    let boolean: extern "C" fn(*mut PrismString, u8) = function(&program, "main::bool");
    let char: extern "C" fn(*mut PrismString, u32) = function(&program, "main::char");
    let pick: extern "C" fn(*const PrismString) -> i64 = function(&program, "main::pick");
    checked(|| {
        let pri = PrismString::of_static("pri");
        let sm = PrismString::owned("sm");
        let prism = returned(|result| join(result, &pri, &sm));
        assert_eq!((prism.as_str(), prism.character_count), ("prism", 5));
        assert_eq!(same(&prism, &PrismString::of_static("prism")), 1);
        assert_eq!(same(&prism, &pri), 0);
        assert_eq!(differ(&prism, &pri), 1);
        let text = PrismString::of_static("héllo");
        assert_eq!(count(&text), 5);
        assert_eq!([at(&text, 1), at(&text, 4)], ['é' as u32, 'o' as u32]);
        let texts: Vec<String> = [
            returned(|result| int(result, -42)),
            returned(|result| double(result, 1.0e7)),
            returned(|result| boolean(result, 1)),
            returned(|result| char(result, 'é' as u32)),
        ]
        .into_iter()
        .map(|string| {
            let text = string.as_str().to_string();
            drop_string(string);
            text
        })
        .collect();
        assert_eq!(texts, ["-42", "1.0E7", "true", "é"]);
        for (word, expected) in [("two", 2), ("one", 1), ("three", 0)] {
            assert_eq!(pick(&PrismString::of_static(word)), expected);
        }
        drop_string(prism);
        drop_string(sm);
    });
}

// Drop and clone glue (D9, D10 and D12 of `runtime-strings-and-drops`).

#[test]
fn drops_free_every_allocation_once() {
    let program = compile(
        "struct User { val name: String, val nick: String?, val age: Int }\n\
         struct Team { val lead: User, val size: Int, val motto: String }\n\
         enum Message { Text(String, Int), Pair(Int, String?), Empty }\n\
         fun text(n: Int): String = n.toString() + \"!\"\n\
         fun user(): Int { val u = User(text(1), text(2), 36); return u.age }\n\
         fun anonymous(): Int { val u = User(text(1), null, 7); return u.age }\n\
         fun team(): Int { val t = Team(User(text(1), text(2), 3), 4, text(5)); return t.size + t.lead.age }\n\
         fun message(n: Int): Int {\n\
             val m = if (n == 0) Message.Text(text(n), n) else if (n == 1) Message.Pair(n, text(n)) else if (n == 2) Message.Pair(n, null) else Message.Empty\n\
             return n\n\
         }\n\
         fun nullable(n: Int): Bool { val s: String? = if (n == 0) null else text(n); return s == null }\n\
         fun take(s: String): Int = s.length\n\
         fun flag(c: Bool): Int { val s = text(1); if (c) return take(s); return 0 }\n\
         fun reassign(): Int { var s = text(1); s = text(22); s += text(3); return s.length }\n\
         fun set(r: &mut String, v: String) { *r = v }\n\
         fun written(): Int { var s = text(1); set(&mut s, text(2)); return s.length }\n\
         fun temporaries(): Int = take(text(1)) + text(23).length\n",
    );
    let user: ToInt = function(&program, "main::user");
    let anonymous: ToInt = function(&program, "main::anonymous");
    let team: ToInt = function(&program, "main::team");
    let message: IntToInt = function(&program, "main::message");
    let nullable: extern "C" fn(i64) -> u8 = function(&program, "main::nullable");
    let flag: extern "C" fn(u8) -> i64 = function(&program, "main::flag");
    let reassign: ToInt = function(&program, "main::reassign");
    let written: ToInt = function(&program, "main::written");
    let temporaries: ToInt = function(&program, "main::temporaries");
    let made = checked(|| {
        assert_eq!(user(), 36);
        assert_eq!(anonymous(), 7);
        assert_eq!(team(), 7);
        assert_eq!(
            [message(0), message(1), message(2), message(3)],
            [0, 1, 2, 3]
        );
        assert_eq!([nullable(0), nullable(4)], [1, 0]);
        assert_eq!([flag(1), flag(0)], [2, 0]);
        assert_eq!(reassign(), 5);
        assert_eq!(written(), 2);
        assert_eq!(temporaries(), 5);
    });
    // Each `toString()` allocates, and each concatenation with a non-empty operand.
    assert!(made > 20, "{}", made);
}

#[test]
fn clones_own_their_buffers() {
    let program = compile(
        "struct User : Clone { var name: String, val nick: String?, val id: Int }\n\
         enum Message : Clone { Text(String, Int), Empty }\n\
         fun text(n: Int): String = n.toString() + \"!\"\n\
         fun users(): Bool {\n\
             var a = User(text(1), text(2), 7); val b = a.clone(); val same = a == b\n\
             a.name = text(3)\n\
             return same && a != b && b.nick == a.nick && b.name == text(1)\n\
         }\n\
         fun messages(): Bool {\n\
             val a = Message.Text(text(1), 2); val b = a.clone(); val e = Message.Empty; val f = e.clone()\n\
             return a == b && e == f\n\
         }\n\
         fun nullables(): Bool {\n\
             val a: String? = text(1); val b = a.clone(); val n: String? = null; val m = n.clone()\n\
             return a == b && m == null\n\
         }\n\
         fun strings(): Bool { var a = text(1); val b = a.clone(); a = text(2); return a != b }\n",
    );
    for name in ["users", "messages", "nullables", "strings"] {
        let test: ToBool = function(&program, &format!("main::{}", name));
        checked(|| assert_eq!(test(), 1, "{}", name));
    }
}

#[test]
fn top_level_values_are_released_before_the_check() {
    let program = compile(
        "val NAME: String = 4.toString() + \"2\"\n\
         fun read(): Int = NAME.length + NAME.length\n",
    );
    let read: ToInt = function(&program, "main::read");
    let _lock = prism_runtime::test_lock();
    prism_runtime::start(Vec::new(), true);
    assert_eq!(read(), 4);
    assert_eq!(allocation_statistics().live_allocations, 1);
    // `finish` drops the registered value, leaving nothing to report.
    assert_eq!(prism_runtime::finish(), 0);
    assert_eq!(allocation_statistics().live_allocations, 0);
    prism_runtime::start(Vec::new(), false);
}

#[test]
fn a_leak_of_compiled_code_is_reported() {
    // The scenario "A leak is reported", through compiled code: the check itself must catch a
    // value that nothing drops, so that every fixture passing it means something. Rust plays
    // the caller that forgets the returned string.
    let program = compile(
        "fun text(): String = 7.toString() + \"!!!!\"\n\
         struct User { val name: String, val id: Int }\n\
         fun user(): User = User(text(), 1)\n",
    );
    let text: StringResult = function(&program, "main::text");
    let user: extern "C" fn(*mut [u64; 5]) = function(&program, "main::user");
    let _lock = prism_runtime::test_lock();
    prism_runtime::start(Vec::new(), true);
    let leaked = returned(|r| text(r));
    assert_eq!((leaked.as_str(), leaked.capacity), ("7!!!!", 5));
    let mut forgotten = [0u64; 5];
    user(&mut forgotten);
    let statistics = allocation_statistics();
    assert_eq!(
        (statistics.live_allocations, statistics.live_bytes),
        (2, 10)
    );
    assert_eq!(
        prism_runtime::finish(),
        prism_runtime::ALLOCATION_CHECK_EXIT_STATUS
    );
    drop_string(leaked);
    // SAFETY: the user's name is at offset 0 (fields are ordered by alignment, then
    // declaration): a string, which nothing else frees.
    unsafe { prism_rt_string_drop(forgotten.as_mut_ptr().cast()) };
    assert_eq!(allocation_statistics().live_allocations, 0);
    prism_runtime::start(Vec::new(), false);
}

#[test]
fn payloads_are_read_at_their_variant_offsets() {
    // `Wide`'s payloads follow the tag at offsets 8 and 16, `Small`'s at 1 (D17 of
    // `payload-patterns`).
    let program = compile(
        "enum Value { Small(Bool), Wide(Int, Int), Empty }\n\
         fun make(n: Int): Value = if (n < 0) Value.Small(n < -5) else if (n == 0) Value.Empty else Value.Wide(n, n * 10)\n\
         fun read(n: Int): Int = when (make(n)) { Value.Wide(val a, val b) -> a + b; Value.Small(true) -> -1; Value.Small(false) -> -2; Value.Empty -> 0 }\n\
         struct Pair { val first: Int, val second: Int }\n\
         fun sum(n: Int): Int { val (second, first) = Pair(n, n + 1); return first * 100 + second }\n",
    );
    let read: IntToInt = function(&program, "main::read");
    assert_eq!([read(3), read(0), read(-9), read(-1)], [33, 0, -1, -2]);
    let sum: IntToInt = function(&program, "main::sum");
    assert_eq!(sum(4), 405);
}

// Boxes (D3 and D12 to D14 of `heap-boxes`).

const BOXES: &str = "struct User : Clone { val name: String }\n\
     struct Point : Copy { val x: Int, val y: Int }\n\
     struct Marker : Copy {}\n\
     struct D(val d: Double)\n\
     enum W { Of(Heap<D>) }\n\
     enum Expr : Clone { Lit(Int), Add(Heap<Expr>, Heap<Expr>), Name(String) }\n\
     struct Node : Clone { val value: Int, val next: Heap<Node>? }\n\
     enum Tree : Clone { Leaf, Branch(Heap<Tree>, Int, Heap<Tree>) }\n\
     fun text(n: Int): String = n.toString() + \"!\"\n\
     fun scalar(n: Int): Int { val h = Heap(n); var g = Heap(n); *g += 1; return h + g.unbox() }\n\
     fun point(): Int { val h = Heap(Point(1, 2)); val p = h.unbox(); return p.x + p.y }\n\
     fun string(): Int { var h = Heap(text(12)); val n = h.length; *h = text(1); return n + h.length }\n\
     fun unboxed(): Int { val s = Heap(text(12)).unbox(); return s.length }\n\
     fun user(): Int { val h = Heap(User(text(5))); val c = h.clone(); val u = h.unbox(); return u.name.length + c.name.length }\n\
     fun marker(): Int { val h = Heap(Marker()); val g = h.clone(); val m = g.unbox(); return 1 }\n\
     fun eval(e: &Expr): Int = when (e) { Expr.Lit(val n) -> n; Expr.Add(val l, val r) -> eval(l) + eval(r); Expr.Name(val s) -> s.length }\n\
     fun expr(): Int {\n\
         val e = Expr.Add(Heap(Expr.Lit(1)), Heap(Expr.Add(Heap(Expr.Name(text(1))), Heap(Expr.Lit(3)))))\n\
         val c = e.clone()\n\
         return eval(&e) * 10 + eval(&c)\n\
     }\n\
     fun Node.total(): Int = value + (next?.total() ?: 0)\n\
     fun list(n: Int): Int {\n\
         var l: Heap<Node>? = null\n\
         var i = 1\n\
         while (i <= n) { l = Heap(Node(i, l)); i += 1 }\n\
         val c = l.clone()\n\
         l = null\n\
         return c?.total() ?: 0\n\
     }\n\
     fun Tree.size(): Int = when (this) { Tree.Leaf -> 0; Tree.Branch(val l, _, val r) -> 1 + l.size() + r.size() }\n\
     fun tree(depth: Int): Tree = if (depth == 0) Tree.Leaf else Tree.Branch(Heap(tree(depth - 1)), depth, Heap(tree(depth - 1)))\n\
     fun trees(): Int { val t = tree(4); val c = t.clone(); return t.size() + c.size() }\n\
     fun equal(): Bool { val a = tree(3); val b = tree(3); val c = tree(2); return a == b && a != c && a.clone() == a }\n\
     fun exprs(): Bool { val a = Expr.Add(Heap(Expr.Lit(1)), Heap(Expr.Name(text(1)))); val b = Expr.Add(Heap(Expr.Lit(1)), Heap(Expr.Name(text(2)))); return a == a.clone() && a != b }\n\
     fun nan(): Bool { val x = 0.0 / 0.0; return W.Of(Heap(D(x))) != W.Of(Heap(D(x))) }\n";

#[test]
fn boxes_hold_and_give_back_their_contents() {
    let program = compile(BOXES);
    let scalar: IntToInt = function(&program, "main::scalar");
    let point: ToInt = function(&program, "main::point");
    let string: ToInt = function(&program, "main::string");
    let unboxed: ToInt = function(&program, "main::unboxed");
    let user: ToInt = function(&program, "main::user");
    let made = checked(|| {
        assert_eq!(scalar(20), 41);
        assert_eq!(point(), 3);
        assert_eq!(string(), 5);
        assert_eq!(unboxed(), 3);
        assert_eq!(user(), 4);
    });
    assert!(made > 10, "{}", made);
}

#[test]
fn a_box_of_a_zero_sized_value_allocates_nothing() {
    // The scenario "A box of a zero-sized value allocates nothing" of the `code-generation`
    // spec.
    let program = compile(BOXES);
    let marker: ToInt = function(&program, "main::marker");
    let made = checked(|| assert_eq!(marker(), 1));
    assert_eq!(made, 0);
}

#[test]
fn recursive_values_are_dropped_and_cloned_once() {
    let program = compile(BOXES);
    let expr: ToInt = function(&program, "main::expr");
    let list: IntToInt = function(&program, "main::list");
    let trees: ToInt = function(&program, "main::trees");
    checked(|| {
        assert_eq!(expr(), 66);
        assert_eq!(list(100), 5050);
        assert_eq!(trees(), 30);
    });
}

#[test]
fn boxes_compare_their_contents() {
    let program = compile(BOXES);
    for name in ["equal", "exprs", "nan"] {
        let test: ToBool = function(&program, &format!("main::{}", name));
        checked(|| assert_eq!(test(), 1, "{}", name));
    }
}

/// A struct of `width` `Int` fields named `f0`, ..., built by `make(n)` with every field `n`
/// except the last, which is `last`.
fn wide_struct(width: usize) -> String {
    let fields: Vec<String> = (0..width)
        .map(|index| format!("val f{}: Int", index))
        .collect();
    let mut arguments = vec!["n"; width];
    arguments[width - 1] = "last";
    format!(
        "struct Wide : Copy {{ {} }}\nfun make(n: Int, last: Int): Wide = Wide({})\n",
        fields.join(", "),
        arguments.join(", ")
    )
}

#[test]
fn large_aggregates_compare_like_their_full_expansion() {
    // The scenario "Equality of a large aggregate" of the `code-generation` spec, over parts
    // beyond the inline budget in every position: a struct field, an enum payload, a nullable's
    // payload, a referent and a box's contents.
    let source = format!(
        "{}\
         struct Holder {{ val name: String, val wide: Wide, val count: Int? }}\n\
         enum Choice : Copy {{ Empty, One(Wide), Two(Wide, Double) }}\n\
         fun holder(name: String, last: Int, count: Int?): Holder = Holder(name, make(1, last), count)\n\
         fun holders(): Bool = holder(\"a\", 2, 3) == holder(\"a\", 2, 3) && holder(\"a\", 2, 3) != holder(\"a\", 4, 3)\n\
             && holder(\"a\", 2, 3) != holder(\"b\", 2, 3) && holder(\"a\", 2, null) != holder(\"a\", 2, 3)\n\
         fun choices(): Bool = Choice.One(make(1, 2)) == Choice.One(make(1, 2)) && Choice.One(make(1, 2)) != Choice.One(make(1, 3))\n\
             && Choice.Two(make(1, 2), 0.5) != Choice.Two(make(1, 2), 1.5) && Choice.Two(make(1, 2), 0.5) != Choice.One(make(1, 2))\n\
             && Choice.Empty == Choice.Empty && Choice.Two(make(1, 2), 0.0 / 0.0) != Choice.Two(make(1, 2), 0.0 / 0.0)\n\
         fun optional(a: Wide?, b: Wide?): Bool = a == b\n\
         fun optionals(): Bool = optional(null, null) && !optional(make(1, 2), null) && !optional(null, make(1, 2))\n\
             && optional(make(1, 2), make(1, 2)) && !optional(make(1, 2), make(1, 3))\n\
         fun same(a: &Wide, b: &Wide): Bool = a == b\n\
         fun references(): Bool = same(&make(1, 2), &make(1, 2)) && !same(&make(1, 2), &make(2, 2))\n\
         fun boxes(): Bool = Heap(make(1, 2)) == Heap(make(1, 2)) && Heap(make(1, 2)) != Heap(make(1, 3))\n",
        wide_struct(40)
    );
    let program = compile(&source);
    for name in ["holders", "choices", "optionals", "references", "boxes"] {
        let test: ToBool = function(&program, &format!("main::{}", name));
        checked(|| assert_eq!(test(), 1, "{}", name));
    }
}

use tulpa::run_captured;

fn run(src: &str) -> String {
    match run_captured(src) {
        Ok(s) => s,
        Err(e) => panic!("program failed: {e}\n--- source ---\n{src}"),
    }
}

fn fails(src: &str) -> String {
    match run_captured(src) {
        Ok(out) => panic!("expected failure, got output: {out}"),
        Err(e) => e,
    }
}

#[test]
fn arithmetic_and_precedence() {
    assert_eq!(
        run("print(5 * 10 + 3, 2 ** 3 ** 2, -2 ** 2, 7 / 2, 7.0 / 2, 7 % 3, -7 % 3)"),
        "53 512 -4 3 3.5 1 2\n"
    );
    assert_eq!(
        run("print(1 + 2 * 3 == 7 and not false, 0x10 | 0b1, 1 << 4, 0xff >> 4, 6 & 3, 6 ^ 3, ~0)"),
        "true 17 16 15 2 5 -1\n"
    );
}

#[test]
fn strings_and_fstrings() {
    assert_eq!(
        run(r#"let n = "Tulpa"; print("Hi " + n + "!", n.upper(), len(n), n[0], n[-1], n[1..3])"#),
        "Hi Tulpa! TULPA 5 T a ul\n"
    );
    assert_eq!(
        run(r#"let v = 3.14159; print(f"{v:.2f}|{255:#x}|{7:03d}|{"ab":>4}|{{lit}}|{1 + 1}")"#),
        "3.14|0xff|007|  ab|{lit}|2\n"
    );
    assert_eq!(
        run(r#"print("a,b,c".split(","), "-".join([1, 2, 3]), "  pad ".strip())"#),
        "[\"a\", \"b\", \"c\"] 1-2-3 pad\n"
    );
}

#[test]
fn semicolons_are_optional_and_newlines_end_calls() {
    let src = "let a = 1\nlet b = 2;\nprint(a + b)\n(a)\n";
    assert_eq!(run(src), "3\n");
}

#[test]
fn functions_closures_defaults_recursion() {
    let src = r#"
fn make_counter() {
    let n = 0
    fn() { n += 1; n }
}
let c = make_counter()
c(); c()
fn fib(n) { if n < 2 { return n }; fib(n - 1) + fib(n - 2) }
fn greet(name, greeting = "Hello") { f"{greeting}, {name}" }
print(c(), fib(20), greet("Ant"), greet("Ant", "Yo"))
"#;
    assert_eq!(run(src), "3 6765 Hello, Ant Yo, Ant\n");
}

#[test]
fn functions_are_hoisted() {
    assert_eq!(run("print(later(2))\nfn later(x) { x * 21 }"), "42\n");
}

#[test]
fn lambdas_and_higher_order() {
    let src = r#"
let xs = [5, 3, 8, 1]
print(map(|x| x * 2, xs), xs.filter(fn(x) => x > 2), reduce(|a, b| a + b, xs, 0), sorted(xs), sorted(xs, |x| -x))
"#;
    assert_eq!(
        run(src),
        "[10, 6, 16, 2] [5, 3, 8] 17 [1, 3, 5, 8] [8, 5, 3, 1]\n"
    );
}

#[test]
fn control_flow() {
    let src = r#"
let out = []
for i in 0..10 {
    if i % 2 == 0 { continue }
    if i > 7 { break }
    out.push(i)
}
let j = 0
while true { j += 1; if j == 4 { break } }
let k = 0
loop { k += 2; if k >= 6 { break } }
for i in 1..=3 { out.push(i * 100) }
let grade = if j > 3 { "big" } else { "small" }
print(out, j, k, grade)
if j == 1 { print("one") } elif j == 4 { print("four") } else if j == 5 { print("five") } else { print("other") }
"#;
    assert_eq!(run(src), "[1, 3, 5, 7, 100, 200, 300] 4 6 big\nfour\n");
}

#[test]
fn collections() {
    let src = r#"
let d = {"a": 1, b: 2}
d["c"] = 3
d.b += 10
let l = [0; 3]
l[1] = 9
l[-1] = 7
let a, b = [1, 2]
print(d, d.keys(), d.get("zz", "none"), "a" in d, l, a + b, [1, 2] + [3], [0] * 3)
for k, v in d { print(k, v) }
"#;
    assert_eq!(
        run(src),
        "{\"a\": 1, \"b\": 12, \"c\": 3} [\"a\", \"b\", \"c\"] none true [0, 9, 7] 3 [1, 2, 3] [0, 0, 0]\na 1\nb 12\nc 3\n"
    );
}

#[test]
fn pattern_matching() {
    let src = r#"
fn kind(v) {
    match v {
        0 => "zero",
        1 | 2 => "one-or-two",
        3..=9 => "single digit",
        "hi" => "greeting",
        [] => "empty",
        [x] => f"one item {x}",
        [x, y, ..rest] => f"{x},{y} +{len(rest)}",
        n if n > 100 => "huge",
        _ => "other",
    }
}
print(kind(0), kind(2), kind(5), kind("hi"), kind([]), kind([4]), kind([1, 2, 3, 4]), kind(500), kind(50))
"#;
    assert_eq!(
        run(src),
        "zero one-or-two single digit greeting empty one item 4 1,2 +2 huge other\n"
    );
}

#[test]
fn contracts_pass_and_fail() {
    let ok = r#"
fn safe_div(a, b)
    require b != 0
    ensure result' * b == a
{ a / b }
print(safe_div(10, 2))
"#;
    assert_eq!(run(ok), "5\n");
    let pre = fails("fn f(x) require x > 0 { x }\nf(-1)");
    assert!(
        pre.contains("precondition violated in f(): require x > 0"),
        "{pre}"
    );
    let post = fails("fn bad(x) ensure result' > x { x - 1 }\nbad(3)");
    assert!(
        post.contains("postcondition violated in bad(): ensure result' > x"),
        "{post}"
    );
}

#[test]
fn primed_parameters_see_final_state() {
    let src = r#"
fn bump(x)
    ensure x' == x + 1
{ x = x + 1; x }
print(bump(41))
"#;
    assert_eq!(run(src), "42\n");
}

#[test]
fn loop_invariants() {
    assert_eq!(
        run("let i = 0\nwhile i < 3 invariant i <= 3 { i += 1 }\nprint(i)"),
        "3\n"
    );
    let e = fails("let i = 0\nwhile i < 5 invariant i < 3 { i += 1 }");
    assert!(e.contains("loop invariant violated: i < 3"), "{e}");
}

#[test]
fn asserts_and_try_catch() {
    assert_eq!(
        run("try { assert 1 == 2, \"math\" } catch e { print(e) }"),
        "assertion failed: 1 == 2: math\n"
    );
    assert_eq!(
        run("try { [1][5] } catch e { print(e) }\nprint(\"after\")"),
        "index 5 out of range for length 1\nafter\n"
    );
}

#[test]
fn constants_cannot_be_reassigned() {
    let e = fails("const A = 1\nA = 2");
    assert!(e.contains("cannot assign to constant 'A'"), "{e}");
}

#[test]
fn user_and_builtin_macros() {
    let src = r#"
define_macro!(repeat_message, (times, msg) {
    simple_loop!(times, { print(msg) })
})
repeat_message!(2, "hey")
let n = 0
define_macro!(inc_twice, (target) { n += target; n += target })
inc_twice!(5)
print(n, format!("{}-{:?}-{:.1}", 1, "s", 2.25), vec!(1, 2))
assert_eq!(1 + 1, 2)
"#;
    assert_eq!(run(src), "hey\nhey\n10 1-\"s\"-2.2 [1, 2]\n");
}

#[test]
fn macro_args_are_call_by_name() {
    // The block argument is re-evaluated each time the parameter is referenced.
    let src = "let c = 0\ndefine_macro!(thrice, (b) { b; b; b })\nthrice!({ c += 1 })\nprint(c)";
    assert_eq!(run(src), "3\n");
}

#[test]
fn tasks_spawn_and_await() {
    let src = r#"
fn work(x) { print("working", x); x * 10 }
let t = spawn work(1)
let u = spawn work(2)
print("spawned")
print(await u, await t, await [t, u])
spawn work(3)
"#;
    assert_eq!(
        run(src),
        "spawned\nworking 2\nworking 1\n20 10 [10, 20]\nworking 3\n"
    );
}

#[test]
fn every_macro_runs_fixed_count() {
    assert_eq!(
        run("let n = 0\nlet r = every!(1, 4, { n += 1 })\nprint(n, r)"),
        "4 4\n"
    );
}

#[test]
fn kinematics_round_trip() {
    let src = r#"
let a = kin::ik2(10, 7.5, 12, 5)
let p = kin::fk2(10, 7.5, a[0], a[1])
print(round(p[0], 6), round(p[1], 6), round(kin::velocity(0, 9.81, 2), 2), round(kin::range(20, 45), 3))
"#;
    assert_eq!(run(src), "12.0 5.0 19.62 40.775\n");
}

#[test]
fn byte_helpers() {
    assert_eq!(
        run("print(int_from_bytes([0xff, 0xfe], \"big\", true), int_from_bytes([0x34, 0x12], \"little\"), int_to_bytes(0x1234, 2), hex(255), int(\"0x1f\"), decode([72, 105]))"),
        "-2 4660 [18, 52] 0xff 31 Hi\n"
    );
}

#[test]
fn runtime_errors_have_lines_and_traces() {
    let e = fails("fn inner() { undefined_thing }\nfn outer() { inner() }\nouter()");
    assert!(
        e.contains("line 1: undefined variable 'undefined_thing'"),
        "{e}"
    );
    assert!(e.contains("at inner()") && e.contains("at outer()"), "{e}");
    assert!(fails("let x = 1\ny = 2").contains("undeclared variable 'y'"));
    assert!(fails("print(1 / 0)").contains("division by zero"));
    assert!(fails("fn f(a) { a }\nf()").contains("f() takes 1 argument, got 0"));
    assert!(fails("fn r() { r() }\nr()").contains("maximum recursion depth"));
}

#[test]
fn syntax_errors_report_position() {
    let e = run_captured("let x = (1 + \nprint(x)").unwrap_err();
    assert!(e.starts_with("parse error at"), "{e}");
    let e = run_captured("let s = \"never closed").unwrap_err();
    assert!(e.contains("unterminated string"), "{e}");
}

#[test]
fn comments_all_styles() {
    assert_eq!(
        run("# hash\n// slashes\n/* block\n comment */ print(1) // trailing"),
        "1\n"
    );
}

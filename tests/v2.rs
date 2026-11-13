//! Tulpa 2.0 features: units, types, kwargs, pipe, structs, machines, signals, rules,
//! monitors, modules, contract fuzzing, PWM/Servo, record/replay.

use tulpa::run_captured;

fn run(src: &str) -> String {
    run_captured(src).unwrap_or_else(|e| panic!("program failed: {e}\n--- source ---\n{src}"))
}

fn fails(src: &str) -> String {
    match run_captured(src) {
        Ok(out) => panic!("expected failure, got output: {out}"),
        Err(e) => e,
    }
}

// ------------------------------------------------------------------ units

#[test]
fn unit_arithmetic_and_conversion() {
    assert_eq!(
        run("print(2[m] + 50[cm], 120[m] / 10[s], (36[km] / 1[h]).to(\"m/s\"), 3[N] * 2[m])"),
        "2.5 m 12 m/s 10 m/s 6 J\n"
    );
    assert_eq!(
        run(
            "print(1[rev].in(\"deg\"), 90[deg] * 2, round(sin(30[deg]), 6), sqrt(9[m^2]), -2[m/s])"
        ),
        "360.0 180 deg 0.5 3 m -2 m/s\n"
    );
    assert_eq!(
        run("print(5[m] / 2[m], type(5[m] / 2[m]), 1[kHz].in(\"Hz\"), 2200[mAh] / 2200[mAh])"),
        "2.5 float 1000.0 1.0\n"
    );
    assert_eq!(
        run("print(f\"{12.3456[V]:.2f}|{3[m]:?}\", 10[cm] < 1[m], 0[m] == 0[cm], abs(-3[A]))"),
        "12.35 V|3 m true true 3 A\n"
    );
}

#[test]
fn unit_errors() {
    assert!(fails("print(2[m] + 3[s])")
        .contains("unit mismatch: cannot apply '+' to length [m] and time [s]"));
    assert!(fails("print(2[m/s] < 3)").contains("cannot compare speed [m/s]"));
    assert!(fails("print(3[m].to(\"s\"))").contains("cannot convert length"));
    assert!(fails("print(sin(3[m]))").contains("sin() expects an angle"));
    assert!(run_captured("print(1[furlong])")
        .unwrap_err()
        .contains("unknown unit 'furlong'"));
    assert!(fails("print(float(3[m]))").contains("ambiguous"));
    // comparing against bare zero is allowed
    assert_eq!(run("print(3[m/s] > 0, -1[V] < 0)"), "true true\n");
}

#[test]
fn units_in_time_apis() {
    assert_eq!(
        run("let n = every!(2[ms], 3, { })\nsleep(1[ms])\nsleep_ms(1[ms])\nprint(n)"),
        "3\n"
    );
}

// ------------------------------------------------------------------ types, kwargs, pipe

#[test]
fn type_annotations_are_enforced() {
    let src = "fn f(a: int, b: str = \"x\", c: [m]? = nil) -> str { b * a }\nprint(f(2), f(3, \"ab\"), f(1, c: 2[m]))";
    assert_eq!(run(src), "xx ababab x\n");
    assert!(fails("fn f(a: int) { a }\nf(1.5)")
        .contains("f() parameter 'a' expects int, got float 1.5"));
    assert!(fails("fn f(v: [m/s]) { v }\nf(2)").contains("did you mean 2[m/s]?"));
    assert!(fails("fn f() -> int { \"no\" }\nf()").contains("f() return value expects int"));
    assert!(fails("struct P { x: int }\nP(\"a\")").contains("P.x expects int"));
}

#[test]
fn keyword_arguments() {
    let src = "fn area(w, h = 1, scale = 1) { w * h * scale }\nprint(area(2, scale: 10), area(h: 3, w: 4), area(1, 2, scale: 3))";
    assert_eq!(run(src), "20 12 6\n");
    assert!(fails("fn f(a, b) { a }\nf(1, c: 2)").contains("f() has no parameter 'c'"));
    assert!(fails("fn f(a, b) { a }\nf(1, a: 2)").contains("got multiple values for 'a'"));
    assert!(fails("fn f(a, b) { a }\nf(b: 2)").contains("missing argument 'a'"));
    assert!(fails("print(len(x: 1))").contains("does not take keyword arguments"));
}

#[test]
fn pipe_operator() {
    let src = "let xs = [3, 1, 2]\nprint(xs |> sorted, xs |> map(|x| x * x) |> sum, 5 |> str |> len, [\"a\", \"b\"] |> \"-\".join)";
    assert_eq!(run(src), "[1, 2, 3] 14 1 a-b\n");
}

// ------------------------------------------------------------------ structs

#[test]
fn structs_fields_methods_and_overloading() {
    let src = r#"
struct V { x = 0, y = 0
    fn op_add(self, o) { V(self.x + o.x, self.y + o.y) }
    fn op_mul(self, k) { V(self.x * k, self.y * k) }
    fn op_eq(self, o) { self.x == o.x and self.y == o.y }
    fn op_cmp(self, o) { self.norm2() - o.norm2() }
    fn norm2(self) { self.x * self.x + self.y * self.y }
    fn to_str(self) { f"({self.x},{self.y})" }
    fn unit_x() { V(1, 0) }
}
let a = V(1, 2)
let b = V(y: 5)
print(a + b, a * 3, 3 * a, a == V(1, 2), a != b, a < b, V.unit_x(), str(a), V.fields())
a.x = 10
print(a, a.norm2())
"#;
    assert_eq!(
        run(src),
        "(1,7) (3,6) (3,6) true true true (1,0) (1,2) [\"x\", \"y\"]\n(10,2) 104\n"
    );
}

#[test]
fn struct_invariants_roll_back() {
    let src = r#"
struct Tank {
    level = 0
    cap = 10
    invariant 0 <= self.level and self.level <= self.cap
    fn fill(self, n) { self.level += n }
}
let t = Tank()
t.fill(7)
try { t.fill(5) } catch e { print("rejected") }
try { t.level = -1 } catch e { print("rejected") }
print(t.level)
try { Tank(level: 99) } catch e { print(e) }
"#;
    let out = run(src);
    assert!(
        out.starts_with("rejected\nrejected\n7\nconstructing Tank: struct invariant violated"),
        "{out}"
    );
}

#[test]
fn struct_errors() {
    assert!(fails("struct P { x }\nP()").contains("missing field 'x'"));
    assert!(fails("struct P { x }\nP(1, 2)").contains("at most 1 field"));
    assert!(fails("struct P { x }\nP(z: 1)").contains("P has no field 'z'"));
    assert!(fails("struct P { x }\nP(1).nope()").contains("P has no method 'nope'"));
}

// ------------------------------------------------------------------ state machines

const LIGHT: &str = r#"
machine Light(brightness = 0) {
    on cut => Off { brightness = 0 }
    state Off {
        on on => On { brightness = 100 }
    }
    state On {
        enter { print("enter On") }
        exit { print("exit On") }
        on dim(n) if n < brightness { brightness = n }
        on off => Off
    }
}
"#;

#[test]
fn machine_transitions_guards_and_internal() {
    let src = format!(
        "{LIGHT}\nlet l = Light()\nprint(l.state, l.on(), l.state, l.brightness)\nprint(l.dim(30), l.brightness, l.dim(90), l.can(\"dim\", 10), l.events())\nl.cut()\nprint(l.state, l.history, l.send(\"nope\"))"
    );
    assert_eq!(
        run(&src),
        "enter On\nOff true On 100\ntrue 30 false true [\"dim\", \"off\", \"cut\"]\nexit On\nOff [\"Off\", \"On\", \"Off\"] false\n"
    );
}

#[test]
fn machine_params_kwargs_and_mermaid() {
    let src = format!("{LIGHT}\nlet l = Light(brightness: 5)\nprint(l.brightness)\nl.brightness = 7\nprint(l.brightness)\nprint(Light.mermaid())");
    let out = run(&src);
    assert!(
        out.starts_with("5\n7\nstateDiagram-v2\n    [*] --> Off\n    Off --> On : on\n"),
        "{out}"
    );
    assert!(
        out.contains("On --> On : dim(n) [guarded]") && out.contains("On --> Off : cut"),
        "{out}"
    );
}

#[test]
fn machine_bad_target_is_a_syntax_error() {
    let e = run_captured("machine M { state A { on go => Nowhere } }").unwrap_err();
    assert!(e.contains("targets unknown state 'Nowhere'"), "{e}");
}

// ------------------------------------------------------------------ signals & rules

#[test]
fn signals_history_derive_subscribe() {
    let src = r#"
let s = signal(1)
let d = derive(|| s.get() * 10)
s.subscribe(|new, old| print("change", old, "->", new))
s.set(4)
s.value = 7
print(s.get(), s.prev(), s.history(), s.avg(), s.min(), s.max(), d.get(), d.value)
try { d.set(1) } catch e { print(e) }
"#;
    assert_eq!(
        run(src),
        "change 1 -> 4\nchange 4 -> 7\n7 4 [1, 4, 7] 4.0 1 7 70 70\ncannot set a derived signal\n"
    );
}

#[test]
fn when_rules_are_edge_triggered() {
    let src = r#"
let level = signal(0)
let x = 0
when level.get() > 5 { print("high") } else { print("low") }
when x == 2 { print("x hit 2") }
for v in [1, 6, 7, 3, 9] { level.set(v) }
for i in 0..4 { x += 1 }
"#;
    assert_eq!(run(src), "high\nlow\nhigh\nx hit 2\n");
}

#[test]
fn always_monitors_handle_or_fail() {
    let src = r#"
let speed = 0
always speed <= 10 else { print("clamped"); speed = 10 }
speed = 4
speed = 50
print(speed)
"#;
    assert_eq!(run(src), "clamped\n10\n");
    let e = fails("let led = GPIO::new(1)\nalways led.read() == 0\nled.set_high()");
    assert!(
        e.contains("safety envelope violated: always led.read() == 0"),
        "{e}"
    );
}

#[test]
fn runaway_rules_are_detected() {
    let e = fails("let s = signal(0)\nwhen s.get() >= 0 { s.set(-1) } else { s.set(1) }");
    assert!(e.contains("did not settle"), "{e}");
}

// ------------------------------------------------------------------ modules

#[test]
fn imports_and_private_names() {
    let src = "import \"examples/modules/lib/drive.rav\" as d\nfrom \"examples/modules/lib/drive\" import Twist\nprint(d.TRACK_WIDTH, d.wheels(Twist(linear: 1[m/s])), \"_calls\" in d)";
    assert_eq!(run(src), "0.18 m [1 m/s, 1 m/s] false\n");
    assert!(fails("import \"nope/missing.rav\"").contains("import \"nope/missing.rav\""));
    assert!(fails("from \"examples/modules/lib/drive\" import nothing")
        .contains("has no export 'nothing'"));
}

// ------------------------------------------------------------------ contract fuzzing

#[test]
fn check_finds_and_shrinks_counterexamples() {
    let src = r#"
fn ok_fn(a: int, b: int) require b != 0 ensure result' * b + a % b == a { a / b }
fn bad(x: int) ensure result' < 1000 { x * 2 }
let r1 = check(ok_fn)
let r2 = check(bad, 500)
print(r1.ok, r1.passed, r2.ok, r2.counterexample)
"#;
    assert_eq!(run(src), "true 200 false [500]\n");
}

#[test]
fn check_handles_units_and_unsatisfiable_requires() {
    let src = "fn f(v: [m/s]) require v > 0 ensure result' > 0[m] { v * 1[s] }\nfn g(x: int) require x > 10 and x < 5 { x }\nlet a = check(f)\nlet b = check(g, 5)\nprint(a.ok, b.ok, b.discarded)";
    assert_eq!(run(src), "true true 5\n");
}

// ------------------------------------------------------------------ devices

#[test]
fn pwm_servo_and_scan() {
    let src = r#"
let p = PWM::new(0, 2)
p.frequency(500[Hz]); p.duty(0.5); p.enable()
let s = Servo::new(0, 3, 1000, 2000, 90)
s.angle(45[deg])
print(p, s.pulse_us(), sim::pwm(0, 3), s.angle())
try { s.angle(120) } catch e { print(e) }
p.pulse(500[us])
print(p.state())
I2C::new(0x40); I2C::new(2, 0x10); sim::i2c_set(0x68, 0, [1])
print(I2C::scan(), I2C::scan(2))
"#;
    assert_eq!(
        run(src),
        "<PWM 0.2 500 Hz duty 50.0%> 1500.0 {\"period_ns\": 20000000, \"duty_ns\": 1500000, \"enabled\": true} 45 deg\nservo angle 120.00 deg outside 0..90 deg\n{\"period_ns\": 2000000, \"duty_ns\": 500000, \"enabled\": true}\n[64, 104] [16]\n"
    );
}

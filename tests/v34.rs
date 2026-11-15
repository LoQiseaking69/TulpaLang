//! Tulpa 3.4: planning (std/plan), middleware (std/link), static proofs (tulpa prove),
//! simulation (std/world, std/emu, std/verify), native `match`, bundles and deploy.

use std::path::PathBuf;
use std::process::Command;
use tulpa::prove::{prove_program, Status};
use tulpa::run_captured;

fn run(src: &str) -> String {
    run_captured(src).unwrap_or_else(|e| panic!("{e}\n---\n{src}"))
}

fn tulpa(args: &[&str]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_tulpa"))
        .args(args)
        .output()
        .unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into(),
        String::from_utf8_lossy(&out.stderr).into(),
    )
}

fn tmpdir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("tulpa-v34-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn have(tool: &str) -> bool {
    Command::new(tool)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

// ------------------------------------------------------------------ std/plan

#[test]
fn behavior_tree_semantics() {
    let out = run(r#"
import "std/plan"
let n = 0
let t = plan.Selector([
    plan.Sequence([plan.Condition("never", |bb| false), plan.Action("skipped", |bb| { n += 100 })]),
    plan.Retry(plan.Action("flaky", |bb| { n += 1; n >= 3 }), 5),
])
print(plan.run_tree(t), n)
let waits = 0
let slow = plan.Sequence([plan.Action("wait", |bb| { waits += 1; if waits < 3 { plan.RUNNING } else { plan.SUCCESS } }),
                          plan.Action("then", |bb| true)])
print(slow.tick(), slow.tick(), slow.tick(), waits)
print(plan.Parallel([plan.Action("a", |bb| true), plan.Action("b", |bb| false), plan.Action("c", |bb| true)], 2).tick())
print(plan.Invert(plan.Condition("c", |bb| bb.x > 1)).tick({x: 5}))
sim::virtual_time(true)
let to = plan.Timeout(plan.Action("hang", |bb| plan.RUNNING), 50[ms])
print(to.tick()); sleep_ms(60); print(to.tick())
"#);
    assert_eq!(
        out,
        "success 3\nrunning running success 3\nsuccess\nfailure\nrunning\nfailure\n"
    );
}

#[test]
fn goap_finds_the_cheapest_plan_and_htn_backtracks() {
    let out = run(r#"
import "std/plan"
let p = plan.Planner()
p.action("walk", {at: "a"}, {at: "b"}, cost: 10)
p.action("call_taxi", {at: "a", cash: |c| c >= 5}, {taxi: true})
p.action("ride", {taxi: true}, {at: "b", taxi: false, cash: |c| c - 5}, cost: 2)
print(p.plan({at: "a", cash: 20, taxi: false}, {at: "b"}))
print(p.plan({at: "a", cash: 0, taxi: false}, {at: "b"}))
print(p.plan({at: "b"}, {at: "b"}), p.plan({at: "c"}, {at: "b"}))
print(p.solve({at: "a", cash: 20, taxi: false}, {at: "b"}).cost)
let d = plan.Domain()
d.primitive("unlock", {key: true}, {open: true})
d.primitive("break_in", {}, {open: true, alarm: true})
d.primitive("enter", {open: true}, {inside: true})
d.method("get_in", "politely", {}, ["unlock", "enter"])
d.method("get_in", "forcefully", {}, ["break_in", "enter"])
print(d.decompose({key: true}, ["get_in"]), d.decompose({key: false}, ["get_in"]))
print(plan.choose([{name: "a", score: |c| 1}, {name: "b", score: |c| 2, active: |c| c}], false).name)
"#);
    assert_eq!(
        out,
        "[\"call_taxi\", \"ride\"]\n[\"walk\"]\n[] nil\n3\n[\"unlock\", \"enter\"] [\"break_in\", \"enter\"]\na\n"
    );
}

#[test]
fn mission_preempts_and_replans() {
    let out = run(r#"
import "std/plan"
let p = plan.Planner()
p.action("work", {tired: false}, {done: true})
p.action("rest", {}, {tired: false})
let w = {tired: false, done: false}
let m = plan.Mission(p)
m.goal("job", {done: true}, priority: 1)
m.goal("rested", {tired: false}, priority: 5, active: |w| w.tired)
let n = 0
m.on("work", |w| { n += 1; if n == 1 { w.tired = true; return false }; w.done = true })
m.on("rest", |w| { w.tired = false })
m.run(w, 0, 50)
print(m.done, w, m.replans)
"#);
    assert_eq!(
        out,
        "[\"rested\", \"job\"] {\"tired\": false, \"done\": true} 4\n"
    );
}

// ------------------------------------------------------------------ std/link

#[test]
fn link_reliable_topics_survive_loss_in_order() {
    let out = run(r#"
import "std/link"
sim::virtual_time(true)
link.network(latency: 1[ms], loss: 0.3, seed: 3)
let a = link.node("a")
let b = link.node("b")
let t = a.advertise("/n", "int", reliable: true, keep: 64)
let got = []
b.subscribe("/n", "int", |v, m| got.push(v), reliable: true)
let loose = link.node("c").subscribe("/n", "int")
link.spin_for(600[ms])
for i in 0..50 { t.publish(i); link.spin_for(5[ms]) }
link.spin_for(800[ms])
print(len(got), got == list(0..50), loose.count < 50, loose.count > 10)
print(link.network().dropped > 0)
"#);
    assert_eq!(out, "50 true true true\ntrue\n");
}

#[test]
fn link_checks_units_across_the_wire() {
    let out = run(r#"
import "std/link"
sim::virtual_time(true)
let a = link.node("a")
let b = link.node("b")
let v = a.advertise("/v", "[m/s]")
let pose = a.advertise("/pose", {x: "[m]", tags: ["str"]}, latched: true)
let kmh = b.subscribe("/v", "[km/h]")
let anyv = b.subscribe("/pose")
let events = []
let c = link.node("c")
c.on_event(|e| { if e.kind == "incompatible" { events.push(e.topic) } })
c.subscribe("/v", "[m]")
link.spin_for(600[ms])
v.publish(10[m/s])
pose.publish({x: 2[m], tags: ["home"]})
link.spin_for(20[ms])
print(kmh.latest, anyv.latest, events)
try { v.publish(3[m]) } catch e { print(e) }
try { pose.publish({x: 1[m]}) } catch e { print(e) }
try { kmh.publish(1[m/s]) } catch e { print(e) }
"#);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines[0],
        "36 km/h {\"x\": 2 m, \"tags\": [\"home\"]} [\"/v\"]"
    );
    assert!(
        lines[1].contains("expected a quantity in [m/s], got a quantity in [m]"),
        "{out}"
    );
    assert!(lines[2].contains("missing field 'tags'"), "{out}");
    assert!(lines[3].contains("call advertise()"), "{out}");
}

#[test]
fn link_services_params_and_liveliness() {
    let out = run(r#"
import "std/link"
sim::virtual_time(true)
link.network(loss: 0.4, seed: 9)
let s = link.node("server")
let c = link.node("client")
let calls = 0
s.serve("/sq", |x| { calls += 1; if x < 0 { error("negative") }; x * x })
s.param("gain", 2.5)
let lost = []
c.on_event(|e| { if e.kind == "peer_lost" { lost.push(e.peer) } })
link.spin_for(800[ms])
print(c.call("/sq", 7), calls, c.get_param("server", "gain"), c.set_param("server", "gain", 4.0), s.get("gain"))
try { c.call("/sq", -1) } catch e { print(e) }
try { c.call("/nope", 1, 200[ms]) } catch e { print(e) }
try { c.set_param("server", "gain", "high") } catch e { print("rejected") }
link.partition(s, c)
link.spin_for(3[s])
print(lost, c.peers())
"#);
    assert_eq!(
        out,
        "49 1 2.5 4.0 4.0\nlink: service /sq failed: negative\nlink: no node serves /nope\nrejected\n[\"server\"] []\n"
    );
}

#[test]
fn link_udp_between_processes() {
    let d = tmpdir("udp");
    let domain = 100 + std::process::id() % 90;
    let pubf = d.join("pub.rav");
    let subf = d.join("sub.rav");
    std::fs::write(
        &pubf,
        format!(
            "import \"std/link\"\nlet n = link.node(\"talker\", domain: {domain}, transport: \"udp\")\nlet t = n.advertise(\"/x\", \"[m]\")\nn.serve(\"/inc\", |v| v + 1)\nfor i in 0..150 {{ t.publish(i * 1[cm]); link.spin_for(20[ms]) }}\nn.close()\n"
        ),
    )
    .unwrap();
    std::fs::write(
        &subf,
        format!(
            "import \"std/link\"\nlet n = link.node(\"listener\", domain: {domain}, transport: \"udp\")\nlet t = n.subscribe(\"/x\", \"[mm]\")\nlink.spin_for(1500[ms])\nprint(t.count > 5, t.from, n.call(\"/inc\", 1))\nn.close()\n"
        ),
    )
    .unwrap();
    let mut talker = Command::new(env!("CARGO_BIN_EXE_tulpa"))
        .arg(&pubf)
        .spawn()
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(200));
    let (code, out, err) = tulpa(&[subf.to_str().unwrap()]);
    let _ = talker.kill();
    let _ = talker.wait();
    if err.contains("no free UDP port") || err.contains("Operation not permitted") {
        eprintln!("UDP unavailable here; skipping: {err}");
        return;
    }
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "true talker 2\n");
}

// ------------------------------------------------------------------ tulpa prove

const PROOFS: &str = r#"
const V_MAX = 2[m/s]
fn clamp_speed(v: [m/s]) -> [m/s]
    ensure result' >= -V_MAX and result' <= V_MAX
{ if v > V_MAX { V_MAX } elif v < -V_MAX { -V_MAX } else { v } }
fn limit(x: float, lo: float, hi: float) -> float
    require lo <= hi
    ensure lo <= result' and result' <= hi
{ clamp(x, lo, hi) }
fn duty(percent: int) -> int
    require 0 <= percent and percent <= 100
    ensure 0 <= result' and result' <= 255
{ percent * 255 / 100 }
fn ramp(target: float, current: float, step: float) -> float
    require step > 0.0
    ensure abs(result' - current) <= step
{ current + clamp(target - current, -step, step) }
fn sum_to(n: int) -> int
    require n >= 0
    ensure result' >= 0
{
    let i = 0
    let total = 0
    while i < n invariant total >= 0 and i >= 0 { total += i; i += 1 }
    total
}
fn uses(x: float) -> float ensure result' <= 10.0 { limit(x, 0.0, 10.0) }
fn risky(a: float, b: float) -> float { a / (b - 1.0) }
fn wrong(x: int) -> int ensure result' > x { x }
struct Joint {
    angle: [deg] = 0[deg]
    limit: [deg] = 90[deg]
    invariant self.angle <= self.limit
    fn move_to(self, a: [deg]) { self.angle = min(a, self.limit) }
    fn nudge(self, d: [deg]) { self.angle = self.angle + d }
}
machine Door {
    state Closed { on open => Open }
    state Open { on close => Closed }
    state Ghost { on x => Closed }
}
"#;

#[test]
fn prover_proves_what_holds_and_nothing_else() {
    let prog = tulpa::parse(PROOFS).unwrap();
    let r = prove_program(&prog);
    let status = |ctx: &str, needle: &str| {
        r.obligations
            .iter()
            .find(|o| o.ctx == ctx && o.text.contains(needle))
            .unwrap_or_else(|| panic!("no obligation {ctx} / {needle}"))
            .status
    };
    for (ctx, needle) in [
        ("clamp_speed()", "ensure"),
        ("limit()", "ensure"),
        ("duty()", "ensure"),
        ("duty()", "divisor"),
        ("ramp()", "ensure"),
        ("sum_to()", "ensure"),
        ("sum_to()", "preserved"),
        ("uses()", "limit(): require"),
        ("uses()", "ensure"),
        ("Joint.move_to()", "invariant"),
        ("Joint()", "default fields"),
    ] {
        assert_eq!(status(ctx, needle), Status::Proved, "{ctx}: {needle}");
    }
    assert_eq!(status("risky()", "divisor"), Status::Unproved);
    assert_eq!(status("wrong()", "ensure"), Status::Unproved);
    assert_eq!(status("Joint.nudge()", "invariant"), Status::Unproved);
    assert_eq!(status("machine Door", "reachable"), Status::Refuted);
}

#[test]
fn prove_command_finds_counterexamples() {
    let d = tmpdir("prove");
    let f = d.join("p.rav");
    std::fs::write(&f, PROOFS).unwrap();
    let (code, out, _) = tulpa(&["prove", f.to_str().unwrap()]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("REFUTED   divisor is never zero"), "{out}");
    assert!(out.contains("counterexample: risky("), "{out}");
    assert!(out.contains("REFUTED   ensure result' > x"), "{out}");
    assert!(out.contains("unreachable: Ghost"), "{out}");
    let (_, json, _) = tulpa(&["prove", "--json", f.to_str().unwrap()]);
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(v[0]["proved"].as_u64().unwrap() >= 11, "{json}");
    assert_eq!(v[0]["obligations"][0]["status"], "proved");

    let ok = d.join("ok.rav");
    std::fs::write(
        &ok,
        "fn half(x: int) -> int require x >= 0 ensure result' <= x { x / 2 }\n",
    )
    .unwrap();
    let (code, out, _) = tulpa(&["prove", "--strict", ok.to_str().unwrap()]);
    assert_eq!(code, 0, "{out}");
}

// ------------------------------------------------------------------ simulation & verification

#[test]
fn world_and_emulated_chips_close_the_loop_deterministically() {
    let src = r#"
import "std/world"
import "std/emu"
from "std/drivers/mpu6050" import Mpu6050
from "std/drivers/pca9685" import Pca9685
sim::virtual_time(true)
let w = world.World(dt: 10[ms], seed: 3)
w.arena(4, 2)
let r = w.rover(x: 0.5, y: 1.0)
let chip = emu.Mpu6050(0x68)
let pwm_chip = emu.Pca9685(0x40)
let imu = Mpu6050(I2C::new(1, 0x68))
let pwm = Pca9685(I2C::new(1, 0x40))
imu.init(); pwm.init(); pwm.set_frequency(50[Hz])
pwm.set_pulse(0, 1.8[ms]); pwm.set_pulse(1, 1.6[ms])
let cmd = |ch| (pwm_chip.pulse(ch).in("ms") - 1.5) / 0.5
for i in 0..150 {
    r.drive(cmd(0), cmd(1))
    w.step()
    let s = r.imu()
    chip.set(ax: s.ax, ay: s.ay, az: s.az, gz: s.gz)
    imu.update()
}
print(f"{r.x:.3f} {r.y:.3f} {imu.gyro_z().in("rad/s"):.2f} {r.w:.2f} {millis()}")
print(imu.probe()); chip.fault("offline"); print(imu.probe()); chip.heal(); print(imu.probe())
"#;
    let a = run(src);
    assert_eq!(a, run(src), "simulation is not deterministic");
    let first: Vec<&str> = a.lines().next().unwrap().split(' ').collect();
    // the driver's reading of the emulated chip agrees with the physics it was fed
    let (gyro, truth): (f64, f64) = (first[2].parse().unwrap(), first[3].parse().unwrap());
    assert!((gyro - truth).abs() <= 0.02 && truth.abs() > 0.5, "{a}");
    assert!(first[4].starts_with("150"), "{a}");
    assert!(a.ends_with("true\nfalse\ntrue\n"), "{a}");
}

#[test]
fn explore_finds_the_shortest_violation() {
    let out = run(r#"
import "std/verify"
machine Door {
    state Closed { on open => Open; on lock => Locked }
    state Open { on close => Closed; on lock => Jammed }
    state Locked { on unlock => Closed }
    state Jammed { }
}
let r = verify.explore(|| Door(), ["open", "close", "lock", "unlock"], depth: 6, property: |d| d.state != "Jammed")
print(r.ok, r.counterexample, r.reason)
let ok = verify.explore(|| Door(), ["open", "close"], depth: 6)
print(ok.ok, ok.states, ok.complete)
"#);
    assert_eq!(
        out,
        "false [\"open\", \"lock\"] property violated\ntrue 2 true\n"
    );
}

// ------------------------------------------------------------------ native match, bundles, deploy

#[test]
fn native_match_matches_the_interpreter() {
    if !have("clang") {
        return;
    }
    let d = tmpdir("match");
    let f = d.join("m.rav");
    std::fs::write(
        &f,
        "fn grade(s: int) -> int {\n    match s { 0 => 0, 1 | 2 => 10, 3..=9 => 20, n if n > 100 => n - 100, _ => 30 }\n}\nlet total = 0\nfor s in [0, 2, 5, 50, 150] {\n    total += grade(s)\n    match s { 0 => print(\"zero\"), 150 => print(\"big\"), _ => {} }\n}\nlet k = match total { 110 => 1, _ => 2 }\nprint(total, k)\n",
    )
    .unwrap();
    let exe = d.join("m");
    let (code, _, err) = tulpa(&[
        "build",
        f.to_str().unwrap(),
        "--target",
        "sim",
        "--exe",
        "-o",
        exe.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "{err}");
    assert!(
        !err.contains("hosted bundle"),
        "match should compile natively:\n{err}"
    );
    let native = Command::new(&exe).output().unwrap();
    let (_, interp, _) = tulpa(&[f.to_str().unwrap()]);
    assert_eq!(String::from_utf8_lossy(&native.stdout), interp);
    assert_eq!(interp, "zero\nbig\n110 1\n");
}

#[test]
fn bundle_carries_the_whole_language() {
    let d = tmpdir("bundle");
    std::fs::write(d.join("util.rav"), "fn twice(f, x) { f(f(x)) }\n").unwrap();
    let f = d.join("app.rav");
    std::fs::write(
        &f,
        "import \"std/plan\"\nfrom \"util\" import twice\nlet p = plan.Planner()\np.action(\"go\", {}, {there: true})\nlet t = spawn twice(|x| x * 3, 2)\nprint(p.plan({there: false}, {there: true}), {args: ARGS}, await t)\n",
    )
    .unwrap();
    let out = d.join("app");
    let (code, _, err) = tulpa(&["bundle", f.to_str().unwrap(), "-o", out.to_str().unwrap()]);
    assert_eq!(code, 0, "{err}");
    assert!(err.contains("3 source file(s)"), "{err}");
    // runs from anywhere, without the sources or a library path
    std::fs::remove_file(&f).unwrap();
    std::fs::remove_file(d.join("util.rav")).unwrap();
    let run = Command::new(&out)
        .args(["a", "b"])
        .current_dir("/")
        .env("TULPA_PATH", "/nonexistent")
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        "[\"go\"] {\"args\": [\"a\", \"b\"]} 18\n",
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
}

#[test]
fn build_exe_falls_back_to_a_bundle_for_dynamic_programs() {
    if !have("clang") {
        return;
    }
    let d = tmpdir("fallback");
    let f = d.join("dyn.rav");
    std::fs::write(
        &f,
        "let d = {a: 1}\nd.b = [1, 2].map(|x| x + d.a)\nprint(d)\n",
    )
    .unwrap();
    let out = d.join("dyn");
    let (code, _, err) = tulpa(&[
        "build",
        f.to_str().unwrap(),
        "--exe",
        "-o",
        out.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "{err}");
    assert!(err.contains("hosted bundle"), "{err}");
    let run = Command::new(&out).output().unwrap();
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        "{\"a\": 1, \"b\": [2, 3]}\n"
    );
    // microcontroller targets cannot host the interpreter: still an error there
    let (code, _, err) = tulpa(&[
        "build",
        f.to_str().unwrap(),
        "--target",
        "rp2040",
        "-o",
        d.join("x.o").to_str().unwrap(),
    ]);
    assert_ne!(code, 0, "{err}");
}

#[test]
fn deploy_runs_the_pipeline() {
    let d = tmpdir("deploy");
    let p = d.join("rover");
    let (code, _, err) = tulpa(&["new", p.to_str().unwrap(), "--board", "rp2040"]);
    assert_eq!(code, 0, "{err}");
    let (code, out, err) = tulpa(&["deploy", p.to_str().unwrap(), "--dry-run"]);
    assert_eq!(code, 0, "{out}{err}");
    for step in [
        "[1/4] prove",
        "[2/4] check",
        "cmake --build build",
        "rover.uf2",
    ] {
        assert!(out.contains(step), "missing {step}:\n{out}");
    }
    // a refuted contract stops the deploy before anything is built
    let app = p.join("app.rav");
    let mut src = std::fs::read_to_string(&app).unwrap();
    src.push_str("\nfn broken(x: int) -> int ensure result' > x { x }\n");
    std::fs::write(&app, src).unwrap();
    let (code, out, err) = tulpa(&["deploy", p.to_str().unwrap(), "--dry-run"]);
    assert_eq!(code, 1, "{out}{err}");
    assert!(!out.contains("[3/4]"), "{out}");
}

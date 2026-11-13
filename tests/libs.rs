//! Tests for the extended libraries: nn (autograd), pack, json, tlm, and the std/ modules.

fn run(src: &str) -> String {
    tulpa::run_captured_in(src, env!("CARGO_MANIFEST_DIR")).unwrap_or_else(|e| panic!("{e}\n--- program ---\n{src}"))
}

#[test]
fn nn_gradients_match_finite_differences() {
    let out = run(r#"
nn::seed(11)
let W = nn::param(nn::randn(4, 3).tolist())
let b = nn::param([[0.1, -0.2, 0.3]])
let x = nn::tensor([[0.5, -1.0, 2.0, 0.1], [1.5, 0.3, -0.7, -2.0]])
fn f(W, b) { ((x.matmul(W) + b).tanh() * (x.matmul(W)).sigmoid() - 0.25).square().mean() + (b.exp().sum() / 10) }
f(W, b).backward()
let gW = W.grad()
let gb = b.grad()
let eps = 1e-6
let err = 0.0
for i in 0..4 { for j in 0..3 {
    let o = W.get(i, j)
    W.set(i, j, o + eps); let p = f(W, b).item()
    W.set(i, j, o - eps); let m = f(W, b).item()
    W.set(i, j, o)
    err = max(err, abs((p - m) / (2 * eps) - gW.get(i, j)))
}}
for j in 0..3 {
    let o = b.get(0, j)
    b.set(0, j, o + eps); let p = f(W, b).item()
    b.set(0, j, o - eps); let m = f(W, b).item()
    b.set(0, j, o)
    err = max(err, abs((p - m) / (2 * eps) - gb.get(0, j)))
}
print(err < 1e-6)
"#);
    assert_eq!(out.trim(), "true");
}

#[test]
fn nn_learns_xor_and_round_trips_weights() {
    let dir = std::env::temp_dir().join("tulpa_nn_test.nn");
    let out = run(&format!(r#"
nn::seed(1)
let net = nn::MLP([2, 16, 1], "tanh")
let opt = nn::Adam(net.params(), 0.05)
let X = nn::tensor([[0,0],[0,1],[1,0],[1,1]])
let Y = nn::tensor([[0],[1],[1],[0]])
for e in 0..500 {{ let l = nn::mse(net.forward(X), Y); opt.zero_grad(); l.backward(); opt.step() }}
print(nn::mse(net.forward(X), Y).item() < 0.01)
net.save({p:?})
let net2 = nn::MLP([2, 16, 1], "tanh")
net2.load({p:?})
print(nn::mse(net2.forward(X), net.forward(X)).item() < 1e-20)
"#, p = dir.display().to_string()));
    assert_eq!(out, "true\ntrue\n");
}

#[test]
fn tensor_operators_and_broadcasting() {
    let out = run(r#"
let a = nn::tensor([[1, 2], [3, 4]])
print((a + nn::tensor([10, 20])).tolist())
print((2 * a - 1).tolist(), (a / 2).tolist(), (1 - a).tolist())
print(a.sum(0).tolist(), a.mean(1).tolist(), a.t().tolist(), (a * a).sum().item())
print(nn::cat([a, a], 1).shape(), nn::cat([a, a], 0).shape(), a.slice_cols(1, 2).tolist())
"#);
    assert_eq!(
        out,
        "[[11.0, 22.0], [13.0, 24.0]]\n[[1.0, 3.0], [5.0, 7.0]] [[0.5, 1.0], [1.5, 2.0]] [[0.0, -1.0], [-2.0, -3.0]]\n[[4.0, 6.0]] [[1.5], [3.5]] [[1.0, 3.0], [2.0, 4.0]] 30.0\n[2, 4] [4, 2] [[2.0], [4.0]]\n"
    );
}

#[test]
fn pack_json_crc_cobs() {
    let out = run(r#"
let raw = pack::pack(">hIf", [-2, 70000, 0.5])
print(raw, pack::unpack(">hIf", raw), pack::calcsize("<60f"))
print(pack::crc16("123456789"), pack::crc32("123456789"))
print(pack::cobs_decode(pack::cobs_encode([0, 1, 0, 0, 2])))
print(json::decode(json::encode({a: [1, 2.5, nil, true], b: "x"})))
"#);
    assert_eq!(
        out,
        "[255, 254, 0, 1, 17, 112, 63, 0, 0, 0] [-2, 70000, 0.5] 240\n10673 3421780262\n[0, 1, 0, 0, 2]\n{\"a\": [1, 2.5, nil, true], \"b\": \"x\"}\n"
    );
}

#[test]
fn std_library_modules() {
    let out = run(r#"
import "std/frame"
import "std/lipm"
import "std/topics"
import "std/eda"
import "std/rtl"
let d = frame.Decoder()
let fr = d.feed([7, 7] + frame.encode([1, 2, 0, 3], type: 4, seq: 9))
print(fr[0].type, fr[0].seq, fr[0].payload)
print(lipm.margin([0.0, 0.0], lipm.support_polygon([[0.0, 0.1], [0.0, -0.1]])))
let bus = topics.Bus()
let seen = []
bus.subscribe("/w", |m| seen.push(m.force[2]))
bus.publish("/w", topics.Wrench([0.0, 0.0, 9.5]))
print(seen)
let b = eda.Board(name: "t")
b.power("V", 5[V]); b.ground("G")
b.part("U1", "IC", "x", pins: ["VCC", "GND"], cost: 2.0, draw: 1[W])
b.connect("V", "U1.VCC"); b.connect("G", "U1.GND")
b.decouple("U1", "VCC", "G", count: 1)
print(len(b.erc()), b.total_cost(), b.bom()[1].refs)
let sim = rtl.Sim()
let q = sim.reg("q", 3)
struct C { q; fn comb(self) {}; fn seq(self) { self.q.d(self.q.value + 1) } }
sim.add(C(q))
sim.run(10)
print(q.value)
"#);
    assert_eq!(out, "4 9 [1, 2, 0, 3]\n0.09\n[9.5]\n0 2.1 [\"C1\"]\n2\n");
}

#[test]
fn scaler_save_load_roundtrip() {
    let p = std::env::temp_dir().join("tulpa_scaler_test.txt");
    let ps = p.display().to_string().replace('\\', "/");
    let src = format!(
        "let s = nn::Scaler(2)\nfor r in [[1.0, 10.0], [3.0, 30.0], [5.0, 50.0]] {{ s.update([r]) }}\ns.save(\"{ps}\")\nlet t = nn::Scaler(2)\nt.load(\"{ps}\")\nprint(t.count(), t.mean())\n"
    );
    let out = tulpa::run_captured(&src).unwrap();
    assert_eq!(out.trim(), "3.0 [3.0, 30.0]");
}

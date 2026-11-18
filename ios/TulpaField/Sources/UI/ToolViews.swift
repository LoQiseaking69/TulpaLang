import SwiftUI
import UserNotifications

// ---------------------------------------------------------------------------------- sensors

struct SensorsView: View {
    @EnvironmentObject var vm: AppModel
    @ObservedObject var sensors: SensorBridge
    @State private var catalog: [SensorInfo] = []
    @State private var locOn = false

    var body: some View {
        VStack(spacing: 0) {
            SubTopBar(title: "Sensors", subtitle: "live readings · what phone:: can see")
            ScrollView {
                LazyVStack(spacing: 8) {
                    Card {
                        HStack(spacing: 10) {
                            Image(systemName: "location.fill").foregroundColor(TF.cyan)
                            VStack(alignment: .leading, spacing: 2) {
                                Text("Location for phone::location()").foregroundColor(TF.text)
                                Text("GPS stays off unless enabled here or a program asks for it.").font(.caption).foregroundColor(TF.dim)
                            }
                            Spacer()
                            Toggle("", isOn: Binding(get: { locOn }, set: { on in
                                if on && !sensors.hasLocationPermission() { sensors.requestLocation() }
                                sensors.setLocationEnabled(on); locOn = on; catalog = sensors.catalog()
                            })).labelsHidden().tint(TF.violet)
                        }
                    }
                    ForEach(catalog) { s in card(s, sensors.latest[s.kind]) }
                }.padding(16)
            }
        }
        .accessibilityIdentifier("sensors-screen")
        .onAppear { sensors.acquire(); catalog = sensors.catalog(); locOn = sensors.locationEnabled }
        .onDisappear { sensors.release() }
        .onChange(of: sensors.locationAuthorized) { _ in catalog = sensors.catalog() }
    }

    private func card(_ s: SensorInfo, _ r: SensorReading?) -> some View {
        let value: String = {
            if !s.available { return s.vendor }
            guard let r, !r.values.isEmpty else { return "waiting for data…" }
            switch s.kind {
            case SensorKind.orientation: return r.values.prefix(3).map { String(format: "%.1f°", Double($0) * 180 / .pi) }.joined(separator: "   ")
            case SensorKind.battery: return String(format: "%.0f%%", r.values[0] * 100) + ((r.values.count > 1 && r.values[1] > 0) ? " · charging" : "")
            case SensorKind.location where r.values.count >= 4: return String(format: "%.5f, %.5f  ±%.0f m", r.values[0], r.values[1], r.values[3])
            default: return r.values.prefix(3).map { fmt($0) }.joined(separator: "   ")
            }
        }()
        return Card {
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 8) {
                    Circle().fill(!s.available ? TF.dim : (r != nil ? TF.ok : TF.warn)).frame(width: 8, height: 8)
                    Text(s.title).font(.subheadline.weight(.semibold)).foregroundColor(TF.text)
                    Spacer()
                    Text(s.unit).font(.caption).foregroundColor(TF.cyan)
                }
                Text(s.call).font(TF.mono(12)).foregroundColor(TF.cyan)
                Text(value).font(TF.mono(18)).foregroundColor(s.available ? TF.text : TF.dim).padding(.top, 4)
                if s.available { Text(s.vendor).font(.system(size: 11)).foregroundColor(TF.dim) }
            }
        }
    }
}

// ---------------------------------------------------------------------------------- machines

struct MachinesView: View {
    @EnvironmentObject var vm: AppModel

    var body: some View {
        VStack(spacing: 0) {
            SubTopBar(title: "Machines", subtitle: vm.editor.file?.lastPathComponent) {
                IconButton(system: "arrow.clockwise", label: "Refresh") { vm.refreshMachines() }
            }
            if vm.machines.isEmpty {
                EmptyStateView(icon: "point.3.connected.trianglepath.dotted", title: "No machines here",
                               message: "This file doesn't declare a machine. Try the State machine template or the Shake alarm example.")
            } else {
                ScrollView {
                    LazyVStack(spacing: 16) { ForEach(vm.machines) { g in machineCard(g) } }.padding(16)
                }
            }
        }
    }

    private func machineCard(_ g: MachineGraph) -> some View {
        Card(padding: 12) {
            VStack(alignment: .leading, spacing: 0) {
                HStack {
                    Text("machine \(g.name)").font(TF.mono(15, weight: .bold)).foregroundColor(Color(Syn.type))
                    Spacer()
                    Text("\(g.states.count) states · \(g.transitions.count) transitions").font(.caption).foregroundColor(TF.dim)
                }
                MachineCanvas(g: g) { vm.reveal(line: $0 - 1) }
                Text("Tap a state to jump to it.").font(.system(size: 11)).foregroundColor(TF.dim)
                ForEach(Array(g.transitions.enumerated()), id: \.offset) { _, t in
                    Button { vm.reveal(line: t.line - 1) } label: {
                        VStack(alignment: .leading, spacing: 1) {
                            HStack(spacing: 0) {
                                Text(t.global ? "∀" : t.from).foregroundColor(Color(Syn.type)).frame(minWidth: 70, alignment: .leading)
                                Text(" —\(t.event)\(t.params.isEmpty ? "" : "(" + t.params.joined(separator: ", ") + ")")→ ").foregroundColor(TF.cyan)
                                Text(t.isInternal ? "(internal)" : t.to).foregroundColor(t.isInternal ? TF.dim : Color(Syn.type))
                                Spacer()
                                Text("\(t.line)").font(.system(size: 11)).foregroundColor(TF.dim)
                            }.font(TF.mono(13))
                            if let gd = t.guardText { Text("    if \(gd)").font(TF.mono(12)).foregroundColor(TF.warn) }
                        }.padding(.vertical, 5).contentShape(Rectangle())
                    }.buttonStyle(.plain)
                }
                HStack(spacing: 16) {
                    Button { UIPasteboard.general.string = g.mermaid; vm.toast("Copied Mermaid diagram") } label: { Label("Copy Mermaid", systemImage: "doc.on.doc") }
                    Button { vm.shareText(g.mermaid, title: "\(g.name).mmd") } label: { Label("Share", systemImage: "square.and.arrow.up") }
                }.font(.subheadline).padding(.top, 8)
            }
        }
    }
}

private struct MachineCanvas: View {
    let g: MachineGraph
    let onTap: (Int) -> Void

    private func positions(_ size: CGSize) -> [String: CGPoint] {
        let n = g.states.count
        let w = size.width, h = size.height
        let cx = w / 2, cy = h / 2
        let rx = max(w / 2 - 60, 40), ry = max(h / 2 - 30, 30)
        var out: [String: CGPoint] = [:]
        for (i, s) in g.states.enumerated() {
            if n == 1 { out[s.name] = CGPoint(x: cx, y: cy); continue }
            let a = -Double.pi / 2 + 2 * Double.pi * Double(i) / Double(n) + (n == 2 ? Double.pi / 2 : 0)
            out[s.name] = CGPoint(x: cx + rx * CGFloat(cos(a)), y: cy + ry * CGFloat(sin(a)))
        }
        return out
    }

    var body: some View {
        let n = g.states.count
        let height: CGFloat = n <= 2 ? 180 : n <= 4 ? 260 : 320
        GeometryReader { geo in
            let pos = positions(geo.size)
            Canvas { ctx, size in
                let nodeR: CGFloat = 26
                var pairCount: [String: Int] = [:]
                for t in g.transitions where !t.global {
                    guard let a = pos[t.from] else { continue }
                    if t.isInternal || t.from == t.to {
                        let c = CGPoint(x: a.x, y: a.y - nodeR - 8)
                        ctx.stroke(Path(ellipseIn: CGRect(x: c.x - 10, y: c.y - 10, width: 20, height: 20)), with: .color(TF.cyan.opacity(0.6)), lineWidth: 1.5)
                        ctx.draw(Text(t.event).font(TF.mono(10)).foregroundColor(TF.cyan), at: CGPoint(x: a.x + 12, y: a.y - nodeR - 22), anchor: .leading)
                        continue
                    }
                    guard let b = pos[t.to] else { continue }
                    let key = t.from + "→" + t.to
                    let k = pairCount[key, default: 0]; pairCount[key] = k + 1
                    let reverse = g.transitions.contains { $0.from == t.to && $0.to == t.from && !$0.global }
                    let bend: CGFloat = (reverse ? 24 : 0) + CGFloat(k) * 14
                    let color = t.guardText != nil ? TF.warn : TF.cyan
                    let ang = atan2(b.y - a.y, b.x - a.x)
                    let nx = -sin(ang), ny = cos(ang)
                    let start = CGPoint(x: a.x + cos(ang) * nodeR + nx * bend * 0.3, y: a.y + sin(ang) * nodeR + ny * bend * 0.3)
                    let end = CGPoint(x: b.x - cos(ang) * (nodeR + 4) + nx * bend * 0.3, y: b.y - sin(ang) * (nodeR + 4) + ny * bend * 0.3)
                    let ctrl = CGPoint(x: (a.x + b.x) / 2 + nx * bend * 1.6, y: (a.y + b.y) / 2 + ny * bend * 1.6)
                    var p = Path(); p.move(to: start); p.addQuadCurve(to: end, control: ctrl)
                    ctx.stroke(p, with: .color(color.opacity(0.8)), lineWidth: 1.8)
                    let ea = atan2(end.y - ctrl.y, end.x - ctrl.x)
                    var head = Path()
                    head.move(to: end)
                    head.addLine(to: CGPoint(x: end.x - cos(ea - 0.4) * 10, y: end.y - sin(ea - 0.4) * 10))
                    head.addLine(to: CGPoint(x: end.x - cos(ea + 0.4) * 10, y: end.y - sin(ea + 0.4) * 10))
                    head.closeSubpath()
                    ctx.fill(head, with: .color(color))
                    let mid = CGPoint(x: (a.x + b.x) / 2, y: (a.y + b.y) / 2)
                    let lp = CGPoint(x: mid.x + nx * (bend + 8), y: mid.y + ny * (bend + 8))
                    let label = ctx.resolve(Text(t.event + (t.guardText != nil ? " ?" : "")).font(TF.mono(10)).foregroundColor(color))
                    let sz = label.measure(in: size)
                    ctx.fill(Path(CGRect(x: lp.x - sz.width / 2 - 3, y: lp.y - sz.height / 2, width: sz.width + 6, height: sz.height)), with: .color(TF.surface))
                    ctx.draw(label, at: lp)
                }
                for s in g.states {
                    guard let c = pos[s.name] else { continue }
                    let r = CGRect(x: c.x - nodeR, y: c.y - nodeR, width: nodeR * 2, height: nodeR * 2)
                    ctx.fill(Path(ellipseIn: r), with: .color(TF.surface2))
                    ctx.stroke(Path(ellipseIn: r), with: .color(s.name == g.initial ? TF.violet : TF.line), lineWidth: 2)
                    if s.name == g.initial { ctx.stroke(Path(ellipseIn: r.insetBy(dx: -4, dy: -4)), with: .color(TF.violet), lineWidth: 1.2) }
                    let label = (s.enter ? "▸" : "") + s.name + (s.exit ? "◂" : "")
                    ctx.draw(Text(label).font(.system(size: 11, weight: .semibold)).foregroundColor(TF.text), at: c)
                }
                let globals = g.transitions.filter { $0.global }
                if !globals.isEmpty {
                    ctx.draw(Text("∀ " + globals.map { "\($0.event)→\($0.to)" }.joined(separator: "  ")).font(TF.mono(10)).foregroundColor(Color(Syn.macro)),
                             at: CGPoint(x: 4, y: size.height - 12), anchor: .leading)
                }
            }
            .contentShape(Rectangle())
            .onTapGesture { p in
                let hit = pos.min { hypot($0.value.x - p.x, $0.value.y - p.y) < hypot($1.value.x - p.x, $1.value.y - p.y) }
                if let hit, hypot(hit.value.x - p.x, hit.value.y - p.y) < 50, let s = g.states.first(where: { $0.name == hit.key }) { onTap(s.line) }
            }
        }
        .frame(height: height).padding(.vertical, 8)
    }
}

// ---------------------------------------------------------------------------------- verify

struct VerifyView: View {
    @EnvironmentObject var vm: AppModel
    @State private var tab = 0

    var body: some View {
        let st = vm.verify
        VStack(spacing: 0) {
            SubTopBar(title: "Verify", subtitle: vm.editor.file?.lastPathComponent)
            Picker("", selection: $tab) { Text("Contracts").tag(0); Text("Tests").tag(1) }
                .pickerStyle(.segmented).padding(.horizontal, 16).padding(.bottom, 6)
            ScrollView {
                VStack(alignment: .leading, spacing: 10) {
                    if vm.editor.file == nil { Text("Open a file first.").foregroundColor(TF.dim) }
                    else if tab == 0 { contracts(st) } else { tests(st) }
                }.padding(16).frame(maxWidth: .infinity, alignment: .leading)
            }
        }
    }

    @ViewBuilder private func contracts(_ st: VerifyState) -> some View {
        Text("Fuzzes every function with an ensure clause or a return type using \(vm.settings.prefs.trials) trials per function, shrinking failures to a minimal counterexample.")
            .font(.system(size: 13)).foregroundColor(TF.dim)
        HStack(spacing: 12) {
            if st.running {
                ProgressView().tint(TF.violet)
                Button("Stop") { vm.stopVerify() }.buttonStyle(OutlineButton())
            } else {
                Button { vm.runVerify() } label: { Label("Check contracts", systemImage: "checkmark.seal") }.buttonStyle(PrimaryButton())
                    .accessibilityIdentifier("check-contracts")
            }
        }
        if let e = st.error { Text(e).font(TF.mono(13)).foregroundColor(TF.err) }
        if let rs = st.results {
            if rs.isEmpty { Text("No contracted functions in this file. Add `require`/`ensure` clauses or a return type.").foregroundColor(TF.dim) }
            else {
                let failed = rs.filter { !$0.ok }.count
                Text(failed == 0 ? "All \(rs.count) functions hold ✓" : "\(failed) of \(rs.count) functions fail")
                    .font(.headline).foregroundColor(failed == 0 ? TF.ok : TF.err)
            }
            ForEach(rs) { r in
                Card(padding: 12) {
                    VStack(alignment: .leading, spacing: 4) {
                        HStack(spacing: 8) {
                            Image(systemName: r.ok ? "checkmark.circle.fill" : "xmark.circle.fill").foregroundColor(r.ok ? TF.ok : TF.err)
                            Text(r.name).font(TF.mono(15, weight: .semibold)).foregroundColor(TF.text)
                            Spacer()
                            Text("\(r.passed) passed · \(r.discarded) discarded").font(.caption).foregroundColor(TF.dim)
                        }
                        if !r.ok {
                            if !r.counterexample.isEmpty {
                                Text("counterexample: \(r.name)(\(r.counterexample.joined(separator: ", ")))").font(TF.mono(13)).foregroundColor(TF.warn).padding(.top, 2)
                            }
                            if let e = r.error { Text(e).font(TF.mono(12)).foregroundColor(TF.err) }
                            if !r.counterexample.isEmpty {
                                Button { vm.tryCounterexample(r) } label: { Label("Try in REPL", systemImage: "terminal") }.font(.subheadline)
                            }
                        }
                    }
                }
            }
        }
    }

    @ViewBuilder private func tests(_ st: VerifyState) -> some View {
        Text("Runs every define_test! block in the file.").font(.system(size: 13)).foregroundColor(TF.dim)
        if st.testsRunning { ProgressView().tint(TF.violet) }
        else { Button { vm.runTests() } label: { Label("Run tests", systemImage: "testtube.2") }.buttonStyle(PrimaryButton()) }
        if let t = st.tests {
            if let e = t.error { Text(e).font(TF.mono(13)).foregroundColor(TF.err) }
            else {
                Text("\(t.passed) passed, \(t.failed.count) failed").font(.headline).foregroundColor(t.failed.isEmpty ? TF.ok : TF.err)
                if t.passed == 0 && t.failed.isEmpty { Text("No tests found. Add one with define_test!(\"name\", { assert!(…) }).").foregroundColor(TF.dim) }
                ForEach(Array(t.failed.enumerated()), id: \.offset) { _, f in
                    Card(padding: 12) {
                        VStack(alignment: .leading, spacing: 2) {
                            Text("✗ \(f.0)").font(TF.mono(14)).foregroundColor(TF.err)
                            Text(f.1).font(TF.mono(12)).foregroundColor(TF.dim)
                        }
                    }
                }
            }
            if !t.output.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                Text("Output").font(.subheadline.weight(.medium)).foregroundColor(TF.dim)
                Text(t.output).font(TF.mono(12)).foregroundColor(TF.text).textSelection(.enabled).padding(10)
                    .frame(maxWidth: .infinity, alignment: .leading).background(TF.bg).clipShape(RoundedRectangle(cornerRadius: 8))
            }
        }
    }
}

// ---------------------------------------------------------------------------------- bytecode

struct BytecodeView: View {
    @EnvironmentObject var vm: AppModel
    var body: some View {
        VStack(spacing: 0) {
            SubTopBar(title: "Bytecode", subtitle: vm.editor.file?.lastPathComponent) {
                IconButton(system: "arrow.clockwise", label: "Refresh") { vm.refreshBytecode() }
            }
            Text("Each function is compiled to stack bytecode on first call; functions using features outside the bytecode tier stay interpreted.")
                .font(.caption).foregroundColor(TF.dim).padding(16).frame(maxWidth: .infinity, alignment: .leading)
            ScrollView([.vertical, .horizontal]) {
                Text(vm.bytecode).font(TF.mono(12)).foregroundColor(TF.text).textSelection(.enabled).padding(16)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
    }
}

// ---------------------------------------------------------------------------------- settings

struct SettingsView: View {
    @EnvironmentObject var vm: AppModel
    @ObservedObject var settings: Settings
    @State private var remote = ""

    var body: some View {
        let p = settings.prefs
        VStack(spacing: 0) {
            SubTopBar(title: "Settings")
            ScrollView {
                VStack(alignment: .leading, spacing: 8) {
                    header("Editor")
                    Text("Font size: \(p.fontSize) pt").foregroundColor(TF.text)
                    Slider(value: Binding(get: { Double(p.fontSize) }, set: { v in settings.update { $0.fontSize = Int(v.rounded()) } }), in: 10...22, step: 1).tint(TF.violet)
                    Text("let speed = 0.8[m/s]").font(TF.mono(CGFloat(p.fontSize))).foregroundColor(TF.dim)
                    toggle("Line numbers", p.lineNumbers) { v in settings.update { $0.lineNumbers = v } }
                    toggle("Pair brackets and quotes", p.pairBrackets) { v in settings.update { $0.pairBrackets = v } }
                    toggle("Save automatically", p.autosave) { v in settings.update { $0.autosave = v } }
                    toggle("Haptic feedback", p.haptics) { v in settings.update { $0.haptics = v } }
                    Divider().background(TF.line).padding(.vertical, 8)
                    header("Running")
                    toggle("Keep screen on while a program runs", p.keepScreenOn) { v in settings.update { $0.keepScreenOn = v } }
                    Text("Contract trials per function: \(p.trials)").foregroundColor(TF.text)
                    HStack(spacing: 8) {
                        ForEach([100, 300, 1000, 3000], id: \.self) { n in Chip(text: "\(n)", selected: p.trials == n) { settings.update { $0.trials = n } } }
                    }
                    Button("Allow notifications for phone::notify") {
                        UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound]) { ok, _ in
                            DispatchQueue.main.async { vm.toast(ok ? "Notifications allowed" : "Notifications are off — enable them in iOS Settings") }
                        }
                    }.font(.subheadline)
                    Divider().background(TF.line).padding(.vertical, 8)
                    header("Co-simulation")
                    Text("Programs that call cosim::connect() run against the on-device MuJoCo model. Enter host:port to drive a desktop HermiSim instead (its co-sim server must listen on your network).")
                        .font(.system(size: 13)).foregroundColor(TF.dim)
                    TextField("Remote simulator (blank = on device), e.g. 192.168.1.20:7300", text: $remote)
                        .textFieldStyle(.roundedBorder).keyboardType(.URL).textInputAutocapitalization(.never).autocorrectionDisabled()
                        .onChange(of: remote) { v in settings.update { $0.remoteCosim = v.trimmingCharacters(in: .whitespaces) } }
                    Divider().background(TF.line).padding(.vertical, 8)
                    header("About")
                    Text("Tulpa Field \(vm.appVersion) (\(vm.buildNumber))").font(.headline).foregroundColor(TF.text)
                    Text("TulpaLang runtime \(vm.tulpaVersion) · interpreter, bytecode VM, units, contracts, machines, signals, Driver SDK, planning, Tulpa Link, std/ libraries")
                        .font(.system(size: 13)).foregroundColor(TF.dim)
                    Text("Physics: MuJoCo \(TulpaNative.simVersion()) (Apache-2.0), rendered with Metal.").font(.system(size: 13)).foregroundColor(TF.dim)
                    Text("Modules: " + vm.modules.keys.sorted().joined(separator: " · ")).font(TF.mono(12)).foregroundColor(TF.dim)
                    Text("Part of the HermiSim family. Everything runs on-device; the network is used only for a remote co-simulator you configure.")
                        .font(.system(size: 13)).foregroundColor(TF.dim)
                }.padding(16)
            }
        }
        .onAppear { remote = settings.prefs.remoteCosim }
    }

    private func header(_ t: String) -> some View { Text(t).font(.subheadline.weight(.semibold)).foregroundColor(TF.violet) }

    private func toggle(_ label: String, _ value: Bool, _ set: @escaping (Bool) -> Void) -> some View {
        Toggle(label, isOn: Binding(get: { value }, set: set)).tint(TF.violet).foregroundColor(TF.text)
    }
}

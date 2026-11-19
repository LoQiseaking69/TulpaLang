import SwiftUI
import UniformTypeIdentifiers

/// MuJoCo viewer: Metal 3D view, HUD, transport controls, model picker and info.
struct SimView: View {
    @EnvironmentObject var vm: AppModel
    @ObservedObject var sim: SimManager
    @ObservedObject var runner: RunManager
    @State private var renderer: SimRenderer? = nil
    @State private var picker = false
    @State private var info = false
    @State private var importing = false
    @State private var metalChecked = false

    private struct Hud { var time: Float = 0; var clients = 0; var playing = false; var geoms = 0 }

    private func hud() -> Hud {
        let h = sim.handle
        guard h != 0 else { return Hud() }
        let f = TulpaNative.simFrame(h)
        guard f.count >= 4 else { return Hud() }
        return Hud(time: f[0], clients: Int(f[2]), playing: f[3] > 0.5, geoms: (f.count - 4) / 21)
    }

    var body: some View {
        let st = sim.state
        let run = runner.state
        VStack(spacing: 0) {
            TopBar(title: "", leading: {
                Button { picker = true } label: {
                    VStack(alignment: .leading, spacing: 1) {
                        HStack(spacing: 4) {
                            Text(st.model?.name ?? "Sim").font(.headline).foregroundColor(TF.text).lineLimit(1)
                            Image(systemName: "chevron.down").font(.caption).foregroundColor(TF.dim)
                        }
                        Text(st.model.map { "\($0.bodies) bodies · \($0.actuators) actuators · MuJoCo \(st.version)" } ?? "MuJoCo \(st.version) · no model")
                            .font(.caption2).foregroundColor(TF.dim).lineLimit(1)
                    }
                }
                .accessibilityIdentifier("sim-model-picker")
            }, trailing: {
                IconButton(system: "info.circle", label: "Model info", disabled: st.model == nil) { info = true }
                IconButton(system: "cube", label: "Choose model") { picker = true }
                ToolsMenu()
            })
            ZStack {
                TF.bg
                SimMetalView(sim: sim) { r in renderer = r; metalChecked = true }
                if metalChecked && renderer == nil {
                    Text("3D view unavailable on this device (no Metal). The simulation still runs.")
                        .font(.caption).foregroundColor(TF.dim).multilineTextAlignment(.center).padding(32)
                }
                TimelineView(.periodic(from: .now, by: 0.2)) { _ in
                    let h = hud()
                    overlay(st: st, run: run, h: h)
                }
            }
        }
        .onAppear { vm.ensureSimModel() }
        .sheet(isPresented: $picker) { pickerSheet(st) }
        .sheet(isPresented: $info) { if let m = st.model { infoSheet(m, st) } }
        .fileImporter(isPresented: $importing, allowedContentTypes: [.xml, .plainText, .data], allowsMultipleSelection: false) { r in
            guard case .success(let urls) = r, let u = urls.first else { return }
            let ok = u.startAccessingSecurityScopedResource()
            defer { if ok { u.stopAccessingSecurityScopedResource() } }
            if let text = try? String(contentsOf: u, encoding: .utf8) { vm.importModel(text: text, name: u.lastPathComponent) }
            else { vm.toast("Cannot read \(u.lastPathComponent)") }
        }
        .onChange(of: vm.importModelRequest) { v in if v { vm.importModelRequest = false; importing = true } }
    }

    @ViewBuilder
    private func overlay(st: SimState, run: RunState, h: Hud) -> some View {
        ZStack {
            VStack(alignment: .leading, spacing: 2) {
                Text(String(format: "t = %.2f s", h.time)).font(TF.mono(13)).foregroundColor(TF.text)
                Group {
                    if h.clients > 0 { Text("● co-sim: driven by \(run.title.isEmpty ? "program" : run.title)").foregroundColor(TF.ok) }
                    else if h.playing { Text("● free-running").foregroundColor(TF.cyan) }
                    else { Text("paused").foregroundColor(TF.dim) }
                }
                .font(.system(size: 11)).accessibilityIdentifier("sim-status")
                Text(String(format: "%.0f fps · %d geoms", renderer?.hud.fps ?? 0, h.geoms)).font(.system(size: 10)).foregroundColor(TF.dim)
            }
            .padding(.horizontal, 10).padding(.vertical, 6)
            .background(TF.bg.opacity(0.72)).clipShape(RoundedRectangle(cornerRadius: 10))
            .padding(12)
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)

            if st.loading { ProgressView().tint(TF.cyan) }
            if let e = st.error {
                Text(e).font(TF.mono(12)).foregroundColor(TF.err).padding(12)
                    .background(TF.err.opacity(0.15)).clipShape(RoundedRectangle(cornerRadius: 10)).padding(24)
            }
            if st.model == nil && !st.loading {
                VStack(spacing: 6) {
                    Text("No model loaded").font(.headline).foregroundColor(TF.text)
                    Text("Pick a MuJoCo model, or run a program that uses cosim::").font(.footnote).foregroundColor(TF.dim)
                        .multilineTextAlignment(.center)
                    Button("Choose model") { picker = true }.buttonStyle(PrimaryButton()).padding(.top, 8)
                }
                .padding(32)
            }

            VStack(spacing: 6) {
                Spacer()
                if run.running || !run.console.isEmpty {
                    HStack {
                        Text(run.console.last(where: { !$0.text.trimmingCharacters(in: .whitespaces).isEmpty })?.text ?? "")
                            .font(TF.mono(11)).foregroundColor(TF.dim).lineLimit(1).frame(maxWidth: .infinity, alignment: .leading)
                        if run.running {
                            IconButton(system: "stop.fill", label: "Stop", tint: TF.err) { runner.stop() }
                        } else {
                            IconButton(system: "terminal", label: "Console") { vm.go(.run) }
                        }
                    }
                    .padding(.leading, 10).padding(.trailing, 4)
                    .background(TF.bg.opacity(0.8)).clipShape(RoundedRectangle(cornerRadius: 10))
                    .padding(.horizontal, 12)
                }
                Text("drag: orbit · two fingers: pan · pinch: zoom · double-tap: frame")
                    .font(.system(size: 10)).foregroundColor(TF.dim.opacity(0.7))
                controls(st: st, run: run, h: h)
            }
        }
    }

    private func controls(st: SimState, run: RunState, h: Hud) -> some View {
        let driven = h.clients > 0
        let has = st.model != nil
        return ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 4) {
                IconButton(system: h.playing ? "pause.fill" : "play.fill", label: h.playing ? "Pause" : "Play", tint: TF.cyan, disabled: !has || driven) {
                    _ = sim.control(h.playing ? "pause" : "play")
                }
                IconButton(system: "arrow.counterclockwise", label: "Reset", disabled: !has || driven) { _ = sim.control("reset") }
                IconButton(system: "hand.raised", label: "Push", disabled: !has) {
                    let az = Double(renderer?.camera.azimuth ?? 135)
                    _ = sim.control("push", (az + 180).truncatingRemainder(dividingBy: 360))
                }
                IconButton(system: "scope", label: "Frame model") { renderer?.reframe = true }
                Chip(text: st.realtime > 0 ? "real time" : "max speed", selected: st.realtime > 0) {
                    sim.setRealtime(st.realtime > 0 ? 0 : 1)
                }
                Button {
                    if run.running { runner.stop() } else { vm.runCurrent() }
                } label: {
                    Label(run.running ? "Stop" : "Run \(vm.editor.file?.lastPathComponent ?? "")",
                          systemImage: run.running ? "stop.fill" : "play.circle.fill").lineLimit(1)
                }
                .buttonStyle(TonalButton())
                .disabled(!run.running && vm.editor.file == nil)
                .accessibilityIdentifier("sim-run")
            }
            .padding(.horizontal, 6).padding(.vertical, 2)
        }
        .background(TF.bg.opacity(0.78)).clipShape(RoundedRectangle(cornerRadius: 16))
        .padding(.horizontal, 12).padding(.bottom, 12)
        .fixedSize(horizontal: false, vertical: true)
    }

    private func pickerSheet(_ st: SimState) -> some View {
        let project = vm.editor.project
        let models = sim.models(project: project)
        let bundledDir = vm.workspace.models.standardizedFileURL.path
        return NavigationView {
            List {
                ForEach(models, id: \.path) { f in
                    let bundled = f.deletingLastPathComponent().standardizedFileURL.path == bundledDir
                    Button {
                        picker = false
                        vm.loadSimModel(f)
                    } label: {
                        HStack(spacing: 12) {
                            Image(systemName: bundled ? "cube" : "doc.text")
                                .foregroundColor(st.model?.path == f.path ? TF.cyan : TF.violet)
                            VStack(alignment: .leading, spacing: 2) {
                                Text(f.deletingPathExtension().lastPathComponent).foregroundColor(TF.text)
                                Text(bundled ? "bundled" : f.lastPathComponent).font(.caption).foregroundColor(TF.dim)
                            }
                        }
                    }
                    .listRowBackground(TF.surface)
                    .accessibilityIdentifier("model-\(f.deletingPathExtension().lastPathComponent)")
                }
                Section {
                    Button {
                        picker = false
                        DispatchQueue.main.asyncAfter(deadline: .now() + 0.4) { importing = true }
                    } label: { Label("Import an MJCF model (.xml)…", systemImage: "square.and.arrow.down") }
                    .listRowBackground(TF.surface)
                } footer: {
                    Text("Programs pick their model from ravide.json (\"model\"), else the first MJCF in their folder, else the model open here.")
                }
            }
            .scrollContentBackground(.hidden)
            .background(TF.bg)
            .navigationTitle("Models")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Done") { picker = false } } }
        }
        .navigationViewStyle(.stack)
        .presentationDetents([.medium, .large])
        .preferredColorScheme(.dark)
    }

    private func infoSheet(_ m: SimModel, _ st: SimState) -> some View {
        let acts = (m.info.arr("actuators") ?? []).compactMap { ($0 as? [String: Any])?.str("name") }
        let dt = m.info.double("timestep", 0)
        return ScrollView {
            VStack(alignment: .leading, spacing: 8) {
                Text(m.name).font(.title3.bold()).foregroundColor(TF.text)
                Text(URL(fileURLWithPath: m.path).lastPathComponent).font(TF.mono(12)).foregroundColor(TF.dim)
                Text("\(m.bodies) bodies · \(m.joints) joints · \(m.actuators) actuators · \(m.sensors) sensors · "
                     + String(format: "%.1f kg · dt %.4f s", m.mass, dt))
                    .font(.footnote).foregroundColor(TF.text)
                if let a = st.address {
                    Text("co-sim server: \(a)").font(TF.mono(12)).foregroundColor(TF.cyan)
                }
                SectionTitle("Actuators")
                if acts.isEmpty { Text("none").font(.caption).foregroundColor(TF.dim) }
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 6) {
                        ForEach(Array(acts.enumerated()), id: \.offset) { _, a in
                            Text(a).font(TF.mono(11)).foregroundColor(TF.text).padding(.horizontal, 10).padding(.vertical, 6)
                                .background(TF.surface2).clipShape(Capsule())
                        }
                    }
                }
                Button {
                    info = false
                    vm.newCosimProject(m)
                } label: { Label("New co-sim project for this model", systemImage: "folder.badge.plus") }
                .buttonStyle(OutlineButton()).padding(.top, 8)
                .accessibilityIdentifier("new-cosim-project")
            }
            .padding(20)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .background(TF.bg.ignoresSafeArea())
        .presentationDetents([.medium, .large])
        .preferredColorScheme(.dark)
    }
}

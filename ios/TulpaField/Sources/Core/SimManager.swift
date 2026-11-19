import Foundation

struct SimModel: Equatable {
    let handle: Int64
    let path: String
    let name: String
    let info: [String: Any]
    let center: SIMD3<Float>
    let extent: Float
    let meshes: Int

    var actuators: Int { info.arr("actuators")?.count ?? 0 }
    var bodies: Int { (info.arr("bodies")?.count ?? 1) - 1 }
    var joints: Int { info.arr("joints")?.count ?? 0 }
    var sensors: Int { info.arr("sensors")?.count ?? 0 }
    var mass: Double { info.double("total_mass", 0) }

    static func == (a: SimModel, b: SimModel) -> Bool { a.handle == b.handle && a.path == b.path }
}

struct SimState: Equatable {
    var model: SimModel? = nil
    var loading = false
    var error: String? = nil
    var address: String? = nil        // local co-sim server, once started
    var realtime = 1.0
    var version = ""
}

/// Owns the on-device MuJoCo model shared by the 3D view and co-simulation. A program that
/// uses `cosim::` gets `TULPA_COSIM` pointing at this model's server (or at a remote .RAV IDE
/// when one is configured in Settings).
final class SimManager: ObservableObject {
    @Published private(set) var state = SimState()
    private let workspace: Workspace
    private let lock = NSRecursiveLock()
    private var current = SimState()   // authoritative copy (any thread)

    /// Bumped whenever the loaded model changes, so the renderer re-uploads meshes.
    private(set) var generation = 0

    init(workspace: Workspace) {
        self.workspace = workspace
        current.version = TulpaNative.simVersion()
        state = current
    }

    private func apply(_ f: (inout SimState) -> Void) {
        lock.lock(); f(&current); let snap = current; lock.unlock()
        if Thread.isMainThread { state = snap } else { DispatchQueue.main.async { self.state = snap } }
    }

    var model: SimModel? { lock.lock(); defer { lock.unlock() }; return current.model }
    var handle: Int64 { model?.handle ?? 0 }

    /// Bundled models and MJCF files in `project`.
    func models(project: URL?) -> [URL] {
        let fm = FileManager.default
        let bundled = ((try? fm.contentsOfDirectory(at: workspace.models, includingPropertiesForKeys: nil)) ?? [])
            .filter { $0.pathExtension == "xml" }.sorted { $0.lastPathComponent < $1.lastPathComponent }
        var own: [URL] = []
        if let project, let e = fm.enumerator(at: project, includingPropertiesForKeys: nil) {
            for case let f as URL in e {
                if e.level > 3 { e.skipDescendants(); continue }
                if f.pathExtension == "xml" && isMjcf(f) { own.append(f) }
            }
        }
        return own + bundled
    }

    private func isMjcf(_ f: URL) -> Bool { f.readText().prefix(4000).contains("<mujoco") }

    /// The model a program in `dir` should co-simulate: `ravide.json` "model" (.RAV IDE
    /// workspaces), else the first MJCF in the folder, else the model already open.
    func modelFor(_ dir: URL) -> URL? {
        var d: URL? = dir
        for _ in 0..<3 {
            guard let cur = d else { break }
            // ravide.json, or hermisim.json from projects made before the .RAV IDE rebrand
            let current = cur.appendingPathComponent("ravide.json")
            let cfg = current.exists ? current : cur.appendingPathComponent("hermisim.json")
            if cfg.exists {
                let m = JSON.object(cfg.readText()).str("model")
                if !m.trimmingCharacters(in: .whitespaces).isEmpty {
                    let f = cur.appendingPathComponent(m).standardizedFileURL
                    if f.exists { return f }
                    let byName = workspace.models.appendingPathComponent((m as NSString).lastPathComponent)
                    if byName.exists { return byName }
                }
            }
            d = cur.deletingLastPathComponent()
        }
        let xmls = ((try? FileManager.default.contentsOfDirectory(at: dir, includingPropertiesForKeys: nil)) ?? [])
            .filter { $0.pathExtension == "xml" }.sorted { $0.lastPathComponent < $1.lastPathComponent }
        if let f = xmls.first(where: isMjcf) { return f }
        return model.map { URL(fileURLWithPath: $0.path) }
    }

    /// Loads `file` (no-op if already open). Returns an error message or nil.
    @discardableResult
    func load(_ file: URL) -> String? {
        lock.lock(); defer { lock.unlock() }
        let cur = current.model
        if let cur, URL(fileURLWithPath: cur.path).standardizedFileURL.resolvingSymlinksInPath().path ==
            file.standardizedFileURL.resolvingSymlinksInPath().path { return nil }
        apply { $0.loading = true; $0.error = nil }
        let r = JSON.object(TulpaNative.simLoad(file.path))
        if r["error"] != nil {
            let e = r.str("error")
            apply { $0.loading = false; $0.error = e }
            return e
        }
        let h = Int64(r.int("handle"))
        let info = JSON.object(TulpaNative.simInfo(h) ?? "{}")
        let st = (info.arr("stat") ?? []).compactMap { ($0 as? NSNumber)?.floatValue }
        let name = info.str("model").trimmingCharacters(in: .whitespaces)
        let m = SimModel(handle: h, path: file.path, name: name.isEmpty ? file.nameWithoutExtension : name, info: info,
                         center: SIMD3(st.count > 0 ? st[0] : 0, st.count > 1 ? st[1] : 0, st.count > 2 ? st[2] : 0),
                         extent: st.count > 3 ? st[3] : 1, meshes: info.int("nmesh"))
        if let cur { TulpaNative.simFree(cur.handle) }
        TulpaNative.simControl(h, "realtime", current.realtime)
        generation += 1
        apply { $0.model = m; $0.loading = false; $0.address = nil; $0.error = nil }
        return nil
    }

    func unload() {
        lock.lock(); defer { lock.unlock() }
        if let m = current.model { TulpaNative.simFree(m.handle) }
        generation += 1
        apply { $0.model = nil; $0.address = nil }
    }

    @discardableResult
    func control(_ cmd: String, _ arg: Double = 0) -> String {
        let h = handle
        return h == 0 ? "no model loaded" : TulpaNative.simControl(h, cmd, arg)
    }

    func setRealtime(_ r: Double) {
        apply { $0.realtime = r }
        control("realtime", r)
    }

    enum PrepareError: Error { case message(String) }

    /// Called before running `source` from `dir`. Returns the co-sim address the program will
    /// use (also exported as TULPA_COSIM), or an error. Programs without `cosim::` clear it.
    func prepareRun(source: String, dir: URL, remote: String?) -> Result<String?, PrepareError> {
        if !source.contains("cosim::") { TulpaNative.setEnv("TULPA_COSIM", nil); return .success(nil) }
        if let remote = remote?.trimmingCharacters(in: .whitespaces), !remote.isEmpty {
            TulpaNative.setEnv("TULPA_COSIM", remote)
            return .success(remote)
        }
        let starter = workspace.models.appendingPathComponent("starter_model.xml")
        guard let m = modelFor(dir) ?? (starter.exists ? starter : nil) else {
            return .failure(.message("cosim: no model — open one in the Sim tab or add a .xml MJCF to the project"))
        }
        if let e = load(m) { return .failure(.message("cosim: cannot load \(m.lastPathComponent): \(e)")) }
        control("pause")
        control("reset")
        let addr = TulpaNative.simServe(handle, lan: false)
        if addr.hasPrefix("error") { return .failure(.message(addr)) }
        apply { $0.address = addr }
        TulpaNative.setEnv("TULPA_COSIM", addr)
        return .success(addr)
    }
}

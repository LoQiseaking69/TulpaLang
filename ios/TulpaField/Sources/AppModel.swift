import Foundation
import SwiftUI
import UIKit

enum Screen: String { case home, projects, code, run, sim, repl, learn, sensors, machines, verify, bytecode, settings }

struct EditorState {
    var project: URL? = nil
    var file: URL? = nil
    var files: [URL] = []
    var text = ""
    /// UTF-16 selection, as UITextView reports it.
    var selection = NSRange(location: 0, length: 0)
    var dirty = false
    var readOnly = false
    var diagnostics: [Diagnostic] = []
    var completions: [Completion] = []
    var prefix = ""
    var revealLine: Int? = nil       // one-shot: scroll here
    /// Bumped whenever the model (not the user typing) changes text or selection.
    var revision = 0
}

struct TestsResult: Equatable { let passed: Int; let failed: [(String, String)]; let output: String; let error: String?
    static func == (a: TestsResult, b: TestsResult) -> Bool { a.passed == b.passed && a.output == b.output && a.error == b.error && a.failed.count == b.failed.count }
}

struct VerifyState: Equatable {
    var running = false
    var results: [VerifyResult]? = nil
    var error: String? = nil
    var tests: TestsResult? = nil
    var testsRunning = false
}

struct ShareItem: Identifiable { let id = UUID(); let items: [Any] }

/// App state and actions; a port of the Android `AppViewModel`.
final class AppModel: ObservableObject {
    let workspace: Workspace
    let settings: Settings
    let sensors: SensorBridge
    let runner: RunManager
    let repl: ReplManager
    let sim: SimManager
    let host = TulpaHost.shared

    @Published var screen: Screen = .home
    private var backStack: [Screen] = []

    @Published private(set) var projects: [Project] = []
    lazy var examples: [Example] = workspace.examples()
    lazy var guide: [GuideDoc] = workspace.guide()
    lazy var modules: [String: [String]] = Analysis.modules()

    @Published var editor = EditorState()
    private var undoStack: [(String, NSRange)] = []
    private var redoStack: [(String, NSRange)] = []
    private var lastUndoPush = Date.distantPast

    @Published var symbols: [Symbol] = []
    @Published var machines: [MachineGraph] = []
    @Published var bytecode = ""
    @Published var verify = VerifyState()
    @Published var guideDoc: String? = nil          // id of open guide chapter
    @Published var guideTab: Int? = nil             // one-shot: 0 library, 1 guide
    @Published var openExampleId: String? = nil     // one-shot: show this example's page
    @Published var newProjectRequest = false
    @Published var importModelRequest = false
    @Published var runArgs = ""
    @Published var virtualTime = false
    @Published var toastText: String? = nil
    @Published var share: ShareItem? = nil

    private var analysisWork: DispatchWorkItem?
    private var completionWork: DispatchWorkItem?
    private var verifyCancel: Int64 = 0
    private var toastWork: DispatchWorkItem?

    let appVersion: String = (Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String) ?? "dev"
    let buildNumber: String = (Bundle.main.infoDictionary?["CFBundleVersion"] as? String) ?? "0"
    lazy var tulpaVersion = TulpaNative.version()

    init() {
        workspace = Workspace()
        let installed = ((try? FileManager.default.attributesOfItem(atPath: Bundle.main.bundlePath)[.modificationDate]) as? Date)?.timeIntervalSince1970 ?? 0
        workspace.installAssets(version: "\(appVersion)+\(buildNumber)+\(Int(installed))")
        // import "std/rtl" etc. resolve against the bundled library
        setenv("TULPA_PATH", workspace.stdlib.path, 1)
        TulpaNative.initialize()
        TulpaHost.shared.setup()
        settings = Settings()
        sensors = SensorBridge()
        runner = RunManager(sensors: sensors)
        repl = ReplManager(workspace: workspace, sensors: sensors)
        sim = SimManager(workspace: workspace)
        TulpaHost.shared.onSimNotify = { [weak runner] m, p in runner?.simNotify(method: m, params: p) }
        TulpaHost.shared.onToast = { [weak self] t in self?.toast(t) }
        refreshProjects()
        if let p = settings.lastProject {
            let dir = URL(fileURLWithPath: p, isDirectory: true)
            if dir.isDirectoryURL {
                let f = settings.lastFile.map { URL(fileURLWithPath: $0) }.flatMap { $0.exists ? $0 : nil }
                openProject(dir, file: f, navigate: false)
            }
        }
    }

    // ------------------------------------------------------------------ navigation

    func go(_ s: Screen) {
        if screen == s { return }
        if screen == .code { autosave() }
        backStack.append(screen)
        if backStack.count > 20 { backStack.removeFirst() }
        screen = s
        switch s {
        case .machines: refreshMachines()
        case .bytecode: refreshBytecode()
        case .projects: refreshProjects()
        case .sim: ensureSimModel()
        default: break
        }
    }

    /// Returns false when there is nothing to go back to.
    @discardableResult
    func back() -> Bool {
        if screen == .learn && guideDoc != nil { guideDoc = nil; return true }
        guard let prev = backStack.popLast() else { return false }
        if screen == .code { autosave() }
        screen = prev
        return true
    }

    var canGoBack: Bool { !backStack.isEmpty }

    func toast(_ t: String) {
        let show = { [self] in
            toastText = t
            toastWork?.cancel()
            let w = DispatchWorkItem { [weak self] in self?.toastText = nil }
            toastWork = w
            DispatchQueue.main.asyncAfter(deadline: .now() + 2.6, execute: w)
        }
        if Thread.isMainThread { show() } else { DispatchQueue.main.async(execute: show) }
    }

    func haptic() {
        if settings.prefs.haptics { UIImpactFeedbackGenerator(style: .light).impactOccurred() }
    }

    // ------------------------------------------------------------------ projects

    func refreshProjects() { projects = workspace.projects() }

    func newProject(_ name: String, template: Template) {
        let dir = workspace.createProject(name, template: template)
        refreshProjects()
        openProject(dir)
    }

    func openProject(_ dir: URL, file: URL? = nil, navigate: Bool = true) {
        autosave()
        let files = workspace.files(dir)
        let f = file ?? files.first { $0.lastPathComponent == "main.rav" } ?? files.first { $0.pathExtension == "rav" } ?? files.first
        settings.lastProject = dir.path
        editor = EditorState(project: dir, files: files, revision: editor.revision + 1)
        repl.setBaseDir(dir)
        if let f { openFile(f) }
        if navigate { go(.code) }
    }

    static let textExtensions: Set<String> = ["rav", "tlp", "txt", "md", "csv", "json", "vcd", "toml", "cir", "net", "xml", ""]

    func openFile(_ f: URL) {
        autosave()
        let text = f.readText()
        undoStack.removeAll(); redoStack.removeAll()
        settings.lastFile = f.path
        let isText = Self.textExtensions.contains(f.pathExtension.lowercased())
        editor.file = f
        editor.text = text
        editor.selection = NSRange(location: 0, length: 0)
        editor.dirty = false
        editor.readOnly = !isText || f.fileSize > 400_000
        editor.diagnostics = []
        editor.completions = []
        editor.revision += 1
        scheduleAnalysis(0)
    }

    func copyExample(_ e: Example) {
        let dir = workspace.copyExample(e)
        refreshProjects()
        openProject(dir)
        toast("Copied “\(e.title)” to your projects")
    }

    func renameProject(_ p: Project, _ name: String) {
        let target = workspace.root.appendingPathComponent(workspace.uniqueName(name), isDirectory: true)
        if (try? FileManager.default.moveItem(at: p.dir, to: target)) != nil, editor.project?.standardizedFileURL == p.dir.standardizedFileURL {
            openProject(target, navigate: false)
        }
        refreshProjects()
    }

    func duplicateProject(_ p: Project) { _ = workspace.duplicate(p.dir); refreshProjects() }

    func deleteProject(_ p: Project) {
        if editor.project?.standardizedFileURL == p.dir.standardizedFileURL {
            editor = EditorState(revision: editor.revision + 1); settings.lastProject = nil
        }
        workspace.delete(p.dir); refreshProjects()
    }

    func shareProject(_ p: Project) {
        // The share sheet zips folders itself ("Save to Files", AirDrop, Mail…)
        share = ShareItem(items: [p.dir])
    }

    func shareFile(_ f: URL) { share = ShareItem(items: [f]) }

    func newFile(_ name: String) {
        guard let p = editor.project else { return }
        let f = workspace.newFile(p, name)
        editor.files = workspace.files(p)
        openFile(f)
    }

    func renameFile(_ name: String) {
        guard let f = editor.file, let p = editor.project else { return }
        autosave()
        let t = workspace.rename(f, name)
        editor.file = t
        editor.files = workspace.files(p)
        settings.lastFile = t.path
    }

    func deleteFile() {
        guard let f = editor.file, let p = editor.project else { return }
        workspace.delete(f)
        let files = workspace.files(p)
        editor.files = files
        editor.file = nil
        editor.text = ""
        editor.dirty = false
        editor.revision += 1
        if let first = files.first { openFile(first) }
    }

    func refreshFiles() {
        guard let p = editor.project else { return }
        editor.files = workspace.files(p)
    }

    /// Imports a file picked in Files or opened with "Open in Tulpa Field".
    func importFile(_ url: URL) {
        let scoped = url.startAccessingSecurityScopedResource()
        defer { if scoped { url.stopAccessingSecurityScopedResource() } }
        guard let data = try? Data(contentsOf: url), let text = String(data: data, encoding: .utf8) ?? String(data: data, encoding: .isoLatin1) else {
            toast("Could not read that file"); return
        }
        if url.pathExtension.lowercased() == "xml" && text.contains("<mujoco") { importModel(text: text, name: url.lastPathComponent); return }
        let f = workspace.importText(text, name: url.lastPathComponent)
        refreshProjects()
        openProject(f.deletingLastPathComponent(), file: f)
        toast("Imported \(f.lastPathComponent) into Inbox")
    }

    func importText(_ text: String, name: String = "shared.rav") {
        let f = workspace.importText(text, name: name)
        refreshProjects()
        openProject(f.deletingLastPathComponent(), file: f)
    }

    // ------------------------------------------------------------------ editing

    /// Called by the code view when the user types or moves the caret.
    func userEdited(text: String, selection: NSRange) {
        if editor.readOnly { editor.selection = selection; return }
        let changed = text != editor.text
        if changed {
            let now = Date()
            if now.timeIntervalSince(lastUndoPush) > 0.7 || undoStack.isEmpty {
                undoStack.append((editor.text, editor.selection))
                if undoStack.count > 200 { undoStack.removeFirst() }
                lastUndoPush = now
            }
            redoStack.removeAll()
            editor.text = text
            editor.dirty = true
        }
        let moved = selection != editor.selection
        editor.selection = selection
        if changed { scheduleAnalysis(0.35); scheduleCompletion() }
        else if moved && !editor.completions.isEmpty { editor.completions = [] }
    }

    /// Programmatic edit (toolbar keys, completion, smart edits): pushes undo and refreshes the view.
    func setText(_ text: String, selection: NSRange) {
        guard !editor.readOnly else { return }
        undoStack.append((editor.text, editor.selection))
        if undoStack.count > 200 { undoStack.removeFirst() }
        lastUndoPush = Date()
        redoStack.removeAll()
        editor.text = text
        editor.selection = selection
        editor.dirty = true
        editor.revision += 1
        scheduleAnalysis(0.35); scheduleCompletion()
    }

    func undo() {
        guard let prev = undoStack.popLast() else { return }
        redoStack.append((editor.text, editor.selection))
        editor.text = prev.0; editor.selection = prev.1; editor.dirty = true; editor.completions = []
        editor.revision += 1
        scheduleAnalysis(0.2)
    }

    func redo() {
        guard let next = redoStack.popLast() else { return }
        undoStack.append((editor.text, editor.selection))
        editor.text = next.0; editor.selection = next.1; editor.dirty = true
        editor.revision += 1
        scheduleAnalysis(0.2)
    }

    var canUndo: Bool { !undoStack.isEmpty }
    var canRedo: Bool { !redoStack.isEmpty }

    func save(quiet: Bool = false) {
        guard let f = editor.file, !editor.readOnly, editor.dirty else { return }
        do {
            try editor.text.write(to: f, atomically: true, encoding: .utf8)
            editor.dirty = false
            if !quiet { toast("Saved \(f.lastPathComponent)") }
        } catch {
            toast("Save failed: \(error.localizedDescription)")
        }
    }

    func autosave() { if settings.prefs.autosave { save(quiet: true) } }

    /// UTF-16 offset of (line, col).
    func offset(line: Int, col: Int) -> Int {
        let ns = editor.text as NSString
        var off = 0, l = 0
        while l < line && off < ns.length {
            let r = ns.range(of: "\n", range: NSRange(location: off, length: ns.length - off))
            if r.location == NSNotFound { off = ns.length; break }
            off = r.location + 1; l += 1
        }
        return min(off + col, ns.length)
    }

    func reveal(line: Int, col: Int = 0) {
        editor.selection = NSRange(location: offset(line: line, col: col), length: 0)
        editor.revealLine = line
        editor.revision += 1
        go(.code)
    }

    func revealHandled() { editor.revealLine = nil }

    /// Cursor as zero-based (line, col).
    func cursor() -> (Int, Int) {
        let ns = editor.text as NSString
        let pos = min(max(editor.selection.location, 0), ns.length)
        let before = ns.substring(to: pos) as NSString
        var line = 0
        var lastNl = -1
        for i in 0..<before.length where before.character(at: i) == 10 { line += 1; lastNl = i }
        return (line, pos - (lastNl + 1))
    }

    private func scheduleAnalysis(_ delay: Double) {
        analysisWork?.cancel()
        let w = DispatchWorkItem { [weak self] in
            guard let self else { return }
            let src = self.editor.text
            guard let ext = self.editor.file?.pathExtension, ext == "rav" || ext == "tlp" else { return }
            Analysis.async({ Analysis.diagnostics(src) }) { d in
                if self.editor.text == src { self.editor.diagnostics = d }
            }
        }
        analysisWork = w
        DispatchQueue.main.asyncAfter(deadline: .now() + delay, execute: w)
    }

    private static func isIdent(_ c: unichar) -> Bool {
        guard let s = Unicode.Scalar(c) else { return false }
        return c == 95 || CharacterSet.alphanumerics.contains(s)
    }

    private func scheduleCompletion() {
        completionWork?.cancel()
        let w = DispatchWorkItem { [weak self] in
            guard let self else { return }
            let sel = self.editor.selection
            let text = self.editor.text
            let ns = text as NSString
            let pos = sel.location
            if sel.length != 0 || pos == 0 || pos > ns.length { self.editor.completions = []; return }
            var st = pos
            while st > 0 && Self.isIdent(ns.character(at: st - 1)) { st -= 1 }
            let prefix = ns.substring(with: NSRange(location: st, length: pos - st))
            let afterColons = st >= 2 && ns.substring(with: NSRange(location: st - 2, length: 2)) == "::"
            if prefix.isEmpty && !afterColons { self.editor.completions = []; self.editor.prefix = ""; return }
            var module = ""
            if afterColons {
                var ms = st - 2
                while ms > 0 && Self.isIdent(ns.character(at: ms - 1)) { ms -= 1 }
                module = ns.substring(with: NSRange(location: ms, length: st - 2 - ms))
            }
            let (line, col) = self.cursor()
            let members = self.modules[module]
            Analysis.async({ Analysis.completion(text, line, col) }) { found in
                var items = found
                if afterColons, let members, !items.contains(where: { members.contains($0.label) }) {
                    items = members.map { Completion(label: $0, kind: 3, detail: "\(module)::") }
                }
                var seen = Set<String>()
                let filtered = items.filter { $0.label.hasPrefix(prefix) && $0.label != prefix && seen.insert($0.label).inserted }
                    .sorted { $0.label.count < $1.label.count }.prefix(30)
                if self.editor.text == text { self.editor.completions = Array(filtered); self.editor.prefix = prefix }
            }
        }
        completionWork = w
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.12, execute: w)
    }

    func applyCompletion(_ c: Completion) {
        let ns = editor.text as NSString
        let pos = editor.selection.location
        let st = max(0, pos - (editor.prefix as NSString).length)
        let t = ns.replacingCharacters(in: NSRange(location: st, length: pos - st), with: c.label)
        setText(t, selection: NSRange(location: st + (c.label as NSString).length, length: 0))
        editor.completions = []
        completionWork?.cancel()
    }

    func insert(_ s: String, caretBack: Int = 0) {
        let ns = editor.text as NSString
        let r = editor.selection
        let t = ns.replacingCharacters(in: r, with: s)
        setText(t, selection: NSRange(location: r.location + (s as NSString).length - caretBack, length: 0))
    }

    func moveCaret(_ delta: Int) {
        let p = min(max(editor.selection.location + delta, 0), (editor.text as NSString).length)
        editor.selection = NSRange(location: p, length: 0)
        editor.revision += 1
    }

    func caretToLineEnd() {
        let ns = editor.text as NSString
        let from = editor.selection.location
        let r = ns.range(of: "\n", range: NSRange(location: from, length: ns.length - from))
        editor.selection = NSRange(location: r.location == NSNotFound ? ns.length : r.location, length: 0)
        editor.revision += 1
    }

    func hoverAtCursor(_ done: @escaping (String?) -> Void) {
        let (l, c) = cursor()
        let src = editor.text
        Analysis.async({ Analysis.hover(src, l, c) ?? Analysis.hover(src, l, max(c - 1, 0)) }, done)
    }

    func gotoDefinition() {
        let (l, c) = cursor()
        let src = editor.text
        Analysis.async({ Analysis.definition(src, l, c) ?? Analysis.definition(src, l, max(c - 1, 0)) }) { d in
            if let d { self.reveal(line: d.0, col: d.1) } else { self.toast("No definition found here") }
        }
    }

    func refreshSymbols() {
        let src = editor.text
        Analysis.async({ Analysis.symbols(src) }) { self.symbols = $0 }
    }

    func refreshMachines() {
        let src = editor.text
        Analysis.async({ Analysis.machines(src) }) { self.machines = $0 }
    }

    func refreshBytecode() {
        let src = editor.text
        bytecode = "compiling…"
        Analysis.async({ TulpaNative.bytecode(src) }) { self.bytecode = $0 }
    }

    // ------------------------------------------------------------------ welcome / navigation helpers

    func openGuide(_ id: String?) { guideTab = 1; guideDoc = id; go(.learn) }
    func openExample(_ id: String) { guideTab = 0; guideDoc = nil; openExampleId = id; go(.learn) }
    func requestNewProject() { newProjectRequest = true }

    // ------------------------------------------------------------------ simulation

    /// Loads a model for the Sim tab if none is open: the project's, else the cart-pole.
    func ensureSimModel() {
        if sim.state.model != nil || sim.state.loading { return }
        let f = editor.project.flatMap { sim.modelFor($0) } ?? workspace.models.appendingPathComponent("cartpole.xml")
        loadSimModel(f)
    }

    func loadSimModel(_ f: URL) {
        DispatchQueue.global(qos: .userInitiated).async {
            if let e = self.sim.load(f) { self.toast("Cannot load \(f.lastPathComponent): \(e)") }
        }
    }

    func importModel(text: String, name: String) {
        guard text.contains("<mujoco") else { toast("That file is not a MuJoCo MJCF model"); return }
        let dir: URL
        if let p = editor.project { dir = p } else { dir = workspace.createProject("Models", template: .blank); refreshProjects() }
        var n = (name as NSString).lastPathComponent
        if !n.hasSuffix(".xml") { n += ".xml" }
        let f = dir.appendingPathComponent(n)
        try? text.write(to: f, atomically: true, encoding: .utf8)
        refreshFiles()
        loadSimModel(f)
        toast("Imported \(f.lastPathComponent) into \(dir.lastPathComponent)")
    }

    /// A new project wired to `m`: hermisim.json + a starter controller using its actuators.
    func newCosimProject(_ m: SimModel) {
        let dir = workspace.createProject("\(m.name) co-sim", template: .blank)
        let modelURL = URL(fileURLWithPath: m.path)
        let bundled = modelURL.deletingLastPathComponent().standardizedFileURL.path == workspace.models.standardizedFileURL.path
        let rel: String
        if bundled { rel = "../../models/\(modelURL.lastPathComponent)" } else {
            try? FileManager.default.copyItem(at: modelURL, to: dir.appendingPathComponent(modelURL.lastPathComponent))
            rel = modelURL.lastPathComponent
        }
        let cfg: [String: Any] = ["name": m.name, "model": rel, "entry": "main.rav"]
        if let d = try? JSONSerialization.data(withJSONObject: cfg, options: [.prettyPrinted, .sortedKeys]) {
            try? d.write(to: dir.appendingPathComponent("hermisim.json"))
        }
        let main = """
        # Co-simulation with \(m.name): \(m.actuators) actuators, \(m.joints) joints.
        # Run this file, then open the Sim tab to watch it in 3D.

        let sim = cosim::connect()
        let info = sim.info()
        print(f"{info.model}: {info.nu} actuators, timestep {info.timestep} s")
        for a in info.actuators { print(" ", a.name, a.ctrlrange) }

        let o = sim.reset()
        let ctrl = [0.0] * info.nu
        while o.time < 5.0 {
            # your controller here: compute ctrl from the observation o
            o = sim.step(ctrl, 10)
            tlm::plot("com z", o.com[2])
        }
        print(f"done at t = {o.time:.2f} s, com = {o.com}")

        """
        try? main.write(to: dir.appendingPathComponent("main.rav"), atomically: true, encoding: .utf8)
        refreshProjects()
        openProject(dir)
    }

    /// Exports TULPA_COSIM for programs that use cosim::; returns a console note or an error.
    private func prepareCosim(_ src: String, _ dir: URL, _ done: @escaping (Result<String?, SimManager.PrepareError>) -> Void) {
        let remote = settings.prefs.remoteCosim.trimmingCharacters(in: .whitespaces)
        DispatchQueue.global(qos: .userInitiated).async {
            let r = self.sim.prepareRun(source: src, dir: dir, remote: remote.isEmpty ? nil : remote).map { addr -> String? in
                guard let addr else { return nil }
                if !remote.isEmpty { return "co-sim: remote simulator at \(addr)" }
                return "co-sim: \(self.sim.model?.name ?? "model") (MuJoCo, on device) — open the Sim tab to watch in 3D"
            }
            DispatchQueue.main.async { done(r) }
        }
    }

    // ------------------------------------------------------------------ running

    private func permissionsFor(_ src: String) {
        if src.contains("phone::location") { sensors.requestLocation() }
        sensors.proximityWanted = src.contains("phone::proximity")
    }

    private func busy() -> Bool {
        if runner.running { toast("A program is already running — stop it first"); return true }
        return false
    }

    func runCurrent() {
        guard let f = editor.file else { toast("Open a file first"); return }
        save(quiet: true)
        if busy() { return }
        let src = editor.text
        permissionsFor(src)
        let args = runArgs.split(whereSeparator: { $0.isWhitespace }).map(String.init)
        let dir = f.deletingLastPathComponent()
        let title = editor.project.map { f.relative(to: $0) } ?? f.lastPathComponent
        let vt = virtualTime
        prepareCosim(src, dir) { r in
            switch r {
            case .failure(.message(let m)): self.toast(m)
            case .success(let note):
                self.runner.start(source: src, file: f, baseDir: dir, args: args, virtualTime: vt, title: title, note: note)
                self.go(note != nil && self.screen == .sim ? .sim : .run)
            }
        }
    }

    func runExample(_ e: Example) {
        if busy() { return }
        let src = e.main.readText()
        permissionsFor(src)
        prepareCosim(src, e.dir) { r in
            switch r {
            case .failure(.message(let m)): self.toast(m)
            case .success(let note):
                self.runner.start(source: src, file: e.main, baseDir: e.dir, args: e.args, virtualTime: e.virtualTime, title: e.title, note: note)
                self.go(note != nil ? .sim : .run)
            }
        }
    }

    func runSnippet(_ code: String, title: String = "snippet") {
        if busy() { return }
        let dir = workspace.scratch
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let f = dir.appendingPathComponent("snippet.rav")
        try? code.write(to: f, atomically: true, encoding: .utf8)
        permissionsFor(code)
        prepareCosim(code, dir) { r in
            switch r {
            case .failure(.message(let m)): self.toast(m)
            case .success(let note):
                self.runner.start(source: code, file: f, baseDir: dir, args: [], virtualTime: false, title: title, note: note)
                self.go(.run)
            }
        }
    }

    func snippetToProject(_ code: String, title: String) {
        let dir = workspace.createProject(title, template: .blank)
        try? code.write(to: dir.appendingPathComponent("main.rav"), atomically: true, encoding: .utf8)
        refreshProjects()
        openProject(dir)
    }

    func loadIntoRepl(_ code: String, name: String) { repl.load(code, name: name); go(.repl) }

    // ------------------------------------------------------------------ verification

    func runVerify() {
        guard let f = editor.file else { return }
        let src = editor.text
        let base = f.deletingLastPathComponent().path
        let trials = settings.prefs.trials
        verify.running = true; verify.error = nil
        let handle = TulpaNative.newCancel()
        verifyCancel = handle
        TulpaThreads.start("tulpa-verify") {
            let json = TulpaNative.verify(source: src, baseDir: base, trials: trials, cancel: handle)
            TulpaNative.freeCancel(handle)
            let (r, err) = Analysis.parseVerify(json)
            DispatchQueue.main.async {
                if self.verifyCancel == handle { self.verifyCancel = 0 }
                self.verify.running = false; self.verify.results = r; self.verify.error = err
            }
        }
    }

    func stopVerify() { if verifyCancel != 0 { TulpaNative.cancel(verifyCancel) } }

    func runTests() {
        guard let f = editor.file else { return }
        let src = editor.text
        verify.testsRunning = true
        let handle = TulpaNative.newCancel()
        let out = NSMutableString()
        TulpaThreads.start("tulpa-tests") {
            let listener = RunListener(onOutput: { text in
                let lines = text.split(separator: "\n", omittingEmptySubsequences: false).filter { !$0.hasPrefix("@@tlm") }
                out.append(lines.joined(separator: "\n"))
            }, onInput: { _ in nil })
            let json = TulpaNative.tests(source: src, file: f.path, baseDir: f.deletingLastPathComponent().path, listener: listener, cancel: handle)
            TulpaNative.freeCancel(handle)
            let o = JSON.object(json)
            let res: TestsResult
            if o["error"] != nil {
                res = TestsResult(passed: 0, failed: [], output: out as String, error: o.str("error"))
            } else {
                let failed = (o.arr("failed") ?? []).compactMap { $0 as? [String: Any] }.map { ($0.str("name"), $0.str("error")) }
                res = TestsResult(passed: o.int("passed"), failed: failed, output: out as String, error: nil)
            }
            DispatchQueue.main.async { self.verify.testsRunning = false; self.verify.tests = res }
        }
    }

    func tryCounterexample(_ r: VerifyResult) {
        repl.load(editor.text, name: editor.file?.lastPathComponent ?? "file")
        repl.submit("\(r.name)(\(r.counterexample.joined(separator: ", ")))")
        go(.repl)
    }

    func shareText(_ text: String, title: String) { share = ShareItem(items: [text]) }

    /// Keeps the screen on while a program runs (Settings) or while it asks for phone::keep_awake.
    func updateIdleTimer() {
        UIApplication.shared.isIdleTimerDisabled = host.keepAwake || (runner.state.running && settings.prefs.keepScreenOn)
    }
}

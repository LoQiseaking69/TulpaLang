import Foundation

enum ReplKind { case input, output, value, error, info }

struct ReplEntry: Identifiable, Equatable { let id: Int; let kind: ReplKind; let text: String }

struct ReplState: Equatable {
    var entries: [ReplEntry] = []
    var busy = false
    var continuation = false
    var names: [String] = []
    var prompt: String? = nil
}

/// One persistent Tulpa session on its own big-stack thread.
final class ReplManager: ObservableObject {
    @Published private(set) var state = ReplState()
    private(set) var history: [String]

    private let worker = TulpaThreads.Worker("tulpa-repl")
    private let sensors: SensorBridge
    private let historyFile: URL
    private var handle: Int64 = 0          // worker thread only
    private var ids = 0
    private let inputQueue = BlockingQueue<String?>()
    private var baseDir: URL
    private var buf = ""                   // worker thread only

    private lazy var listener = RunListener(
        onOutput: { [weak self] text in
            guard let self else { return }
            self.buf += text
            while let nl = self.buf.firstIndex(of: "\n") {
                let l = String(self.buf[..<nl])
                self.buf = String(self.buf[self.buf.index(after: nl)...])
                if !l.hasPrefix("@@tlm ") { self.add(.output, l) }
            }
        },
        onInput: { [weak self] prompt in
            guard let self else { return nil }
            self.update { $0.prompt = prompt }
            let a = self.inputQueue.take()
            self.update { $0.prompt = nil }
            return a
        })

    init(workspace: Workspace, sensors: SensorBridge) {
        self.sensors = sensors
        historyFile = workspace.support.appendingPathComponent("repl_history.json")
        baseDir = workspace.support
        history = JSON.array(historyFile.readText()).compactMap { $0 as? String }
        add(.info, "Tulpa \(TulpaNative.version()) REPL — definitions persist; phone:: works here too.")
    }

    private func update(_ f: @escaping (inout ReplState) -> Void) {
        if Thread.isMainThread { f(&state) } else { DispatchQueue.main.async { f(&self.state) } }
    }

    private func add(_ kind: ReplKind, _ text: String) {
        update { [self] s in
            ids += 1
            s.entries.append(ReplEntry(id: ids, kind: kind, text: text))
            if s.entries.count > 2000 { s.entries.removeFirst(s.entries.count - 2000) }
        }
    }

    private func flushBuf() {
        if !buf.isEmpty { add(.output, buf); buf = "" }
    }

    /// Worker thread only.
    private func ensure() {
        if handle == 0 {
            handle = TulpaNative.sessionNew(listener: listener, baseDir: baseDir.path)
            sensors.acquire()
        }
    }

    private func names() -> [String] { JSON.array(TulpaNative.sessionNames(handle)).compactMap { $0 as? String } }

    func setBaseDir(_ dir: URL) { baseDir = dir }

    func submit(_ line: String) {
        if !line.trimmingCharacters(in: .whitespaces).isEmpty && history.last != line { history.append(line); saveHistory() }
        add(.input, (state.continuation ? "… " : "› ") + line)
        state.busy = true
        worker.post { [self] in
            ensure()
            let r = JSON.object(TulpaNative.sessionFeed(handle, line))
            flushBuf()
            switch r.str("kind") {
            case "more": update { $0.continuation = true }
            case "value":
                if !r.isNull("text") { add(.value, r.str("text")) }
                update { $0.continuation = false }
            default:
                add(.error, r.str("text", "error"))
                update { $0.continuation = false }
            }
            let n = names()
            update { $0.busy = false; $0.names = n }
        }
    }

    /// Runs a whole file into the session (definitions and top-level code).
    func load(_ source: String, name: String) {
        add(.info, "loaded \(name)")
        state.busy = true
        let cont = state.continuation
        worker.post { [self] in
            ensure()
            if cont { TulpaNative.sessionCancelEntry(handle) }
            let r = JSON.object(TulpaNative.sessionFeed(handle, source))
            flushBuf()
            if r.str("kind") == "error" { add(.error, r.str("text")) }
            if r.str("kind") == "more" {
                TulpaNative.sessionCancelEntry(handle)
                add(.error, "file is incomplete (unbalanced brackets)")
            }
            let n = names()
            update { $0.busy = false; $0.continuation = false; $0.names = n }
        }
    }

    func answer(_ text: String) {
        add(.input, (state.prompt ?? "") + text)
        inputQueue.put(text)
    }

    func interrupt() {
        let h = handle
        if h != 0 { TulpaNative.sessionInterrupt(h) }
        if state.prompt != nil { inputQueue.put(nil) }
    }

    func cancelEntry() {
        worker.post { [self] in if handle != 0 { TulpaNative.sessionCancelEntry(handle) } }
        state.continuation = false
        add(.info, "entry discarded")
    }

    func reset() {
        interrupt()
        worker.clearPending()
        worker.post { [self] in
            if handle != 0 {
                TulpaNative.sessionFree(handle); handle = 0; sensors.release()
            }
            update { $0 = ReplState() }
            add(.info, "session reset")
        }
    }

    func clearScreen() { state.entries = [] }

    private func saveHistory() {
        let h = Array(history.suffix(300))
        try? JSON.string(h).write(to: historyFile, atomically: true, encoding: .utf8)
    }
}

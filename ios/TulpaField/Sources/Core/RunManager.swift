import Foundation

enum LineKind { case out, err, info, event, input }

struct ConsoleLine: Identifiable, Equatable {
    let id: Int
    let kind: LineKind
    let text: String
    var line: Int? = nil
}

struct PlotSeries: Equatable {
    let name: String
    let unit: String?
    var t: [Float]
    var v: [Float]
    var count: Int
    var last: Float? { count > 0 ? v[(count - 1) % v.count] : nil }

    /// Points in chronological order (the buffers are rings of PLOT_POINTS).
    func ordered() -> [(Float, Float)] {
        let n = min(count, v.count)
        let start = count > v.count ? count % v.count : 0
        return (0..<n).map { k in let i = (start + k) % v.count; return (t[i], v[i]) }
    }
}

struct Artifact: Equatable, Identifiable { var id: String { path }; let path: String; let kind: String? }

struct RunState: Equatable {
    var running = false
    var title = ""
    var console: [ConsoleLine] = []
    var plots: [String: PlotSeries] = [:]
    var plotOrder: [String] = []
    var events: [(Float, String)] = []
    var status: String? = nil
    var artifacts: [Artifact] = []
    var prompt: String? = nil
    var startedAt = Date.distantPast
    var finishedAt = Date.distantPast
    var exit: String? = nil       // nil while running / on success; error text otherwise
    var ok: Bool? = nil

    static func == (a: RunState, b: RunState) -> Bool {
        a.running == b.running && a.title == b.title && a.console == b.console && a.plots == b.plots &&
            a.events.count == b.events.count && a.status == b.status && a.artifacts == b.artifacts &&
            a.prompt == b.prompt && a.exit == b.exit && a.ok == b.ok
    }
}

/// Runs one Tulpa program at a time on a 64 MiB thread; turns its output into console lines,
/// `@@tlm` telemetry into plots/events/status/artifacts, and bridges `input()`.
final class RunManager: ObservableObject {
    static let maxLines = 4000
    static let plotPoints = 600
    private static let lineRe = try! NSRegularExpression(pattern: "(?:line |:)(\\d+)(?::\\d+)?")

    @Published private(set) var state = RunState()

    private let sensors: SensorBridge
    private let lock = NSRecursiveLock()
    private var s = RunState()           // working copy (any thread), published on main
    private var publishPending = false
    private var cancel: Int64 = 0
    private var ids = 0
    private var partial = ""
    private var inputQueue: BlockingQueue<String?>?
    private var plotT0: Double?
    private var generation = 0

    init(sensors: SensorBridge) { self.sensors = sensors }

    var running: Bool { lock.lock(); defer { lock.unlock() }; return s.running }

    func clear() {
        lock.lock(); defer { lock.unlock() }
        if !s.running { s = RunState(); publish(now: true) }
    }

    private func mutate(_ f: (inout RunState) -> Void) {
        lock.lock(); f(&s); lock.unlock()
        publish()
    }

    /// Coalesces state updates into at most ~30 UI refreshes per second.
    private func publish(now: Bool = false) {
        lock.lock()
        if publishPending && !now { lock.unlock(); return }
        publishPending = true
        lock.unlock()
        DispatchQueue.main.asyncAfter(deadline: .now() + (now ? 0 : 0.033)) { [weak self] in
            guard let self else { return }
            self.lock.lock(); let snap = self.s; self.publishPending = false; self.lock.unlock()
            if self.state != snap { self.state = snap }
        }
    }

    func start(source: String, file: URL, baseDir: URL, args: [String], virtualTime: Bool,
               title: String? = nil, useSensors: Bool? = nil, note: String? = nil) {
        lock.lock()
        if s.running { lock.unlock(); return }
        generation += 1
        let gen = generation
        cancel = TulpaNative.newCancel()
        let handle = cancel
        partial = ""; plotT0 = nil
        let q = BlockingQueue<String?>()
        inputQueue = q
        let title = title ?? file.lastPathComponent
        var head = "━━ Run: \(title)"
        if !args.isEmpty { head += " " + args.joined(separator: " ") }
        if virtualTime { head += "  (virtual time)" }
        var console = [ConsoleLine(id: nextId(), kind: .info, text: head)]
        if let note { console.append(ConsoleLine(id: nextId(), kind: .info, text: note)) }
        s = RunState(running: true, title: title, console: console, startedAt: Date())
        lock.unlock()
        publish(now: true)
        TulpaHost.shared.resetForRun()
        TulpaHost.shared.onEvent = { [weak self] t in self?.emit(.event, t) }
        let sensorsOn = useSensors ?? source.contains("phone::")
        if sensorsOn { sensors.acquire() }

        TulpaThreads.start("tulpa-run") { [self] in
            let listener = RunListener(
                onOutput: { [weak self] text in
                    guard let self else { return }
                    self.lock.lock(); let live = gen == self.generation; self.lock.unlock()
                    if live { self.ingest(text) }
                },
                onInput: { [weak self] prompt in
                    guard let self else { return nil }
                    self.flushPartial()
                    self.mutate { $0.prompt = prompt }
                    while true {
                        if let a = q.poll(0.2) {
                            self.mutate { $0.prompt = nil }
                            return a
                        }
                        self.lock.lock(); let live = gen == self.generation; self.lock.unlock()
                        if !live { return nil }
                    }
                })
            let err = TulpaNative.run(source: source, file: file.path, baseDir: baseDir.path, args: args,
                                      virtualTime: virtualTime, listener: listener, cancel: handle)
            self.flushPartial()
            if sensorsOn { self.sensors.release() }
            TulpaHost.shared.releaseAll()
            TulpaNative.freeCancel(handle)
            let now = Date()
            self.lock.lock(); let secs = now.timeIntervalSince(self.s.startedAt); self.lock.unlock()
            if let err {
                if err == "interrupted" || err.hasSuffix(": interrupted") {
                    self.emit(.info, String(format: "━━ stopped after %.2f s", secs))
                } else {
                    self.emit(.err, err, self.lineOf(err))
                }
            } else {
                self.emit(.info, String(format: "━━ finished in %.2f s", secs))
            }
            self.mutate { st in st.running = false; st.prompt = nil; st.finishedAt = now; st.exit = err; st.ok = err == nil }
            self.lock.lock(); if self.cancel == handle { self.cancel = 0 }; self.lock.unlock()
            self.publish(now: true)
        }
    }

    /// `cosim` notifications from the program: `sim.plot(name, v)` and `sim.log(text)`.
    func simNotify(method: String, params: [String: Any]) {
        switch method {
        case "log": emit(.event, "▣ " + params.str("text"))
        case "plot":
            var x = Double.nan
            if let n = params["v"] as? NSNumber { x = n.doubleValue }
            else if let a = params["v"] as? [Any], let n = a.first as? NSNumber { x = n.doubleValue }
            telemetry(["k": "plot", "name": params.str("name"), "v": x, "t": Date().timeIntervalSince1970])
        default: break
        }
    }

    func stop() {
        lock.lock(); let h = cancel; let q = inputQueue; lock.unlock()
        if h != 0 { TulpaNative.cancel(h) }
        q?.put(nil)
    }

    func answer(_ text: String?) {
        lock.lock(); let p = s.prompt ?? ""; let q = inputQueue; lock.unlock()
        if let text { emit(.input, p + text) }
        q?.put(text)
    }

    private func nextId() -> Int { ids += 1; return ids }

    private func lineOf(_ err: String) -> Int? {
        let r = NSRange(err.startIndex..., in: err)
        guard let m = Self.lineRe.firstMatch(in: err, range: r), let g = Range(m.range(at: 1), in: err) else { return nil }
        return Int(err[g])
    }

    // ------------------------------------------------------------------ output parsing

    private func ingest(_ text: String) {
        lock.lock()
        partial += text
        while let nl = partial.firstIndex(of: "\n") {
            let line = String(partial[..<nl])
            partial = String(partial[partial.index(after: nl)...])
            handleLine(line)
        }
        lock.unlock()
        publish()
    }

    private func flushPartial() {
        lock.lock()
        if !partial.isEmpty { handleLine(partial); partial = "" }
        lock.unlock()
        publish()
    }

    /// Called with the lock held.
    private func handleLine(_ line: String) {
        if line.hasPrefix("@@tlm ") {
            if let o = JSON.parse(String(line.dropFirst(6))) as? [String: Any] { telemetry(o) }
            return
        }
        let isErr = line.hasPrefix("error") || line.hasPrefix("Error") || line.contains("safety envelope violated")
        append(ConsoleLine(id: nextId(), kind: isErr ? .err : .out, text: line, line: isErr ? lineOf(line) : nil))
    }

    private func append(_ l: ConsoleLine) {
        s.console.append(l)
        if s.console.count > Self.maxLines { s.console.removeFirst(s.console.count - Self.maxLines) }
    }

    private func telemetry(_ o: [String: Any]) {
        lock.lock(); defer { lock.unlock(); publish() }
        switch o.str("k") {
        case "plot":
            let name = o.str("name")
            let v = o.double("v")
            if v.isNaN { return }
            let t = o.double("t", 0)
            let t0 = plotT0 ?? t
            if plotT0 == nil { plotT0 = t }
            let unit: String? = o.isNull("unit") ? nil : o.str("unit")
            var p = s.plots[name] ?? PlotSeries(name: name, unit: unit, t: [Float](repeating: 0, count: Self.plotPoints),
                                                v: [Float](repeating: 0, count: Self.plotPoints), count: 0)
            if s.plots[name] == nil { s.plotOrder.append(name) }
            let i = p.count % Self.plotPoints
            p.t[i] = Float(t - t0); p.v[i] = Float(v); p.count += 1
            s.plots[name] = p
        case "event":
            var label = o.str("name")
            if let d = o["data"], !(d is NSNull) { label += " " + ((d as? String) ?? JSON.string(d)) }
            let t = Float(o.double("t", 0) - (plotT0 ?? 0))
            s.events.append((t, label))
            if s.events.count > 100 { s.events.removeFirst(s.events.count - 100) }
            append(ConsoleLine(id: nextId(), kind: .event, text: "◆ \(label)"))
        case "status":
            s.status = o.str("text")
        case "artifact":
            let a = Artifact(path: o.str("path"), kind: o.isNull("kind") ? nil : o.str("kind"))
            s.artifacts = s.artifacts.filter { $0.path != a.path } + [a]
        default: break
        }
    }

    private func emit(_ kind: LineKind, _ text: String, _ line: Int? = nil) {
        lock.lock(); append(ConsoleLine(id: nextId(), kind: kind, text: text, line: line)); lock.unlock()
        publish()
    }
}

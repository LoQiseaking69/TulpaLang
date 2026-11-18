import Foundation

/// Swift face of the Rust runtime (`ios/native`). Mirrors `TulpaNative.kt` on Android: every
/// call returns the same JSON so the two apps share one contract.
enum TulpaNative {
    /// Takes ownership of a string returned by Rust.
    static func take(_ p: UnsafeMutablePointer<CChar>?) -> String? {
        guard let p else { return nil }
        defer { tf_string_free(p) }
        return String(cString: p)
    }

    static func takeFloats(_ p: UnsafeMutablePointer<Float>?, _ n: Int32) -> [Float] {
        guard let p, n > 0 else { return [] }
        defer { tf_floats_free(p, n) }
        return Array(UnsafeBufferPointer(start: p, count: Int(n)))
    }

    static func initialize() {
        tf_init { kind, payload in
            let k = kind.map { String(cString: $0) } ?? ""
            let p = payload.map { String(cString: $0) } ?? "{}"
            TulpaHost.shared.action(kind: k, payload: p)
        }
    }

    static func version() -> String { take(tf_version()) ?? "" }
    static func simVersion() -> String { take(tf_sim_version()) ?? "" }
    static func setEnv(_ name: String, _ value: String?) {
        if let value { tf_set_env(name, value) } else { tf_set_env(name, nil) }
    }

    static func newCancel() -> Int64 { tf_cancel_new() }
    static func cancel(_ h: Int64) { tf_cancel(h) }
    static func freeCancel(_ h: Int64) { tf_cancel_free(h) }

    // ------------------------------------------------------------------ running

    /// Runs on the calling thread (which needs a big stack). Returns nil on success.
    static func run(source: String, file: String, baseDir: String, args: [String], virtualTime: Bool,
                    listener: RunListener, cancel: Int64) -> String? {
        let ctx = Unmanaged.passRetained(listener).toOpaque()
        defer { Unmanaged<RunListener>.fromOpaque(ctx).release() }
        let cargs: [UnsafePointer<CChar>?] = args.map { UnsafePointer(strdup($0)) }
        defer { cargs.forEach { free(UnsafeMutablePointer(mutating: $0)) } }
        return cargs.withUnsafeBufferPointer { argv in
            take(tf_run(source, file, baseDir, argv.baseAddress, Int32(args.count), virtualTime,
                        ctx, RunListener.outputThunk, RunListener.inputThunk, cancel))
        }
    }

    static func tests(source: String, file: String, baseDir: String, listener: RunListener, cancel: Int64) -> String {
        let ctx = Unmanaged.passRetained(listener).toOpaque()
        defer { Unmanaged<RunListener>.fromOpaque(ctx).release() }
        return take(tf_tests(source, file, baseDir, ctx, RunListener.outputThunk, RunListener.inputThunk, cancel)) ?? "{\"error\":\"no result\"}"
    }

    static func verify(source: String, baseDir: String, trials: Int, cancel: Int64) -> String {
        take(tf_verify(source, baseDir, Int32(trials), cancel)) ?? "{\"error\":\"no result\"}"
    }

    // ------------------------------------------------------------------ REPL

    /// The listener is retained until `sessionFree`.
    static func sessionNew(listener: RunListener, baseDir: String) -> Int64 {
        let ctx = Unmanaged.passRetained(listener).toOpaque()
        let h = tf_session_new(ctx, RunListener.outputThunk, RunListener.inputThunk, baseDir)
        if h == 0 { Unmanaged<RunListener>.fromOpaque(ctx).release() } else { sessionListeners[h] = ctx }
        return h
    }
    private static var sessionListeners: [Int64: UnsafeMutableRawPointer] = [:]
    static func sessionFeed(_ h: Int64, _ line: String) -> String { take(tf_session_feed(h, line)) ?? "{}" }
    static func sessionInterrupt(_ h: Int64) { tf_session_interrupt(h) }
    static func sessionCancelEntry(_ h: Int64) { tf_session_cancel_entry(h) }
    static func sessionNames(_ h: Int64) -> String { take(tf_session_names(h)) ?? "[]" }
    static func sessionFree(_ h: Int64) {
        tf_session_free(h)
        if let ctx = sessionListeners.removeValue(forKey: h) { Unmanaged<RunListener>.fromOpaque(ctx).release() }
    }

    // ------------------------------------------------------------------ analysis

    static func diagnostics(_ src: String) -> String { take(tf_diagnostics(src)) ?? "[]" }
    static func completion(_ src: String, _ line: Int, _ col: Int) -> String { take(tf_completion(src, Int32(line), Int32(col))) ?? "[]" }
    static func hover(_ src: String, _ line: Int, _ col: Int) -> String? { take(tf_hover(src, Int32(line), Int32(col))) }
    static func symbols(_ src: String) -> String { take(tf_symbols(src)) ?? "[]" }
    static func definition(_ src: String, _ line: Int, _ col: Int) -> String { take(tf_definition(src, Int32(line), Int32(col))) ?? "null" }
    static func machines(_ src: String) -> String { take(tf_machines(src)) ?? "[]" }
    static func bytecode(_ src: String) -> String { take(tf_bytecode(src)) ?? "" }
    static func modules() -> String { take(tf_modules()) ?? "{}" }

    // ------------------------------------------------------------------ sensors

    static func pushSensor(_ kind: Int, _ values: [Float]) {
        values.withUnsafeBufferPointer { tf_push_sensor(Int32(kind), $0.baseAddress, Int32($0.count)) }
    }
    static func pushLocation(lat: Double, lon: Double, alt: Double, accuracy: Double, speed: Double, bearing: Double) {
        tf_push_location(lat, lon, alt, accuracy, speed, bearing)
    }
    static func resetSensors() { tf_reset_sensors() }

    // ------------------------------------------------------------------ simulator

    static func simLoad(_ path: String) -> String { take(tf_sim_load(path)) ?? "{\"error\":\"no result\"}" }
    static func simFree(_ h: Int64) { tf_sim_free(h) }
    static func simInfo(_ h: Int64) -> String? { take(tf_sim_info(h)) }
    static func simFrame(_ h: Int64) -> [Float] { var n: Int32 = 0; let p = tf_sim_frame(h, &n); return takeFloats(p, n) }
    static func simMesh(_ h: Int64, _ id: Int) -> [Float] { var n: Int32 = 0; let p = tf_sim_mesh(h, Int32(id), &n); return takeFloats(p, n) }
    static func simServe(_ h: Int64, lan: Bool) -> String { take(tf_sim_serve(h, lan)) ?? "error: no result" }
    @discardableResult
    static func simControl(_ h: Int64, _ cmd: String, _ arg: Double = 0) -> String { take(tf_sim_control(h, cmd, arg)) ?? "" }
}

/// Receives a program's output and answers its `input()` calls. Called on the program thread.
class RunListener {
    let onOutput: (String) -> Void
    let onInput: (String) -> String?

    init(onOutput: @escaping (String) -> Void, onInput: @escaping (String) -> String?) {
        self.onOutput = onOutput
        self.onInput = onInput
    }

    static let outputThunk: tf_output_cb = { ctx, text in
        guard let ctx, let text else { return }
        Unmanaged<RunListener>.fromOpaque(ctx).takeUnretainedValue().onOutput(String(cString: text))
    }

    static let inputThunk: tf_input_cb = { ctx, prompt in
        guard let ctx else { return nil }
        let l = Unmanaged<RunListener>.fromOpaque(ctx).takeUnretainedValue()
        guard let answer = l.onInput(prompt.map { String(cString: $0) } ?? "") else { return nil }
        return strdup(answer)
    }
}

/// Tulpa's parser and evaluator are recursive: programs run on threads with big stacks.
enum TulpaThreads {
    static let stack = 64 * 1024 * 1024

    @discardableResult
    static func start(_ name: String, stackSize: Int = stack, _ body: @escaping () -> Void) -> Thread {
        let t = Thread(block: body)
        t.name = name
        t.stackSize = stackSize
        t.qualityOfService = .userInitiated
        t.start()
        return t
    }

    /// A single long-lived thread (REPL, analysis) fed through a queue.
    final class Worker {
        private let cond = NSCondition()
        private var queue: [() -> Void] = []

        init(_ name: String, stackSize: Int = TulpaThreads.stack) {
            TulpaThreads.start(name, stackSize: stackSize) { [unowned self] in self.loop() }
        }

        private func loop() {
            while true {
                cond.lock()
                while queue.isEmpty { cond.wait() }
                let job = queue.removeFirst()
                cond.unlock()
                autoreleasepool { job() }
            }
        }

        func post(_ job: @escaping () -> Void) {
            cond.lock(); queue.append(job); cond.signal(); cond.unlock()
        }

        func clearPending() {
            cond.lock(); queue.removeAll(); cond.unlock()
        }
    }
}

/// A blocking FIFO used to hand `input()` answers to the program thread.
final class BlockingQueue<T> {
    private let cond = NSCondition()
    private var items: [T] = []

    func put(_ x: T) { cond.lock(); items.append(x); cond.signal(); cond.unlock() }

    func take() -> T {
        cond.lock(); defer { cond.unlock() }
        while items.isEmpty { cond.wait() }
        return items.removeFirst()
    }

    /// Waits up to `timeout` seconds; nil on timeout.
    func poll(_ timeout: TimeInterval) -> T? {
        cond.lock(); defer { cond.unlock() }
        let deadline = Date().addingTimeInterval(timeout)
        while items.isEmpty { if !cond.wait(until: deadline) { break } }
        return items.isEmpty ? nil : items.removeFirst()
    }
}

// ---------------------------------------------------------------------- JSON helpers

enum JSON {
    static func parse(_ s: String) -> Any? {
        guard let d = s.data(using: .utf8) else { return nil }
        return try? JSONSerialization.jsonObject(with: d, options: [.fragmentsAllowed])
    }
    static func object(_ s: String) -> [String: Any] { parse(s) as? [String: Any] ?? [:] }
    static func array(_ s: String) -> [Any] { parse(s) as? [Any] ?? [] }
    static func string(_ v: Any) -> String {
        guard JSONSerialization.isValidJSONObject(v), let d = try? JSONSerialization.data(withJSONObject: v, options: [.sortedKeys]) else {
            if let s = v as? String { return "\"\(s)\"" }
            return "\(v)"
        }
        return String(data: d, encoding: .utf8) ?? ""
    }
}

extension Dictionary where Key == String, Value == Any {
    func str(_ k: String, _ d: String = "") -> String {
        if let s = self[k] as? String { return s }
        if let n = self[k] as? NSNumber { return n.stringValue }
        return d
    }
    func int(_ k: String, _ d: Int = 0) -> Int { (self[k] as? NSNumber)?.intValue ?? d }
    func double(_ k: String, _ d: Double = .nan) -> Double { (self[k] as? NSNumber)?.doubleValue ?? d }
    func bool(_ k: String, _ d: Bool = false) -> Bool { (self[k] as? NSNumber)?.boolValue ?? d }
    func obj(_ k: String) -> [String: Any]? { self[k] as? [String: Any] }
    func arr(_ k: String) -> [Any]? { self[k] as? [Any] }
    func strings(_ k: String) -> [String] { (self[k] as? [Any])?.map { ($0 as? String) ?? "\($0)" } ?? [] }
    func isNull(_ k: String) -> Bool { self[k] == nil || self[k] is NSNull }
}

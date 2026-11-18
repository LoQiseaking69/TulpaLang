import Foundation

struct Diagnostic: Equatable { let line, col, endLine, endCol: Int; let message: String; let error: Bool }
struct Completion: Equatable, Identifiable { var id: String { label }; let label: String; let kind: Int; let detail: String }
struct Symbol: Equatable, Identifiable { var id: String { "\(name):\(line)" }; let name: String; let kind: Int; let line: Int; let detail: String }

struct MachineState: Equatable { let name: String; let line: Int; let enter: Bool; let exit: Bool }
struct MachineTransition: Equatable {
    let from, to, event: String
    let params: [String]
    let guardText: String?
    let isInternal: Bool
    let global: Bool
    let line: Int
}
struct MachineGraph: Equatable, Identifiable {
    var id: String { name }
    let name, initial: String
    let states: [MachineState]
    let transitions: [MachineTransition]
    let mermaid: String
}

struct VerifyResult: Equatable, Identifiable {
    var id: String { name }
    let name: String
    let ok: Bool
    let passed, discarded: Int
    let counterexample: [String]
    let error: String?
}

/// Editor intelligence, executed on a dedicated big-stack worker (the parser is recursive).
enum Analysis {
    static let worker = TulpaThreads.Worker("tulpa-analysis", stackSize: 32 * 1024 * 1024)

    /// Runs `f` on the analysis worker and delivers the result on the main queue.
    static func async<T>(_ f: @escaping () -> T, _ done: @escaping (T) -> Void) {
        worker.post { let r = f(); DispatchQueue.main.async { done(r) } }
    }

    private static func line(_ o: [String: Any]) -> Int {
        o.obj("range")?.obj("start")?.int("line") ?? o.obj("location")?.obj("range")?.obj("start")?.int("line") ?? 0
    }

    static func diagnostics(_ src: String) -> [Diagnostic] {
        JSON.array(TulpaNative.diagnostics(src)).compactMap { x in
            guard let o = x as? [String: Any], let r = o.obj("range"), let s = r.obj("start"), let e = r.obj("end") else { return nil }
            return Diagnostic(line: s.int("line"), col: s.int("character"), endLine: e.int("line"), endCol: e.int("character"),
                              message: o.str("message"), error: o.int("severity", 1) == 1)
        }
    }

    static func completion(_ src: String, _ line: Int, _ col: Int) -> [Completion] {
        JSON.array(TulpaNative.completion(src, line, col)).compactMap { x in
            guard let o = x as? [String: Any] else { return nil }
            return Completion(label: o.str("label"), kind: o.int("kind"), detail: o.str("detail"))
        }
    }

    static func symbols(_ src: String) -> [Symbol] {
        JSON.array(TulpaNative.symbols(src)).compactMap { x in
            guard let o = x as? [String: Any] else { return nil }
            return Symbol(name: o.str("name"), kind: o.int("kind"), line: line(o), detail: o.str("detail"))
        }
    }

    static func hover(_ src: String, _ line: Int, _ col: Int) -> String? { TulpaNative.hover(src, line, col) }

    /// Zero-based (line, col) of the declaration, or nil.
    static func definition(_ src: String, _ line: Int, _ col: Int) -> (Int, Int)? {
        let s = TulpaNative.definition(src, line, col)
        guard s != "null", let o = JSON.parse(s) as? [String: Any], o["line"] != nil else { return nil }
        return (o.int("line"), o.int("col"))
    }

    static func machines(_ src: String) -> [MachineGraph] {
        JSON.array(TulpaNative.machines(src)).compactMap { x in
            guard let o = x as? [String: Any] else { return nil }
            let states = (o.arr("states") ?? []).compactMap { $0 as? [String: Any] }.map {
                MachineState(name: $0.str("name"), line: $0.int("line"), enter: $0.bool("enter"), exit: $0.bool("exit"))
            }
            let trs = (o.arr("transitions") ?? []).compactMap { $0 as? [String: Any] }.map { t in
                MachineTransition(from: t.str("from"), to: t.str("to"), event: t.str("event"), params: t.strings("params"),
                                  guardText: t.isNull("guard") ? nil : t.str("guard"), isInternal: t.bool("internal"),
                                  global: t.bool("global"), line: t.int("line"))
            }
            return MachineGraph(name: o.str("name"), initial: o.str("initial"), states: states, transitions: trs, mermaid: o.str("mermaid"))
        }
    }

    static func parseVerify(_ json: String) -> ([VerifyResult], String?) {
        let v = JSON.parse(json)
        if let o = v as? [String: Any] { return ([], o.str("error", "verification failed")) }
        guard let a = v as? [Any] else { return ([], "unreadable verification report") }
        return (a.compactMap { x in
            guard let o = x as? [String: Any] else { return nil }
            return VerifyResult(name: o.str("name"), ok: o.bool("ok"), passed: o.int("passed"), discarded: o.int("discarded"),
                                counterexample: o.strings("counterexample"), error: o.isNull("error") ? nil : o.str("error"))
        }, nil)
    }

    static func modules() -> [String: [String]] {
        var out: [String: [String]] = [:]
        for (k, v) in JSON.object(TulpaNative.modules()) { out[k] = (v as? [Any])?.compactMap { $0 as? String } ?? [] }
        return out
    }
}

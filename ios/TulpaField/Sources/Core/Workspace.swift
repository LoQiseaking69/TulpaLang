import Foundation

struct Project: Identifiable, Equatable {
    var id: String { dir.path }
    let name: String
    let dir: URL
    let files: Int
    let modified: Date
}

struct Example: Identifiable, Equatable {
    let id: String
    let title: String
    let category: String
    let summary: String
    let tags: [String]
    let virtualTime: Bool
    let dir: URL
    var args: [String] = []

    var main: URL { dir.appendingPathComponent("main.rav") }
    var expected: URL { dir.appendingPathComponent("expected.txt") }
    var cosim: Bool { ((try? String(contentsOf: main, encoding: .utf8)) ?? "").contains("cosim::") }
}

struct GuideDoc: Identifiable, Equatable {
    let id: String
    let title: String
    let section: String
    let asset: String
}

extension URL {
    var isDirectoryURL: Bool { (try? resourceValues(forKeys: [.isDirectoryKey]).isDirectory) ?? false }
    var modified: Date { (try? resourceValues(forKeys: [.contentModificationDateKey]).contentModificationDate) ?? .distantPast }
    var fileSize: Int { (try? resourceValues(forKeys: [.fileSizeKey]).fileSize) ?? 0 }
    var exists: Bool { FileManager.default.fileExists(atPath: path) }
    var nameWithoutExtension: String { deletingPathExtension().lastPathComponent }

    /// Path of `self` relative to `base` (or the last component when unrelated).
    func relative(to base: URL) -> String {
        let b = base.standardizedFileURL.resolvingSymlinksInPath().path
        let p = standardizedFileURL.resolvingSymlinksInPath().path
        if p.hasPrefix(b + "/") { return String(p.dropFirst(b.count + 1)) }
        return lastPathComponent
    }

    func readText() -> String { (try? String(contentsOf: self, encoding: .utf8)) ?? (try? String(contentsOf: self, encoding: .isoLatin1)) ?? "" }
}

/// On-device storage (mirrors the Android app):
///   Documents/projects/<name>/…            editable projects (visible in the Files app)
///   Application Support/library/<id>/…     bundled examples (refreshed on every app update)
///   Application Support/stdlib/…           the std/ library (TULPA_PATH points here)
///   Application Support/models/…           bundled MJCF models (examples use ../../models/…)
final class Workspace {
    let fm = FileManager.default
    let docs: URL
    let support: URL
    let root: URL
    let library: URL
    let stdlib: URL
    let models: URL
    let scratch: URL
    let assets: URL

    init() {
        docs = fm.urls(for: .documentDirectory, in: .userDomainMask)[0]
        support = fm.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        root = docs.appendingPathComponent("projects", isDirectory: true)
        library = support.appendingPathComponent("library", isDirectory: true)
        stdlib = support.appendingPathComponent("stdlib", isDirectory: true)
        models = support.appendingPathComponent("models", isDirectory: true)
        scratch = support.appendingPathComponent("scratch", isDirectory: true)
        assets = Bundle.main.resourceURL!.appendingPathComponent("TulpaAssets", isDirectory: true)
        try? fm.createDirectory(at: root, withIntermediateDirectories: true)
        try? fm.createDirectory(at: support, withIntermediateDirectories: true)
    }

    /// Copies bundled assets when the app version changes (or on first launch).
    func installAssets(version: String) {
        let marker = support.appendingPathComponent(".assets-version")
        if marker.readText() == version && library.exists && stdlib.exists && models.exists { return }
        for (asset, dest) in [("examples", library), ("stdlib", stdlib), ("models", models)] {
            try? fm.removeItem(at: dest)
            let src = assets.appendingPathComponent(asset)
            if src.exists { try? fm.copyItem(at: src, to: dest) } else { try? fm.createDirectory(at: dest, withIntermediateDirectories: true) }
        }
        try? version.write(to: marker, atomically: true, encoding: .utf8)
    }

    // ------------------------------------------------------------------ projects

    private func walk(_ dir: URL) -> [URL] {
        guard let e = fm.enumerator(at: dir, includingPropertiesForKeys: [.isDirectoryKey, .contentModificationDateKey, .fileSizeKey]) else { return [] }
        return e.compactMap { $0 as? URL }
    }

    func projects() -> [Project] {
        let dirs = ((try? fm.contentsOfDirectory(at: root, includingPropertiesForKeys: [.isDirectoryKey])) ?? []).filter { $0.isDirectoryURL }
        return dirs.map { d in
            let all = walk(d)
            let files = all.filter { !$0.isDirectoryURL }
            let mod = ([d] + all).map { $0.modified }.max() ?? .distantPast
            return Project(name: d.lastPathComponent, dir: d, files: files.count, modified: mod)
        }.sorted { $0.modified > $1.modified }
    }

    func files(_ project: URL) -> [URL] {
        walk(project).filter { !$0.isDirectoryURL && !$0.lastPathComponent.hasPrefix(".") }
            .sorted { $0.relative(to: project) < $1.relative(to: project) }
    }

    func uniqueName(_ base: String) -> String {
        var clean = base.unicodeScalars.filter { CharacterSet.alphanumerics.contains($0) && $0.isASCII || "_- ".unicodeScalars.contains($0) }
            .map(String.init).joined().trimmingCharacters(in: .whitespaces)
        if clean.isEmpty { clean = "project" }
        if !root.appendingPathComponent(clean).exists { return clean }
        var i = 2
        while root.appendingPathComponent("\(clean) \(i)").exists { i += 1 }
        return "\(clean) \(i)"
    }

    @discardableResult
    func createProject(_ name: String, template: Template) -> URL {
        let dir = root.appendingPathComponent(uniqueName(name), isDirectory: true)
        try? fm.createDirectory(at: dir, withIntermediateDirectories: true)
        try? template.source.write(to: dir.appendingPathComponent("main.rav"), atomically: true, encoding: .utf8)
        return dir
    }

    func copyExample(_ e: Example) -> URL {
        let dir = root.appendingPathComponent(uniqueName(e.title), isDirectory: true)
        try? fm.copyItem(at: e.dir, to: dir)
        try? fm.removeItem(at: dir.appendingPathComponent("expected.txt"))
        return dir
    }

    func newFile(_ project: URL, _ name: String) -> URL {
        let n = name.contains(".") ? name : "\(name).rav"
        let f = project.appendingPathComponent(n)
        try? fm.createDirectory(at: f.deletingLastPathComponent(), withIntermediateDirectories: true)
        if !f.exists { try? "# \(f.nameWithoutExtension)\n\n".write(to: f, atomically: true, encoding: .utf8) }
        return f
    }

    func rename(_ f: URL, _ newName: String) -> URL {
        let target = f.deletingLastPathComponent().appendingPathComponent(newName)
        if !target.exists { try? fm.moveItem(at: f, to: target) }
        return target
    }

    func delete(_ f: URL) { try? fm.removeItem(at: f) }

    func duplicate(_ dir: URL) -> URL {
        let copy = root.appendingPathComponent(uniqueName(dir.lastPathComponent + " copy"), isDirectory: true)
        try? fm.copyItem(at: dir, to: copy)
        return copy
    }

    /// Imports a shared or opened file into an "Inbox" project; returns the new file.
    func importText(_ text: String, name: String) -> URL {
        let inbox = root.appendingPathComponent("Inbox", isDirectory: true)
        try? fm.createDirectory(at: inbox, withIntermediateDirectories: true)
        var n = (name as NSString).lastPathComponent
        if n.isEmpty { n = "imported.rav" }
        if !n.hasSuffix(".rav") && !n.hasSuffix(".tlp") { n += ".rav" }
        var f = inbox.appendingPathComponent(n)
        var i = 2
        let stem = (n as NSString).deletingPathExtension, ext = (n as NSString).pathExtension
        while f.exists { f = inbox.appendingPathComponent("\(stem) \(i).\(ext)"); i += 1 }
        try? text.write(to: f, atomically: true, encoding: .utf8)
        return f
    }

    // ------------------------------------------------------------------ library & guide

    func examples() -> [Example] {
        let catalog = JSON.array(assets.appendingPathComponent("catalog.json").readText())
        var known = Set<String>()
        var out: [Example] = []
        for case let o as [String: Any] in catalog {
            let id = o.str("id")
            let dir = library.appendingPathComponent(id, isDirectory: true)
            if !dir.appendingPathComponent("main.rav").exists { continue }
            known.insert(id)
            out.append(Example(id: id, title: o.str("title"), category: o.str("category"), summary: o.str("summary"),
                               tags: o.strings("tags"), virtualTime: o.bool("virtualTime"), dir: dir, args: o.strings("args")))
        }
        // anything bundled but not catalogued still shows up
        let dirs = ((try? fm.contentsOfDirectory(at: library, includingPropertiesForKeys: [.isDirectoryKey])) ?? [])
            .filter { $0.isDirectoryURL && !known.contains($0.lastPathComponent) && $0.appendingPathComponent("main.rav").exists }
            .sorted { $0.lastPathComponent < $1.lastPathComponent }
        for d in dirs {
            let first = d.appendingPathComponent("main.rav").readText().split(separator: "\n").first { $0.hasPrefix("#") }
                .map { String($0).trimmingCharacters(in: CharacterSet(charactersIn: "# ")) } ?? ""
            let title = d.lastPathComponent.replacingOccurrences(of: "_", with: " ")
            out.append(Example(id: d.lastPathComponent, title: title.prefix(1).uppercased() + title.dropFirst(), category: "More",
                               summary: first, tags: [], virtualTime: false, dir: d))
        }
        return out
    }

    func guide() -> [GuideDoc] {
        JSON.array(assets.appendingPathComponent("guide.json").readText()).compactMap { x in
            guard let o = x as? [String: Any] else { return nil }
            let d = GuideDoc(id: o.str("id"), title: o.str("title"), section: o.str("section"), asset: o.str("asset"))
            return assets.appendingPathComponent(d.asset).exists ? d : nil
        }
    }

    func readAsset(_ path: String) -> String { assets.appendingPathComponent(path).readText() }
}

/// Starting points for a new project.
enum Template: String, CaseIterable, Identifiable {
    case blank, sensors, machine, contract
    var id: String { rawValue }

    var title: String {
        switch self {
        case .blank: return "Blank"
        case .sensors: return "Phone sensor loop"
        case .machine: return "State machine"
        case .contract: return "Contract + check()"
        }
    }

    var source: String {
        switch self {
        case .blank:
            return "# New Tulpa program\n\nprint(\"hello from Tulpa Field\")\n"
        case .sensors:
            return """
            # Read the phone's sensors 10× per second; plot and gauge them.

            every!(100[ms], 300, {
                let a = phone::accel()
                tlm::plot("az", a[2].in("m/s^2"))
                phone::gauge("tilt", phone::tilt(), 0[deg], 90[deg])
            })

            """
        case .machine:
            return """
            # A state machine with guarded transitions and a safety envelope.

            machine Light {
                state Off { on toggle => On }
                state On {
                    enter { print("on") }
                    on toggle => Off
                    on overheat(t) if t > 80 => Fault
                }
                state Fault { on reset => Off }
            }

            let l = Light()
            l.toggle()
            l.overheat(95)
            print(l.state, l.history)

            """
        case .contract:
            return """
            # Specify a function, then let check() fuzz it for counterexamples.

            fn stopping_distance(v: [m/s], decel: [m/s^2]) -> [m]
                require v >= 0[m/s] and decel > 0[m/s^2]
                ensure result' >= 0[m]
            {
                v ** 2 / (2 * decel)
            }

            print(stopping_distance(13.9[m/s], 6.5[m/s^2]))
            print(check(stopping_distance))

            """
        }
    }
}

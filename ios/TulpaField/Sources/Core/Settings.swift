import Foundation

struct Prefs: Equatable {
    var fontSize = 14
    var autosave = true
    var keepScreenOn = true
    var trials = 300
    var lineNumbers = true
    var pairBrackets = true
    var haptics = true
    var remoteCosim = ""
}

final class Settings: ObservableObject {
    private let d = UserDefaults.standard
    @Published private(set) var prefs: Prefs

    init() {
        let d = UserDefaults.standard
        func b(_ k: String, _ def: Bool) -> Bool { d.object(forKey: k) == nil ? def : d.bool(forKey: k) }
        func i(_ k: String, _ def: Int) -> Int { d.object(forKey: k) == nil ? def : d.integer(forKey: k) }
        prefs = Prefs(fontSize: i("fontSize", 14), autosave: b("autosave", true), keepScreenOn: b("keepScreenOn", true),
                      trials: i("trials", 300), lineNumbers: b("lineNumbers", true), pairBrackets: b("pairBrackets", true),
                      haptics: b("haptics", true), remoteCosim: d.string(forKey: "remoteCosim") ?? "")
    }

    func update(_ f: (inout Prefs) -> Void) {
        var p = prefs
        f(&p)
        prefs = p
        d.set(p.fontSize, forKey: "fontSize"); d.set(p.autosave, forKey: "autosave")
        d.set(p.keepScreenOn, forKey: "keepScreenOn"); d.set(p.trials, forKey: "trials")
        d.set(p.lineNumbers, forKey: "lineNumbers"); d.set(p.pairBrackets, forKey: "pairBrackets")
        d.set(p.haptics, forKey: "haptics"); d.set(p.remoteCosim, forKey: "remoteCosim")
    }

    var lastProject: String? {
        get { d.string(forKey: "lastProject") }
        set { d.set(newValue, forKey: "lastProject") }
    }
    var lastFile: String? {
        get { d.string(forKey: "lastFile") }
        set { d.set(newValue, forKey: "lastFile") }
    }
}

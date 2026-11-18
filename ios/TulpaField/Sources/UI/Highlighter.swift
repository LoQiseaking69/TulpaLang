import UIKit

/// A small hand-written lexer for Tulpa syntax colouring (a port of the Android Highlighter).
/// Works on UTF-16 code units so ranges map directly onto NSAttributedString.
enum Highlighter {
    static let keywords: Set<String> = [
        "let", "const", "fn", "def", "return", "if", "elif", "else", "while", "loop", "for", "in", "not", "and", "or",
        "break", "continue", "match", "struct", "machine", "interface", "impl", "state", "on", "enter", "exit", "when", "always", "after",
        "within", "ensure", "require", "import", "from", "as", "use", "try", "catch", "throw", "spawn", "await", "unit",
        "contract",
    ]
    static let constants: Set<String> = ["true", "false", "nil", "result"]

    enum Style { case comment, str, num, unit, keyword, constant, macro, module, funcC, type }

    struct Span { let start: Int; let end: Int; let style: Style }

    private static func isDigit(_ c: unichar) -> Bool { c >= 48 && c <= 57 }
    private static func isLetter(_ c: unichar) -> Bool {
        (c >= 65 && c <= 90) || (c >= 97 && c <= 122) || (c > 127 && (Unicode.Scalar(c).map { CharacterSet.letters.contains($0) } ?? false))
    }
    private static func isUpper(_ c: unichar) -> Bool { c >= 65 && c <= 90 }
    private static func isIdent(_ c: unichar) -> Bool { isLetter(c) || isDigit(c) || c == 95 }
    private static func isSpace(_ c: unichar) -> Bool { c == 32 || c == 9 || c == 10 || c == 13 }

    static func spans(_ text: String) -> [Span] {
        let u = Array(text.utf16)
        let n = u.count
        var out: [Span] = []
        func span(_ s: Int, _ e: Int, _ st: Style) { if e > s { out.append(Span(start: s, end: min(e, n), style: st)) } }
        func indexOf(_ c: unichar, _ from: Int) -> Int { var k = from; while k < n { if u[k] == c { return k }; k += 1 }; return -1 }
        func lineContains(_ a: Int, _ b: Int) -> Bool { var k = a; while k < b { if u[k] == 10 { return true }; k += 1 }; return false }
        let q1: unichar = 34, q2: unichar = 39, hash: unichar = 35, slash: unichar = 47, star: unichar = 42, nl: unichar = 10
        var i = 0
        while i < n {
            let c = u[i]
            if c == hash || (c == slash && i + 1 < n && u[i + 1] == slash) {
                let e = indexOf(nl, i); let end = e < 0 ? n : e
                span(i, end, .comment); i = end
            } else if c == slash && i + 1 < n && u[i + 1] == star {
                var e = i + 2
                while e + 1 < n && !(u[e] == star && u[e + 1] == slash) { e += 1 }
                let end = e + 1 < n ? e + 2 : n
                span(i, end, .comment); i = end
            } else if c == q1 || c == q2 || (c == 102 && i + 1 < n && (u[i + 1] == q1 || u[i + 1] == q2) && (i == 0 || !(isLetter(u[i - 1]) || isDigit(u[i - 1])))) {
                let fstr = c == 102
                if fstr { span(i, i + 1, .macro); i += 1 }
                let q = u[i]
                var seg = i
                i += 1
                while i < n && u[i] != q && u[i] != nl {
                    if u[i] == 92 { i += 2; continue }
                    if fstr && u[i] == 123 {
                        let close = indexOf(125, i)
                        if close > 0 && !lineContains(i, close) {
                            span(seg, i, .str); span(i, close + 1, .unit)
                            i = close + 1; seg = i; continue
                        }
                    }
                    i += 1
                }
                i = min(i + 1, n)
                span(seg, i, .str)
            } else if isDigit(c) {
                let s = i
                if c == 48 && i + 1 < n && [120, 88, 98, 66].contains(u[i + 1]) {
                    i += 2
                    while i < n && isIdent(u[i]) { i += 1 }
                } else {
                    while i < n {
                        let d = u[i]
                        let isE = d == 101 || d == 69
                        if isDigit(d) || d == 95 || (d == 46 && i + 1 < n && isDigit(u[i + 1])) || (isE && i + 1 < n && (isDigit(u[i + 1]) || u[i + 1] == 45)) {
                            if isE { i += 1 }
                            i += 1
                        } else { break }
                    }
                }
                span(s, i, .num)
                if i < n && u[i] == 91 {
                    let e = indexOf(93, i)
                    if e > 0 && !lineContains(i, e) && e - i < 24 { span(i, e + 1, .unit); i = e + 1 }
                }
            } else if isLetter(c) || c == 95 {
                let s = i
                while i < n && isIdent(u[i]) { i += 1 }
                let w = String(utf16CodeUnits: Array(u[s..<i]), count: i - s)
                let next: unichar = i < n ? u[i] : 32
                var style: Style? = nil
                var end = i
                if keywords.contains(w) { style = .keyword }
                else if constants.contains(w) { style = .constant }
                else if next == 33 && (i + 1 >= n || u[i + 1] != 61) { style = .macro; end = i + 1 }
                else if next == 58 && i + 1 < n && u[i + 1] == 58 { style = .module }
                else if next == 40 { style = isUpper(u[s]) ? .type : .funcC }
                else if isUpper(u[s]) { style = .type }
                if let style { span(s, end, style) }
                if i < n && u[i] == q2 { span(i, i + 1, .macro); i += 1 }   // primes: result'
            } else if c == 61 && i + 1 < n && u[i + 1] == 62 {
                span(i, i + 2, .keyword); i += 2
            } else {
                i += 1
            }
        }
        return out
    }

    static func color(_ s: Style) -> UIColor {
        switch s {
        case .comment: return Syn.comment
        case .str: return Syn.str
        case .num, .constant: return Syn.num
        case .unit: return Syn.unit
        case .keyword: return Syn.keyword
        case .macro: return Syn.macro
        case .module: return Syn.module
        case .funcC: return Syn.funcC
        case .type: return Syn.type
        }
    }

    /// UTF-16 range of a diagnostic.
    static func range(_ text: NSString, _ d: Diagnostic) -> NSRange {
        func off(_ line: Int, _ col: Int) -> Int {
            var o = 0, l = 0
            while l < line {
                let r = text.range(of: "\n", range: NSRange(location: o, length: text.length - o))
                if r.location == NSNotFound { return text.length }
                o = r.location + 1; l += 1
            }
            let r = text.range(of: "\n", range: NSRange(location: o, length: text.length - o))
            let end = r.location == NSNotFound ? text.length : r.location
            return min(o + col, end)
        }
        let s = off(d.line, d.col)
        var e = off(d.endLine, d.endCol)
        if e <= s { e = min(s + 1, text.length) }
        return NSRange(location: s, length: max(0, e - s))
    }

    /// Applies colours (and diagnostic underlines) to `storage` in place.
    static func apply(to storage: NSTextStorage, font: UIFont, italic: UIFont, bold: UIFont, diagnostics: [Diagnostic], paragraph: NSParagraphStyle) {
        let text = storage.string
        let full = NSRange(location: 0, length: storage.length)
        storage.beginEditing()
        storage.setAttributes([.font: font, .foregroundColor: Syn.text, .paragraphStyle: paragraph], range: full)
        if storage.length <= 200_000 {
            for s in spans(text) {
                let r = NSRange(location: s.start, length: s.end - s.start)
                storage.addAttribute(.foregroundColor, value: color(s.style), range: r)
                if s.style == .comment { storage.addAttribute(.font, value: italic, range: r) }
                if s.style == .keyword { storage.addAttribute(.font, value: bold, range: r) }
            }
        }
        let ns = text as NSString
        for d in diagnostics {
            let r = range(ns, d)
            guard r.location + r.length <= storage.length, r.length > 0 else { continue }
            let c = d.error ? Syn.err : Syn.warn
            storage.addAttributes([.underlineStyle: NSUnderlineStyle.single.rawValue | NSUnderlineStyle.patternDot.rawValue,
                                   .underlineColor: c, .backgroundColor: c.withAlphaComponent(0.18)], range: r)
        }
        storage.endEditing()
    }

    /// A highlighted attributed string for read-only code (guide, examples).
    static func attributed(_ code: String, size: CGFloat) -> AttributedString {
        let s = NSMutableAttributedString(string: code, attributes: [.font: UIFont.monospacedSystemFont(ofSize: size, weight: .regular), .foregroundColor: Syn.text])
        for sp in spans(code) {
            s.addAttribute(.foregroundColor, value: color(sp.style), range: NSRange(location: sp.start, length: sp.end - sp.start))
        }
        return (try? AttributedString(s, including: \.uiKit)) ?? AttributedString(code)
    }
}

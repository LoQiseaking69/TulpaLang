import SwiftUI
import UIKit

private enum MDBlock: Hashable {
    case heading(Int, String)
    case para(String)
    case bullet(String, String?)
    case code(String, String)
    case table([[String]])
    case quote(String)
    case rule
}

private func parseMarkdown(_ md: String) -> [MDBlock] {
    var out: [MDBlock] = []
    let lines = md.components(separatedBy: "\n")
    var i = 0
    var para = ""
    func flush() { let p = para.trimmingCharacters(in: .whitespaces); if !p.isEmpty { out.append(.para(p)) }; para = "" }
    func trim(_ s: String) -> String { s.trimmingCharacters(in: .whitespaces) }
    let sepRe = try! NSRegularExpression(pattern: "^:?-{2,}:?$")
    let olRe = try! NSRegularExpression(pattern: "^\\d+\\. ")
    func matches(_ re: NSRegularExpression, _ s: String) -> Bool { re.firstMatch(in: s, range: NSRange(location: 0, length: (s as NSString).length)) != nil }
    while i < lines.count {
        let l = lines[i]
        let t = trim(l)
        if t.hasPrefix("```") {
            flush()
            let lang = trim(String(t.dropFirst(3)))
            var code: [String] = []
            i += 1
            while i < lines.count && !trim(lines[i]).hasPrefix("```") { code.append(lines[i]); i += 1 }
            var c = code.joined(separator: "\n")
            while c.last?.isWhitespace == true { c.removeLast() }
            out.append(.code(lang, c))
        } else if t.hasPrefix("#") {
            flush()
            let lvl = t.prefix { $0 == "#" }.count
            out.append(.heading(lvl, trim(String(t.dropFirst(lvl)))))
        } else if t.hasPrefix("|") {
            flush()
            var rows: [[String]] = []
            while i < lines.count && trim(lines[i]).hasPrefix("|") {
                var r = trim(lines[i])
                while r.hasPrefix("|") { r.removeFirst() }
                while r.hasSuffix("|") { r.removeLast() }
                let cells = r.components(separatedBy: "|").map(trim)
                if !cells.allSatisfy({ matches(sepRe, $0) }) { rows.append(cells) }
                i += 1
            }
            i -= 1
            out.append(.table(rows))
        } else if t.hasPrefix("- ") || t.hasPrefix("* ") {
            flush()
            var s = String(t.dropFirst(2))
            while i + 1 < lines.count && lines[i + 1].hasPrefix("  ") && !trim(lines[i + 1]).isEmpty && !trim(lines[i + 1]).hasPrefix("- ") {
                i += 1; s += " " + trim(lines[i])
            }
            out.append(.bullet(s, nil))
        } else if matches(olRe, t), let dot = t.range(of: ". ") {
            flush()
            let num = String(t[..<dot.lowerBound])
            var s = String(t[dot.upperBound...])
            while i + 1 < lines.count && lines[i + 1].hasPrefix("   ") && !trim(lines[i + 1]).isEmpty { i += 1; s += " " + trim(lines[i]) }
            out.append(.bullet(s, num))
        } else if t.hasPrefix(">") {
            flush()
            out.append(.quote(String(t.drop { $0 == ">" || $0 == " " })))
        } else if t == "---" || t == "***" {
            flush(); out.append(.rule)
        } else if t.isEmpty {
            flush()
        } else {
            para += t + " "
        }
        i += 1
    }
    flush()
    return out
}

/// Inline markdown: **bold**, *italic*, `code`, [link](url) (shown as its text).
func inlineMarkdown(_ s: String) -> AttributedString {
    var out = AttributedString()
    let c = Array(s)
    var i = 0
    func find(_ pat: [Character], from: Int) -> Int? {
        var k = from
        while k + pat.count <= c.count { if Array(c[k..<k + pat.count]) == pat { return k }; k += 1 }
        return nil
    }
    var plain = ""
    func flushPlain() { if !plain.isEmpty { out += AttributedString(plain); plain = "" } }
    while i < c.count {
        if c[i] == "*" && i + 1 < c.count && c[i + 1] == "*", let e = find(["*", "*"], from: i + 2) {
            flushPlain()
            var a = inlineMarkdown(String(c[(i + 2)..<e]))
            a.inlinePresentationIntent = .stronglyEmphasized
            a.foregroundColor = TF.text
            out += a; i = e + 2
        } else if c[i] == "`", let e = find(["`"], from: i + 1) {
            flushPlain()
            var a = AttributedString(String(c[(i + 1)..<e]))
            a.font = .system(.callout, design: .monospaced)
            a.foregroundColor = TF.cyan
            a.backgroundColor = TF.surface2
            out += a; i = e + 1
        } else if c[i] == "[", let mid = find(["]", "("], from: i), let e = find([")"], from: mid) {
            flushPlain()
            var a = AttributedString(String(c[(i + 1)..<mid]))
            a.foregroundColor = TF.violet
            let url = String(c[(mid + 2)..<e])
            if url.hasPrefix("http"), let u = URL(string: url) { a.link = u }
            out += a; i = e + 1
        } else if c[i] == "*" && i + 1 < c.count && c[i + 1] != " ", let e = find(["*"], from: i + 1) {
            flushPlain()
            var a = AttributedString(String(c[(i + 1)..<e]))
            a.inlinePresentationIntent = .emphasized
            out += a; i = e + 1
        } else {
            plain.append(c[i]); i += 1
        }
    }
    flushPlain()
    return out
}

/// Renders a markdown document. Tulpa code blocks get Run / Copy / As-project buttons when `onRun` is given.
struct MarkdownView: View {
    private let blocks: [MDBlock]
    let onRun: ((String) -> Void)?
    let onProject: ((String) -> Void)?

    init(_ md: String, onRun: ((String) -> Void)? = nil, onProject: ((String) -> Void)? = nil) {
        blocks = parseMarkdown(md)
        self.onRun = onRun
        self.onProject = onProject
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            ForEach(Array(blocks.enumerated()), id: \.offset) { _, b in block(b) }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    @ViewBuilder private func block(_ b: MDBlock) -> some View {
        switch b {
        case .heading(let lvl, let t):
            Text(inlineMarkdown(t))
                .font(lvl == 1 ? .title2.bold() : lvl == 2 ? .title3.weight(.semibold) : .headline)
                .foregroundColor(lvl == 1 ? TF.text : TF.violet)
                .padding(.top, lvl == 1 ? 0 : 8)
        case .para(let t):
            Text(inlineMarkdown(t)).foregroundColor(TF.text.opacity(0.9)).lineSpacing(4).fixedSize(horizontal: false, vertical: true)
        case .bullet(let t, let n):
            HStack(alignment: .firstTextBaseline, spacing: 0) {
                Text(n.map { "\($0)." } ?? "•").foregroundColor(TF.cyan).frame(width: 24, alignment: .leading)
                Text(inlineMarkdown(t)).foregroundColor(TF.text.opacity(0.9)).lineSpacing(4).fixedSize(horizontal: false, vertical: true)
            }
        case .quote(let t):
            Text(inlineMarkdown(t)).foregroundColor(TF.dim).padding(10).frame(maxWidth: .infinity, alignment: .leading)
                .background(TF.surface2).clipShape(RoundedRectangle(cornerRadius: 6))
        case .rule:
            Divider().background(TF.line)
        case .table(let rows):
            ScrollView(.horizontal, showsIndicators: false) {
                VStack(alignment: .leading, spacing: 0) {
                    ForEach(Array(rows.enumerated()), id: \.offset) { r, row in
                        HStack(alignment: .top, spacing: 12) {
                            ForEach(Array(row.enumerated()), id: \.offset) { _, cell in
                                Text(inlineMarkdown(cell)).font(.system(size: 13, weight: r == 0 ? .semibold : .regular))
                                    .foregroundColor(r == 0 ? TF.violet : TF.text)
                                    .frame(minWidth: 90, maxWidth: 260, alignment: .leading)
                                    .fixedSize(horizontal: false, vertical: true)
                            }
                        }.padding(.vertical, 4)
                        if r == 0 { Divider().background(TF.line) }
                    }
                }.padding(8)
            }
            .background(TF.surface).clipShape(RoundedRectangle(cornerRadius: 8))
        case .code(let lang, let code):
            CodeBlock(lang: lang, code: code, onRun: onRun, onProject: onProject)
        }
    }
}

struct CodeBlock: View {
    let lang: String
    let code: String
    var onRun: ((String) -> Void)? = nil
    var onProject: ((String) -> Void)? = nil
    @State private var copied = false

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            ScrollView(.horizontal, showsIndicators: false) {
                Group {
                    if ["tulpa", "rav", "text", ""].contains(lang) { Text(Highlighter.attributed(code, size: 13)) }
                    else { Text(code).font(TF.mono(13)).foregroundColor(TF.text) }
                }
                .lineSpacing(4).textSelection(.enabled).padding(12)
            }
            if lang == "tulpa", let onRun {
                Divider().background(TF.line)
                HStack(spacing: 4) {
                    Button { onRun(code) } label: { Label("Run", systemImage: "play.fill").foregroundColor(TF.text) }
                        .tint(TF.ok).accessibilityIdentifier("snippet-run")
                    Button { UIPasteboard.general.string = code; copied = true } label: { Label(copied ? "Copied" : "Copy", systemImage: "doc.on.doc") }
                    if let onProject { Button { onProject(code) } label: { Label("As project", systemImage: "folder.badge.plus") } }
                }
                .font(.subheadline).foregroundColor(TF.text).buttonStyle(.borderless).padding(.horizontal, 10).padding(.vertical, 8)
            }
        }
        .background(TF.bg)
        .clipShape(RoundedRectangle(cornerRadius: 10))
        .overlay(RoundedRectangle(cornerRadius: 10).stroke(TF.line, lineWidth: 1))
    }
}

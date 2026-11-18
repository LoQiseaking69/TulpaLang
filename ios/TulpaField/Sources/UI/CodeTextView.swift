import SwiftUI
import UIKit

/// The code editor: a TextKit 1 UITextView with syntax colouring, a line-number gutter,
/// diagnostics underlines, auto-indent and bracket pairing.
struct CodeTextView: UIViewRepresentable {
    @ObservedObject var vm: AppModel
    var fontSize: CGFloat
    var lineNumbers: Bool
    var pairBrackets: Bool

    func makeCoordinator() -> Coordinator { Coordinator(self) }

    func makeUIView(context: Context) -> CodeUITextView {
        let tv = CodeUITextView.make()
        tv.delegate = context.coordinator
        tv.backgroundColor = Syn.bg
        tv.tintColor = UIColor(hex: 0x3FD0FF)
        tv.autocapitalizationType = .none
        tv.autocorrectionType = .no
        tv.spellCheckingType = .no
        tv.smartQuotesType = .no
        tv.smartDashesType = .no
        tv.smartInsertDeleteType = .no
        tv.keyboardType = .asciiCapable
        tv.keyboardAppearance = .dark
        tv.alwaysBounceVertical = true
        tv.keyboardDismissMode = .interactive
        tv.layoutManager.allowsNonContiguousLayout = false
        tv.accessibilityIdentifier = "code-editor"
        context.coordinator.configure(tv)
        context.coordinator.load(tv, force: true)
        return tv
    }

    func updateUIView(_ tv: CodeUITextView, context: Context) {
        let c = context.coordinator
        c.parent = self
        c.configure(tv)
        c.load(tv, force: false)
    }

    final class Coordinator: NSObject, UITextViewDelegate {
        var parent: CodeTextView
        private var revision = -1
        private var applying = false
        private var diagnostics: [Diagnostic] = []
        private var fontSize: CGFloat = 0
        private var font = UIFont.monospacedSystemFont(ofSize: 14, weight: .regular)
        private var italic = UIFont.monospacedSystemFont(ofSize: 14, weight: .regular)
        private var bold = UIFont.monospacedSystemFont(ofSize: 14, weight: .semibold)
        private var paragraph = NSParagraphStyle()

        init(_ p: CodeTextView) { parent = p }

        func configure(_ tv: CodeUITextView) {
            let st = parent.vm.editor
            tv.isEditable = !st.readOnly
            tv.gutter.isHidden = !parent.lineNumbers
            tv.gutterWidth = parent.lineNumbers ? 44 : 0
            if fontSize != parent.fontSize {
                fontSize = parent.fontSize
                font = UIFont.monospacedSystemFont(ofSize: fontSize, weight: .regular)
                bold = UIFont.monospacedSystemFont(ofSize: fontSize, weight: .semibold)
                if let d = font.fontDescriptor.withSymbolicTraits(.traitItalic) { italic = UIFont(descriptor: d, size: fontSize) } else { italic = font }
                let p = NSMutableParagraphStyle()
                p.lineHeightMultiple = 1.2
                p.lineBreakMode = .byCharWrapping
                paragraph = p
                tv.gutter.font = UIFont.monospacedSystemFont(ofSize: max(fontSize - 2, 9), weight: .regular)
                tv.typingAttributes = [.font: font, .foregroundColor: Syn.text, .paragraphStyle: paragraph]
                highlight(tv)
            }
            if diagnostics != st.diagnostics {
                diagnostics = st.diagnostics
                tv.gutter.marks = Dictionary(st.diagnostics.map { ($0.line, $0.error) }, uniquingKeysWith: { $0 || $1 })
                highlight(tv)
            }
        }

        /// Pulls text/selection from the model when it changed programmatically.
        func load(_ tv: CodeUITextView, force: Bool) {
            let st = parent.vm.editor
            guard force || st.revision != revision else { return }
            revision = st.revision
            applying = true
            if tv.text != st.text {
                let offset = tv.contentOffset
                tv.text = st.text
                highlight(tv)
                tv.layoutIfNeeded()
                if !force { tv.setContentOffset(offset, animated: false) }
            }
            let len = (st.text as NSString).length
            let loc = min(max(st.selection.location, 0), len)
            tv.selectedRange = NSRange(location: loc, length: min(st.selection.length, len - loc))
            applying = false
            tv.gutter.update(tv)
            if let line = st.revealLine {
                DispatchQueue.main.async {
                    tv.layoutIfNeeded()
                    let off = self.parent.vm.offset(line: max(line - 3, 0), col: 0)
                    let end = self.parent.vm.offset(line: line + 6, col: 0)
                    tv.scrollRangeToVisible(NSRange(location: off, length: max(0, end - off)))
                    self.parent.vm.revealHandled()
                    tv.gutter.update(tv)
                }
            } else if !force {
                DispatchQueue.main.async { tv.scrollRangeToVisible(tv.selectedRange) }
            }
        }

        func highlight(_ tv: CodeUITextView) {
            guard tv.markedTextRange == nil else { return }
            let sel = tv.selectedRange
            Highlighter.apply(to: tv.textStorage, font: font, italic: italic, bold: bold, diagnostics: diagnostics, paragraph: paragraph)
            tv.typingAttributes = [.font: font, .foregroundColor: Syn.text, .paragraphStyle: paragraph]
            if tv.selectedRange != sel { tv.selectedRange = sel }
            tv.gutter.recount(tv.text)
            tv.gutter.update(tv)
        }

        // MARK: delegate

        func textView(_ tv: UITextView, shouldChangeTextIn range: NSRange, replacementText s: String) -> Bool {
            guard let tv = tv as? CodeUITextView, tv.markedTextRange == nil, range.length == 0, (s as NSString).length == 1 else { return true }
            guard let edit = Self.smartEdit(tv.text as NSString, at: range.location, typed: s, pair: parent.pairBrackets) else { return true }
            tv.textStorage.replaceCharacters(in: edit.range, with: edit.text)
            tv.selectedRange = NSRange(location: edit.caret, length: 0)
            textViewDidChange(tv)
            return false
        }

        func textViewDidChange(_ tv: UITextView) {
            guard !applying, let tv = tv as? CodeUITextView else { return }
            highlight(tv)
            parent.vm.userEdited(text: tv.text, selection: tv.selectedRange)
        }

        func textViewDidChangeSelection(_ tv: UITextView) {
            guard !applying, let tv = tv as? CodeUITextView, tv.markedTextRange == nil else { return }
            let text = tv.text ?? ""
            DispatchQueue.main.async { [weak self] in
                guard let self, !self.applying else { return }
                if text == tv.text { self.parent.vm.userEdited(text: text, selection: tv.selectedRange) }
            }
            tv.gutter.cursor = tv.gutter.line(at: tv.selectedRange.location)
            tv.gutter.setNeedsDisplay()
        }

        func scrollViewDidScroll(_ sv: UIScrollView) { (sv as? CodeUITextView)?.gutter.update(sv as! CodeUITextView) }

        // MARK: smart editing

        struct Edit { let range: NSRange; let text: String; let caret: Int }

        private static func ch(_ s: NSString, _ i: Int) -> unichar? { i >= 0 && i < s.length ? s.character(at: i) : nil }
        private static func isSpace(_ c: unichar) -> Bool { c == 32 || c == 9 || c == 10 || c == 13 }
        private static func isAlnum(_ c: unichar) -> Bool { (c >= 48 && c <= 57) || (c >= 65 && c <= 90) || (c >= 97 && c <= 122) }

        /// Auto-indent, bracket pairing and over-typing of closers (port of the Android smartEdit).
        static func smartEdit(_ o: NSString, at p: Int, typed: String, pair: Bool) -> Edit? {
            let c = (typed as NSString).character(at: 0)
            let next = ch(o, p)
            let closers: [unichar] = [41, 93, 125, 44]   // ) ] } ,
            let lineStart: Int = {
                if p == 0 { return 0 }
                let r = o.range(of: "\n", options: .backwards, range: NSRange(location: 0, length: p))
                return r.location == NSNotFound ? 0 : r.location + 1
            }()
            if c == 10 {
                let prevLine = o.substring(with: NSRange(location: lineStart, length: p - lineStart))
                let indent = String(prevLine.prefix { $0 == " " || $0 == "\t" })
                let trimmed = prevLine.trimmingCharacters(in: .whitespaces)
                let opens = trimmed.hasSuffix("{") || trimmed.hasSuffix("(") || trimmed.hasSuffix("[")
                let extra = opens ? "    " : ""
                if opens, let n = next, [41, 93, 125].contains(n) {
                    let ins = "\n" + indent + extra + "\n" + indent
                    return Edit(range: NSRange(location: p, length: 0), text: ins, caret: p + 1 + (indent as NSString).length + 4)
                }
                if indent.isEmpty && extra.isEmpty { return nil }
                let ins = "\n" + indent + extra
                return Edit(range: NSRange(location: p, length: 0), text: ins, caret: p + (ins as NSString).length)
            }
            guard pair else { return nil }
            if [41, 93, 125, 34].contains(c), next == c, c != 34 || ch(o, p - 1) != 92 {
                return Edit(range: NSRange(location: p, length: 0), text: "", caret: p + 1)
            }
            let freeAfter = next == nil || isSpace(next!) || closers.contains(next!)
            if [40, 91, 123].contains(c) && freeAfter {
                let close = c == 40 ? ")" : c == 91 ? "]" : "}"
                return Edit(range: NSRange(location: p, length: 0), text: typed + close, caret: p + 1)
            }
            if c == 34 && freeAfter && (p < 1 || !isAlnum(o.character(at: p - 1)) || o.character(at: p - 1) == 102) {
                return Edit(range: NSRange(location: p, length: 0), text: "\"\"", caret: p + 1)
            }
            if c == 125 {
                let before = o.substring(with: NSRange(location: lineStart, length: p - lineStart))
                if before.count >= 4 && before.allSatisfy({ $0 == " " || $0 == "\t" }) {
                    let dedented = String(before.dropFirst(4))
                    return Edit(range: NSRange(location: lineStart, length: p - lineStart), text: dedented + "}", caret: p - 3)
                }
            }
            return nil
        }
    }
}

/// UITextView with a line-number gutter pinned to the visible area.
final class CodeUITextView: UITextView {
    let gutter = GutterView()
    private var keepStorage: NSTextStorage?
    private var keepLayout: NSLayoutManager?
    var gutterWidth: CGFloat = 44 {
        didSet { if oldValue != gutterWidth { textContainerInset = UIEdgeInsets(top: 8, left: gutterWidth + 6, bottom: 200, right: 12); gutter.update(self) } }
    }

    override init(frame: CGRect, textContainer: NSTextContainer?) {
        super.init(frame: frame, textContainer: textContainer)
        setup()
    }
    /// TextKit 1 storage stack so that the layout manager is available for the gutter.
    static func make() -> CodeUITextView {
        let storage = NSTextStorage()
        let lm = NSLayoutManager()
        storage.addLayoutManager(lm)
        let container = NSTextContainer(size: CGSize(width: 0, height: CGFloat.greatestFiniteMagnitude))
        container.widthTracksTextView = true
        lm.addTextContainer(container)
        let tv = CodeUITextView(frame: .zero, textContainer: container)
        tv.keepStorage = storage
        tv.keepLayout = lm
        return tv
    }
    required init?(coder: NSCoder) { fatalError() }

    private func setup() {
        textContainerInset = UIEdgeInsets(top: 8, left: gutterWidth + 6, bottom: 200, right: 12)
        textContainer.lineFragmentPadding = 0
        addSubview(gutter)
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        gutter.update(self)
    }
}

final class GutterView: UIView {
    var font = UIFont.monospacedSystemFont(ofSize: 12, weight: .regular)
    var marks: [Int: Bool] = [:] { didSet { setNeedsDisplay() } }
    var cursor = 0
    private var lineStarts: [Int] = [0]
    private weak var tv: CodeUITextView?

    override init(frame: CGRect) {
        super.init(frame: frame)
        backgroundColor = Syn.surface
        isUserInteractionEnabled = false
        contentMode = .redraw
    }
    required init?(coder: NSCoder) { fatalError() }

    func recount(_ text: String) {
        var starts = [0]
        var i = 0
        for u in text.utf16 { i += 1; if u == 10 { starts.append(i) } }
        lineStarts = starts
    }

    /// Zero-based line containing UTF-16 offset `off`.
    func line(at off: Int) -> Int {
        var lo = 0, hi = lineStarts.count - 1
        while lo < hi { let mid = (lo + hi + 1) / 2; if lineStarts[mid] <= off { lo = mid } else { hi = mid - 1 } }
        return lo
    }

    func update(_ tv: CodeUITextView) {
        self.tv = tv
        let f = CGRect(x: 0, y: tv.contentOffset.y, width: tv.gutterWidth, height: tv.bounds.height)
        if frame != f { frame = f }
        setNeedsDisplay()
    }

    override func draw(_ rect: CGRect) {
        guard let tv, !isHidden else { return }
        let lm = tv.layoutManager
        let visible = CGRect(x: 0, y: tv.contentOffset.y - tv.textContainerInset.top, width: tv.bounds.width, height: tv.bounds.height)
        let glyphs = lm.glyphRange(forBoundingRect: visible, in: tv.textContainer)
        let total = (tv.text as NSString).length
        let para = NSMutableParagraphStyle(); para.alignment = .right
        func drawNumber(_ line: Int, y: CGFloat, h: CGFloat) {
            let mark = marks[line]
            let color: UIColor = mark == true ? Syn.err : mark == false ? Syn.warn : (line == cursor ? Syn.text : Syn.comment)
            let s = (mark != nil ? "● " : "") + String(line + 1)
            let attrs: [NSAttributedString.Key: Any] = [.font: font, .foregroundColor: color, .paragraphStyle: para]
            let yy = y - tv.contentOffset.y + tv.textContainerInset.top + (h - font.lineHeight) / 2
            (s as NSString).draw(in: CGRect(x: 0, y: yy, width: bounds.width - 8, height: font.lineHeight + 2), withAttributes: attrs)
        }
        var lastLine = -1
        lm.enumerateLineFragments(forGlyphRange: glyphs) { r, _, _, g, _ in
            let ci = lm.characterIndexForGlyph(at: g.location)
            let l = self.line(at: ci)
            if l != lastLine && self.lineStarts[l] == ci { drawNumber(l, y: r.minY, h: r.height) }
            lastLine = l
        }
        // empty trailing line after a final newline
        if total == 0 || (lineStarts.last ?? 0) == total {
            let r = lm.extraLineFragmentRect
            if r.height > 0 { drawNumber(lineStarts.count - 1, y: r.minY, h: r.height) }
        }
    }
}

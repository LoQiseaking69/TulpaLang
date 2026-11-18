import SwiftUI

struct ReplView: View {
    @EnvironmentObject var vm: AppModel
    @ObservedObject var repl: ReplManager
    @State private var input = ""
    @State private var histIdx = -1
    @FocusState private var focused: Bool

    private func submit() {
        let t = input
        if repl.state.prompt != nil { repl.answer(t) } else { repl.submit(t) }
        input = ""; histIdx = -1
    }

    private func recall(_ delta: Int) {
        let h = repl.history
        if h.isEmpty { return }
        if histIdx < 0 { if delta < 0 { histIdx = h.count - 1 } else { return } } else { histIdx += delta }
        if histIdx >= h.count { histIdx = -1; input = ""; return }
        histIdx = min(max(histIdx, 0), h.count - 1)
        input = h[histIdx]
    }

    private func color(_ k: ReplKind) -> Color {
        switch k { case .input: return TF.violet; case .output: return TF.text; case .value: return TF.cyan; case .error: return TF.err; case .info: return TF.dim }
    }

    var body: some View {
        let st = repl.state
        let size = CGFloat(vm.settings.prefs.fontSize)
        VStack(spacing: 0) {
            TopBar(title: "REPL", subtitle: st.busy ? "evaluating…" : "\(st.names.count) names defined") {
                if st.busy { IconButton(system: "stop.fill", label: "Interrupt", tint: TF.err) { repl.interrupt() } }
                IconButton(system: "square.and.arrow.down.on.square", label: "Load open file", disabled: vm.editor.file == nil) {
                    if let f = vm.editor.file { repl.load(vm.editor.text, name: f.lastPathComponent) }
                }
                Menu {
                    Button { if let f = vm.editor.file { repl.load(vm.editor.text, name: f.lastPathComponent) } } label: {
                        Label("Load \(vm.editor.file?.lastPathComponent ?? "file")", systemImage: "square.and.arrow.down")
                    }.disabled(vm.editor.file == nil)
                    Button { repl.cancelEntry() } label: { Label("Discard unfinished entry", systemImage: "delete.left") }.disabled(!st.continuation)
                    Button { repl.clearScreen() } label: { Label("Clear screen", systemImage: "clear") }
                    Button { repl.reset() } label: { Label("Reset session", systemImage: "arrow.counterclockwise") }
                } label: { Image(systemName: "ellipsis.circle").font(.system(size: 18)).foregroundColor(TF.text).frame(width: 38, height: 38) }
                ToolsMenu()
            }
            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 2) {
                        ForEach(st.entries) { e in
                            Text(e.kind == .value ? "= \(e.text)" : e.text)
                                .font(.system(size: size, design: .monospaced)).foregroundColor(color(e.kind))
                                .frame(maxWidth: .infinity, alignment: .leading).textSelection(.enabled).id(e.id)
                        }
                    }.padding(12)
                }
                .background(TF.bg)
                .accessibilityIdentifier("repl-log")
                .onChange(of: st.entries.last?.id) { id in if let id { withAnimation { proxy.scrollTo(id, anchor: .bottom) } } }
                .onTapGesture { focused = true }
            }
            suggestions(st)
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 4) {
                    ForEach(["↑", "↓", "{", "}", "(", ")", "[", "]", "\"", "::", ".", "=", "phone::"], id: \.self) { k in
                        Button {
                            switch k { case "↑": recall(-1); case "↓": recall(1); default: input += k }
                        } label: { Text(k).font(TF.mono(14)) }.buttonStyle(TonalButton())
                    }
                }.padding(4)
            }.background(TF.surface)
            HStack(spacing: 8) {
                Text(st.prompt.map { $0.trimmingCharacters(in: .whitespaces).isEmpty ? "?" : $0 } ?? (st.continuation ? "…" : "›"))
                    .font(.system(size: size, design: .monospaced)).foregroundColor(TF.violet)
                TextField("", text: $input, axis: .vertical)
                    .font(.system(size: size, design: .monospaced)).foregroundColor(TF.text)
                    .lineLimit(1...6)
                    .textInputAutocapitalization(.never).autocorrectionDisabled().keyboardType(.asciiCapable)
                    .submitLabel(.send).focused($focused)
                    .onChange(of: input) { v in
                        // Return at the end of the line submits; pasted multi-line text is kept
                        if v.hasSuffix("\n") && !v.dropLast().contains("\n") { input = String(v.dropLast()); submit() }
                    }
                    .onSubmit(submit)
                    .accessibilityLabel("REPL input")
                    .accessibilityIdentifier("repl-input")
                IconButton(system: "paperplane.fill", label: "Send", tint: TF.cyan) { submit() }
            }
            .padding(.horizontal, 10).padding(.vertical, 8).background(TF.surface2)
        }
    }

    @ViewBuilder private func suggestions(_ st: ReplState) -> some View {
        let word = String(input.reversed().prefix { $0.isLetter || $0.isNumber || $0 == "_" }.reversed())
        let sugg = word.isEmpty ? [] : Array(st.names.filter { $0.hasPrefix(word) && $0 != word }.prefix(12))
        if !sugg.isEmpty {
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 6) {
                    ForEach(sugg, id: \.self) { s in Chip(text: s) { input = String(input.dropLast(word.count)) + s } }
                }.padding(4)
            }.background(TF.surface2)
        }
    }
}

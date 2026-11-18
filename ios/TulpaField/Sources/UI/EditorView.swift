import SwiftUI

private let UNITS = [
    "m", "cm", "mm", "km", "s", "ms", "us", "min", "h", "Hz", "kHz",
    "deg", "rad", "rad/s", "m/s", "km/h", "m/s^2", "g", "kg", "N", "N*m", "J", "W", "kW",
    "V", "mV", "A", "mA", "ohm", "kohm", "F", "uF", "Pa", "kPa", "hPa", "bar", "K", "degC", "lux",
]

func kindGlyph(_ k: Int) -> String { [2: ".", 11: "u", 3: "ƒ", 14: "kw", 6: "v", 9: "::", 22: "S", 7: "M", 21: "c"][k] ?? "·" }
func kindColor(_ k: Int) -> Color {
    switch k { case 11: return TF.cyan; case 2, 3: return Color(Syn.funcC); case 14: return Color(Syn.keyword); case 9: return Color(Syn.module); case 7, 22: return Color(Syn.type); default: return TF.dim }
}
func symbolGlyph(_ k: Int) -> String { [12: "ƒ", 23: "S", 5: "M", 14: "c", 13: "v"][k] ?? "·" }
func symbolColor(_ k: Int) -> Color {
    switch k { case 12: return Color(Syn.funcC); case 23, 5: return Color(Syn.type); case 14: return Color(Syn.num); default: return TF.dim }
}

private enum EditorSheet: String, Identifiable { case problems, outline, inspect, units; var id: String { rawValue } }

struct EditorView: View {
    @EnvironmentObject var vm: AppModel
    @ObservedObject var settings: Settings
    @ObservedObject var runner: RunManager
    @State private var sheet: EditorSheet? = nil
    @State private var inspectText = ""
    @State private var findOpen = false
    @State private var prompt: TextPrompt? = nil
    @State private var confirmDelete = false

    var body: some View {
        let st = vm.editor
        if let project = st.project, let file = st.file {
            let errors = st.diagnostics.filter { $0.error }.count
            let warnings = st.diagnostics.count - errors
            VStack(spacing: 0) {
                TopBar(title: "", leading: {
                    fileMenu(project: project, file: file, dirty: st.dirty)
                }, trailing: {
                    if !st.diagnostics.isEmpty {
                        Button { sheet = .problems } label: {
                            HStack(spacing: 4) {
                                Image(systemName: errors > 0 ? "xmark.octagon.fill" : "exclamationmark.triangle.fill")
                                Text("\(errors > 0 ? errors : warnings)")
                            }.foregroundColor(errors > 0 ? TF.err : TF.warn).font(.subheadline)
                        }.accessibilityLabel("Problems")
                    }
                    IconButton(system: "arrow.uturn.backward", label: "Undo", disabled: !vm.canUndo) { vm.undo() }
                    IconButton(system: runner.state.running ? "hourglass" : "play.fill", label: "Run", tint: TF.ok) {
                        if runner.state.running { vm.go(.run) } else { vm.runCurrent() }
                    }
                    overflowMenu(file: file)
                })
                if findOpen { FindBar(vm: vm) { findOpen = false } }
                CodeTextView(vm: vm, fontSize: CGFloat(settings.prefs.fontSize), lineNumbers: settings.prefs.lineNumbers,
                             pairBrackets: settings.prefs.pairBrackets)
                if !st.completions.isEmpty {
                    ScrollView(.horizontal, showsIndicators: false) {
                        HStack(spacing: 6) {
                            ForEach(st.completions) { c in
                                Button { vm.applyCompletion(c) } label: {
                                    HStack(spacing: 6) {
                                        Text(kindGlyph(c.kind)).font(TF.mono(11)).foregroundColor(kindColor(c.kind))
                                        Text(c.label).font(TF.mono(13)).foregroundColor(TF.text)
                                    }
                                    .padding(.horizontal, 10).padding(.vertical, 6)
                                    .background(TF.surface).clipShape(RoundedRectangle(cornerRadius: 8))
                                    .overlay(RoundedRectangle(cornerRadius: 8).stroke(TF.line, lineWidth: 1))
                                }
                            }
                        }.padding(.horizontal, 6).padding(.vertical, 4)
                    }.background(TF.surface2)
                }
                if !st.readOnly { SymbolBar(vm: vm) { sheet = .units } }
            }
            .sheet(item: $sheet) { s in sheetView(s).presentationDetents([.medium, .large]).preferredColorScheme(.dark) }
            .textPrompt($prompt)
            .alert("Delete \(file.lastPathComponent)?", isPresented: $confirmDelete) {
                Button("Delete", role: .destructive) { vm.deleteFile() }
                Button("Cancel", role: .cancel) {}
            } message: { Text("This can't be undone.") }
        } else {
            VStack(spacing: 0) {
                TopBar(title: "Code") { ToolsMenu() }
                EmptyStateView(icon: "chevron.left.forwardslash.chevron.right", title: "No file open",
                               message: "Open a project, or copy an example from the Library to start editing.",
                               actions: [("Projects", { vm.go(.projects) }), ("Library", { vm.go(.learn) })])
            }
        }
    }

    private func fileMenu(project: URL, file: URL, dirty: Bool) -> some View {
        Menu {
            ForEach(vm.editor.files, id: \.self) { f in
                Button { vm.openFile(f) } label: {
                    Label(f.relative(to: project), systemImage: f == file ? "checkmark" : (f.pathExtension == "rav" ? "doc.text" : "doc"))
                }
            }
            Divider()
            Button { prompt = TextPrompt(title: "New file", label: "name (e.g. lib/drive.rav)", initial: "", confirm: "Create") { vm.newFile($0) } } label: { Label("New file…", systemImage: "doc.badge.plus") }
            Button { prompt = TextPrompt(title: "Rename file", label: "new name", initial: file.lastPathComponent, confirm: "Rename") { vm.renameFile($0) } } label: { Label("Rename file…", systemImage: "pencil") }
            Button { vm.save(quiet: true); vm.shareFile(file) } label: { Label("Share file", systemImage: "square.and.arrow.up") }
            Button(role: .destructive) { confirmDelete = true } label: { Label("Delete file", systemImage: "trash") }
            Button { vm.refreshFiles() } label: { Label("Refresh file list", systemImage: "arrow.clockwise") }
        } label: {
            HStack(spacing: 4) {
                VStack(alignment: .leading, spacing: 1) {
                    Text(file.relative(to: project) + (dirty ? " •" : "")).font(.headline).foregroundColor(TF.text).lineLimit(1)
                    Text(project.lastPathComponent).font(.caption2).foregroundColor(TF.dim).lineLimit(1)
                }
                Image(systemName: "chevron.down").font(.caption).foregroundColor(TF.dim)
            }
        }
        .accessibilityIdentifier("file-menu")
    }

    private func overflowMenu(file: URL) -> some View {
        Menu {
            Button { vm.redo() } label: { Label("Redo", systemImage: "arrow.uturn.forward") }.disabled(!vm.canRedo)
            Button { vm.save() } label: { Label("Save", systemImage: "square.and.arrow.down") }
            Button { findOpen = true } label: { Label("Find / replace", systemImage: "magnifyingglass") }
            Button { vm.refreshSymbols(); sheet = .outline } label: { Label("Outline", systemImage: "list.bullet.indent") }
            Button {
                vm.hoverAtCursor { t in inspectText = t ?? "Nothing to show here — place the cursor on a name."; sheet = .inspect }
            } label: { Label("Inspect at cursor", systemImage: "info.circle") }
            Button { vm.gotoDefinition() } label: { Label("Go to definition", systemImage: "arrow.up.right") }
            Divider()
            Button { vm.go(.machines) } label: { Label("Machines", systemImage: "point.3.connected.trianglepath.dotted") }
            Button { vm.go(.verify) } label: { Label("Verify & tests", systemImage: "checkmark.seal") }
            Button { vm.go(.bytecode) } label: { Label("Bytecode", systemImage: "cpu") }
            Button { vm.loadIntoRepl(vm.editor.text, name: file.lastPathComponent) } label: { Label("Load into REPL", systemImage: "terminal") }
        } label: { Image(systemName: "ellipsis.circle").font(.system(size: 18)).foregroundColor(TF.text).frame(width: 38, height: 38) }
        .accessibilityLabel("More")
    }

    @ViewBuilder private func sheetView(_ s: EditorSheet) -> some View {
        switch s {
        case .problems:
            SheetList(title: "Problems") {
                ForEach(Array(vm.editor.diagnostics.enumerated()), id: \.offset) { _, d in
                    Button { sheet = nil; vm.reveal(line: d.line, col: d.col) } label: {
                        HStack(alignment: .top, spacing: 12) {
                            Image(systemName: d.error ? "xmark.octagon.fill" : "exclamationmark.triangle.fill").foregroundColor(d.error ? TF.err : TF.warn)
                            VStack(alignment: .leading, spacing: 2) {
                                Text(d.message).foregroundColor(TF.text).multilineTextAlignment(.leading)
                                Text("line \(d.line + 1), column \(d.col + 1)").font(.caption).foregroundColor(TF.dim)
                            }
                        }
                    }
                }
            }
        case .outline:
            SheetList(title: "Outline") {
                if vm.symbols.isEmpty { Text("No top-level definitions.").foregroundColor(TF.dim) }
                ForEach(vm.symbols) { s in
                    Button { sheet = nil; vm.reveal(line: s.line) } label: {
                        HStack(spacing: 12) {
                            Text(symbolGlyph(s.kind)).font(TF.mono(15, weight: .bold)).foregroundColor(symbolColor(s.kind)).frame(width: 20)
                            VStack(alignment: .leading, spacing: 2) {
                                Text(s.name).font(TF.mono(15)).foregroundColor(TF.text)
                                if !s.detail.isEmpty { Text(s.detail).font(TF.mono(12)).foregroundColor(TF.dim).lineLimit(1) }
                            }
                            Spacer()
                            Text("\(s.line + 1)").foregroundColor(TF.dim)
                        }
                    }
                }
            }
        case .inspect:
            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    Text("Inspect").font(.headline).foregroundColor(TF.text)
                    MarkdownView(inspectText)
                    Button("Go to definition") { sheet = nil; vm.gotoDefinition() }.buttonStyle(OutlineButton())
                }.padding(20)
            }.background(TF.surface.ignoresSafeArea())
        case .units:
            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    Text("Units").font(.headline).foregroundColor(TF.text)
                    LazyVGrid(columns: [GridItem(.adaptive(minimum: 76), spacing: 8)], spacing: 8) {
                        ForEach(UNITS, id: \.self) { u in
                            Button { sheet = nil; vm.insert("[\(u)]") } label: {
                                Text("[\(u)]").font(TF.mono(14)).foregroundColor(TF.cyan).frame(maxWidth: .infinity).padding(.vertical, 8)
                                    .background(TF.surface2).clipShape(RoundedRectangle(cornerRadius: 8))
                            }
                        }
                    }
                }.padding(20)
            }.background(TF.surface.ignoresSafeArea())
        }
    }
}

struct SheetList<Content: View>: View {
    let title: String
    @ViewBuilder var content: () -> Content
    var body: some View {
        NavigationStack {
            List { content().listRowBackground(TF.surface2) }
                .scrollContentBackground(.hidden)
                .background(TF.surface)
                .navigationTitle(title)
                .navigationBarTitleDisplayMode(.inline)
        }
    }
}

private struct SymbolBar: View {
    @ObservedObject var vm: AppModel
    let onUnits: () -> Void
    private let keys = ["⇥", "←", "→", "{", "}", "(", ")", "[", "]", "\"", ":", "::", "=>", ".", ",", "|", "=", "#", "!"]

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 4) {
                Key(label: "tab") { vm.insert("    ") }
                ForEach(keys, id: \.self) { k in
                    Key(label: k) {
                        switch k {
                        case "⇥": vm.caretToLineEnd()
                        case "←": vm.moveCaret(-1)
                        case "→": vm.moveCaret(1)
                        case "=>": vm.insert(" => ")
                        default: vm.insert(k)
                        }
                    }
                }
                Key(label: "[m]", accent: true, onLong: onUnits) { vm.insert("[m]") }
                Key(label: "[s]", accent: true, onLong: onUnits) { vm.insert("[s]") }
                Key(label: "[deg]", accent: true, onLong: onUnits) { vm.insert("[deg]") }
                Key(label: "units…", accent: true) { onUnits() }
            }.padding(.horizontal, 4).padding(.vertical, 4)
        }.background(TF.surface)
    }

    private struct Key: View {
        let label: String
        var accent = false
        var onLong: (() -> Void)? = nil
        let action: () -> Void
        var body: some View {
            Text(label).font(TF.mono(15)).foregroundColor(accent ? TF.cyan : TF.text)
                .padding(.horizontal, 10).frame(minWidth: 38, minHeight: 38)
                .background(TF.surface2).clipShape(RoundedRectangle(cornerRadius: 8))
                .overlay(RoundedRectangle(cornerRadius: 8).stroke(TF.line, lineWidth: 1))
                .contentShape(Rectangle())
                .onTapGesture(perform: action)
                .onLongPressGesture { onLong?() }
        }
    }
}

private struct FindBar: View {
    @ObservedObject var vm: AppModel
    let onClose: () -> Void
    @State private var q = ""
    @State private var r = ""

    private var count: Int { q.isEmpty ? 0 : vm.editor.text.components(separatedBy: q).count - 1 }

    private func find(_ forward: Bool) {
        guard !q.isEmpty else { return }
        let t = vm.editor.text as NSString
        let sel = vm.editor.selection
        var found: NSRange
        if forward {
            let from = min(sel.location + sel.length, t.length)
            found = t.range(of: q, range: NSRange(location: from, length: t.length - from))
            if found.location == NSNotFound { found = t.range(of: q) }
        } else {
            let to = max(sel.location, 0)
            found = t.range(of: q, options: .backwards, range: NSRange(location: 0, length: to))
            if found.location == NSNotFound { found = t.range(of: q, options: .backwards) }
        }
        guard found.location != NSNotFound else { return }
        let line = t.substring(to: found.location).filter { $0 == "\n" }.count
        vm.reveal(line: line)
        vm.editor.selection = found
        vm.editor.revision += 1
    }

    var body: some View {
        VStack(spacing: 4) {
            HStack(spacing: 4) {
                TextField("find", text: $q).font(TF.mono(14)).textFieldStyle(.roundedBorder)
                    .textInputAutocapitalization(.never).autocorrectionDisabled().submitLabel(.search).onSubmit { find(true) }
                Text(" \(count) ").foregroundColor(TF.dim)
                IconButton(system: "chevron.up", label: "Previous") { find(false) }
                IconButton(system: "chevron.down", label: "Next") { find(true) }
                IconButton(system: "xmark", label: "Close", action: onClose)
            }
            HStack(spacing: 4) {
                TextField("replace with", text: $r).font(TF.mono(14)).textFieldStyle(.roundedBorder)
                    .textInputAutocapitalization(.never).autocorrectionDisabled()
                Button("Replace all") {
                    if !q.isEmpty && count > 0 { vm.setText(vm.editor.text.replacingOccurrences(of: q, with: r), selection: NSRange(location: 0, length: 0)) }
                }.font(.subheadline)
            }
        }
        .padding(.horizontal, 8).padding(.vertical, 4).background(TF.surface2)
    }
}

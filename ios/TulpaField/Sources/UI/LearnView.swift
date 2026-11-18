import SwiftUI

private let CATEGORY_ICON: [String: String] = [
    "Start here": "paperplane", "Phone sensors": "sensor", "Robotics": "gearshape.2",
    "Timing & reactive": "timer", "Verification": "checkmark.seal", "Libraries": "books.vertical",
    "Native": "cpu", "Simulation": "cube.transparent", "More": "ellipsis",
]

struct LearnView: View {
    @EnvironmentObject var vm: AppModel
    @SceneStorage("learnTab") private var tab = 0
    @State private var example: Example? = nil

    var body: some View {
        Group {
            if let e = example {
                ExampleDetail(e: e) { example = nil }
            } else if let id = vm.guideDoc {
                GuideReader(id: id)
            } else {
                VStack(spacing: 0) {
                    TopBar(title: "Learn") { ToolsMenu() }
                    Picker("", selection: $tab) {
                        Text("Library").tag(0)
                        Text("Guide").tag(1)
                    }
                    .pickerStyle(.segmented).padding(.horizontal, 16).padding(.bottom, 6)
                    if tab == 0 { LibraryList { example = $0 } } else { GuideIndex() }
                }
            }
        }
        .onAppear(perform: consume)
        .onChange(of: vm.guideTab) { _ in consume() }
        .onChange(of: vm.openExampleId) { _ in consume() }
    }

    private func consume() {
        if let t = vm.guideTab { tab = t; vm.guideTab = nil; if t == 1 && vm.guideDoc == nil { example = nil } }
        if let id = vm.openExampleId { example = vm.examples.first { $0.id == id }; vm.openExampleId = nil }
    }
}

private struct LibraryList: View {
    @EnvironmentObject var vm: AppModel
    let open: (Example) -> Void
    @SceneStorage("libQuery") private var q = ""
    @State private var cat: String? = nil

    var body: some View {
        let all = vm.examples
        var cats: [String] = []
        for e in all where !cats.contains(e.category) { cats.append(e.category) }
        let shown = all.filter { e in
            (cat == nil || e.category == cat) &&
            (q.trimmingCharacters(in: .whitespaces).isEmpty || [e.title, e.summary, e.tags.joined(separator: " "), e.id].contains { $0.localizedCaseInsensitiveContains(q) })
        }
        var groups: [(String, [Example])] = []
        for e in shown {
            if let i = groups.firstIndex(where: { $0.0 == e.category }) { groups[i].1.append(e) } else { groups.append((e.category, [e])) }
        }
        return ScrollView {
            LazyVStack(alignment: .leading, spacing: 8) {
                HStack {
                    Image(systemName: "magnifyingglass").foregroundColor(TF.dim)
                    TextField("Search examples", text: $q).autocorrectionDisabled().textInputAutocapitalization(.never)
                        .accessibilityIdentifier("search-examples")
                }
                .padding(10).background(TF.surface).clipShape(RoundedRectangle(cornerRadius: 10))
                .overlay(RoundedRectangle(cornerRadius: 10).stroke(TF.line, lineWidth: 1))
                .padding(.horizontal, 16).padding(.top, 4)
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 8) {
                        Chip(text: "All", selected: cat == nil) { cat = nil }
                        ForEach(cats, id: \.self) { c in Chip(text: c, selected: cat == c) { cat = cat == c ? nil : c } }
                    }.padding(.horizontal, 16)
                }
                ForEach(groups, id: \.0) { c, list in
                    HStack(spacing: 8) {
                        Image(systemName: CATEGORY_ICON[c] ?? "folder").foregroundColor(TF.violet).font(.subheadline)
                        Text(c).font(.subheadline.weight(.semibold)).foregroundColor(TF.violet)
                    }.padding(.leading, 20).padding(.top, 14)
                    ForEach(list) { e in
                        HStack(spacing: 8) {
                            VStack(alignment: .leading, spacing: 3) {
                                Text(e.title).font(.subheadline.weight(.semibold)).foregroundColor(TF.text)
                                Text(e.summary).font(.system(size: 13)).foregroundColor(TF.dim).lineLimit(2)
                                if !e.tags.isEmpty { Text(e.tags.joined(separator: "  ·  ")).font(.system(size: 11)).foregroundColor(TF.cyan.opacity(0.8)).padding(.top, 2) }
                            }
                            Spacer()
                            Button { vm.runExample(e) } label: { Image(systemName: "play.circle.fill").font(.system(size: 30)).foregroundColor(TF.ok) }
                                .accessibilityLabel("Run \(e.title)")
                        }
                        .padding(14).background(TF.surface).clipShape(RoundedRectangle(cornerRadius: 12))
                        .contentShape(Rectangle())
                        .onTapGesture { open(e) }
                        .accessibilityElement(children: .contain)
                        .accessibilityIdentifier("example-\(e.id)")
                        .padding(.horizontal, 16)
                    }
                }
                if shown.isEmpty { Text("No examples match “\(q)”.").foregroundColor(TF.dim).padding(24) }
            }.padding(.bottom, 24)
        }
    }
}

private struct ExampleDetail: View {
    @EnvironmentObject var vm: AppModel
    let e: Example
    let onBack: () -> Void
    @State private var current: URL? = nil
    @State private var showExpected = false

    private var files: [URL] {
        let fm = FileManager.default
        guard let en = fm.enumerator(at: e.dir, includingPropertiesForKeys: [.isRegularFileKey]) else { return [e.main] }
        return en.compactMap { $0 as? URL }.filter { !$0.isDirectoryURL && $0.lastPathComponent != "expected.txt" }.sorted { $0.path < $1.path }
    }

    var body: some View {
        let cur = current ?? e.main
        let fs = files
        let expected = e.expected.exists ? e.expected.readText() : nil
        VStack(spacing: 0) {
            TopBar(title: e.title, subtitle: e.category, leading: {
                Button(action: onBack) { Image(systemName: "chevron.left").font(.system(size: 18, weight: .semibold)) }
                    .frame(width: 36, height: 36).accessibilityLabel("Back")
            }, trailing: { EmptyView() })
            ScrollView {
                VStack(alignment: .leading, spacing: 0) {
                    Text(e.summary).foregroundColor(TF.text.opacity(0.9)).fixedSize(horizontal: false, vertical: true)
                    if e.virtualTime { Text("Runs on the virtual clock.").font(.caption).foregroundColor(TF.cyan).padding(.top, 4) }
                    if e.cosim { Text("Co-simulation: runs against a MuJoCo model on the phone and opens the 3D Sim view.").font(.caption).foregroundColor(TF.cyan).padding(.top, 4) }
                    if fs.count > 1 {
                        ScrollView(.horizontal, showsIndicators: false) {
                            HStack(spacing: 8) { ForEach(fs, id: \.self) { f in Chip(text: f.relative(to: e.dir), selected: f == cur) { current = f } } }
                        }.padding(.top, 12)
                    }
                    CodeBlock(lang: "tulpa", code: cur.readText()).padding(.top, 12)
                    if let expected {
                        Button { showExpected.toggle() } label: {
                            Label("Expected output", systemImage: showExpected ? "chevron.up" : "chevron.down").font(.subheadline)
                        }.padding(.top, 10)
                        if showExpected {
                            ScrollView(.horizontal) { Text(expected).font(TF.mono(12)).foregroundColor(TF.dim).padding(12) }
                                .background(TF.surface).clipShape(RoundedRectangle(cornerRadius: 10)).padding(.top, 6)
                        }
                    }
                }.padding(16)
            }
            HStack(spacing: 8) {
                Button { vm.runExample(e) } label: { Label("Run", systemImage: "play.fill") }.buttonStyle(PrimaryButton())
                    .accessibilityIdentifier("example-run")
                Button { vm.copyExample(e) } label: { Label("Open as project", systemImage: "folder.badge.plus") }.buttonStyle(OutlineButton())
                    .accessibilityIdentifier("open-as-project")
                Spacer()
                IconButton(system: "terminal", label: "Load into REPL") { vm.loadIntoRepl(e.main.readText(), name: e.title) }
            }
            .padding(12).background(TF.surface)
        }
    }
}

private struct GuideIndex: View {
    @EnvironmentObject var vm: AppModel
    var body: some View {
        var groups: [(String, [GuideDoc])] = []
        for d in vm.guide {
            if let i = groups.firstIndex(where: { $0.0 == d.section }) { groups[i].1.append(d) } else { groups.append((d.section, [d])) }
        }
        return ScrollView {
            LazyVStack(alignment: .leading, spacing: 0) {
                ForEach(groups, id: \.0) { section, docs in
                    Text(section).font(.subheadline.weight(.semibold)).foregroundColor(TF.violet).padding(.leading, 20).padding(.top, 16).padding(.bottom, 4)
                    ForEach(docs) { d in
                        Button { vm.guideDoc = d.id } label: {
                            HStack(spacing: 14) {
                                Image(systemName: section == "Reference" ? "doc.plaintext" : "book").foregroundColor(TF.cyan)
                                Text(d.title).foregroundColor(TF.text)
                                Spacer()
                                Image(systemName: "chevron.right").foregroundColor(TF.dim).font(.caption)
                            }.padding(.horizontal, 20).padding(.vertical, 12).contentShape(Rectangle())
                        }.buttonStyle(.plain)
                    }
                }
            }.padding(.vertical, 8)
        }
    }
}

private struct GuideReader: View {
    @EnvironmentObject var vm: AppModel
    let id: String

    var body: some View {
        let docs = vm.guide
        let idx = max(docs.firstIndex { $0.id == id } ?? 0, 0)
        if docs.isEmpty {
            EmptyStateView(icon: "book", title: "Guide not bundled", message: "This build has no guide chapters.")
        } else {
            let d = docs[idx]
            let md: String = { let s = vm.workspace.readAsset(d.asset); return s.isEmpty ? "# Missing\n\nThis chapter is not bundled." : s }()
            VStack(spacing: 0) {
                TopBar(title: d.title, subtitle: d.section, leading: {
                    Button { vm.guideDoc = nil } label: { Image(systemName: "chevron.left").font(.system(size: 18, weight: .semibold)) }
                        .frame(width: 36, height: 36).accessibilityLabel("Back")
                }, trailing: { EmptyView() })
                ScrollViewReader { proxy in
                    ScrollView {
                        VStack(alignment: .leading, spacing: 0) {
                            Color.clear.frame(height: 0).id("top")
                            MarkdownView(md, onRun: { vm.runSnippet($0, title: d.title) }, onProject: { vm.snippetToProject($0, title: d.title) })
                            HStack {
                                if idx > 0 {
                                    let p = docs[idx - 1]
                                    Button("‹ \(p.title)") { vm.guideDoc = p.id }.buttonStyle(OutlineButton()).lineLimit(1).frame(maxWidth: 160, alignment: .leading)
                                }
                                Spacer()
                                if idx + 1 < docs.count {
                                    let n = docs[idx + 1]
                                    Button("\(n.title) ›") { vm.guideDoc = n.id }.buttonStyle(PrimaryButton()).lineLimit(1).frame(maxWidth: 180, alignment: .trailing)
                                }
                            }.padding(.top, 24).padding(.bottom, 32)
                        }
                        .padding(.horizontal, 18).padding(.vertical, 12)
                    }
                    .onChange(of: id) { _ in proxy.scrollTo("top", anchor: .top) }
                }
            }
        }
    }
}

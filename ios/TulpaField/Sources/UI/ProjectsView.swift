import SwiftUI
import UniformTypeIdentifiers

struct ProjectsView: View {
    @EnvironmentObject var vm: AppModel
    @State private var newDialog = false
    @State private var importing = false
    @State private var prompt: TextPrompt? = nil
    @State private var deleteFor: Project? = nil

    static let importTypes: [UTType] = {
        var t: [UTType] = [.plainText, .xml, .json, .sourceCode, .text]
        for ext in ["rav", "tlp"] { if let u = UTType(filenameExtension: ext) { t.append(u) } }
        return t
    }()

    var body: some View {
        VStack(spacing: 0) {
            TopBar(title: "Projects", subtitle: "Tulpa \(vm.tulpaVersion)", leading: { BackButton() }) {
                IconButton(system: "doc.badge.arrow.up", label: "Import a file") { importing = true }
                ToolsMenu()
            }
            if vm.projects.isEmpty {
                EmptyStateView(icon: "folder", title: "No projects yet",
                               message: "Start from a template, or open an example from the Library and make it yours.",
                               actions: [("New project", { newDialog = true }), ("Library", { vm.go(.learn) })])
            } else {
                ScrollView {
                    LazyVStack(spacing: 10) {
                        ForEach(vm.projects) { p in row(p) }
                    }.padding(.horizontal, 16).padding(.top, 4).padding(.bottom, 96)
                }
            }
        }
        .overlay(alignment: .bottomTrailing) {
            Button { newDialog = true } label: { Label("New project", systemImage: "plus").padding(.vertical, 6) }
                .buttonStyle(PrimaryButton()).shadow(radius: 6).padding(20)
                .accessibilityIdentifier("new-project")
        }
        .sheet(isPresented: $newDialog) { NewProjectSheet { n, t in newDialog = false; vm.newProject(n, template: t) } }
        .fileImporter(isPresented: $importing, allowedContentTypes: Self.importTypes, allowsMultipleSelection: false) { r in
            if case .success(let urls) = r, let u = urls.first { vm.importFile(u) }
        }
        .textPrompt($prompt)
        .alert("Delete “\(deleteFor?.name ?? "")”?", isPresented: Binding(get: { deleteFor != nil }, set: { if !$0 { deleteFor = nil } })) {
            Button("Delete", role: .destructive) { if let p = deleteFor { vm.deleteProject(p) }; deleteFor = nil }
            Button("Cancel", role: .cancel) { deleteFor = nil }
        } message: { Text("All \(deleteFor?.files ?? 0) files in this project will be removed. This can't be undone.") }
        .onAppear {
            vm.refreshProjects()
            if vm.newProjectRequest { vm.newProjectRequest = false; newDialog = true }
        }
        .onChange(of: vm.newProjectRequest) { w in if w { vm.newProjectRequest = false; newDialog = true } }
    }

    private func row(_ p: Project) -> some View {
        let current = vm.editor.project?.standardizedFileURL == p.dir.standardizedFileURL
        return HStack(spacing: 14) {
            Image(systemName: p.name == "Inbox" ? "tray.fill" : "folder.fill").foregroundColor(current ? TF.cyan : TF.violet)
            VStack(alignment: .leading, spacing: 2) {
                Text(p.name).font(.headline).foregroundColor(TF.text)
                Text("\(p.files) file\(p.files == 1 ? "" : "s") · \(p.modified.formatted(date: .abbreviated, time: .shortened))")
                    .font(.caption).foregroundColor(TF.dim)
            }
            Spacer()
            Menu { menu(p) } label: { Image(systemName: "ellipsis").foregroundColor(TF.dim).frame(width: 36, height: 36) }
                .accessibilityLabel("Project menu")
        }
        .padding(16)
        .background(current ? TF.surface2 : TF.surface).clipShape(RoundedRectangle(cornerRadius: 12))
        .contentShape(Rectangle())
        .onTapGesture { vm.openProject(p.dir) }
        .contextMenu { menu(p) }
    }

    @ViewBuilder private func menu(_ p: Project) -> some View {
        Button { vm.openProject(p.dir) } label: { Label("Open", systemImage: "folder") }
        Button { prompt = TextPrompt(title: "Rename project", label: "name", initial: p.name, confirm: "Rename") { vm.renameProject(p, $0) } } label: { Label("Rename…", systemImage: "pencil") }
        Button { vm.duplicateProject(p) } label: { Label("Duplicate", systemImage: "plus.square.on.square") }
        Button { vm.shareProject(p) } label: { Label("Share", systemImage: "square.and.arrow.up") }
        Button(role: .destructive) { deleteFor = p } label: { Label("Delete…", systemImage: "trash") }
    }
}

struct NewProjectSheet: View {
    let onCreate: (String, Template) -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var name = ""
    @State private var tpl: Template = .sensors

    var body: some View {
        NavigationStack {
            Form {
                Section { TextField(tpl.title, text: $name).autocorrectionDisabled().accessibilityIdentifier("project-name") } header: { Text("Name") }
                Section {
                    ForEach(Template.allCases) { t in
                        Button { tpl = t } label: {
                            HStack {
                                Image(systemName: tpl == t ? "largecircle.fill.circle" : "circle").foregroundColor(TF.violet)
                                Text(t.title).foregroundColor(TF.text)
                            }
                        }
                    }
                } header: { Text("Start from") }
            }
            .scrollContentBackground(.hidden)
            .background(TF.surface)
            .navigationTitle("New project").navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Create") { onCreate(name.trimmingCharacters(in: .whitespaces).isEmpty ? tpl.title : name, tpl) }
                }
            }
        }
        .presentationDetents([.medium, .large])
        .preferredColorScheme(.dark)
    }
}

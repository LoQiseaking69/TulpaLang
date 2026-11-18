import SwiftUI
import Combine
import UniformTypeIdentifiers

@main
struct TulpaFieldApp: App {
    @StateObject private var vm = AppModel()
    @Environment(\.scenePhase) private var scenePhase

    var body: some Scene {
        WindowGroup {
            RootView()
                .environmentObject(vm)
                .preferredColorScheme(.dark)
                .tint(TF.violet)
                .onOpenURL { vm.importFile($0) }
                .onChange(of: scenePhase) { p in if p != .active { vm.autosave() } }
        }
    }
}

private struct Tab: Identifiable { let screen: Screen; let label: String; let icon: String; var id: Screen { screen } }

private let TABS = [
    Tab(screen: .home, label: "Home", icon: "house"),
    Tab(screen: .code, label: "Code", icon: "chevron.left.forwardslash.chevron.right"),
    Tab(screen: .run, label: "Run", icon: "play.circle"),
    Tab(screen: .sim, label: "Sim", icon: "cube.transparent"),
    Tab(screen: .repl, label: "REPL", icon: "terminal"),
    Tab(screen: .learn, label: "Learn", icon: "book"),
]

struct RootView: View {
    @EnvironmentObject var vm: AppModel
    @State private var keyboard = false

    var body: some View {
        let isTab = TABS.contains { $0.screen == vm.screen } && !keyboard
        VStack(spacing: 0) {
            content.frame(maxWidth: .infinity, maxHeight: .infinity)
            if isTab { TabBar(runner: vm.runner) }
        }
        .background(TF.bg.ignoresSafeArea())
        .foregroundColor(TF.text)
        .overlay(alignment: .bottom) {
            if let t = vm.toastText {
                Text(t).font(.subheadline).foregroundColor(TF.text).multilineTextAlignment(.center)
                    .padding(.horizontal, 16).padding(.vertical, 10)
                    .background(TF.surface2).clipShape(Capsule())
                    .overlay(Capsule().stroke(TF.line, lineWidth: 1))
                    .padding(.horizontal, 24).padding(.bottom, 90)
                    .transition(.opacity.combined(with: .move(edge: .bottom)))
                    .accessibilityIdentifier("toast")
            }
        }
        .animation(.easeOut(duration: 0.2), value: vm.toastText)
        .sheet(item: $vm.share) { s in ShareSheet(items: s.items).presentationDetents([.medium, .large]) }
        .onReceive(NotificationCenter.default.publisher(for: UIResponder.keyboardWillShowNotification)) { _ in keyboard = true }
        .onReceive(NotificationCenter.default.publisher(for: UIResponder.keyboardWillHideNotification)) { _ in keyboard = false }
        .onReceive(vm.runner.$state.map(\.running).removeDuplicates()) { _ in vm.updateIdleTimer() }
        .onReceive(vm.host.$keepAwake) { _ in DispatchQueue.main.async { vm.updateIdleTimer() } }
    }

    @ViewBuilder private var content: some View {
        switch vm.screen {
        case .home: HomeView()
        case .projects: ProjectsView()
        case .code: EditorView(settings: vm.settings, runner: vm.runner)
        case .run: RunView(runner: vm.runner, host: vm.host)
        case .sim: SimView(sim: vm.sim, runner: vm.runner)
        case .repl: ReplView(repl: vm.repl)
        case .learn: LearnView()
        case .sensors: SensorsView(sensors: vm.sensors)
        case .machines: MachinesView()
        case .verify: VerifyView()
        case .bytecode: BytecodeView()
        case .settings: SettingsView(settings: vm.settings)
        }
    }
}

private struct TabBar: View {
    @EnvironmentObject var vm: AppModel
    @ObservedObject var runner: RunManager

    var body: some View {
        HStack(spacing: 0) {
            ForEach(TABS) { t in
                let sel = vm.screen == t.screen
                Button { vm.go(t.screen) } label: {
                    VStack(spacing: 3) {
                        ZStack(alignment: .topTrailing) {
                            Image(systemName: t.icon).font(.system(size: 19))
                                .frame(width: 52, height: 28)
                                .background(sel ? TF.violet.opacity(0.25) : .clear).clipShape(Capsule())
                            if t.screen == .run && runner.state.running {
                                Circle().fill(TF.ok).frame(width: 8, height: 8).offset(x: -10, y: 2)
                            }
                        }
                        Text(t.label).font(.system(size: 11)).lineLimit(1)
                    }
                    .foregroundColor(sel ? TF.cyan : TF.dim)
                    .frame(maxWidth: .infinity)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityIdentifier("tab-\(t.label)")
                .accessibilityLabel(t.label)
            }
        }
        .padding(.top, 6).padding(.bottom, 4)
        .background(TF.surface.ignoresSafeArea(edges: .bottom))
    }
}

/// The instruments menu shown on every main screen.
struct ToolsMenu: View {
    @EnvironmentObject var vm: AppModel
    var body: some View {
        Menu {
            Button { vm.go(.sensors) } label: { Label("Sensors", systemImage: "sensor") }
            Button { vm.go(.machines) } label: { Label("Machines", systemImage: "point.3.connected.trianglepath.dotted") }
            Button { vm.go(.verify) } label: { Label("Verify & tests", systemImage: "checkmark.seal") }
            Button { vm.go(.bytecode) } label: { Label("Bytecode", systemImage: "cpu") }
            Button { vm.go(.settings) } label: { Label("Settings", systemImage: "gearshape") }
        } label: {
            Image(systemName: "square.grid.2x2").font(.system(size: 18)).foregroundColor(TF.text).frame(width: 38, height: 38)
        }
        .accessibilityLabel("Instruments")
        .accessibilityIdentifier("instruments")
    }
}

/// Back button + title bar for secondary screens.
struct SubTopBar<Trailing: View>: View {
    let title: String
    var subtitle: String? = nil
    @ViewBuilder var trailing: () -> Trailing
    var body: some View {
        TopBar(title: title, subtitle: subtitle, leading: { BackButton() }, trailing: trailing)
    }
}

extension SubTopBar where Trailing == EmptyView {
    init(title: String, subtitle: String? = nil) { self.init(title: title, subtitle: subtitle, trailing: { EmptyView() }) }
}

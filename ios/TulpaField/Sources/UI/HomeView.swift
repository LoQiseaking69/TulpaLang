import SwiftUI

/// The welcome page: what Tulpa Field is, where to start, and what you were working on.
struct HomeView: View {
    @EnvironmentObject var vm: AppModel
    @Environment(\.openURL) private var openURL

    var body: some View {
        VStack(spacing: 0) {
            TopBar(title: "") {
                IconButton(system: "gearshape", label: "Settings") { vm.go(.settings) }
                ToolsMenu()
            }
            ScrollView {
                VStack(alignment: .leading, spacing: 0) {
                    hero
                    if let f = vm.editor.file {
                        SectionTitle("Continue")
                        Button { vm.go(.code) } label: {
                            Card {
                                HStack(spacing: 12) {
                                    Image(systemName: "chevron.left.forwardslash.chevron.right").foregroundColor(TF.cyan)
                                    VStack(alignment: .leading, spacing: 2) {
                                        Text(f.lastPathComponent).font(TF.mono(15, weight: .semibold)).foregroundColor(TF.text)
                                        Text(vm.editor.project?.lastPathComponent ?? "").font(.caption).foregroundColor(TF.dim)
                                    }
                                    Spacer()
                                    Button { vm.runCurrent() } label: { Label("Run", systemImage: "play.fill") }.buttonStyle(TonalButton())
                                }
                            }
                        }.buttonStyle(.plain)
                    }

                    SectionTitle("Start")
                    LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: 8), count: 3), spacing: 8) {
                        tile("New project", "plus") { vm.go(.projects); vm.requestNewProject() }
                        tile("3D simulation", "cube.transparent") { vm.go(.sim) }
                        tile("Phone sensors", "sensor") { vm.go(.sensors) }
                        tile("REPL", "terminal") { vm.go(.repl) }
                        tile("Guide", "book") { vm.openGuide(nil) }
                        tile("All projects", "folder") { vm.go(.projects) }
                    }

                    SectionTitle("What makes it different")
                    VStack(spacing: 8) {
                        highlight("iphone", "Your phone is the device",
                                  "phone:: reads the accelerometer, gyro, compass, barometer and GPS as unit-checked quantities, and drives haptics, the torch and speech.") { vm.openExample("bubble_level") }
                        highlight("cube.transparent", "MuJoCo physics + 3D co-simulation",
                                  "cosim:: programs drive a MuJoCo model on the phone, the same protocol .RAV IDE uses. Watch it live in 3D, push it, reset it.") { vm.openExample("cosim_cartpole") }
                        highlight("point.3.connected.trianglepath.dotted", "State machines you can see",
                                  "Every machine is drawn as a tappable graph; contracts are fuzzed for counterexamples with one tap.") { vm.openExample("shake_machine") }
                        highlight("book", "Learn by running", "30+ runnable examples and a guide whose code blocks run with one tap.") { vm.go(.learn) }
                    }

                    if !vm.projects.isEmpty {
                        HStack {
                            SectionTitle("Recent projects")
                            Button("All") { vm.go(.projects) }.padding(.top, 14)
                        }
                        ForEach(vm.projects.prefix(3)) { p in
                            Button { vm.openProject(p.dir) } label: {
                                HStack(spacing: 12) {
                                    Image(systemName: "folder.fill").foregroundColor(TF.violet)
                                    Text(p.name).foregroundColor(TF.text).lineLimit(1)
                                    Spacer()
                                    Text(p.modified, style: .date).font(.caption).foregroundColor(TF.dim)
                                }.padding(.vertical, 10).padding(.horizontal, 4).contentShape(Rectangle())
                            }.buttonStyle(.plain)
                        }
                    }

                    Divider().background(TF.line).padding(.top, 20)
                    HStack {
                        VStack(alignment: .leading, spacing: 2) {
                            Text("Tulpa Field \(vm.appVersion)").font(.system(size: 13, weight: .semibold))
                            Text("TulpaLang \(vm.tulpaVersion) · MuJoCo \(vm.sim.state.version)").font(.caption).foregroundColor(TF.dim)
                        }
                        Spacer()
                        Button("GitHub") { openURL(URL(string: "https://github.com/LoQiseaking69/TulpaLang")!) }
                    }.padding(.vertical, 12)
                    Spacer(minLength: 16)
                }
                .padding(.horizontal, 18)
            }
        }
        .onAppear { vm.refreshProjects() }
    }

    private var hero: some View {
        VStack(alignment: .leading, spacing: 0) {
            Image("Logo").resizable().frame(width: 64, height: 64)
            Text("Welcome to Tulpa Field").font(.title2.bold()).foregroundColor(TF.text).padding(.top, 10)
            Text("TulpaLang in your pocket: write code for things that move and sense, run it against your phone's real sensors or a MuJoCo physics model in 3D. Everything runs offline, on the device.")
                .foregroundColor(TF.text.opacity(0.85)).lineSpacing(3).padding(.top, 6).fixedSize(horizontal: false, vertical: true)
            HStack(spacing: 10) {
                Button { vm.openGuide("welcome") } label: { Label("Start the tour", systemImage: "book") }.buttonStyle(PrimaryButton())
                    .accessibilityIdentifier("start-tour")
                Button("Examples") { vm.guideTab = 0; vm.go(.learn) }.buttonStyle(OutlineButton())
            }.padding(.top, 14)
        }
        .padding(20)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(
            ZStack {
                LinearGradient(colors: [Color(hex: 0x1A1340), Color(hex: 0x0B1E33), TF.bg], startPoint: .topLeading, endPoint: .bottomTrailing)
                GeometryReader { g in
                    let c = CGPoint(x: g.size.width * 0.86, y: g.size.height * 0.22)
                    ForEach(1..<5) { k in
                        Circle().stroke(TF.violet.opacity(0.10 / Double(k)), lineWidth: 1)
                            .frame(width: 60 * CGFloat(k), height: 60 * CGFloat(k)).position(c)
                    }
                }
            }
        )
        .clipShape(RoundedRectangle(cornerRadius: 22))
        .overlay(RoundedRectangle(cornerRadius: 22).stroke(TF.line, lineWidth: 1))
    }

    private func tile(_ label: String, _ icon: String, _ f: @escaping () -> Void) -> some View {
        Button(action: f) {
            VStack(spacing: 6) {
                Image(systemName: icon).font(.system(size: 20)).foregroundColor(TF.cyan)
                Text(label).font(.system(size: 12)).foregroundColor(TF.text).lineLimit(1).minimumScaleFactor(0.8)
            }
            .frame(maxWidth: .infinity, minHeight: 84)
            .background(TF.surface).clipShape(RoundedRectangle(cornerRadius: 14))
            .overlay(RoundedRectangle(cornerRadius: 14).stroke(TF.line, lineWidth: 1))
        }.buttonStyle(.plain)
    }

    private func highlight(_ icon: String, _ title: String, _ body: String, _ f: @escaping () -> Void) -> some View {
        Button(action: f) {
            Card {
                HStack(alignment: .top, spacing: 12) {
                    Image(systemName: icon).foregroundColor(TF.violet).frame(width: 38, height: 38)
                        .background(TF.violet.opacity(0.18)).clipShape(RoundedRectangle(cornerRadius: 10))
                    VStack(alignment: .leading, spacing: 2) {
                        Text(title).font(.subheadline.weight(.semibold)).foregroundColor(TF.text)
                        Text(body).font(.system(size: 13)).foregroundColor(TF.dim).multilineTextAlignment(.leading).fixedSize(horizontal: false, vertical: true)
                    }
                    Spacer(minLength: 0)
                }
            }
        }.buttonStyle(.plain)
    }
}

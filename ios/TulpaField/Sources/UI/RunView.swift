import SwiftUI

struct RunView: View {
    @EnvironmentObject var vm: AppModel
    @ObservedObject var runner: RunManager
    @ObservedObject var host: TulpaHost
    @State private var showPlots = true
    @State private var prompt: TextPrompt? = nil

    var body: some View {
        let st = runner.state
        VStack(spacing: 0) {
            TimelineView(.periodic(from: .now, by: 0.2)) { ctx in
                TopBar(title: st.title.isEmpty ? "Run" : st.title, subtitle: nil) {
                    if st.running {
                        Button { runner.stop() } label: { Label("Stop", systemImage: "stop.fill").foregroundColor(TF.err) }
                            .buttonStyle(TonalButton()).accessibilityIdentifier("stop")
                    } else {
                        Button { vm.runCurrent() } label: { Label("Run", systemImage: "play.fill") }
                            .buttonStyle(TonalButton()).disabled(vm.editor.file == nil).accessibilityIdentifier("run")
                    }
                    optionsMenu(st)
                    ToolsMenu()
                }
                .overlay(alignment: .bottomLeading) {
                    Text(statusLabel(st, now: ctx.date)).font(.caption2).foregroundColor(statusColor(st))
                        .padding(.leading, 12).padding(.bottom, 2).accessibilityIdentifier("run-status")
                }
            }
            if let s = st.status {
                Text(s).font(.system(size: 13)).foregroundColor(TF.cyan).frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, 16).padding(.vertical, 6).background(TF.surface2)
            }
            if !host.gaugeOrder.isEmpty {
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 10) { ForEach(host.gaugeOrder, id: \.self) { n in if let g = host.gauges[n] { GaugeCard(g: g) } } }
                        .padding(.horizontal, 12).padding(.vertical, 8)
                }
            }
            if !st.plots.isEmpty {
                Button { showPlots.toggle() } label: {
                    HStack(spacing: 6) {
                        Image(systemName: "chart.xyaxis.line").foregroundColor(TF.cyan).font(.caption)
                        Text("Live plots (\(st.plots.count))").font(.subheadline.weight(.medium)).foregroundColor(TF.dim)
                        Spacer()
                        Image(systemName: showPlots ? "chevron.up" : "chevron.down").foregroundColor(TF.dim)
                    }.padding(.horizontal, 16).padding(.vertical, 4).contentShape(Rectangle())
                }.buttonStyle(.plain)
                if showPlots {
                    ScrollView {
                        VStack(spacing: 8) {
                            ForEach(Array(st.plotOrder.enumerated()), id: \.element) { i, name in
                                if let s = st.plots[name] { PlotCard(s: s, color: TF.series[i % TF.series.count], events: st.events.map { $0.0 }) }
                            }
                        }.padding(.horizontal, 12)
                    }.frame(maxHeight: 300).fixedSize(horizontal: false, vertical: st.plots.count < 2)
                }
            }
            if !st.artifacts.isEmpty {
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 8) {
                        ForEach(st.artifacts) { a in
                            let f = URL(fileURLWithPath: a.path)
                            Button { vm.shareFile(f) } label: {
                                Label(f.lastPathComponent, systemImage: "paperclip").font(.caption).foregroundColor(TF.text)
                                    .padding(.horizontal, 10).padding(.vertical, 6).background(TF.surface2).clipShape(Capsule())
                            }
                        }
                    }.padding(.horizontal, 12).padding(.vertical, 4)
                }
            }
            ConsoleView(runner: runner, fontSize: CGFloat(max(vm.settings.prefs.fontSize - 1, 10)))
            if let p = st.prompt {
                InputRow(prompt: p, onSend: { runner.answer($0) }, onEof: { runner.answer(nil) })
            }
        }
        .textPrompt($prompt)
    }

    private func statusLabel(_ st: RunState, now: Date) -> String {
        let end = st.running ? now : st.finishedAt
        let secs = max(0, end.timeIntervalSince(st.startedAt))
        if st.running { return String(format: "running · %.1f s", secs) }
        if st.ok == true { return String(format: "finished · %.2f s", secs) }
        if st.ok == false { return (st.exit?.contains("interrupted") == true) ? "stopped" : "failed" }
        return "idle"
    }

    private func statusColor(_ st: RunState) -> Color {
        st.running ? TF.cyan : st.ok == false ? TF.err : st.ok == true ? TF.ok : TF.dim
    }

    private func optionsMenu(_ st: RunState) -> some View {
        Menu {
            Toggle("Virtual time", isOn: $vm.virtualTime)
            Button {
                prompt = TextPrompt(title: "Program arguments", label: "space-separated, read with args()", initial: vm.runArgs, confirm: "Set") { vm.runArgs = $0 }
            } label: { Label(vm.runArgs.isEmpty ? "Run with arguments…" : "Arguments: \(vm.runArgs)", systemImage: "text.cursor") }
            Button { runner.clear() } label: { Label("Clear console", systemImage: "clear") }.disabled(st.running)
            Button { vm.shareText(st.console.map(\.text).joined(separator: "\n"), title: "Tulpa output") } label: { Label("Share output", systemImage: "square.and.arrow.up") }
            Button { vm.go(.sensors) } label: { Label("Sensors", systemImage: "sensor") }
        } label: { Image(systemName: "slider.horizontal.3").font(.system(size: 17)).foregroundColor(TF.text).frame(width: 38, height: 38) }
        .accessibilityLabel("Run options")
    }
}

private struct ConsoleView: View {
    @EnvironmentObject var vm: AppModel
    @ObservedObject var runner: RunManager
    let fontSize: CGFloat

    var body: some View {
        let lines = runner.state.console
        if lines.isEmpty {
            EmptyStateView(icon: "terminal", title: "Nothing has run yet",
                           message: "Run the open file from here or from the editor — or try an example from the Library.",
                           actions: [("Library", { vm.go(.learn) })])
        } else {
            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 1) {
                        ForEach(lines) { l in
                            Text(l.text.isEmpty ? " " : l.text)
                                .font(.system(size: fontSize, weight: l.kind == .info ? .semibold : .regular, design: .monospaced))
                                .foregroundColor(color(l.kind))
                                .frame(maxWidth: .infinity, alignment: .leading)
                                .textSelection(.enabled)
                                .onTapGesture { if let n = l.line { vm.reveal(line: n - 1) } }
                                .id(l.id)
                        }
                    }.padding(12)
                }
                .background(TF.bg)
                .accessibilityIdentifier("console")
                .onChange(of: lines.last?.id) { id in if let id { proxy.scrollTo(id, anchor: .bottom) } }
                .onAppear { if let id = lines.last?.id { proxy.scrollTo(id, anchor: .bottom) } }
            }
        }
    }

    private func color(_ k: LineKind) -> Color {
        switch k { case .out: return TF.text; case .err: return TF.err; case .info: return TF.violet; case .event: return TF.cyan; case .input: return TF.ok }
    }
}

struct InputRow: View {
    let prompt: String
    let onSend: (String) -> Void
    var onEof: (() -> Void)? = nil
    @State private var v = ""
    @FocusState private var focused: Bool

    var body: some View {
        HStack(spacing: 6) {
            TextField(prompt.trimmingCharacters(in: .whitespaces).isEmpty ? "input" : prompt, text: $v)
                .font(TF.mono(14)).textFieldStyle(.roundedBorder)
                .textInputAutocapitalization(.never).autocorrectionDisabled()
                .submitLabel(.send).focused($focused)
                .onSubmit { onSend(v); v = "" }
                .accessibilityIdentifier("input-field")
            if let onEof { Button("EOF", action: onEof).font(.subheadline) }
            IconButton(system: "paperplane.fill", label: "Send", tint: TF.cyan) { onSend(v); v = "" }
        }
        .padding(8).background(TF.surface2)
        .onAppear { focused = true }
    }
}

private struct GaugeCard: View {
    let g: Gauge
    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(g.name).font(.caption.weight(.medium)).foregroundColor(TF.dim).lineLimit(1)
            HStack(alignment: .lastTextBaseline, spacing: 4) {
                Text(fmt(g.value)).font(TF.mono(26, weight: .bold)).foregroundColor(TF.text).lineLimit(1)
                Text(g.unit).font(.system(size: 13)).foregroundColor(TF.cyan).lineLimit(1)
            }
            if let lo = g.min, let hi = g.max, hi > lo {
                ProgressView(value: min(max((g.value - lo) / (hi - lo), 0), 1)).tint(TF.cyan)
            }
        }
        .padding(12).frame(minWidth: 156, maxWidth: 240, alignment: .leading)
        .background(TF.surface2).clipShape(RoundedRectangle(cornerRadius: 14))
    }
}

struct PlotCard: View {
    let s: PlotSeries
    let color: Color
    let events: [Float]

    var body: some View {
        let pts = s.ordered()
        var lo = pts.map { $0.1 }.min() ?? 0
        var hi = pts.map { $0.1 }.max() ?? 1
        if hi - lo < 1e-9 { hi += 0.5; lo -= 0.5 }
        let t0 = pts.first?.0 ?? 0
        let t1 = pts.last?.0 ?? 1
        return VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 6) {
                Circle().fill(color).frame(width: 8, height: 8)
                Text(s.name).font(.subheadline.weight(.medium)).foregroundColor(TF.text)
                Spacer()
                Text((s.last.map { fmt($0) } ?? "—") + (s.unit.map { " \($0)" } ?? "")).font(TF.mono(14)).foregroundColor(color)
            }
            HStack(spacing: 0) {
                VStack(alignment: .leading) {
                    Text(fmt(hi)); Spacer(); Text(fmt(lo))
                }.font(TF.mono(10)).foregroundColor(TF.dim).frame(width: 44, height: 90, alignment: .leading)
                Canvas { ctx, size in
                    let w = size.width, h = size.height
                    for g in 0...3 {
                        let y = h * CGFloat(g) / 3
                        var p = Path(); p.move(to: CGPoint(x: 0, y: y)); p.addLine(to: CGPoint(x: w, y: y))
                        ctx.stroke(p, with: .color(TF.line), style: StrokeStyle(lineWidth: 1, dash: [4, 6]))
                    }
                    guard pts.count >= 2 else { return }
                    let span = CGFloat(max(t1 - t0, 1e-6))
                    for e in events where e >= t0 && e <= t1 {
                        let x = CGFloat(e - t0) / span * w
                        var p = Path(); p.move(to: CGPoint(x: x, y: 0)); p.addLine(to: CGPoint(x: x, y: h))
                        ctx.stroke(p, with: .color(Color(Syn.macro).opacity(0.6)), lineWidth: 1.5)
                    }
                    var path = Path()
                    for (k, pt) in pts.enumerated() {
                        let x = CGFloat(pt.0 - t0) / span * w
                        let y = h - CGFloat((pt.1 - lo) / (hi - lo)) * h
                        if k == 0 { path.move(to: CGPoint(x: x, y: y)) } else { path.addLine(to: CGPoint(x: x, y: y)) }
                    }
                    ctx.stroke(path, with: .color(color), lineWidth: 2)
                }.frame(height: 90)
            }
            Text(String(format: "%.1f s", t1 - t0) + " window · \(s.count) samples").font(.system(size: 10)).foregroundColor(TF.dim)
        }
        .padding(10).background(TF.surface).clipShape(RoundedRectangle(cornerRadius: 12))
    }
}

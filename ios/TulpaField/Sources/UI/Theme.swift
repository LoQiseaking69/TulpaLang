import SwiftUI
import UIKit

extension UIColor {
    convenience init(hex: UInt32, alpha: CGFloat = 1) {
        self.init(red: CGFloat((hex >> 16) & 0xFF) / 255, green: CGFloat((hex >> 8) & 0xFF) / 255,
                  blue: CGFloat(hex & 0xFF) / 255, alpha: alpha)
    }
}

extension Color {
    init(hex: UInt32, alpha: Double = 1) { self.init(UIColor(hex: hex, alpha: CGFloat(alpha))) }
}

/// Tulpa Field palette (shared with the Android app).
enum TF {
    static let bg = Color(hex: 0x070A14)
    static let surface = Color(hex: 0x0E1324)
    static let surface2 = Color(hex: 0x151B33)
    static let line = Color(hex: 0x232B4A)
    static let violet = Color(hex: 0x7C5CFF)
    static let cyan = Color(hex: 0x3FD0FF)
    static let text = Color(hex: 0xE6E9F5)
    static let dim = Color(hex: 0x8A93B8)
    static let ok = Color(hex: 0x4ADE80)
    static let warn = Color(hex: 0xFFB547)
    static let err = Color(hex: 0xFF5C7A)

    static let series: [Color] = [cyan, violet, Color(hex: 0xFF7EB6), ok, warn, Color(hex: 0x82AAFF)]

    static func mono(_ size: CGFloat, weight: Font.Weight = .regular) -> Font { .system(size: size, weight: weight, design: .monospaced) }
}

/// UIKit colours for the code editor.
enum Syn {
    static let text = UIColor(hex: 0xE6E9F5)
    static let dim = UIColor(hex: 0x8A93B8)
    static let bg = UIColor(hex: 0x070A14)
    static let surface = UIColor(hex: 0x0E1324)
    static let line = UIColor(hex: 0x232B4A)
    static let keyword = UIColor(hex: 0xB79CFF)
    static let str = UIColor(hex: 0x9BE59B)
    static let num = UIColor(hex: 0xFFB86C)
    static let unit = UIColor(hex: 0x3FD0FF)
    static let comment = UIColor(hex: 0x5D6790)
    static let funcC = UIColor(hex: 0x82AAFF)
    static let type = UIColor(hex: 0xFFD580)
    static let macro = UIColor(hex: 0xFF7EB6)
    static let module = UIColor(hex: 0x5EEAD4)
    static let err = UIColor(hex: 0xFF5C7A)
    static let warn = UIColor(hex: 0xFFB547)
    static let violet = UIColor(hex: 0x7C5CFF)
}

func fmt(_ v: Float) -> String {
    if v.isNaN { return "—" }
    if v == 0 { return "0" }
    let a = abs(v)
    if a >= 10000 || a < 0.01 { return String(format: "%.3g", v) }
    if a >= 100 { return String(format: "%.1f", v) }
    var s = String(format: "%.3f", v)
    while s.hasSuffix("0") { s.removeLast() }
    if s.hasSuffix(".") { s.removeLast() }
    return s
}

func fmt(_ v: Double) -> String { fmt(Float(v)) }

// ---------------------------------------------------------------------- common views

struct TopBar<Leading: View, Trailing: View>: View {
    let title: String
    var subtitle: String? = nil
    @ViewBuilder var leading: () -> Leading
    @ViewBuilder var trailing: () -> Trailing

    var body: some View {
        HStack(spacing: 6) {
            leading()
            if !title.isEmpty { VStack(alignment: .leading, spacing: 1) {
                Text(title).font(.headline).foregroundColor(TF.text).lineLimit(1)
                if let subtitle, !subtitle.isEmpty {
                    Text(subtitle).font(.caption).foregroundColor(TF.dim).lineLimit(1)
                }
            } }
            Spacer(minLength: 4)
            trailing()
        }
        .padding(.horizontal, 12)
        .frame(height: 50)
        .background(TF.bg)
    }
}

extension TopBar where Leading == EmptyView {
    init(title: String, subtitle: String? = nil, @ViewBuilder trailing: @escaping () -> Trailing) {
        self.init(title: title, subtitle: subtitle, leading: { EmptyView() }, trailing: trailing)
    }
}

struct BackButton: View {
    @EnvironmentObject var vm: AppModel
    var body: some View {
        Button { _ = vm.back() } label: { Image(systemName: "chevron.left").font(.system(size: 18, weight: .semibold)) }
            .frame(width: 36, height: 36)
            .accessibilityLabel("Back")
    }
}

struct IconButton: View {
    let system: String
    let label: String
    var tint: Color = TF.text
    var disabled = false
    let action: () -> Void
    var body: some View {
        Button(action: action) { Image(systemName: system).font(.system(size: 17)).foregroundColor(disabled ? TF.dim.opacity(0.5) : tint) }
            .frame(width: 38, height: 38)
            .disabled(disabled)
            .accessibilityLabel(label)
    }
}

struct EmptyStateView: View {
    let icon: String
    let title: String
    let message: String
    var actions: [(String, () -> Void)] = []
    var body: some View {
        VStack(spacing: 0) {
            Image(systemName: icon).font(.system(size: 50)).foregroundColor(TF.violet)
            Text(title).font(.title3.bold()).foregroundColor(TF.text).padding(.top, 16)
            Text(message).foregroundColor(TF.dim).multilineTextAlignment(.center).padding(.top, 8)
            HStack(spacing: 12) {
                ForEach(Array(actions.enumerated()), id: \.offset) { i, a in
                    if i == 0 { Button(a.0, action: a.1).buttonStyle(PrimaryButton()) } else { Button(a.0, action: a.1).buttonStyle(OutlineButton()) }
                }
            }.padding(.top, 20)
        }
        .padding(32)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

struct PrimaryButton: ButtonStyle {
    var color: Color = TF.violet
    func makeBody(configuration: Configuration) -> some View {
        configuration.label.font(.subheadline.weight(.semibold)).foregroundColor(.white)
            .padding(.horizontal, 16).padding(.vertical, 9)
            .background(color.opacity(configuration.isPressed ? 0.7 : 1)).clipShape(Capsule())
    }
}

struct OutlineButton: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label.font(.subheadline.weight(.semibold)).foregroundColor(TF.text)
            .padding(.horizontal, 16).padding(.vertical, 9)
            .overlay(Capsule().stroke(TF.line, lineWidth: 1))
            .opacity(configuration.isPressed ? 0.6 : 1)
    }
}

struct TonalButton: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label.font(.subheadline.weight(.semibold)).foregroundColor(TF.text)
            .padding(.horizontal, 14).padding(.vertical, 8)
            .background(TF.violet.opacity(configuration.isPressed ? 0.15 : 0.28)).clipShape(Capsule())
    }
}

struct Card<Content: View>: View {
    var padding: CGFloat = 14
    @ViewBuilder var content: () -> Content
    var body: some View {
        content().padding(padding).frame(maxWidth: .infinity, alignment: .leading)
            .background(TF.surface).clipShape(RoundedRectangle(cornerRadius: 12))
    }
}

struct SectionTitle: View {
    let text: String
    init(_ t: String) { text = t }
    var body: some View {
        Text(text).font(.subheadline.weight(.semibold)).foregroundColor(TF.violet)
            .frame(maxWidth: .infinity, alignment: .leading).padding(.top, 22).padding(.bottom, 8)
    }
}

struct Chip: View {
    let text: String
    var selected = false
    var action: (() -> Void)? = nil
    var body: some View {
        let label = Text(text).font(.caption.weight(.medium)).foregroundColor(selected ? .white : TF.text)
            .padding(.horizontal, 10).padding(.vertical, 6)
            .background(selected ? TF.violet : TF.surface2).clipShape(Capsule())
            .overlay(Capsule().stroke(selected ? Color.clear : TF.line, lineWidth: 1))
        if let action { Button(action: action) { label }.buttonStyle(.plain) } else { label }
    }
}

/// A text prompt presented as an alert (rename, new file, …).
struct TextPrompt: Identifiable {
    let id = UUID()
    let title: String
    let label: String
    let initial: String
    let confirm: String
    let onConfirm: (String) -> Void
}

struct TextPromptModifier: ViewModifier {
    @Binding var prompt: TextPrompt?
    @State private var value = ""

    func body(content: Content) -> some View {
        content.alert(prompt?.title ?? "", isPresented: Binding(get: { prompt != nil }, set: { if !$0 { prompt = nil } })) {
            TextField(prompt?.label ?? "", text: $value)
                .textInputAutocapitalization(.never).autocorrectionDisabled()
            Button("Cancel", role: .cancel) { prompt = nil }
            Button(prompt?.confirm ?? "OK") {
                let v = value.trimmingCharacters(in: .whitespaces)
                let p = prompt
                prompt = nil
                if !v.isEmpty { p?.onConfirm(v) }
            }
        }
        .onChange(of: prompt?.id) { _ in value = prompt?.initial ?? "" }
    }
}

extension View {
    func textPrompt(_ p: Binding<TextPrompt?>) -> some View { modifier(TextPromptModifier(prompt: p)) }
}

/// UIActivityViewController wrapper for sharing files and text.
struct ShareSheet: UIViewControllerRepresentable {
    let items: [Any]
    func makeUIViewController(context: Context) -> UIActivityViewController { UIActivityViewController(activityItems: items, applicationActivities: nil) }
    func updateUIViewController(_ vc: UIActivityViewController, context: Context) {}
}

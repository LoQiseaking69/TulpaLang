import AVFoundation
import AudioToolbox
import CoreHaptics
import Foundation
import UIKit
import UserNotifications

/// A live value pushed by `phone::gauge(name, value, min, max)`.
struct Gauge: Equatable, Identifiable {
    var id: String { name }
    let name: String
    let value: Double
    let unit: String
    let min: Double?
    let max: Double?
}

/// Receives `phone::` actuator calls from Rust (called on the program thread) and performs them
/// with iOS APIs: Core Haptics, the camera torch, speech synthesis, a tone generator, in-app
/// toasts and local notifications.
final class TulpaHost: NSObject, ObservableObject, UNUserNotificationCenterDelegate {
    static let shared = TulpaHost()

    @Published private(set) var gauges: [String: Gauge] = [:]
    @Published private(set) var gaugeOrder: [String] = []
    @Published private(set) var keepAwake = false
    @Published private(set) var torch = false

    /// Actuator events shown in the Run console, e.g. "vibrate 200 ms".
    var onEvent: ((String) -> Void)?
    /// `cosim` notifications from a program (plot / log), forwarded by the co-sim server.
    var onSimNotify: ((String, [String: Any]) -> Void)?
    /// In-app toast (set by the app model).
    var onToast: ((String) -> Void)?

    private let speech = AVSpeechSynthesizer()
    private var haptics: CHHapticEngine?
    private var hapticPlayer: CHHapticPatternPlayer?
    private let audio = AVAudioEngine()
    private let tone = AVAudioPlayerNode()
    private var audioReady = false
    private var torchOn = false

    func setup() {
        UNUserNotificationCenter.current().delegate = self
        if CHHapticEngine.capabilitiesForHardware().supportsHaptics {
            haptics = try? CHHapticEngine()
            haptics?.isAutoShutdownEnabled = true
            haptics?.resetHandler = { [weak self] in try? self?.haptics?.start() }
        }
    }

    func resetForRun() {
        DispatchQueue.main.async { self.gauges = [:]; self.gaugeOrder = [] }
    }

    /// Called when a program ends: release actuators it may have left on.
    func releaseAll() {
        if torchOn { try? setTorch(false) }
        DispatchQueue.main.async { self.keepAwake = false }
        try? hapticPlayer?.stop(atTime: CHHapticTimeImmediate)
    }

    func action(kind: String, payload: String) {
        let p = JSON.object(payload)
        do {
            switch kind {
            case "vibrate": try vibrate(p)
            case "torch": try setTorch(p["on"] == nil ? true : p.bool("on"))
            case "say": say(p.str("text"), rate: p.double("rate", 1.0))
            case "beep": try beep(hz: p.double("hz", 880), ms: p.int("ms", 150))
            case "toast": let t = p.str("text"); DispatchQueue.main.async { self.onToast?(t) }
            case "notify": notify(p.str("title"), p.str("text"))
            case "keep_awake": let on = p["on"] == nil ? true : p.bool("on"); DispatchQueue.main.async { self.keepAwake = on }
            case "gauge": gauge(p)
            case "sim_notify": onSimNotify?(p.str("method"), p.obj("params") ?? [:])
            default: break
            }
        } catch {
            onEvent?("phone::\(kind) failed: \(error.localizedDescription)")
        }
    }

    private struct Failure: LocalizedError { let errorDescription: String? }

    // ------------------------------------------------------------------ actuators

    private func vibrate(_ p: [String: Any]) throws {
        let ms = (p.arr("ms") ?? []).compactMap { ($0 as? NSNumber)?.doubleValue }.map { min(max($0, 1), 10_000) }
        let amp = Float(min(max(p.double("amplitude", 1.0), 0.05), 1.0))
        let pattern = ms.isEmpty ? [80.0] : ms
        if let engine = haptics {
            // pattern: on, off, on, … (milliseconds)
            var events: [CHHapticEvent] = []
            var t = 0.0
            for (i, d) in pattern.enumerated() {
                if i % 2 == 0 {
                    events.append(CHHapticEvent(eventType: .hapticContinuous, parameters: [
                        CHHapticEventParameter(parameterID: .hapticIntensity, value: amp),
                        CHHapticEventParameter(parameterID: .hapticSharpness, value: 0.5),
                    ], relativeTime: t, duration: d / 1000))
                }
                t += d / 1000
            }
            try engine.start()
            let player = try engine.makePlayer(with: CHHapticPattern(events: events, parameters: []))
            hapticPlayer = player
            try player.start(atTime: CHHapticTimeImmediate)
        } else {
            AudioServicesPlaySystemSound(kSystemSoundID_Vibrate)
        }
        onEvent?("⟿ vibrate \(pattern.map { String(Int($0)) }.joined(separator: "/")) ms")
    }

    private func setTorch(_ on: Bool) throws {
        guard let dev = AVCaptureDevice.default(for: .video), dev.hasTorch else {
            throw Failure(errorDescription: "this phone has no flashlight")
        }
        try dev.lockForConfiguration()
        if on { try dev.setTorchModeOn(level: AVCaptureDevice.maxAvailableTorchLevel) } else { dev.torchMode = .off }
        dev.unlockForConfiguration()
        if torchOn != on { onEvent?(on ? "☀ torch on" : "☀ torch off") }
        torchOn = on
        DispatchQueue.main.async { self.torch = on }
    }

    private func say(_ text: String, rate: Double) {
        let u = AVSpeechUtterance(string: text)
        u.rate = Float(min(max(Double(AVSpeechUtteranceDefaultSpeechRate) * rate, Double(AVSpeechUtteranceMinimumSpeechRate)),
                           Double(AVSpeechUtteranceMaximumSpeechRate)))
        try? AVAudioSession.sharedInstance().setCategory(.playback, options: [.mixWithOthers, .duckOthers])
        speech.speak(u)
        onEvent?("🗣 \"\(text)\"")
    }

    private func beep(hz: Double, ms: Int) throws {
        let fmt = AVAudioFormat(standardFormatWithSampleRate: 44_100, channels: 1)!
        if !audioReady {
            try? AVAudioSession.sharedInstance().setCategory(.playback, options: [.mixWithOthers])
            audio.attach(tone)
            audio.connect(tone, to: audio.mainMixerNode, format: fmt)
            audioReady = true
        }
        if !audio.isRunning { try audio.start() }
        let n = AVAudioFrameCount(44_100 * Double(min(max(ms, 20), 3000)) / 1000)
        guard let buf = AVAudioPCMBuffer(pcmFormat: fmt, frameCapacity: n) else { return }
        buf.frameLength = n
        let f = min(max(hz, 50), 8000)
        let data = buf.floatChannelData![0]
        let ramp = min(Int(n) / 4, 441)
        for i in 0..<Int(n) {
            let env = Float(min(1, Double(min(i, Int(n) - 1 - i)) / Double(max(ramp, 1))))
            data[i] = 0.35 * env * Float(sin(2 * .pi * f * Double(i) / 44_100))
        }
        tone.scheduleBuffer(buf, at: nil, options: [], completionHandler: nil)
        if !tone.isPlaying { tone.play() }
    }

    private func notify(_ title: String, _ text: String) {
        let c = UNUserNotificationCenter.current()
        c.requestAuthorization(options: [.alert, .sound]) { granted, _ in
            guard granted else {
                DispatchQueue.main.async { self.onToast?("\(title): \(text)") }
                return
            }
            let n = UNMutableNotificationContent()
            n.title = title.isEmpty ? "Tulpa Field" : title
            n.body = text
            n.sound = .default
            c.add(UNNotificationRequest(identifier: UUID().uuidString, content: n, trigger: nil))
        }
    }

    // Show program notifications while the app is in the foreground too.
    func userNotificationCenter(_ center: UNUserNotificationCenter, willPresent notification: UNNotification,
                                withCompletionHandler done: @escaping (UNNotificationPresentationOptions) -> Void) {
        done([.banner, .sound, .list])
    }

    private func gauge(_ p: [String: Any]) {
        let g = Gauge(name: p.str("name"), value: p.double("value"), unit: p.str("unit"),
                      min: p.isNull("min") ? nil : p.double("min"), max: p.isNull("max") ? nil : p.double("max"))
        DispatchQueue.main.async {
            if self.gauges[g.name] == nil { self.gaugeOrder.append(g.name) }
            self.gauges[g.name] = g
        }
    }
}

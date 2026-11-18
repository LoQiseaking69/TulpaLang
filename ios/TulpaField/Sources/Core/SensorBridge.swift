import CoreLocation
import CoreMotion
import Foundation
import UIKit

/// Kinds shared with android/native/src/phone.rs.
enum SensorKind {
    static let accel = 1, gyro = 2, mag = 3, orientation = 4, light = 5, pressure = 6, proximity = 7
    static let battery = 8, location = 9, gravity = 10, linear = 11, steps = 12
}

struct SensorReading: Equatable { let kind: Int; let values: [Float]; let time: Date }

struct SensorInfo: Identifiable, Equatable {
    var id: Int { kind }
    let kind: Int
    let title: String
    let call: String
    let unit: String
    let available: Bool
    let vendor: String
}

/// Streams the handset's sensors into the Tulpa runtime (`phone::*`). Started while a program,
/// the REPL or the Sensors screen needs it; stopped otherwise to save battery.
///
/// Values follow the Android conventions the runtime was written against: accelerations in m/s²
/// with +z out of the screen when the phone lies face-up (iOS reports g with the opposite sign),
/// orientation as azimuth/pitch/roll in radians, pressure in hPa.
final class SensorBridge: NSObject, ObservableObject, CLLocationManagerDelegate {
    @Published private(set) var latest: [Int: SensorReading] = [:]
    @Published private(set) var locationAuthorized = false

    private let motion = CMMotionManager()
    private let altimeter = CMAltimeter()
    private let pedometer = CMPedometer()
    private let location = CLLocationManager()
    private let queue: OperationQueue = { let q = OperationQueue(); q.name = "tulpa-sensors"; q.maxConcurrentOperationCount = 1; return q }()
    private let lock = NSLock()
    private var users = 0
    private var locationOn = false
    /** Set when the running program reads phone::proximity(). */
    var proximityWanted = false
    private var pending: [Int: SensorReading] = [:]
    private var lastUi = Date.distantPast
    private var observers: [NSObjectProtocol] = []
    private static let g: Double = 9.80665

    override init() {
        super.init()
        location.delegate = self
        location.desiredAccuracy = kCLLocationAccuracyBest
        locationAuthorized = Self.authorized(location.authorizationStatus)
    }

    private static func authorized(_ s: CLAuthorizationStatus) -> Bool { s == .authorizedWhenInUse || s == .authorizedAlways }

    func catalog() -> [SensorInfo] {
        let dm = motion.isDeviceMotionAvailable
        return [
            SensorInfo(kind: SensorKind.accel, title: "Accelerometer", call: "phone::accel()", unit: "m/s²", available: motion.isAccelerometerAvailable, vendor: "Core Motion"),
            SensorInfo(kind: SensorKind.gyro, title: "Gyroscope", call: "phone::gyro()", unit: "rad/s", available: motion.isGyroAvailable, vendor: "Core Motion"),
            SensorInfo(kind: SensorKind.mag, title: "Magnetometer", call: "phone::mag()", unit: "µT", available: motion.isMagnetometerAvailable, vendor: "Core Motion"),
            SensorInfo(kind: SensorKind.orientation, title: "Orientation (attitude)", call: "phone::orientation() · phone::heading()", unit: "deg", available: dm, vendor: "Core Motion device motion"),
            SensorInfo(kind: SensorKind.gravity, title: "Gravity", call: "phone::gravity() · phone::tilt()", unit: "m/s²", available: dm, vendor: "Core Motion device motion"),
            SensorInfo(kind: SensorKind.linear, title: "Linear acceleration", call: "phone::linear() · phone::shake()", unit: "m/s²", available: dm, vendor: "Core Motion device motion"),
            SensorInfo(kind: SensorKind.light, title: "Light", call: "phone::light()", unit: "lux", available: false, vendor: "not available to iOS apps"),
            SensorInfo(kind: SensorKind.pressure, title: "Barometer", call: "phone::pressure() · phone::altitude()", unit: "hPa", available: CMAltimeter.isRelativeAltitudeAvailable(), vendor: "CMAltimeter"),
            SensorInfo(kind: SensorKind.proximity, title: "Proximity", call: "phone::proximity()", unit: "cm", available: UIDevice.current.userInterfaceIdiom == .phone, vendor: "UIDevice proximity (near / far)"),
            SensorInfo(kind: SensorKind.steps, title: "Step counter", call: "phone::steps()", unit: "steps", available: CMPedometer.isStepCountingAvailable(), vendor: "CMPedometer"),
            SensorInfo(kind: SensorKind.battery, title: "Battery", call: "phone::battery() · phone::charging()", unit: "0–1", available: true, vendor: "UIDevice"),
            SensorInfo(kind: SensorKind.location, title: "Location (GPS)", call: "phone::location()", unit: "deg / m", available: locationAuthorized, vendor: "Core Location"),
        ]
    }

    func acquire() {
        lock.lock(); users += 1; let first = users == 1; lock.unlock()
        if first { DispatchQueue.main.async { self.startAll() } }
    }

    func release() {
        lock.lock()
        if users == 0 { lock.unlock(); return }
        users -= 1; let last = users == 0
        lock.unlock()
        if last { DispatchQueue.main.async { self.stopAll() } }
    }

    private var active: Bool { lock.lock(); defer { lock.unlock() }; return users > 0 }

    private func startAll() {
        guard active, observers.isEmpty else { return }
        let hz = 1.0 / 50.0
        if motion.isAccelerometerAvailable {
            motion.accelerometerUpdateInterval = hz
            motion.startAccelerometerUpdates(to: queue) { [weak self] d, _ in
                guard let a = d?.acceleration else { return }
                self?.push(SensorKind.accel, [-a.x * Self.g, -a.y * Self.g, -a.z * Self.g])
            }
        }
        if motion.isGyroAvailable {
            motion.gyroUpdateInterval = hz
            motion.startGyroUpdates(to: queue) { [weak self] d, _ in
                guard let r = d?.rotationRate else { return }
                self?.push(SensorKind.gyro, [r.x, r.y, r.z])
            }
        }
        if motion.isMagnetometerAvailable {
            motion.magnetometerUpdateInterval = hz
            motion.startMagnetometerUpdates(to: queue) { [weak self] d, _ in
                guard let m = d?.magneticField else { return }
                self?.push(SensorKind.mag, [m.x, m.y, m.z])
            }
        }
        if motion.isDeviceMotionAvailable {
            motion.deviceMotionUpdateInterval = hz
            let frames = CMMotionManager.availableAttitudeReferenceFrames()
            let frame: CMAttitudeReferenceFrame = frames.contains(.xMagneticNorthZVertical) ? .xMagneticNorthZVertical : .xArbitraryZVertical
            motion.startDeviceMotionUpdates(using: frame, to: queue) { [weak self] d, _ in
                guard let self, let d else { return }
                let gr = d.gravity, ua = d.userAcceleration
                self.push(SensorKind.gravity, [-gr.x * Self.g, -gr.y * Self.g, -gr.z * Self.g])
                self.push(SensorKind.linear, [-ua.x * Self.g, -ua.y * Self.g, -ua.z * Self.g])
                // Android getOrientation(): azimuth (clockwise from north), pitch, roll — radians
                let az: Double = d.heading >= 0 ? d.heading * .pi / 180 : -d.attitude.yaw
                self.push(SensorKind.orientation, [az, -d.attitude.pitch, d.attitude.roll])
            }
        }
        if CMAltimeter.isRelativeAltitudeAvailable() {
            altimeter.startRelativeAltitudeUpdates(to: queue) { [weak self] d, _ in
                guard let d else { return }
                self?.push(SensorKind.pressure, [d.pressure.doubleValue * 10])   // kPa → hPa
            }
        }
        if CMPedometer.isStepCountingAvailable() {
            pedometer.startUpdates(from: Date()) { [weak self] d, _ in
                guard let d else { return }
                self?.push(SensorKind.steps, [d.numberOfSteps.doubleValue])
            }
        }
        let dev = UIDevice.current
        dev.isBatteryMonitoringEnabled = true
        // iOS blanks the screen while something is near: only when a program asks for it
        dev.isProximityMonitoringEnabled = proximityWanted
        let nc = NotificationCenter.default
        observers.append(nc.addObserver(forName: UIDevice.batteryLevelDidChangeNotification, object: nil, queue: .main) { [weak self] _ in self?.pushBattery() })
        observers.append(nc.addObserver(forName: UIDevice.batteryStateDidChangeNotification, object: nil, queue: .main) { [weak self] _ in self?.pushBattery() })
        observers.append(nc.addObserver(forName: UIDevice.proximityStateDidChangeNotification, object: nil, queue: .main) { [weak self] _ in self?.pushProximity() })
        pushBattery()
        if dev.isProximityMonitoringEnabled { pushProximity() }
        if locationOn { startLocation() }
    }

    private func stopAll() {
        guard !active else { return }
        motion.stopAccelerometerUpdates(); motion.stopGyroUpdates(); motion.stopMagnetometerUpdates()
        motion.stopDeviceMotionUpdates(); altimeter.stopRelativeAltitudeUpdates(); pedometer.stopUpdates()
        observers.forEach { NotificationCenter.default.removeObserver($0) }
        observers.removeAll()
        UIDevice.current.isProximityMonitoringEnabled = false
        location.stopUpdatingLocation()
    }

    private func pushBattery() {
        let dev = UIDevice.current
        let level = dev.batteryLevel
        guard level >= 0 else { return }
        let charging = dev.batteryState == .charging || dev.batteryState == .full
        push(SensorKind.battery, [Double(level), charging ? 1 : 0])
    }

    private func pushProximity() {
        push(SensorKind.proximity, [UIDevice.current.proximityState ? 0 : 5])
    }

    // ------------------------------------------------------------------ location

    func hasLocationPermission() -> Bool { Self.authorized(location.authorizationStatus) }

    func requestLocation() {
        if location.authorizationStatus == .notDetermined { location.requestWhenInUseAuthorization() }
        setLocationEnabled(true)
    }

    func setLocationEnabled(_ on: Bool) {
        locationOn = on
        DispatchQueue.main.async {
            if self.active { if on { self.startLocation() } else { self.location.stopUpdatingLocation() } }
        }
    }

    var locationEnabled: Bool { locationOn }

    private func startLocation() {
        guard hasLocationPermission() else { return }
        location.startUpdatingLocation()
        if let l = location.location { locationManager(location, didUpdateLocations: [l]) }
    }

    func locationManagerDidChangeAuthorization(_ m: CLLocationManager) {
        let ok = Self.authorized(m.authorizationStatus)
        DispatchQueue.main.async { self.locationAuthorized = ok }
        if ok && locationOn && active { m.startUpdatingLocation() }
    }

    func locationManager(_ m: CLLocationManager, didUpdateLocations locs: [CLLocation]) {
        guard let l = locs.last else { return }
        let speed = max(l.speed, 0), course = max(l.course, 0)
        TulpaNative.pushLocation(lat: l.coordinate.latitude, lon: l.coordinate.longitude, alt: l.altitude,
                                 accuracy: l.horizontalAccuracy, speed: speed, bearing: course)
        publish(SensorKind.location, [Float(l.coordinate.latitude), Float(l.coordinate.longitude), Float(l.altitude),
                                      Float(l.horizontalAccuracy), Float(speed), Float(course)])
    }

    func locationManager(_ m: CLLocationManager, didFailWithError error: Error) {}

    // ------------------------------------------------------------------ plumbing

    private func push(_ kind: Int, _ v: [Double]) {
        let f = v.map { Float($0) }
        TulpaNative.pushSensor(kind, f)
        publish(kind, f)
    }

    /// Latest reading per kind, throttled to ~10 Hz for the UI.
    private func publish(_ kind: Int, _ values: [Float]) {
        let now = Date()
        lock.lock()
        pending[kind] = SensorReading(kind: kind, values: values, time: now)
        guard now.timeIntervalSince(lastUi) >= 0.1 else { lock.unlock(); return }
        lastUi = now
        let batch = pending
        pending.removeAll()
        lock.unlock()
        DispatchQueue.main.async { self.latest.merge(batch) { _, n in n } }
    }
}

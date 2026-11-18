import MetalKit
import SwiftUI
import simd

/// Orbit camera around a target, z-up like MuJoCo.
final class OrbitCamera {
    var azimuth: Float = 135
    var elevation: Float = 22
    var distance: Float = 4
    var target = SIMD3<Float>(0, 0, 0.5)
    var extent: Float = 1

    func frame(_ center: SIMD3<Float>, _ ext: Float) {
        extent = max(ext, 0.05)
        target = center
        distance = extent * 1.7
        azimuth = 135; elevation = 20
    }

    func eye() -> SIMD3<Float> {
        let a = azimuth * .pi / 180, e = elevation * .pi / 180
        return target + SIMD3(distance * cos(e) * cos(a), distance * cos(e) * sin(a), distance * sin(e))
    }

    func rotate(_ dx: Float, _ dy: Float) {
        azimuth = (azimuth - dx * 0.35).truncatingRemainder(dividingBy: 360)
        elevation = min(max(elevation + dy * 0.3, -10), 89)
    }

    func zoom(_ f: Float) { distance = min(max(distance / f, extent * 0.15), extent * 40) }

    func pan(_ dx: Float, _ dy: Float, viewH: Float) {
        let s = distance / viewH * 1.1
        let a = azimuth * .pi / 180
        target.x += -sin(a) * -dx * s
        target.y += cos(a) * -dx * s
        target.z = max(target.z + dy * s, -extent)
    }
}

private struct Uniforms {
    var mvp = matrix_identity_float4x4
    var model = matrix_identity_float4x4
    var nrm = matrix_identity_float4x4
    var color = SIMD4<Float>(1, 1, 1, 1)
    var eye = SIMD4<Float>(0, 0, 0, 0)
    var params = SIMD4<Float>(0, 0, 0, 0)
    var center = SIMD4<Float>(0, 0, 0, 0)
}

private struct Mesh { let buffer: MTLBuffer; let count: Int }

/// Unit primitives (triangle lists, position + normal).
private enum Prim {
    static func sphere(lat: Int = 18, lon: Int = 28, hemi: Bool = false) -> [Float] {
        var v: [Float] = []
        let top = hemi ? Double.pi / 2 : Double.pi
        func p(_ i: Int, _ j: Int) {
            let th = top * Double(i) / Double(lat), ph = 2 * Double.pi * Double(j) / Double(lon)
            let x = Float(sin(th) * cos(ph)), y = Float(sin(th) * sin(ph)), z = Float(cos(th))
            v += [x, y, z, x, y, z]
        }
        for i in 0..<lat { for j in 0..<lon { p(i, j); p(i + 1, j); p(i + 1, j + 1); p(i, j); p(i + 1, j + 1); p(i, j + 1) } }
        return v
    }

    static func cylinder(caps: Bool, n: Int = 28) -> [Float] {
        var v: [Float] = []
        for j in 0..<n {
            let a0 = 2 * Double.pi * Double(j) / Double(n), a1 = 2 * Double.pi * Double(j + 1) / Double(n)
            let x0 = Float(cos(a0)), y0 = Float(sin(a0)), x1 = Float(cos(a1)), y1 = Float(sin(a1))
            v += [x0, y0, -1, x0, y0, 0, x1, y1, -1, x1, y1, 0, x1, y1, 1, x1, y1, 0]
            v += [x0, y0, -1, x0, y0, 0, x1, y1, 1, x1, y1, 0, x0, y0, 1, x0, y0, 0]
            if caps {
                v += [0, 0, 1, 0, 0, 1, x0, y0, 1, 0, 0, 1, x1, y1, 1, 0, 0, 1]
                v += [0, 0, -1, 0, 0, -1, x1, y1, -1, 0, 0, -1, x0, y0, -1, 0, 0, -1]
            }
        }
        return v
    }

    static func box() -> [Float] {
        var v: [Float] = []
        let faces: [SIMD3<Float>] = [[1, 0, 0], [-1, 0, 0], [0, 1, 0], [0, -1, 0], [0, 0, 1], [0, 0, -1]]
        for n in faces {
            let u: SIMD3<Float> = n.x != 0 ? [0, 1, 0] : [1, 0, 0]
            let w = cross(n, u)
            func c(_ a: Float, _ b: Float) -> [Float] { let p = n + a * u + b * w; return [p.x, p.y, p.z, n.x, n.y, n.z] }
            v += c(-1, -1); v += c(1, -1); v += c(1, 1); v += c(-1, -1); v += c(1, 1); v += c(-1, 1)
        }
        return v
    }

    static let quad: [Float] = [
        -1, -1, 0, 0, 0, 1, 1, -1, 0, 0, 0, 1, 1, 1, 0, 0, 0, 1,
        -1, -1, 0, 0, 0, 1, 1, 1, 0, 0, 0, 1, -1, 1, 0, 0, 0, 1,
    ]
}

/// Live HUD numbers published by the renderer.
final class SimHud { var fps: Float = 0; var geoms = 0 }

final class SimRenderer: NSObject, MTKViewDelegate {
    let camera = OrbitCamera()
    let hud = SimHud()
    var reframe = true
    var shadows = true
    private let sim: SimManager
    private let device: MTLDevice
    private let queue: MTLCommandQueue
    private let lit: MTLRenderPipelineState
    private let floor: MTLRenderPipelineState
    private let shadow: MTLRenderPipelineState
    private let depthOn: MTLDepthStencilState
    private let depthNoWrite: MTLDepthStencilState
    private let sphere, hemi, tube, cyl, box, quad: Mesh
    private var meshes: [Int: Mesh] = [:]
    private var meshGen = -1
    private var framedGen = -1
    private var lastT = CACurrentMediaTime()
    private var frames = 0

    static let floorColor = SIMD4<Float>(0.055, 0.07, 0.13, 1)

    init?(view: MTKView, sim: SimManager) {
        guard let device = view.device ?? MTLCreateSystemDefaultDevice(),
              let queue = device.makeCommandQueue(),
              let lib = device.makeDefaultLibrary() else { return nil }
        self.sim = sim
        self.device = device
        self.queue = queue
        func pipeline(_ frag: String) -> MTLRenderPipelineState? {
            let d = MTLRenderPipelineDescriptor()
            d.vertexFunction = lib.makeFunction(name: "tf_vertex")
            d.fragmentFunction = lib.makeFunction(name: frag)
            d.colorAttachments[0].pixelFormat = view.colorPixelFormat
            d.colorAttachments[0].isBlendingEnabled = true
            d.colorAttachments[0].sourceRGBBlendFactor = .sourceAlpha
            d.colorAttachments[0].destinationRGBBlendFactor = .oneMinusSourceAlpha
            d.colorAttachments[0].sourceAlphaBlendFactor = .one
            d.colorAttachments[0].destinationAlphaBlendFactor = .oneMinusSourceAlpha
            d.depthAttachmentPixelFormat = view.depthStencilPixelFormat
            d.rasterSampleCount = view.sampleCount
            return try? device.makeRenderPipelineState(descriptor: d)
        }
        guard let lit = pipeline("tf_lit"), let floor = pipeline("tf_floor"), let shadow = pipeline("tf_shadow") else { return nil }
        self.lit = lit; self.floor = floor; self.shadow = shadow
        let dd = MTLDepthStencilDescriptor()
        dd.depthCompareFunction = .lessEqual
        dd.isDepthWriteEnabled = true
        guard let on = device.makeDepthStencilState(descriptor: dd) else { return nil }
        dd.isDepthWriteEnabled = false
        guard let nw = device.makeDepthStencilState(descriptor: dd) else { return nil }
        depthOn = on; depthNoWrite = nw
        func up(_ a: [Float]) -> Mesh? {
            guard !a.isEmpty, let b = device.makeBuffer(bytes: a, length: a.count * 4, options: .storageModeShared) else { return nil }
            return Mesh(buffer: b, count: a.count / 6)
        }
        guard let s = up(Prim.sphere()), let h = up(Prim.sphere(lat: 9, hemi: true)), let t = up(Prim.cylinder(caps: false)),
              let c = up(Prim.cylinder(caps: true)), let bx = up(Prim.box()), let q = up(Prim.quad) else { return nil }
        sphere = s; hemi = h; tube = t; cyl = c; box = bx; quad = q
        super.init()
    }

    private func upload(_ a: [Float]) -> Mesh? {
        guard !a.isEmpty, let b = device.makeBuffer(bytes: a, length: a.count * 4, options: .storageModeShared) else { return nil }
        return Mesh(buffer: b, count: a.count / 6)
    }

    func mtkView(_ view: MTKView, drawableSizeWillChange size: CGSize) {}

    private static func perspective(fovy: Float, aspect: Float, near: Float, far: Float) -> float4x4 {
        let y = 1 / tan(fovy * 0.5), x = y / aspect, z = far / (near - far)
        return float4x4(columns: (SIMD4(x, 0, 0, 0), SIMD4(0, y, 0, 0), SIMD4(0, 0, z, -1), SIMD4(0, 0, z * near, 0)))
    }

    private static func lookAt(eye: SIMD3<Float>, target: SIMD3<Float>, up: SIMD3<Float>) -> float4x4 {
        let f = normalize(target - eye)
        var s = cross(f, up)
        if length(s) < 1e-6 { s = SIMD3(1, 0, 0) }
        s = normalize(s)
        let u = cross(s, f)
        return float4x4(columns: (SIMD4(s.x, u.x, -f.x, 0), SIMD4(s.y, u.y, -f.y, 0), SIMD4(s.z, u.z, -f.z, 0),
                                  SIMD4(-dot(s, eye), -dot(u, eye), dot(f, eye), 1)))
    }

    /// Model matrix from MuJoCo pos + row-major 3x3, then local scale/offset along z.
    private static func place(_ f: [Float], _ o: Int, _ sx: Float, _ sy: Float, _ sz: Float, _ oz: Float = 0) -> float4x4 {
        let p = o + 5, r = o + 8
        let c0 = SIMD3(f[r], f[r + 3], f[r + 6]), c1 = SIMD3(f[r + 1], f[r + 4], f[r + 7]), c2 = SIMD3(f[r + 2], f[r + 5], f[r + 8])
        let t = SIMD3(f[p], f[p + 1], f[p + 2]) + c2 * oz
        return float4x4(columns: (SIMD4(c0 * sx, 0), SIMD4(c1 * sy, 0), SIMD4(c2 * sz, 0), SIMD4(t, 1)))
    }

    private static func scaleTranslate(_ sx: Float, _ sy: Float, _ t: SIMD3<Float> = .zero) -> float4x4 {
        float4x4(columns: (SIMD4(sx, 0, 0, 0), SIMD4(0, sy, 0, 0), SIMD4(0, 0, 1, 0), SIMD4(t, 1)))
    }

    private static func normalMatrix(_ m: float4x4) -> float4x4 {
        let m3 = float3x3(SIMD3(m.columns.0.x, m.columns.0.y, m.columns.0.z), SIMD3(m.columns.1.x, m.columns.1.y, m.columns.1.z),
                          SIMD3(m.columns.2.x, m.columns.2.y, m.columns.2.z))
        let d = m3.determinant
        let n = abs(d) < 1e-12 ? m3 : m3.inverse.transpose
        return float4x4(columns: (SIMD4(n.columns.0, 0), SIMD4(n.columns.1, 0), SIMD4(n.columns.2, 0), SIMD4(0, 0, 0, 1)))
    }

    private static func cellSize(_ ext: Float) -> Float {
        let raw = ext / 4
        let p = powf(10, floorf(log10f(raw)))
        return raw / p < 2 ? p : raw / p < 5 ? 2 * p : 5 * p
    }

    func draw(in view: MTKView) {
        let model = sim.model
        let handle = model?.handle ?? 0
        if sim.generation != meshGen {
            meshes.removeAll()
            if let m = model { for i in 0..<m.meshes { if let mesh = upload(TulpaNative.simMesh(handle, i)) { meshes[i] = mesh } } }
            meshGen = sim.generation
        }
        if let m = model, framedGen != sim.generation || reframe { camera.frame(m.center, m.extent); framedGen = sim.generation; reframe = false }
        else if model == nil && reframe { camera.frame(SIMD3(0, 0, 0.5), 1.5); reframe = false }

        guard let pass = view.currentRenderPassDescriptor, let drawable = view.currentDrawable,
              let cmd = queue.makeCommandBuffer(), let enc = cmd.makeRenderCommandEncoder(descriptor: pass) else { return }
        let w = Float(max(view.drawableSize.width, 1)), h = Float(max(view.drawableSize.height, 1))
        let ext = camera.extent
        let proj = Self.perspective(fovy: 45 * .pi / 180, aspect: w / h, near: max(ext * 0.01, 0.005), far: ext * 200)
        let eye = camera.eye()
        let vp = proj * Self.lookAt(eye: eye, target: camera.target, up: SIMD3(0, 0, 1))
        var u = Uniforms()
        u.eye = SIMD4(eye, 1)

        func draw(_ mesh: Mesh, _ m: float4x4, _ color: SIMD4<Float>) {
            u.model = m; u.mvp = vp * m; u.nrm = Self.normalMatrix(m); u.color = color
            enc.setVertexBuffer(mesh.buffer, offset: 0, index: 0)
            enc.setVertexBytes(&u, length: MemoryLayout<Uniforms>.stride, index: 1)
            enc.setFragmentBytes(&u, length: MemoryLayout<Uniforms>.stride, index: 1)
            enc.drawPrimitives(type: .triangle, vertexStart: 0, vertexCount: mesh.count)
        }

        let frame = handle != 0 ? TulpaNative.simFrame(handle) : []
        let stride = 21
        let n = frame.count >= 4 ? (frame.count - 4) / stride : 0
        enc.setCullMode(.none)

        // floor: every plane geom, or a default stage at z = 0
        enc.setRenderPipelineState(floor)
        enc.setDepthStencilState(depthOn)
        let cell = Self.cellSize(ext)
        u.params = SIMD4(cell, max(ext * 12, cell * 40), 0, 0)
        var planes = 0
        for g in 0..<n {
            let o = 4 + g * stride
            if Int(frame[o]) != 0 { continue }
            planes += 1
            let sx = frame[o + 2] > 0 ? frame[o + 2] : ext * 60
            let sy = frame[o + 3] > 0 ? frame[o + 3] : ext * 60
            draw(quad, Self.place(frame, o, sx, sy, 1), Self.floorColor)
        }
        if planes == 0 { draw(quad, Self.scaleTranslate(ext * 60, ext * 60), Self.floorColor) }

        // soft contact shadows under bodies
        if shadows && n > 0 {
            enc.setRenderPipelineState(shadow)
            enc.setDepthStencilState(depthNoWrite)
            for g in 0..<n {
                let o = 4 + g * stride
                let t = Int(frame[o])
                if t == 0 || t == 1 { continue }
                let r: Float = (t == 6 ? max(frame[o + 2], frame[o + 3]) : t == 7 ? ext * 0.08 : max(frame[o + 2], 0.01)) * 1.6
                let zH = frame[o + 7]
                if zH < -0.01 || zH > ext * 3 { continue }
                let a = min(max(0.38 * (1 - zH / (ext * 3)), 0), 0.38)
                let rad = r + zH * 0.25
                u.params = SIMD4(rad, 0, 0, 0)
                u.center = SIMD4(frame[o + 5], frame[o + 6], 0, 0)
                draw(quad, Self.scaleTranslate(rad, rad, SIMD3(frame[o + 5], frame[o + 6], 0.001 * ext + 0.0005)), SIMD4(0, 0, 0, a))
            }
        }

        enc.setRenderPipelineState(lit)
        enc.setDepthStencilState(depthOn)
        for g in 0..<n {
            let o = 4 + g * stride
            let t = Int(frame[o])
            let sx = frame[o + 2], sy = frame[o + 3], sz = frame[o + 4]
            let col = SIMD4(frame[o + 17], frame[o + 18], frame[o + 19], frame[o + 20])
            switch t {
            case 2: draw(sphere, Self.place(frame, o, sx, sx, sx), col)
            case 3:   // capsule: radius sx, half-length sy
                draw(tube, Self.place(frame, o, sx, sx, sy), col)
                draw(hemi, Self.place(frame, o, sx, sx, sx, sy), col)
                draw(hemi, Self.place(frame, o, sx, sx, -sx, -sy), col)
            case 4: draw(sphere, Self.place(frame, o, sx, sy, sz), col)
            case 5: draw(cyl, Self.place(frame, o, sx, sx, sy), col)
            case 6: draw(box, Self.place(frame, o, sx, sy, sz), col)
            case 7: if let m = meshes[Int(frame[o + 1])] { draw(m, Self.place(frame, o, 1, 1, 1), col) }
            default: break
            }
        }
        enc.endEncoding()
        cmd.present(drawable)
        cmd.commit()

        hud.geoms = n
        frames += 1
        let now = CACurrentMediaTime()
        if now - lastT > 1 { hud.fps = Float(Double(frames) / (now - lastT)); frames = 0; lastT = now }
    }
}

/// MTKView with orbit (1 finger), pan (2 fingers), pinch zoom and double-tap reframe.
final class SimMTKView: MTKView, UIGestureRecognizerDelegate {
    var renderer: SimRenderer?
    private var lastPinch: CGFloat = 1

    func installGestures() {
        let orbit = UIPanGestureRecognizer(target: self, action: #selector(onOrbit(_:)))
        orbit.maximumNumberOfTouches = 1
        let pan = UIPanGestureRecognizer(target: self, action: #selector(onPan(_:)))
        pan.minimumNumberOfTouches = 2
        pan.delegate = self
        let pinch = UIPinchGestureRecognizer(target: self, action: #selector(onPinch(_:)))
        pinch.delegate = self
        let dbl = UITapGestureRecognizer(target: self, action: #selector(onDouble))
        dbl.numberOfTapsRequired = 2
        [orbit, pan, pinch, dbl].forEach(addGestureRecognizer)
    }

    func gestureRecognizer(_ g: UIGestureRecognizer, shouldRecognizeSimultaneouslyWith o: UIGestureRecognizer) -> Bool { true }

    @objc private func onOrbit(_ g: UIPanGestureRecognizer) {
        let t = g.translation(in: self)
        renderer?.camera.rotate(Float(t.x), Float(t.y))
        g.setTranslation(.zero, in: self)
    }

    @objc private func onPan(_ g: UIPanGestureRecognizer) {
        let t = g.translation(in: self)
        renderer?.camera.pan(Float(t.x), Float(t.y), viewH: Float(max(bounds.height, 1)))
        g.setTranslation(.zero, in: self)
    }

    @objc private func onPinch(_ g: UIPinchGestureRecognizer) {
        if g.state == .began { lastPinch = 1 }
        let f = g.scale / lastPinch
        lastPinch = g.scale
        renderer?.camera.zoom(Float(f))
    }

    @objc private func onDouble() { renderer?.reframe = true }
}

/// SwiftUI host for the Metal view. Shows nothing (transparent) when Metal is unavailable.
struct SimMetalView: UIViewRepresentable {
    let sim: SimManager
    let onRenderer: (SimRenderer?) -> Void

    func makeUIView(context: Context) -> SimMTKView {
        let v = SimMTKView(frame: .zero, device: MTLCreateSystemDefaultDevice())
        v.colorPixelFormat = .bgra8Unorm
        v.depthStencilPixelFormat = .depth32Float
        v.sampleCount = 4
        v.clearColor = MTLClearColor(red: 0.027, green: 0.039, blue: 0.078, alpha: 1)
        v.preferredFramesPerSecond = 60
        v.backgroundColor = UIColor(hex: 0x070A14)
        v.isAccessibilityElement = true
        v.accessibilityLabel = "3D simulation view"
        v.accessibilityIdentifier = "sim-view"
        if v.device != nil, let r = SimRenderer(view: v, sim: sim) {
            v.renderer = r
            v.delegate = r
            v.installGestures()
        } else {
            v.isPaused = true
            v.enableSetNeedsDisplay = true
        }
        let r = v.renderer
        DispatchQueue.main.async { onRenderer(r) }
        return v
    }

    func updateUIView(_ v: SimMTKView, context: Context) {}

    static func dismantleUIView(_ v: SimMTKView, coordinator: ()) { v.isPaused = true; v.delegate = nil }
}

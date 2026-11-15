package dev.hermitech.tulpafield.sim

import android.annotation.SuppressLint
import android.content.Context
import android.opengl.GLES30.*
import android.opengl.GLSurfaceView
import android.opengl.Matrix
import android.view.GestureDetector
import android.view.MotionEvent
import android.view.ScaleGestureDetector
import dev.hermitech.tulpafield.core.SimManager
import dev.hermitech.tulpafield.core.TulpaNative
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.FloatBuffer
import javax.microedition.khronos.egl.EGLConfig
import javax.microedition.khronos.opengles.GL10
import kotlin.math.PI
import kotlin.math.cos
import kotlin.math.max
import kotlin.math.min
import kotlin.math.sin
import kotlin.math.sqrt

/** Orbit camera around a target, z-up like MuJoCo. Shared between touch input and GL thread. */
class OrbitCamera {
    @Volatile var azimuth = 135f          // degrees around +z
    @Volatile var elevation = 22f         // degrees above the floor
    @Volatile var distance = 4f
    @Volatile var tx = 0f
    @Volatile var ty = 0f
    @Volatile var tz = 0.5f
    @Volatile var extent = 1f

    fun frame(center: FloatArray, ext: Float) {
        extent = max(ext, 0.05f)
        tx = center[0]; ty = center[1]; tz = center[2]
        distance = extent * 2.4f
        azimuth = 135f; elevation = 20f
    }

    fun eye(): FloatArray {
        val a = Math.toRadians(azimuth.toDouble()); val e = Math.toRadians(elevation.toDouble())
        return floatArrayOf(
            tx + (distance * cos(e) * cos(a)).toFloat(),
            ty + (distance * cos(e) * sin(a)).toFloat(),
            tz + (distance * sin(e)).toFloat(),
        )
    }

    fun rotate(dx: Float, dy: Float) {
        azimuth = (azimuth - dx * 0.35f) % 360f
        elevation = (elevation + dy * 0.3f).coerceIn(-10f, 89f)
    }

    fun zoom(f: Float) { distance = (distance / f).coerceIn(extent * 0.15f, extent * 40f) }

    fun pan(dx: Float, dy: Float, viewH: Float) {
        val s = distance / viewH * 1.1f
        val a = Math.toRadians(azimuth.toDouble())
        // screen right = (-sin a, cos a, 0); screen up ≈ +z
        tx += (-sin(a) * -dx * s).toFloat()
        ty += (cos(a) * -dx * s).toFloat()
        tz = (tz + dy * s).coerceAtLeast(-extent)
    }
}

private fun fbuf(a: FloatArray): FloatBuffer =
    ByteBuffer.allocateDirect(a.size * 4).order(ByteOrder.nativeOrder()).asFloatBuffer().apply { put(a); position(0) }

private class Mesh(val vao: Int, val vbo: Int, val count: Int) {
    fun delete() { glDeleteBuffers(1, intArrayOf(vbo), 0); glDeleteVertexArrays(1, intArrayOf(vao), 0) }
}

private fun upload(data: FloatArray): Mesh {
    val ids = IntArray(2)
    glGenVertexArrays(1, ids, 0); glGenBuffers(1, ids, 1)
    glBindVertexArray(ids[0]); glBindBuffer(GL_ARRAY_BUFFER, ids[1])
    glBufferData(GL_ARRAY_BUFFER, data.size * 4, fbuf(data), GL_STATIC_DRAW)
    glEnableVertexAttribArray(0); glVertexAttribPointer(0, 3, GL_FLOAT, false, 24, 0)
    glEnableVertexAttribArray(1); glVertexAttribPointer(1, 3, GL_FLOAT, false, 24, 12)
    glBindVertexArray(0)
    return Mesh(ids[0], ids[1], data.size / 6)
}

/** Unit primitives (triangle lists, position + normal). */
private object Prim {
    fun sphere(lat: Int = 18, lon: Int = 28): FloatArray {
        val v = ArrayList<Float>()
        fun p(i: Int, j: Int) {
            val th = PI * i / lat; val ph = 2 * PI * j / lon
            val x = (sin(th) * cos(ph)).toFloat(); val y = (sin(th) * sin(ph)).toFloat(); val z = cos(th).toFloat()
            v += listOf(x, y, z, x, y, z)
        }
        for (i in 0 until lat) for (j in 0 until lon) { p(i, j); p(i + 1, j); p(i + 1, j + 1); p(i, j); p(i + 1, j + 1); p(i, j + 1) }
        return v.toFloatArray()
    }

    /** Hemisphere z ≥ 0 (for capsule ends). */
    fun hemi(lat: Int = 9, lon: Int = 28): FloatArray {
        val v = ArrayList<Float>()
        fun p(i: Int, j: Int) {
            val th = PI / 2 * i / lat; val ph = 2 * PI * j / lon
            val x = (sin(th) * cos(ph)).toFloat(); val y = (sin(th) * sin(ph)).toFloat(); val z = cos(th).toFloat()
            v += listOf(x, y, z, x, y, z)
        }
        for (i in 0 until lat) for (j in 0 until lon) { p(i, j); p(i + 1, j); p(i + 1, j + 1); p(i, j); p(i + 1, j + 1); p(i, j + 1) }
        return v.toFloatArray()
    }

    /** Radius 1, z in [-1, 1]; [caps] adds flat ends. */
    fun cylinder(caps: Boolean, n: Int = 28): FloatArray {
        val v = ArrayList<Float>()
        for (j in 0 until n) {
            val a0 = 2 * PI * j / n; val a1 = 2 * PI * (j + 1) / n
            val x0 = cos(a0).toFloat(); val y0 = sin(a0).toFloat(); val x1 = cos(a1).toFloat(); val y1 = sin(a1).toFloat()
            v += listOf(x0, y0, -1f, x0, y0, 0f, x1, y1, -1f, x1, y1, 0f, x1, y1, 1f, x1, y1, 0f)
            v += listOf(x0, y0, -1f, x0, y0, 0f, x1, y1, 1f, x1, y1, 0f, x0, y0, 1f, x0, y0, 0f)
            if (caps) {
                v += listOf(0f, 0f, 1f, 0f, 0f, 1f, x0, y0, 1f, 0f, 0f, 1f, x1, y1, 1f, 0f, 0f, 1f)
                v += listOf(0f, 0f, -1f, 0f, 0f, -1f, x1, y1, -1f, 0f, 0f, -1f, x0, y0, -1f, 0f, 0f, -1f)
            }
        }
        return v.toFloatArray()
    }

    fun box(): FloatArray {
        val v = ArrayList<Float>()
        val faces = arrayOf(
            floatArrayOf(1f, 0f, 0f), floatArrayOf(-1f, 0f, 0f), floatArrayOf(0f, 1f, 0f),
            floatArrayOf(0f, -1f, 0f), floatArrayOf(0f, 0f, 1f), floatArrayOf(0f, 0f, -1f),
        )
        for (n in faces) {
            // two tangents
            val u = if (n[0] != 0f) floatArrayOf(0f, 1f, 0f) else floatArrayOf(1f, 0f, 0f)
            val w = floatArrayOf(n[1] * u[2] - n[2] * u[1], n[2] * u[0] - n[0] * u[2], n[0] * u[1] - n[1] * u[0])
            fun c(a: Float, b: Float) = listOf(n[0] + a * u[0] + b * w[0], n[1] + a * u[1] + b * w[1], n[2] + a * u[2] + b * w[2], n[0], n[1], n[2])
            v += c(-1f, -1f); v += c(1f, -1f); v += c(1f, 1f); v += c(-1f, -1f); v += c(1f, 1f); v += c(-1f, 1f)
        }
        return v.toFloatArray()
    }

    fun quad(): FloatArray = floatArrayOf(
        -1f, -1f, 0f, 0f, 0f, 1f, 1f, -1f, 0f, 0f, 0f, 1f, 1f, 1f, 0f, 0f, 0f, 1f,
        -1f, -1f, 0f, 0f, 0f, 1f, 1f, 1f, 0f, 0f, 0f, 1f, -1f, 1f, 0f, 0f, 0f, 1f,
    )
}

private const val VS = """#version 300 es
layout(location=0) in vec3 aPos;
layout(location=1) in vec3 aNrm;
uniform mat4 uMVP; uniform mat4 uModel; uniform mat3 uNrm;
out vec3 vN; out vec3 vW;
void main() { vec4 w = uModel * vec4(aPos, 1.0); vW = w.xyz; vN = normalize(uNrm * aNrm); gl_Position = uMVP * vec4(aPos, 1.0); }"""

private const val FS = """#version 300 es
precision highp float;
in vec3 vN; in vec3 vW;
uniform vec4 uColor; uniform vec3 uEye;
out vec4 o;
void main() {
  vec3 n = normalize(vN); vec3 v = normalize(uEye - vW);
  if (dot(n, v) < 0.0) n = -n;
  vec3 l1 = normalize(vec3(0.35, -0.5, 0.8)); vec3 l2 = normalize(vec3(-0.6, 0.4, 0.35));
  float d = max(dot(n, l1), 0.0) * 0.75 + max(dot(n, l2), 0.0) * 0.25;
  float s = pow(max(dot(reflect(-l1, n), v), 0.0), 32.0) * 0.35;
  float rim = pow(1.0 - max(dot(n, v), 0.0), 3.0) * 0.25;
  vec3 c = uColor.rgb * (0.28 + d) + vec3(s) + vec3(0.25, 0.82, 1.0) * rim;
  o = vec4(c, uColor.a);
}"""

private const val FLOOR_FS = """#version 300 es
precision highp float;
in vec3 vN; in vec3 vW;
uniform vec4 uColor; uniform vec3 uEye; uniform float uCell; uniform float uFade;
out vec4 o;
float grid(vec2 p, float cell, float w) { vec2 g = abs(fract(p / cell - 0.5) - 0.5) / fwidth(p / cell); return 1.0 - min(min(g.x, g.y) / w, 1.0); }
void main() {
  float minor = grid(vW.xy, uCell, 1.0);
  float major = grid(vW.xy, uCell * 5.0, 1.3);
  float dist = length(vW.xy - uEye.xy);
  float fade = clamp(1.0 - dist / uFade, 0.0, 1.0);
  vec3 base = uColor.rgb;
  vec3 c = mix(base, vec3(0.20, 0.25, 0.42), minor * 0.55 * fade);
  c = mix(c, vec3(0.30, 0.36, 0.70), major * 0.8 * fade);
  float ax = 1.0 - min(abs(vW.y) / fwidth(vW.y) / 1.5, 1.0); float ay = 1.0 - min(abs(vW.x) / fwidth(vW.x) / 1.5, 1.0);
  c = mix(c, vec3(0.86, 0.30, 0.42), ax * fade * 0.8); c = mix(c, vec3(0.30, 0.86, 0.50), ay * fade * 0.8);
  o = vec4(c, 1.0);
}"""

private const val SHADOW_FS = """#version 300 es
precision highp float;
in vec3 vN; in vec3 vW; uniform vec4 uColor; uniform vec3 uCenter; uniform float uRadius;
out vec4 o;
void main() { float r = length(vW.xy - uCenter.xy) / uRadius; o = vec4(0.0, 0.0, 0.0, uColor.a * smoothstep(1.0, 0.2, r)); }"""

private fun program(vs: String, fs: String): Int {
    fun sh(t: Int, src: String): Int {
        val s = glCreateShader(t); glShaderSource(s, src); glCompileShader(s)
        val ok = IntArray(1); glGetShaderiv(s, GL_COMPILE_STATUS, ok, 0)
        if (ok[0] == 0) throw RuntimeException("shader: " + glGetShaderInfoLog(s))
        return s
    }
    val p = glCreateProgram()
    glAttachShader(p, sh(GL_VERTEX_SHADER, vs)); glAttachShader(p, sh(GL_FRAGMENT_SHADER, fs)); glLinkProgram(p)
    val ok = IntArray(1); glGetProgramiv(p, GL_LINK_STATUS, ok, 0)
    if (ok[0] == 0) throw RuntimeException("link: " + glGetProgramInfoLog(p))
    return p
}

/** Live HUD numbers published by the GL thread. */
class SimHud { @Volatile var time = 0f; @Volatile var clients = 0; @Volatile var playing = false; @Volatile var geoms = 0; @Volatile var fps = 0f }

class SimRenderer(private val sim: SimManager, val camera: OrbitCamera, val hud: SimHud) : GLSurfaceView.Renderer {
    private var prog = 0; private var floorProg = 0; private var shadowProg = 0
    private lateinit var sphere: Mesh; private lateinit var hemi: Mesh; private lateinit var tube: Mesh
    private lateinit var cyl: Mesh; private lateinit var box: Mesh; private lateinit var quad: Mesh
    private val meshes = HashMap<Int, Mesh>()
    private var meshGen = -1
    private var framedGen = -1
    @Volatile var reframe = true
    @Volatile var shadows = true
    private val proj = FloatArray(16); private val view = FloatArray(16); private val vp = FloatArray(16)
    private val model = FloatArray(16); private val mvp = FloatArray(16); private val tmp = FloatArray(16)
    private val nrm = FloatArray(9)
    private var w = 1; private var h = 1
    private var lastT = System.nanoTime(); private var frames = 0

    override fun onSurfaceCreated(gl: GL10?, config: EGLConfig?) {
        prog = program(VS, FS); floorProg = program(VS, FLOOR_FS); shadowProg = program(VS, SHADOW_FS)
        sphere = upload(Prim.sphere()); hemi = upload(Prim.hemi()); tube = upload(Prim.cylinder(false))
        cyl = upload(Prim.cylinder(true)); box = upload(Prim.box()); quad = upload(Prim.quad())
        meshes.clear(); meshGen = -1
        glEnable(GL_DEPTH_TEST); glEnable(GL_BLEND); glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA)
    }

    override fun onSurfaceChanged(gl: GL10?, width: Int, height: Int) {
        w = max(width, 1); h = max(height, 1); glViewport(0, 0, w, h)
    }

    private fun setModel(p: Int, color: FloatArray, alpha: Float = color[3]) {
        Matrix.multiplyMM(mvp, 0, vp, 0, model, 0)
        glUniformMatrix4fv(glGetUniformLocation(p, "uMVP"), 1, false, mvp, 0)
        glUniformMatrix4fv(glGetUniformLocation(p, "uModel"), 1, false, model, 0)
        // normal matrix = inverse-transpose of the upper 3x3
        Matrix.invertM(tmp, 0, model, 0)
        nrm[0] = tmp[0]; nrm[1] = tmp[4]; nrm[2] = tmp[8]; nrm[3] = tmp[1]; nrm[4] = tmp[5]; nrm[5] = tmp[9]; nrm[6] = tmp[2]; nrm[7] = tmp[6]; nrm[8] = tmp[10]
        glUniformMatrix3fv(glGetUniformLocation(p, "uNrm"), 1, false, nrm, 0)
        glUniform4f(glGetUniformLocation(p, "uColor"), color[0], color[1], color[2], alpha)
    }

    private fun draw(m: Mesh) { glBindVertexArray(m.vao); glDrawArrays(GL_TRIANGLES, 0, m.count) }

    /** Model matrix from MuJoCo pos + row-major 3x3, then local scale/offset. */
    private fun place(f: FloatArray, o: Int, sx: Float, sy: Float, sz: Float, oz: Float = 0f) {
        val p = o + 5; val r = o + 8
        model[0] = f[r] * sx; model[1] = f[r + 3] * sx; model[2] = f[r + 6] * sx; model[3] = 0f
        model[4] = f[r + 1] * sy; model[5] = f[r + 4] * sy; model[6] = f[r + 7] * sy; model[7] = 0f
        model[8] = f[r + 2] * sz; model[9] = f[r + 5] * sz; model[10] = f[r + 8] * sz; model[11] = 0f
        model[12] = f[p] + f[r + 2] * oz; model[13] = f[p + 1] + f[r + 5] * oz; model[14] = f[p + 2] + f[r + 8] * oz; model[15] = 1f
    }

    override fun onDrawFrame(gl: GL10?) {
        val st = sim.state.value
        val m = st.model
        val handle = m?.handle ?: 0L
        if (sim.generation != meshGen) {
            meshes.values.forEach { it.delete() }; meshes.clear()
            if (m != null) for (i in 0 until m.meshes) TulpaNative.simMesh(handle, i)?.takeIf { it.isNotEmpty() }?.let { meshes[i] = upload(it) }
            meshGen = sim.generation
        }
        if (m != null && (framedGen != sim.generation || reframe)) { camera.frame(m.center, m.extent); framedGen = sim.generation; reframe = false }
        else if (m == null && reframe) { camera.frame(floatArrayOf(0f, 0f, 0.5f), 1.5f); reframe = false }

        glClearColor(0.027f, 0.039f, 0.078f, 1f)
        glClear(GL_COLOR_BUFFER_BIT or GL_DEPTH_BUFFER_BIT)
        val ext = camera.extent
        Matrix.perspectiveM(proj, 0, 45f, w.toFloat() / h, max(ext * 0.01f, 0.005f), ext * 200f)
        val eye = camera.eye()
        Matrix.setLookAtM(view, 0, eye[0], eye[1], eye[2], camera.tx, camera.ty, camera.tz, 0f, 0f, 1f)
        Matrix.multiplyMM(vp, 0, proj, 0, view, 0)

        val frame = if (handle != 0L) TulpaNative.simFrame(handle) else null
        val stride = 21
        val n = if (frame != null && frame.size >= 4) (frame.size - 4) / stride else 0

        // floor: every plane geom, or a default stage at z = 0
        glUseProgram(floorProg)
        glUniform3f(glGetUniformLocation(floorProg, "uEye"), eye[0], eye[1], eye[2])
        val cell = cellSize(ext)
        glUniform1f(glGetUniformLocation(floorProg, "uCell"), cell)
        glUniform1f(glGetUniformLocation(floorProg, "uFade"), max(ext * 12f, cell * 40f))
        var planes = 0
        for (g in 0 until n) {
            val o = 4 + g * stride
            if (frame!![o].toInt() != 0) continue
            planes++
            val sx = if (frame[o + 2] > 0f) frame[o + 2] else ext * 60f
            val sy = if (frame[o + 3] > 0f) frame[o + 3] else ext * 60f
            place(frame, o, sx, sy, 1f)
            setModel(floorProg, FLOOR); draw(quad)
        }
        if (planes == 0) {
            Matrix.setIdentityM(model, 0); Matrix.scaleM(model, 0, ext * 60f, ext * 60f, 1f)
            setModel(floorProg, FLOOR); draw(quad)
        }

        // soft contact shadows under bodies (drawn on the floor, before the geoms)
        if (shadows && n > 0) {
            glUseProgram(shadowProg); glDepthMask(false)
            for (g in 0 until n) {
                val o = 4 + g * stride
                val t = frame!![o].toInt()
                if (t == 0 || t == 1) continue
                val r = when (t) { 6 -> max(frame[o + 2], frame[o + 3]); 7 -> ext * 0.08f; else -> max(frame[o + 2], 0.01f) } * 1.6f
                val zH = frame[o + 7]
                if (zH < -0.01f || zH > ext * 3f) continue
                val a = (0.38f * (1f - zH / (ext * 3f))).coerceIn(0f, 0.38f)
                val rad = r + zH * 0.25f
                Matrix.setIdentityM(model, 0); Matrix.translateM(model, 0, frame[o + 5], frame[o + 6], 0.001f * ext + 0.0005f)
                Matrix.scaleM(model, 0, rad, rad, 1f)
                glUniform3f(glGetUniformLocation(shadowProg, "uCenter"), frame[o + 5], frame[o + 6], 0f)
                glUniform1f(glGetUniformLocation(shadowProg, "uRadius"), rad)
                setModel(shadowProg, BLACK, a); draw(quad)
            }
            glDepthMask(true)
        }

        glUseProgram(prog)
        glUniform3f(glGetUniformLocation(prog, "uEye"), eye[0], eye[1], eye[2])
        val col = FloatArray(4)
        for (g in 0 until n) {
            val o = 4 + g * stride
            val t = frame!![o].toInt()
            val sx = frame[o + 2]; val sy = frame[o + 3]; val sz = frame[o + 4]
            col[0] = frame[o + 17]; col[1] = frame[o + 18]; col[2] = frame[o + 19]; col[3] = frame[o + 20]
            when (t) {
                2 -> { place(frame, o, sx, sx, sx); setModel(prog, col); draw(sphere) }
                3 -> {            // capsule: radius sx, half-length sy
                    place(frame, o, sx, sx, sy); setModel(prog, col); draw(tube)
                    place(frame, o, sx, sx, sx, sy); setModel(prog, col); draw(hemi)
                    place(frame, o, sx, sx, -sx, -sy); setModel(prog, col); draw(hemi)
                }
                4 -> { place(frame, o, sx, sy, sz); setModel(prog, col); draw(sphere) }
                5 -> { place(frame, o, sx, sx, sy); setModel(prog, col); draw(cyl) }
                6 -> { place(frame, o, sx, sy, sz); setModel(prog, col); draw(box) }
                7 -> meshes[frame[o + 1].toInt()]?.let { place(frame, o, 1f, 1f, 1f); setModel(prog, col); draw(it) }
            }
        }
        glBindVertexArray(0)

        if (frame != null && frame.size >= 4) { hud.time = frame[0]; hud.clients = frame[2].toInt(); hud.playing = frame[3] > 0f }
        hud.geoms = n
        frames++
        val now = System.nanoTime()
        if (now - lastT > 1_000_000_000L) { hud.fps = frames * 1e9f / (now - lastT); frames = 0; lastT = now }
    }

    private fun cellSize(ext: Float): Float {
        val raw = ext / 4f
        val p = Math.pow(10.0, Math.floor(Math.log10(raw.toDouble()))).toFloat()
        return when { raw / p < 2f -> p; raw / p < 5f -> 2 * p; else -> 5 * p }
    }

    companion object {
        private val FLOOR = floatArrayOf(0.055f, 0.07f, 0.13f, 1f)
        private val BLACK = floatArrayOf(0f, 0f, 0f, 1f)
    }
}

/** GLSurfaceView with orbit (1 finger), pan (2 fingers), pinch zoom and double-tap reframe. */
@SuppressLint("ViewConstructor", "ClickableViewAccessibility")
class SimView(context: Context, sim: SimManager, val camera: OrbitCamera, val hud: SimHud) : GLSurfaceView(context) {
    val renderer = SimRenderer(sim, camera, hud)
    private var lastX = 0f; private var lastY = 0f; private var pointers = 0
    private val scale = ScaleGestureDetector(context, object : ScaleGestureDetector.SimpleOnScaleGestureListener() {
        override fun onScale(d: ScaleGestureDetector): Boolean { camera.zoom(d.scaleFactor); return true }
    })
    private val taps = GestureDetector(context, object : GestureDetector.SimpleOnGestureListener() {
        override fun onDoubleTap(e: MotionEvent): Boolean { renderer.reframe = true; return true }
    })

    init {
        setEGLContextClientVersion(3)
        setEGLConfigChooser(8, 8, 8, 8, 24, 0)
        preserveEGLContextOnPause = true
        setRenderer(renderer)
        renderMode = RENDERMODE_CONTINUOUSLY
    }

    override fun onTouchEvent(e: MotionEvent): Boolean {
        scale.onTouchEvent(e); taps.onTouchEvent(e)
        val cx = (0 until e.pointerCount).map { e.getX(it) }.average().toFloat()
        val cy = (0 until e.pointerCount).map { e.getY(it) }.average().toFloat()
        when (e.actionMasked) {
            MotionEvent.ACTION_DOWN, MotionEvent.ACTION_POINTER_DOWN, MotionEvent.ACTION_POINTER_UP -> { lastX = cx; lastY = cy; pointers = e.pointerCount }
            MotionEvent.ACTION_MOVE -> {
                if (e.pointerCount != pointers) { lastX = cx; lastY = cy; pointers = e.pointerCount }
                val dx = cx - lastX; val dy = cy - lastY
                if (e.pointerCount == 1 && !scale.isInProgress) camera.rotate(dx, dy)
                else if (e.pointerCount >= 2) camera.pan(dx, dy, height.toFloat())
                lastX = cx; lastY = cy
            }
        }
        parent?.requestDisallowInterceptTouchEvent(true)
        return true
    }

    @Suppress("unused") private fun clamp(v: Float, a: Float, b: Float) = min(max(v, a), b)
    @Suppress("unused") private fun len(x: Float, y: Float) = sqrt(x * x + y * y)
}

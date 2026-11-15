package dev.hermitech.tulpafield.core

import android.Manifest
import android.annotation.SuppressLint
import android.app.NotificationChannel
import android.app.NotificationManager
import android.content.Context
import android.content.pm.PackageManager
import android.hardware.camera2.CameraCharacteristics
import android.hardware.camera2.CameraManager
import android.media.AudioManager
import android.media.ToneGenerator
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.VibrationEffect
import android.os.Vibrator
import android.os.VibratorManager
import android.speech.tts.TextToSpeech
import android.widget.Toast
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.ContextCompat
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.update
import org.json.JSONObject
import java.util.Locale

/** A live value pushed by `phone::gauge(name, value, min, max)`. */
data class Gauge(val name: String, val value: Double, val unit: String, val min: Double?, val max: Double?)

/**
 * Receives `phone::` actuator calls from Rust (`action(kind, jsonPayload)`, called on the program
 * thread) and performs them with Android APIs.
 */
object TulpaHost {
    private lateinit var app: Context
    private val main = Handler(Looper.getMainLooper())
    private var tts: TextToSpeech? = null
    private var ttsReady = false
    private var tone: ToneGenerator? = null
    private var torchId: String? = null
    private var notifyId = 1000

    private val _gauges = MutableStateFlow<Map<String, Gauge>>(emptyMap())
    val gauges: StateFlow<Map<String, Gauge>> = _gauges
    private val _keepAwake = MutableStateFlow(false)
    val keepAwake: StateFlow<Boolean> = _keepAwake
    private val _torch = MutableStateFlow(false)
    val torch: StateFlow<Boolean> = _torch

    /** Actuator events shown in the Run console, e.g. "vibrate 200 ms". */
    var onEvent: ((String) -> Unit)? = null

    fun init(context: Context) {
        app = context.applicationContext
        tts = TextToSpeech(app) { status ->
            ttsReady = status == TextToSpeech.SUCCESS
            if (ttsReady) tts?.language = Locale.getDefault()
        }
        val cam = app.getSystemService(CameraManager::class.java)
        torchId = runCatching {
            cam.cameraIdList.firstOrNull { id ->
                cam.getCameraCharacteristics(id).get(CameraCharacteristics.FLASH_INFO_AVAILABLE) == true
            }
        }.getOrNull()
        val nm = app.getSystemService(NotificationManager::class.java)
        nm.createNotificationChannel(NotificationChannel("programs", "Program notifications", NotificationManager.IMPORTANCE_DEFAULT))
    }

    fun resetForRun() {
        _gauges.value = emptyMap()
    }

    /** Called when a program ends: release actuators it may have left on. */
    fun releaseAll() {
        if (_torch.value) setTorch(false)
        _keepAwake.value = false
        vibrator().cancel()
    }

    @JvmStatic
    fun action(kind: String, payload: String): String {
        val p = runCatching { JSONObject(payload) }.getOrDefault(JSONObject())
        return try {
            when (kind) {
                "vibrate" -> vibrate(p)
                "torch" -> setTorch(p.optBoolean("on", true))
                "say" -> say(p.optString("text"), p.optDouble("rate", 1.0))
                "beep" -> beep(p.optDouble("hz", 880.0), p.optInt("ms", 150))
                "toast" -> main.post { Toast.makeText(app, p.optString("text"), Toast.LENGTH_SHORT).show() }
                "notify" -> notify(p.optString("title"), p.optString("text"))
                "keep_awake" -> _keepAwake.value = p.optBoolean("on", true)
                "gauge" -> gauge(p)
            }
            ""
        } catch (t: Throwable) {
            onEvent?.invoke("phone::$kind failed: ${t.message}")
            ""
        }
    }

    private fun vibrator(): Vibrator =
        if (Build.VERSION.SDK_INT >= 31) app.getSystemService(VibratorManager::class.java).defaultVibrator
        else @Suppress("DEPRECATION") app.getSystemService(Vibrator::class.java)

    private fun vibrate(p: JSONObject) {
        val arr = p.optJSONArray("ms")
        val ms = (0 until (arr?.length() ?: 0)).map { arr!!.getLong(it).coerceIn(1, 10_000) }
        val amp = (p.optDouble("amplitude", 1.0) * 255).toInt().coerceIn(1, 255)
        val v = vibrator()
        if (ms.size <= 1) {
            v.vibrate(VibrationEffect.createOneShot(ms.firstOrNull() ?: 80L, amp))
        } else {
            // pattern: on, off, on, … → Android timings start with an initial delay of 0
            val timings = longArrayOf(0) + ms.toLongArray()
            val amps = IntArray(timings.size) { i -> if (i % 2 == 1) amp else 0 }
            v.vibrate(VibrationEffect.createWaveform(timings, amps, -1))
        }
        onEvent?.invoke("⟿ vibrate ${ms.joinToString("/")} ms")
    }

    private fun setTorch(on: Boolean) {
        val id = torchId ?: throw IllegalStateException("this phone has no flashlight")
        app.getSystemService(CameraManager::class.java).setTorchMode(id, on)
        if (_torch.value != on) onEvent?.invoke(if (on) "☀ torch on" else "☀ torch off")
        _torch.value = on
    }

    private fun say(text: String, rate: Double) {
        if (!ttsReady) throw IllegalStateException("text-to-speech is not ready")
        tts?.setSpeechRate(rate.toFloat())
        tts?.speak(text, TextToSpeech.QUEUE_ADD, null, "tulpa-${System.nanoTime()}")
        onEvent?.invoke("🗣 \"$text\"")
    }

    private fun beep(hz: Double, ms: Int) {
        // ToneGenerator has fixed tones; pick the closest DTMF-ish pitch band
        val t = tone ?: ToneGenerator(AudioManager.STREAM_MUSIC, 80).also { tone = it }
        val type = when {
            hz < 700 -> ToneGenerator.TONE_PROP_BEEP
            hz < 1100 -> ToneGenerator.TONE_CDMA_PIP
            else -> ToneGenerator.TONE_PROP_BEEP2
        }
        t.startTone(type, ms.coerceIn(20, 3000))
    }

    @SuppressLint("MissingPermission")
    private fun notify(title: String, text: String) {
        if (Build.VERSION.SDK_INT >= 33 &&
            ContextCompat.checkSelfPermission(app, Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
        ) {
            main.post { Toast.makeText(app, "$title: $text", Toast.LENGTH_LONG).show() }
            return
        }
        val n = NotificationCompat.Builder(app, "programs")
            .setSmallIcon(android.R.drawable.stat_notify_more)
            .setContentTitle(title.ifEmpty { "Tulpa Field" })
            .setContentText(text)
            .setAutoCancel(true)
            .build()
        NotificationManagerCompat.from(app).notify(notifyId++, n)
    }

    private fun gauge(p: JSONObject) {
        val g = Gauge(
            name = p.optString("name"),
            value = p.optDouble("value", Double.NaN),
            unit = p.optString("unit"),
            min = if (p.isNull("min")) null else p.optDouble("min"),
            max = if (p.isNull("max")) null else p.optDouble("max"),
        )
        _gauges.update { it + (g.name to g) }
    }
}

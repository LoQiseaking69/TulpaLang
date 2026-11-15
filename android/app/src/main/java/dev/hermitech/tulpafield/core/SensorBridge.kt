package dev.hermitech.tulpafield.core

import android.Manifest
import android.annotation.SuppressLint
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.PackageManager
import android.hardware.Sensor
import android.hardware.SensorEvent
import android.hardware.SensorEventListener
import android.hardware.SensorManager
import android.location.Location
import android.location.LocationListener
import android.location.LocationManager
import android.os.BatteryManager
import android.os.Looper
import androidx.core.content.ContextCompat
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow

/** Kinds shared with android/native/src/phone.rs. */
object SensorKind {
    const val ACCEL = 1; const val GYRO = 2; const val MAG = 3; const val ORIENTATION = 4
    const val LIGHT = 5; const val PRESSURE = 6; const val PROXIMITY = 7; const val BATTERY = 8
    const val LOCATION = 9; const val GRAVITY = 10; const val LINEAR = 11; const val STEPS = 12
}

data class SensorReading(val kind: Int, val values: FloatArray, val time: Long)

data class SensorInfo(val kind: Int, val title: String, val call: String, val unit: String, val available: Boolean, val vendor: String)

/**
 * Streams the handset's sensors into the Tulpa runtime (`phone::*`). Started while a program,
 * the REPL or the Sensors screen needs it; stopped otherwise to save battery.
 */
class SensorBridge(private val context: Context) : SensorEventListener {
    private val sm = context.getSystemService(SensorManager::class.java)
    private var users = 0
    private val rotation = FloatArray(9)
    private val orient = FloatArray(3)
    private var locationOn = false

    private val _latest = MutableStateFlow<Map<Int, SensorReading>>(emptyMap())
    /** Latest reading per kind, throttled to ~10 Hz for the UI. */
    val latest: StateFlow<Map<Int, SensorReading>> = _latest
    private val pending = HashMap<Int, SensorReading>()
    private var lastUi = 0L

    private val types = listOf(
        Sensor.TYPE_ACCELEROMETER to SensorKind.ACCEL,
        Sensor.TYPE_GYROSCOPE to SensorKind.GYRO,
        Sensor.TYPE_MAGNETIC_FIELD to SensorKind.MAG,
        Sensor.TYPE_ROTATION_VECTOR to SensorKind.ORIENTATION,
        Sensor.TYPE_GRAVITY to SensorKind.GRAVITY,
        Sensor.TYPE_LINEAR_ACCELERATION to SensorKind.LINEAR,
        Sensor.TYPE_LIGHT to SensorKind.LIGHT,
        Sensor.TYPE_PRESSURE to SensorKind.PRESSURE,
        Sensor.TYPE_PROXIMITY to SensorKind.PROXIMITY,
        Sensor.TYPE_STEP_COUNTER to SensorKind.STEPS,
    )

    fun catalog(): List<SensorInfo> {
        val meta = mapOf(
            SensorKind.ACCEL to Triple("Accelerometer", "phone::accel()", "m/s²"),
            SensorKind.GYRO to Triple("Gyroscope", "phone::gyro()", "rad/s"),
            SensorKind.MAG to Triple("Magnetometer", "phone::mag()", "µT"),
            SensorKind.ORIENTATION to Triple("Orientation (rotation vector)", "phone::orientation() · phone::heading()", "deg"),
            SensorKind.GRAVITY to Triple("Gravity", "phone::gravity() · phone::tilt()", "m/s²"),
            SensorKind.LINEAR to Triple("Linear acceleration", "phone::linear() · phone::shake()", "m/s²"),
            SensorKind.LIGHT to Triple("Light", "phone::light()", "lux"),
            SensorKind.PRESSURE to Triple("Barometer", "phone::pressure() · phone::altitude()", "hPa"),
            SensorKind.PROXIMITY to Triple("Proximity", "phone::proximity()", "cm"),
            SensorKind.STEPS to Triple("Step counter", "phone::steps()", "steps"),
        )
        val list = types.map { (t, k) ->
            val s = sm.getDefaultSensor(t)
            val (title, call, unit) = meta.getValue(k)
            SensorInfo(k, title, call, unit, s != null, s?.let { "${it.vendor} ${it.name}" } ?: "not present")
        }.toMutableList()
        list += SensorInfo(SensorKind.BATTERY, "Battery", "phone::battery() · phone::charging()", "0–1", true, "BatteryManager")
        list += SensorInfo(SensorKind.LOCATION, "Location (GPS)", "phone::location()", "deg / m", hasLocationPermission(), "LocationManager")
        return list
    }

    @Synchronized
    fun acquire() {
        users++
        if (users > 1) return
        for ((t, _) in types) {
            sm.getDefaultSensor(t)?.let { sm.registerListener(this, it, SensorManager.SENSOR_DELAY_GAME) }
        }
        context.registerReceiver(battery, IntentFilter(Intent.ACTION_BATTERY_CHANGED))?.let { battery.onReceive(context, it) }
        if (locationOn) startLocation()
    }

    @Synchronized
    fun release() {
        if (users == 0) return
        users--
        if (users > 0) return
        sm.unregisterListener(this)
        runCatching { context.unregisterReceiver(battery) }
        stopLocation()
    }

    fun hasLocationPermission() =
        ContextCompat.checkSelfPermission(context, Manifest.permission.ACCESS_FINE_LOCATION) == PackageManager.PERMISSION_GRANTED

    @Synchronized
    fun setLocationEnabled(on: Boolean) {
        locationOn = on
        if (users > 0) { if (on) startLocation() else stopLocation() }
    }

    val locationEnabled get() = locationOn

    @SuppressLint("MissingPermission")
    private fun startLocation() {
        if (!hasLocationPermission()) return
        val lm = context.getSystemService(LocationManager::class.java)
        for (p in listOf(LocationManager.GPS_PROVIDER, LocationManager.NETWORK_PROVIDER)) {
            if (lm.isProviderEnabled(p)) lm.requestLocationUpdates(p, 1000L, 0f, locationListener, Looper.getMainLooper())
        }
        lm.getLastKnownLocation(LocationManager.GPS_PROVIDER)?.let { locationListener.onLocationChanged(it) }
    }

    private fun stopLocation() {
        runCatching { context.getSystemService(LocationManager::class.java).removeUpdates(locationListener) }
    }

    private val locationListener = LocationListener { l: Location ->
        TulpaNative.pushLocation(l.latitude, l.longitude, l.altitude, l.accuracy.toDouble(), l.speed.toDouble(), l.bearing.toDouble())
        publish(SensorKind.LOCATION, floatArrayOf(l.latitude.toFloat(), l.longitude.toFloat(), l.altitude.toFloat(), l.accuracy, l.speed, l.bearing))
    }

    private val battery = object : BroadcastReceiver() {
        override fun onReceive(c: Context, i: Intent) {
            val level = i.getIntExtra(BatteryManager.EXTRA_LEVEL, -1)
            val scale = i.getIntExtra(BatteryManager.EXTRA_SCALE, 100)
            val status = i.getIntExtra(BatteryManager.EXTRA_STATUS, -1)
            val charging = status == BatteryManager.BATTERY_STATUS_CHARGING || status == BatteryManager.BATTERY_STATUS_FULL
            if (level >= 0) {
                val v = floatArrayOf(level.toFloat() / scale, if (charging) 1f else 0f)
                TulpaNative.pushSensor(SensorKind.BATTERY, v)
                publish(SensorKind.BATTERY, v)
            }
        }
    }

    override fun onSensorChanged(e: SensorEvent) {
        val kind = types.firstOrNull { it.first == e.sensor.type }?.second ?: return
        val values = if (kind == SensorKind.ORIENTATION) {
            SensorManager.getRotationMatrixFromVector(rotation, e.values)
            SensorManager.getOrientation(rotation, orient)
            orient.copyOf()        // azimuth, pitch, roll (rad)
        } else e.values.copyOf()
        TulpaNative.pushSensor(kind, values)
        publish(kind, values)
    }

    private fun publish(kind: Int, values: FloatArray) {
        val now = System.currentTimeMillis()
        synchronized(pending) {
            pending[kind] = SensorReading(kind, values, now)
            if (now - lastUi >= 100) {
                lastUi = now
                _latest.value = _latest.value + pending
                pending.clear()
            }
        }
    }

    override fun onAccuracyChanged(sensor: Sensor?, accuracy: Int) {}
}

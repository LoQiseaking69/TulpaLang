//! `phone::` — the handset as a Tulpa device.
//!
//! Sensors are pushed from Android (`SensorManager`, `LocationManager`, `BatteryManager`) into a
//! shared state; Tulpa reads the latest sample. Actuators (vibration, torch, speech, tones,
//! toasts, notifications) are forwarded to the app through [`set_action_handler`].
//!
//! ```tulpa
//! always phone::tilt() < 30[deg] else { phone::vibrate(200[ms]) }
//! every!(100[ms], { tlm::plot("az", phone::accel()[2]) })
//! ```

use std::rc::Rc;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;
use tulpa::units::{parse as unit, Quantity};
use tulpa::{Interpreter, Value};

#[derive(Default, Clone)]
pub struct Sensors {
    pub accel: Option<[f64; 3]>,
    pub gyro: Option<[f64; 3]>,
    pub mag: Option<[f64; 3]>,
    pub gravity: Option<[f64; 3]>,
    pub linear: Option<[f64; 3]>,
    /// azimuth (yaw), pitch, roll in radians, from the rotation-vector sensor
    pub orientation: Option<[f64; 3]>,
    pub light: Option<f64>,
    pub pressure_hpa: Option<f64>,
    pub proximity_cm: Option<f64>,
    pub battery: Option<f64>,
    pub charging: Option<bool>,
    /// lat, lon (deg), altitude (m), accuracy (m), speed (m/s), bearing (deg)
    pub location: Option<[f64; 6]>,
    pub steps: Option<f64>,
    pub updated: Option<Instant>,
}

/// Sensor kinds used by `pushSensor` (must match `SensorBridge.kt`).
pub mod kind {
    pub const ACCEL: i32 = 1;
    pub const GYRO: i32 = 2;
    pub const MAG: i32 = 3;
    pub const ORIENTATION: i32 = 4;
    pub const LIGHT: i32 = 5;
    pub const PRESSURE: i32 = 6;
    pub const PROXIMITY: i32 = 7;
    pub const BATTERY: i32 = 8;
    pub const LOCATION: i32 = 9;
    pub const GRAVITY: i32 = 10;
    pub const LINEAR: i32 = 11;
    pub const STEPS: i32 = 12;
}

pub fn state() -> &'static Mutex<Sensors> {
    static S: OnceLock<Mutex<Sensors>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(Sensors::default()))
}

/// f32 → f64 through its shortest decimal form, so 9.81f32 reads back as 9.81, not 9.8100004.
fn f(x: f32) -> f64 {
    let s = format!("{x}");
    s.parse().unwrap_or(x as f64)
}

fn v3(v: &[f32]) -> Option<[f64; 3]> {
    (v.len() >= 3).then(|| [f(v[0]), f(v[1]), f(v[2])])
}

/// Stores a sample from Android.
pub fn push(kind: i32, v: &[f32]) {
    let mut s = state().lock().unwrap_or_else(|e| e.into_inner());
    match kind {
        kind::ACCEL => s.accel = v3(v),
        kind::GYRO => s.gyro = v3(v),
        kind::MAG => s.mag = v3(v),
        kind::ORIENTATION => s.orientation = v3(v),
        kind::GRAVITY => s.gravity = v3(v),
        kind::LINEAR => s.linear = v3(v),
        kind::LIGHT => s.light = v.first().map(|x| f(*x)),
        kind::PRESSURE => s.pressure_hpa = v.first().map(|x| f(*x)),
        kind::PROXIMITY => s.proximity_cm = v.first().map(|x| f(*x)),
        kind::STEPS => s.steps = v.first().map(|x| f(*x)),
        kind::BATTERY => {
            s.battery = v.first().map(|x| f(*x));
            s.charging = v.get(1).map(|x| *x > 0.5);
        }
        kind::LOCATION if v.len() >= 6 => {
            s.location = Some([f(v[0]), f(v[1]), f(v[2]), f(v[3]), f(v[4]), f(v[5])])
        }
        _ => {}
    }
    s.updated = Some(Instant::now());
}

/// Location uses f64 for latitude/longitude precision.
pub fn push_location(v: [f64; 6]) {
    let mut s = state().lock().unwrap_or_else(|e| e.into_inner());
    s.location = Some(v);
    s.updated = Some(Instant::now());
}

pub fn reset() {
    *state().lock().unwrap_or_else(|e| e.into_inner()) = Sensors::default();
}

fn snapshot() -> Sensors {
    state().lock().unwrap_or_else(|e| e.into_inner()).clone()
}

// ---------------------------------------------------------------------------- actions

/// `(kind, payload) -> reply` — implemented by the app (JNI) or by tests.
pub type ActionHandler = Box<dyn Fn(&str, &str) -> String + Send + Sync>;

fn handler() -> &'static Mutex<Option<ActionHandler>> {
    static H: OnceLock<Mutex<Option<ActionHandler>>> = OnceLock::new();
    H.get_or_init(|| Mutex::new(None))
}

pub fn set_action_handler(h: ActionHandler) {
    *handler().lock().unwrap_or_else(|e| e.into_inner()) = Some(h);
}

fn action(kind: &str, payload: serde_json::Value) -> Result<String, String> {
    let h = handler().lock().unwrap_or_else(|e| e.into_inner());
    match h.as_ref() {
        Some(f) => Ok(f(kind, &payload.to_string())),
        None => Err(format!(
            "phone::{kind}: not available outside the Tulpa Field app"
        )),
    }
}

// ---------------------------------------------------------------------------- values

fn q(v: f64, u: &str) -> Value {
    match unit(u) {
        Ok(un) => Value::Quantity(Quantity::new(v, Rc::new(un))),
        Err(_) => Value::Float(v),
    }
}

fn vec3(v: Option<[f64; 3]>, u: Option<&str>, what: &str) -> Result<Value, String> {
    let v = v.ok_or_else(|| {
        format!("phone::{what}: this phone has no {what} sensor (or it isn't enabled in Sensors)")
    })?;
    Ok(Value::list(
        v.iter()
            .map(|x| match u {
                Some(u) => q(*x, u),
                None => Value::Float(*x),
            })
            .collect(),
    ))
}

fn secs_of(v: &Value) -> Option<f64> {
    match v {
        Value::Quantity(qq) => Some(qq.si),
        other => other.as_f64().map(|ms| ms / 1000.0), // bare numbers are milliseconds
    }
}

fn text(a: &[Value], i: usize) -> String {
    a.get(i).map(|v| v.to_string()).unwrap_or_default()
}

type I<'a> = &'a mut Interpreter;
type Res = Result<Value, String>;

fn accel(_: I, _: Vec<Value>) -> Res {
    vec3(snapshot().accel, Some("m/s^2"), "accel")
}
fn gyro(_: I, _: Vec<Value>) -> Res {
    vec3(snapshot().gyro, Some("rad/s"), "gyro")
}
fn mag(_: I, _: Vec<Value>) -> Res {
    vec3(snapshot().mag, None, "mag")
}
fn gravity(_: I, _: Vec<Value>) -> Res {
    vec3(snapshot().gravity, Some("m/s^2"), "gravity")
}
fn linear(_: I, _: Vec<Value>) -> Res {
    vec3(snapshot().linear, Some("m/s^2"), "linear")
}

fn orientation(_: I, _: Vec<Value>) -> Res {
    let o = snapshot()
        .orientation
        .ok_or("phone::orientation: no rotation-vector sensor")?;
    let mut d = tulpa::value::Dict::default();
    for (k, v) in [("yaw", o[0]), ("pitch", o[1]), ("roll", o[2])] {
        let _ = d.set(Value::str(k), q(v.to_degrees(), "deg"));
    }
    Ok(Value::dict(d))
}

/// Angle between the screen normal and vertical: 0° flat on a table, 90° upright.
fn tilt(_: I, _: Vec<Value>) -> Res {
    let s = snapshot();
    let g = s
        .gravity
        .or(s.accel)
        .ok_or("phone::tilt: no accelerometer")?;
    let n = (g[0] * g[0] + g[1] * g[1] + g[2] * g[2]).sqrt().max(1e-9);
    Ok(q((g[2] / n).clamp(-1.0, 1.0).acos().to_degrees(), "deg"))
}

/// Magnitude of linear acceleration (gravity removed) — a shake/impact meter.
fn shake(_: I, _: Vec<Value>) -> Res {
    let s = snapshot();
    let l = match (s.linear, s.accel) {
        (Some(l), _) => l,
        (None, Some(a)) => [a[0], a[1], a[2] - 9.80665],
        _ => return Err("phone::shake: no accelerometer".into()),
    };
    Ok(q((l[0] * l[0] + l[1] * l[1] + l[2] * l[2]).sqrt(), "m/s^2"))
}

fn heading(_: I, _: Vec<Value>) -> Res {
    let o = snapshot()
        .orientation
        .ok_or("phone::heading: no rotation-vector sensor")?;
    Ok(q((o[0].to_degrees() + 360.0) % 360.0, "deg"))
}

fn light(_: I, _: Vec<Value>) -> Res {
    snapshot()
        .light
        .map(Value::Float)
        .ok_or_else(|| "phone::light: no light sensor".into())
}
fn pressure(_: I, _: Vec<Value>) -> Res {
    snapshot()
        .pressure_hpa
        .map(|h| q(h * 100.0, "Pa"))
        .ok_or_else(|| "phone::pressure: no barometer".into())
}
/// Barometric altitude (international standard atmosphere), relative to 1013.25 hPa.
fn altitude(_: I, a: Vec<Value>) -> Res {
    let p = snapshot()
        .pressure_hpa
        .ok_or("phone::altitude: no barometer")?;
    let p0 = a
        .first()
        .and_then(|v| match v {
            Value::Quantity(x) => Some(x.si / 100.0),
            o => o.as_f64(),
        })
        .unwrap_or(1013.25);
    Ok(q(44330.0 * (1.0 - (p / p0).powf(1.0 / 5.255)), "m"))
}
fn proximity(_: I, _: Vec<Value>) -> Res {
    snapshot()
        .proximity_cm
        .map(|c| q(c, "cm"))
        .ok_or_else(|| "phone::proximity: no proximity sensor".into())
}
fn battery(_: I, _: Vec<Value>) -> Res {
    snapshot()
        .battery
        .map(Value::Float)
        .ok_or_else(|| "phone::battery: unknown".into())
}
fn charging(_: I, _: Vec<Value>) -> Res {
    Ok(Value::Bool(snapshot().charging.unwrap_or(false)))
}
fn steps(_: I, _: Vec<Value>) -> Res {
    snapshot()
        .steps
        .map(|s| Value::Int(s as i64))
        .ok_or_else(|| "phone::steps: no step counter (or permission denied)".into())
}

fn location(_: I, _: Vec<Value>) -> Res {
    let Some(l) = snapshot().location else {
        return Ok(Value::Nil);
    };
    let mut d = tulpa::value::Dict::default();
    let _ = d.set(Value::str("lat"), q(l[0], "deg"));
    let _ = d.set(Value::str("lon"), q(l[1], "deg"));
    let _ = d.set(Value::str("alt"), q(l[2], "m"));
    let _ = d.set(Value::str("accuracy"), q(l[3], "m"));
    let _ = d.set(Value::str("speed"), q(l[4], "m/s"));
    let _ = d.set(Value::str("bearing"), q(l[5], "deg"));
    Ok(Value::dict(d))
}

/// Every available reading in one dictionary (cheap way to log a full sample).
fn sample(i: I, _: Vec<Value>) -> Res {
    let mut d = tulpa::value::Dict::default();
    #[allow(clippy::type_complexity)]
    let tries: [(&str, fn(I, Vec<Value>) -> Res); 9] = [
        ("accel", accel),
        ("gyro", gyro),
        ("mag", mag),
        ("light", light),
        ("pressure", pressure),
        ("proximity", proximity),
        ("battery", battery),
        ("tilt", tilt),
        ("heading", heading),
    ];
    for (k, f) in tries {
        if let Ok(v) = f(i, vec![]) {
            let _ = d.set(Value::str(k), v);
        }
    }
    Ok(Value::dict(d))
}

fn has(_: I, a: Vec<Value>) -> Res {
    let s = snapshot();
    Ok(Value::Bool(match text(&a, 0).as_str() {
        "accel" => s.accel.is_some(),
        "gyro" => s.gyro.is_some(),
        "mag" => s.mag.is_some(),
        "orientation" | "heading" => s.orientation.is_some(),
        "light" => s.light.is_some(),
        "pressure" | "altitude" => s.pressure_hpa.is_some(),
        "proximity" => s.proximity_cm.is_some(),
        "location" => s.location.is_some(),
        "steps" => s.steps.is_some(),
        "battery" => s.battery.is_some(),
        _ => false,
    }))
}

fn vibrate(_: I, a: Vec<Value>) -> Res {
    // vibrate(200[ms])  or  vibrate([100[ms], 50[ms], 100[ms]]) for a pattern (on, off, on, …)
    let ms: Vec<i64> = match a.first() {
        Some(Value::List(l)) => l
            .borrow()
            .iter()
            .filter_map(secs_of)
            .map(|s| (s * 1000.0) as i64)
            .collect(),
        Some(v) => vec![(secs_of(v).ok_or("phone::vibrate(duration)")? * 1000.0) as i64],
        None => vec![80],
    };
    let amp = a
        .get(1)
        .and_then(|v| v.as_f64())
        .unwrap_or(1.0)
        .clamp(0.0, 1.0);
    action("vibrate", serde_json::json!({"ms": ms, "amplitude": amp}))?;
    Ok(Value::Nil)
}
fn torch(_: I, a: Vec<Value>) -> Res {
    action(
        "torch",
        serde_json::json!({"on": a.first().map(|v| v.truthy()).unwrap_or(true)}),
    )?;
    Ok(Value::Nil)
}
fn say(_: I, a: Vec<Value>) -> Res {
    let rate = a.get(1).and_then(|v| v.as_f64()).unwrap_or(1.0);
    action(
        "say",
        serde_json::json!({"text": text(&a, 0), "rate": rate}),
    )?;
    Ok(Value::Nil)
}
fn beep(_: I, a: Vec<Value>) -> Res {
    // beep(880[Hz], 150[ms])
    let hz = a
        .first()
        .and_then(|v| match v {
            Value::Quantity(x) => Some(x.si),
            o => o.as_f64(),
        })
        .unwrap_or(880.0);
    let s = a.get(1).and_then(secs_of).unwrap_or(0.15);
    action(
        "beep",
        serde_json::json!({"hz": hz, "ms": (s * 1000.0) as i64}),
    )?;
    Ok(Value::Nil)
}
fn toast(_: I, a: Vec<Value>) -> Res {
    action("toast", serde_json::json!({"text": text(&a, 0)}))?;
    Ok(Value::Nil)
}
fn notify(_: I, a: Vec<Value>) -> Res {
    action(
        "notify",
        serde_json::json!({"title": text(&a, 0), "text": text(&a, 1)}),
    )?;
    Ok(Value::Nil)
}
fn keep_awake(_: I, a: Vec<Value>) -> Res {
    action(
        "keep_awake",
        serde_json::json!({"on": a.first().map(|v| v.truthy()).unwrap_or(true)}),
    )?;
    Ok(Value::Nil)
}
/// Shows a big live value on the Run screen's gauge strip: gauge("speed", v, 0, 3[m/s]).
fn gauge(_: I, a: Vec<Value>) -> Res {
    let num = |v: Option<&Value>| {
        v.and_then(|v| match v {
            Value::Quantity(x) => Some(x.si),
            o => o.as_f64(),
        })
    };
    let unit = match a.get(1) {
        Some(Value::Quantity(x)) => x.unit_name(),
        _ => String::new(),
    };
    let display = match a.get(1) {
        Some(Value::Quantity(x)) => x.display_value(),
        Some(v) => v.as_f64().unwrap_or(f64::NAN),
        None => f64::NAN,
    };
    let scale = match a.get(1) {
        Some(Value::Quantity(x)) if x.si != 0.0 => x.display_value() / x.si,
        _ => 1.0,
    };
    action(
        "gauge",
        serde_json::json!({
            "name": text(&a, 0), "value": display, "unit": unit,
            "min": num(a.get(2)).map(|m| m * scale), "max": num(a.get(3)).map(|m| m * scale),
        }),
    )?;
    Ok(Value::Nil)
}

pub const FUNCTIONS: &[(&str, tulpa::value::NativeFn)] = &[
    ("accel", accel),
    ("gyro", gyro),
    ("mag", mag),
    ("gravity", gravity),
    ("linear", linear),
    ("orientation", orientation),
    ("tilt", tilt),
    ("shake", shake),
    ("heading", heading),
    ("light", light),
    ("pressure", pressure),
    ("altitude", altitude),
    ("proximity", proximity),
    ("battery", battery),
    ("charging", charging),
    ("steps", steps),
    ("location", location),
    ("sample", sample),
    ("has", has),
    ("vibrate", vibrate),
    ("torch", torch),
    ("say", say),
    ("beep", beep),
    ("toast", toast),
    ("notify", notify),
    ("keep_awake", keep_awake),
    ("gauge", gauge),
];

/// Registers `phone::*` as a Tulpa host module (idempotent).
pub fn register() {
    for (name, f) in FUNCTIONS {
        tulpa::host::register("phone", name, *f);
    }
}

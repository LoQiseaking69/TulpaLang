//! C ABI bridge for the Tulpa Field iOS IDE.
//!
//! Swift side: `ios/TulpaField/Sources/Core/TulpaNative.swift`. Every `tf_*` symbol mirrors one
//! function of the Android JNI bridge (`android/native/src/lib.rs`) and returns the same JSON, so
//! both apps share one runtime contract. The `phone::` and simulator modules are compiled from
//! the Android crate's sources verbatim.
//!
//! Memory contract:
//! * strings passed in are borrowed, NUL-terminated UTF-8 (NULL reads as "");
//! * strings returned are owned by the caller and must be released with `tf_string_free`;
//! * float buffers returned are released with `tf_floats_free(ptr, len)`;
//! * strings returned *by Swift* from the input callback must be `malloc`ed (e.g. `strdup`);
//!   Rust releases them with `free`.
//!
//! Threading contract (enforced on the Swift side):
//! * `tf_run`/`tf_tests`/`tf_verify` run on the calling thread, which must have a large stack
//!   (Swift starts a dedicated 64 MiB thread per run);
//! * a REPL session must only be used from the thread that created it.

#[path = "../../../android/native/src/phone.rs"]
pub mod phone;
#[path = "../../../android/native/src/sim.rs"]
pub mod sim;

use std::cell::RefCell;
use std::ffi::{c_char, c_void, CStr, CString};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Instant;
use tulpa::embed::{self, Feed, RunConfig};

/// `(ctx, text)` — receives batched program output.
pub type OutputCb = extern "C" fn(ctx: *mut c_void, text: *const c_char);
/// `(ctx, prompt) -> line` — returns a malloc'ed string, or NULL at end of input.
pub type InputCb = extern "C" fn(ctx: *mut c_void, prompt: *const c_char) -> *mut c_char;
/// `(kind, json_payload)` — a `phone::` actuator call or a co-sim notification.
pub type ActionCb = extern "C" fn(kind: *const c_char, payload: *const c_char);

extern "C" {
    fn free(p: *mut c_void);
}

static ACTION: OnceLock<ActionCb> = OnceLock::new();

// ---------------------------------------------------------------------------- helpers

fn s(p: *const c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
}

fn out(s: &str) -> *mut c_char {
    CString::new(s.replace('\0', "\u{FFFD}"))
        .map(CString::into_raw)
        .unwrap_or(std::ptr::null_mut())
}

fn out_opt(s: Option<String>) -> *mut c_char {
    s.map(|s| out(&s)).unwrap_or(std::ptr::null_mut())
}

fn floats(v: Vec<f32>, len: *mut i32) -> *mut f32 {
    if !len.is_null() {
        unsafe { *len = v.len() as i32 };
    }
    if v.is_empty() {
        return std::ptr::null_mut();
    }
    Box::into_raw(v.into_boxed_slice()) as *mut f32
}

fn cancel_flag(h: i64) -> Arc<AtomicBool> {
    if h == 0 {
        return Arc::default();
    }
    unsafe { (*(h as *const Arc<AtomicBool>)).clone() }
}

/// Output sink that batches writes (flush at 4 KiB or every 30 ms) and forwards them to Swift.
struct Sink {
    ctx: *mut c_void,
    out: Option<OutputCb>,
    input: Option<InputCb>,
    buf: RefCell<String>,
    last: RefCell<Instant>,
}

impl Sink {
    fn new(ctx: *mut c_void, out: Option<OutputCb>, input: Option<InputCb>) -> Rc<Sink> {
        Rc::new(Sink {
            ctx,
            out,
            input,
            buf: RefCell::new(String::new()),
            last: RefCell::new(Instant::now()),
        })
    }

    fn write(&self, s: &str) {
        let mut b = self.buf.borrow_mut();
        b.push_str(s);
        let due = b.len() > 4096 || self.last.borrow().elapsed().as_millis() >= 30;
        drop(b);
        if due {
            self.flush();
        }
    }

    fn flush(&self) {
        let text = std::mem::take(&mut *self.buf.borrow_mut());
        *self.last.borrow_mut() = Instant::now();
        if text.is_empty() {
            return;
        }
        if let (Some(cb), Ok(c)) = (self.out, CString::new(text.replace('\0', "\u{FFFD}"))) {
            cb(self.ctx, c.as_ptr());
        }
    }

    fn input(&self, prompt: &str) -> Option<String> {
        self.flush();
        let cb = self.input?;
        let p = CString::new(prompt.replace('\0', "")).ok()?;
        let r = cb(self.ctx, p.as_ptr());
        if r.is_null() {
            return None;
        }
        let line = unsafe { CStr::from_ptr(r) }.to_string_lossy().into_owned();
        unsafe { free(r as *mut c_void) };
        Some(line)
    }
}

/// Programs resolve relative file paths (write_file, VCDs, netlists) against the process working
/// directory. Point it at the project folder; one program runs at a time.
fn enter_dir(dir: &str) {
    if !dir.is_empty() {
        let _ = std::env::set_current_dir(dir);
    }
}

fn config(
    sink: &Rc<Sink>,
    base_dir: String,
    args: Vec<String>,
    virtual_time: bool,
    cancel: Arc<AtomicBool>,
) -> RunConfig {
    enter_dir(&base_dir);
    let s1 = sink.clone();
    let s2 = sink.clone();
    RunConfig {
        base_dir: base_dir.into(),
        args,
        virtual_time,
        output: Rc::new(move |t: &str| s1.write(t)),
        input: Some(Rc::new(move |p: &str| s2.input(p))),
        cancel,
    }
}

fn guard<T>(f: impl FnOnce() -> T) -> std::thread::Result<T> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f))
}

// ---------------------------------------------------------------------------- lifecycle

/// Must be called once before anything else.
#[no_mangle]
pub extern "C" fn tf_init(action: Option<ActionCb>) {
    // tlm:: emits `@@tlm {json}` lines that the Run screen turns into live charts
    std::env::set_var("TULPA_TELEMETRY", "1");
    phone::register();
    if let Some(cb) = action {
        let _ = ACTION.set(cb);
    }
    phone::set_action_handler(Box::new(|kind, payload| {
        if let (Some(cb), Ok(k), Ok(p)) = (ACTION.get(), CString::new(kind), CString::new(payload))
        {
            cb(k.as_ptr(), p.as_ptr());
        }
        String::new()
    }));
}

#[no_mangle]
pub extern "C" fn tf_string_free(p: *mut c_char) {
    if !p.is_null() {
        drop(unsafe { CString::from_raw(p) });
    }
}

#[no_mangle]
pub extern "C" fn tf_floats_free(p: *mut f32, len: i32) {
    if !p.is_null() && len > 0 {
        drop(unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(p, len as usize)) });
    }
}

#[no_mangle]
pub extern "C" fn tf_version() -> *mut c_char {
    out(embed::version())
}

#[no_mangle]
pub extern "C" fn tf_cancel_new() -> i64 {
    Box::into_raw(Box::new(Arc::new(AtomicBool::new(false)))) as i64
}

#[no_mangle]
pub extern "C" fn tf_cancel(h: i64) {
    if h != 0 {
        unsafe { (*(h as *const Arc<AtomicBool>)).store(true, Ordering::Relaxed) }
    }
}

#[no_mangle]
pub extern "C" fn tf_cancel_free(h: i64) {
    if h != 0 {
        drop(unsafe { Box::from_raw(h as *mut Arc<AtomicBool>) });
    }
}

#[no_mangle]
pub extern "C" fn tf_set_env(name: *const c_char, value: *const c_char) {
    let n = s(name);
    if n.is_empty() {
        return;
    }
    if value.is_null() {
        std::env::remove_var(&n);
    } else {
        std::env::set_var(&n, s(value));
    }
}

// ---------------------------------------------------------------------------- running

/// Runs a program to completion. Returns NULL on success or an owned error message.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub extern "C" fn tf_run(
    source: *const c_char,
    file: *const c_char,
    base_dir: *const c_char,
    argv: *const *const c_char,
    argc: i32,
    virtual_time: bool,
    ctx: *mut c_void,
    out_cb: Option<OutputCb>,
    in_cb: Option<InputCb>,
    cancel: i64,
) -> *mut c_char {
    let src = s(source);
    let file = s(file);
    let base = s(base_dir);
    let mut args = Vec::new();
    if !argv.is_null() {
        for k in 0..argc.max(0) as usize {
            args.push(s(unsafe { *argv.add(k) }));
        }
    }
    let sink = Sink::new(ctx, out_cb, in_cb);
    let cfg = config(&sink, base, args, virtual_time, cancel_flag(cancel));
    let r = guard(|| embed::run(&src, &file, cfg));
    sink.flush();
    match r {
        Ok(Ok(())) => std::ptr::null_mut(),
        Ok(Err(e)) => out(&e),
        Err(_) => out("internal error: the Tulpa runtime panicked"),
    }
}

/// Runs a file's tests; returns the JSON report or `{"error": ...}`.
#[no_mangle]
pub extern "C" fn tf_tests(
    source: *const c_char,
    file: *const c_char,
    base_dir: *const c_char,
    ctx: *mut c_void,
    out_cb: Option<OutputCb>,
    in_cb: Option<InputCb>,
    cancel: i64,
) -> *mut c_char {
    let src = s(source);
    let file = s(file);
    let base = s(base_dir);
    let sink = Sink::new(ctx, out_cb, in_cb);
    let cfg = config(&sink, base, vec![], false, cancel_flag(cancel));
    let r = guard(|| embed::tests(&src, &file, cfg));
    sink.flush();
    let j = match r {
        Ok(Ok(j)) => j,
        Ok(Err(e)) => serde_json::json!({ "error": e }).to_string(),
        Err(_) => "{\"error\":\"internal error: runtime panicked\"}".into(),
    };
    out(&j)
}

/// Checks contracts with randomized trials; returns the JSON report or `{"error": ...}`.
#[no_mangle]
pub extern "C" fn tf_verify(
    source: *const c_char,
    base_dir: *const c_char,
    trials: i32,
    cancel: i64,
) -> *mut c_char {
    let src = s(source);
    let base = s(base_dir);
    let r = guard(|| embed::verify(&src, base.into(), trials.max(1) as usize, cancel_flag(cancel)));
    let j = match r {
        Ok(Ok(j)) => j,
        Ok(Err(e)) => serde_json::json!({ "error": e }).to_string(),
        Err(_) => "{\"error\":\"internal error: runtime panicked\"}".into(),
    };
    out(&j)
}

// ---------------------------------------------------------------------------- REPL sessions

struct SessionBox {
    session: embed::Session,
    sink: Rc<Sink>,
    cancel: Arc<AtomicBool>,
}

#[no_mangle]
pub extern "C" fn tf_session_new(
    ctx: *mut c_void,
    out_cb: Option<OutputCb>,
    in_cb: Option<InputCb>,
    base_dir: *const c_char,
) -> i64 {
    let base = s(base_dir);
    enter_dir(&base);
    let sink = Sink::new(ctx, out_cb, in_cb);
    let cancel = Arc::new(AtomicBool::new(false));
    let (s1, s2) = (sink.clone(), sink.clone());
    let r = guard(|| {
        let mut session = embed::Session::new(
            Rc::new(move |t: &str| s1.write(t)),
            Some(Rc::new(move |p: &str| s2.input(p))),
            cancel.clone(),
        );
        session.set_base_dir(base.into());
        session
    });
    match r {
        Ok(session) => Box::into_raw(Box::new(SessionBox {
            session,
            sink,
            cancel,
        })) as i64,
        Err(_) => 0,
    }
}

/// Feeds one line; returns `{"kind":"more"|"value"|"error","text":...}`.
#[no_mangle]
pub extern "C" fn tf_session_feed(h: i64, line: *const c_char) -> *mut c_char {
    if h == 0 {
        return out("{\"kind\":\"error\",\"text\":\"no session\"}");
    }
    let line = s(line);
    let sb = unsafe { &mut *(h as *mut SessionBox) };
    sb.cancel.store(false, Ordering::Relaxed);
    let r = guard(|| sb.session.feed(&line));
    sb.sink.flush();
    let j = match r {
        Ok(Feed::More) => serde_json::json!({"kind": "more"}),
        Ok(Feed::Value(v)) => serde_json::json!({"kind": "value", "text": v}),
        Ok(Feed::Error(e)) => serde_json::json!({"kind": "error", "text": e}),
        Err(_) => serde_json::json!({"kind": "error", "text": "internal error: runtime panicked"}),
    };
    out(&j.to_string())
}

#[no_mangle]
pub extern "C" fn tf_session_interrupt(h: i64) {
    if h != 0 {
        unsafe { (*(h as *const SessionBox)).cancel.store(true, Ordering::Relaxed) }
    }
}

#[no_mangle]
pub extern "C" fn tf_session_cancel_entry(h: i64) {
    if h != 0 {
        unsafe { (*(h as *mut SessionBox)).session.cancel_entry() }
    }
}

#[no_mangle]
pub extern "C" fn tf_session_names(h: i64) -> *mut c_char {
    let names = if h == 0 {
        vec![]
    } else {
        unsafe { (*(h as *const SessionBox)).session.names() }
    };
    out(&serde_json::json!(names).to_string())
}

#[no_mangle]
pub extern "C" fn tf_session_free(h: i64) {
    if h != 0 {
        drop(unsafe { Box::from_raw(h as *mut SessionBox) });
    }
}

// ---------------------------------------------------------------------------- analysis

#[no_mangle]
pub extern "C" fn tf_diagnostics(src: *const c_char) -> *mut c_char {
    let s = s(src);
    out(&guard(|| embed::diagnostics(&s)).unwrap_or_else(|_| "[]".into()))
}

#[no_mangle]
pub extern "C" fn tf_completion(src: *const c_char, line: i32, col: i32) -> *mut c_char {
    let s = s(src);
    let r = guard(|| embed::completion(&s, line.max(0) as usize, col.max(0) as usize));
    out(&r.unwrap_or_else(|_| "[]".into()))
}

/// Returns NULL when there is nothing to show.
#[no_mangle]
pub extern "C" fn tf_hover(src: *const c_char, line: i32, col: i32) -> *mut c_char {
    let s = s(src);
    out_opt(guard(|| embed::hover(&s, line.max(0) as usize, col.max(0) as usize)).unwrap_or(None))
}

#[no_mangle]
pub extern "C" fn tf_symbols(src: *const c_char) -> *mut c_char {
    let s = s(src);
    out(&guard(|| embed::symbols(&s)).unwrap_or_else(|_| "[]".into()))
}

#[no_mangle]
pub extern "C" fn tf_definition(src: *const c_char, line: i32, col: i32) -> *mut c_char {
    let s = s(src);
    let r = guard(|| embed::definition(&s, line.max(0) as usize, col.max(0) as usize));
    out(&r.unwrap_or_else(|_| "null".into()))
}

#[no_mangle]
pub extern "C" fn tf_machines(src: *const c_char) -> *mut c_char {
    let s = s(src);
    out(&guard(|| embed::machines(&s)).unwrap_or_else(|_| "[]".into()))
}

#[no_mangle]
pub extern "C" fn tf_bytecode(src: *const c_char) -> *mut c_char {
    let s = s(src);
    out(&match guard(|| embed::bytecode(&s)) {
        Ok(Ok(t)) => t,
        Ok(Err(e)) => e,
        Err(_) => "internal error".into(),
    })
}

#[no_mangle]
pub extern "C" fn tf_modules() -> *mut c_char {
    out(&embed::modules())
}

// ---------------------------------------------------------------------------- sensors

/// Kinds match `phone::kind` (1 accel … 12 steps).
#[no_mangle]
pub extern "C" fn tf_push_sensor(kind: i32, values: *const f32, n: i32) {
    if values.is_null() || n <= 0 {
        return;
    }
    let v = unsafe { std::slice::from_raw_parts(values, (n as usize).min(16)) };
    phone::push(kind, v);
}

#[no_mangle]
pub extern "C" fn tf_push_location(
    lat: f64,
    lon: f64,
    alt: f64,
    accuracy: f64,
    speed: f64,
    bearing: f64,
) {
    phone::push_location([lat, lon, alt, accuracy, speed, bearing]);
}

#[no_mangle]
pub extern "C" fn tf_reset_sensors() {
    phone::reset();
}

// ---------------------------------------------------------------------------- simulator

/// Loads an MJCF model. Returns `{"handle": n}` or `{"error": "..."}`.
#[no_mangle]
pub extern "C" fn tf_sim_load(path: *const c_char) -> *mut c_char {
    let path = s(path);
    let r = match guard(|| sim::Sim::load(&path)) {
        Ok(Ok(sm)) => {
            sm.set_notify(Box::new(|method, params| {
                let _ = phone::action(
                    "sim_notify",
                    serde_json::json!({"method": method, "params": params}),
                );
            }));
            serde_json::json!({"handle": sim::insert(sm)})
        }
        Ok(Err(e)) => serde_json::json!({"error": e}),
        Err(_) => serde_json::json!({"error": "internal error: MuJoCo bridge panicked"}),
    };
    out(&r.to_string())
}

#[no_mangle]
pub extern "C" fn tf_sim_free(h: i64) {
    sim::remove(h as u64);
}

/// Model info JSON (plus `stat`, `nmesh`, `path`), or NULL for an unknown handle.
#[no_mangle]
pub extern "C" fn tf_sim_info(h: i64) -> *mut c_char {
    let r = sim::get(h as u64).map(|sm| {
        let m = sm.model.lock().unwrap_or_else(|e| e.into_inner());
        let mut i = m.info();
        i["stat"] = serde_json::json!(m.stat());
        i["nmesh"] = serde_json::json!(m.nmesh());
        i["path"] = serde_json::json!(m.path);
        i.to_string()
    });
    out_opt(r)
}

/// `[time, steps, clients, playing, then GEOM_STRIDE floats per visible geom]`.
#[no_mangle]
pub extern "C" fn tf_sim_frame(h: i64, len: *mut i32) -> *mut f32 {
    let Some(sm) = sim::get(h as u64) else {
        return floats(vec![], len);
    };
    let (t, geoms) = sm.frame();
    let mut v = Vec::with_capacity(geoms.len() + 4);
    v.push(t as f32);
    v.push(sm.steps.load(Ordering::Relaxed) as f32);
    v.push(sm.clients.load(Ordering::Relaxed) as f32);
    v.push(if sm.playing() { 1.0 } else { 0.0 });
    v.extend_from_slice(&geoms);
    floats(v, len)
}

/// Mesh triangles: 6 floats (position, normal) per vertex.
#[no_mangle]
pub extern "C" fn tf_sim_mesh(h: i64, id: i32, len: *mut i32) -> *mut f32 {
    let Some(sm) = sim::get(h as u64) else {
        return floats(vec![], len);
    };
    let v = sm
        .model
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .mesh(id.max(0) as usize);
    floats(v, len)
}

/// Starts the co-simulation server (idempotent); returns "host:port" or "error: ...".
#[no_mangle]
pub extern "C" fn tf_sim_serve(h: i64, lan: bool) -> *mut c_char {
    let r = match sim::get(h as u64) {
        Some(sm) => sm
            .serve(if lan { "0.0.0.0" } else { "127.0.0.1" })
            .unwrap_or_else(|e| format!("error: {e}")),
        None => "error: no model loaded".into(),
    };
    out(&r)
}

/// `play` | `pause` | `reset` | `realtime <factor>` | `push <deg>`; returns "" or an error.
#[no_mangle]
pub extern "C" fn tf_sim_control(h: i64, cmd: *const c_char, arg: f64) -> *mut c_char {
    let cmd = s(cmd);
    let Some(sm) = sim::get(h as u64) else {
        return out("no model loaded");
    };
    let r: Result<(), String> = match cmd.as_str() {
        "play" => {
            sm.set_playing(true);
            Ok(())
        }
        "pause" => {
            sm.set_playing(false);
            Ok(())
        }
        "reset" => {
            sm.reset();
            Ok(())
        }
        "realtime" => {
            sm.set_realtime(arg);
            Ok(())
        }
        "push" => {
            let m = sm.model.lock().unwrap_or_else(|e| e.into_inner());
            let mass_scale = m.info()["total_mass"].as_f64().unwrap_or(1.0).max(0.1);
            drop(m);
            let a = arg.to_radians();
            sm.push(
                None,
                [a.cos() * 15.0 * mass_scale, a.sin() * 15.0 * mass_scale, 0.0],
                0.15,
            )
        }
        other => Err(format!("unknown command {other}")),
    };
    out(&r.err().unwrap_or_default())
}

#[no_mangle]
pub extern "C" fn tf_sim_version() -> *mut c_char {
    out(&sim::version())
}

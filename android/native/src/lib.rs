//! JNI bridge for the Tulpa Field Android IDE.
//!
//! Kotlin side: `dev.hermitech.tulpafield.core.TulpaNative` (functions) and
//! `dev.hermitech.tulpafield.core.TulpaHost` (static `action(kind, payload): String` for
//! `phone::` actuators). Every exported symbol below mirrors one `external fun`.
//!
//! Threading contract (enforced on the Kotlin side):
//! * `run`/`tests`/`verify` run on the calling thread, which must have a large stack
//!   (Kotlin starts a dedicated 64 MiB thread per run).
//! * A REPL session must only be used from the thread that created it (one REPL thread).

pub mod phone;

use jni::objects::{GlobalRef, JClass, JFloatArray, JObject, JObjectArray, JString, JValue};
use jni::sys::{jboolean, jdouble, jint, jlong, jstring, JNI_VERSION_1_6};
use jni::{JNIEnv, JavaVM};
use std::cell::RefCell;
use std::ffi::c_void;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Instant;
use tulpa::embed::{self, Feed, RunConfig};

static VM: OnceLock<JavaVM> = OnceLock::new();
static HOST: OnceLock<GlobalRef> = OnceLock::new();

#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "system" fn JNI_OnLoad(vm: *mut jni::sys::JavaVM, _: *mut c_void) -> jint {
    if let Ok(vm) = unsafe { JavaVM::from_raw(vm) } {
        let _ = VM.set(vm);
    }
    // tlm:: emits `@@tlm {json}` lines that the Run screen turns into live charts
    std::env::set_var("TULPA_TELEMETRY", "1");
    phone::register();
    JNI_VERSION_1_6
}

// ---------------------------------------------------------------------------- helpers

fn jstr(env: &mut JNIEnv, s: &JString) -> String {
    if s.is_null() {
        return String::new();
    }
    env.get_string(s).map(|x| x.into()).unwrap_or_default()
}

fn out(env: &mut JNIEnv, s: &str) -> jstring {
    env.new_string(s)
        .map(|x| x.into_raw())
        .unwrap_or(std::ptr::null_mut())
}

fn out_opt(env: &mut JNIEnv, s: Option<String>) -> jstring {
    match s {
        Some(s) => out(env, &s),
        None => std::ptr::null_mut(),
    }
}

fn clear_exception(env: &mut JNIEnv) {
    if env.exception_check().unwrap_or(false) {
        let _ = env.exception_describe();
        let _ = env.exception_clear();
    }
}

fn cancel_flag(h: jlong) -> Arc<AtomicBool> {
    if h == 0 {
        return Arc::default();
    }
    unsafe { (*(h as *const Arc<AtomicBool>)).clone() }
}

/// Output sink that batches writes and forwards them to `listener.onOutput(String)`.
struct Sink {
    listener: GlobalRef,
    buf: RefCell<String>,
    last: RefCell<Instant>,
}

impl Sink {
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
        let Some(vm) = VM.get() else { return };
        let Ok(mut env) = vm.get_env() else { return };
        if let Ok(js) = env.new_string(&text) {
            let _ = env.call_method(
                self.listener.as_obj(),
                "onOutput",
                "(Ljava/lang/String;)V",
                &[JValue::Object(&js)],
            );
            let _ = env.delete_local_ref(js);
        }
        clear_exception(&mut env);
    }

    fn input(&self, prompt: &str) -> Option<String> {
        self.flush();
        let vm = VM.get()?;
        let mut env = vm.get_env().ok()?;
        let jp = env.new_string(prompt).ok()?;
        let r = env.call_method(
            self.listener.as_obj(),
            "onInput",
            "(Ljava/lang/String;)Ljava/lang/String;",
            &[JValue::Object(&jp)],
        );
        let _ = env.delete_local_ref(jp);
        match r.and_then(|v| v.l()) {
            Ok(obj) if !obj.is_null() => {
                let js = JString::from(obj);
                let s: Option<String> = env.get_string(&js).ok().map(|x| x.into());
                let _ = env.delete_local_ref(js);
                s
            }
            _ => {
                clear_exception(&mut env);
                None
            }
        }
    }
}

fn make_sink(env: &mut JNIEnv, listener: &JObject) -> Option<Rc<Sink>> {
    let g = env.new_global_ref(listener).ok()?;
    Some(Rc::new(Sink {
        listener: g,
        buf: RefCell::new(String::new()),
        last: RefCell::new(Instant::now()),
    }))
}

/// Programs resolve relative file paths (write_file, VCDs, netlists) against the process working
/// directory, which is `/` on Android. Point it at the project folder; one program runs at a time.
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

// ---------------------------------------------------------------------------- lifecycle

#[no_mangle]
pub extern "system" fn Java_dev_hermitech_tulpafield_core_TulpaNative_init(
    env: JNIEnv,
    _: JClass,
    host: JClass,
) {
    if let Ok(g) = env.new_global_ref(&host) {
        let _ = HOST.set(g);
    }
    phone::set_action_handler(Box::new(|kind, payload| {
        let (Some(vm), Some(host)) = (VM.get(), HOST.get()) else {
            return String::new();
        };
        let Ok(mut env) = vm.attach_current_thread_permanently() else {
            return String::new();
        };
        let reply = (|| -> Option<String> {
            let k = env.new_string(kind).ok()?;
            let p = env.new_string(payload).ok()?;
            let cls = JClass::from(env.new_local_ref(host.as_obj()).ok()?);
            let r = env
                .call_static_method(
                    &cls,
                    "action",
                    "(Ljava/lang/String;Ljava/lang/String;)Ljava/lang/String;",
                    &[JValue::Object(&k), JValue::Object(&p)],
                )
                .ok()?
                .l()
                .ok()?;
            if r.is_null() {
                return Some(String::new());
            }
            let js = JString::from(r);
            env.get_string(&js).ok().map(|x| x.into())
        })();
        clear_exception(&mut env);
        reply.unwrap_or_default()
    }));
}

#[no_mangle]
pub extern "system" fn Java_dev_hermitech_tulpafield_core_TulpaNative_version(
    mut env: JNIEnv,
    _: JClass,
) -> jstring {
    out(&mut env, embed::version())
}

#[no_mangle]
pub extern "system" fn Java_dev_hermitech_tulpafield_core_TulpaNative_newCancel(
    _: JNIEnv,
    _: JClass,
) -> jlong {
    Box::into_raw(Box::new(Arc::new(AtomicBool::new(false)))) as jlong
}

#[no_mangle]
pub extern "system" fn Java_dev_hermitech_tulpafield_core_TulpaNative_cancel(
    _: JNIEnv,
    _: JClass,
    h: jlong,
) {
    if h != 0 {
        unsafe { (*(h as *const Arc<AtomicBool>)).store(true, Ordering::Relaxed) }
    }
}

#[no_mangle]
pub extern "system" fn Java_dev_hermitech_tulpafield_core_TulpaNative_freeCancel(
    _: JNIEnv,
    _: JClass,
    h: jlong,
) {
    if h != 0 {
        drop(unsafe { Box::from_raw(h as *mut Arc<AtomicBool>) });
    }
}

// ---------------------------------------------------------------------------- running

#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn Java_dev_hermitech_tulpafield_core_TulpaNative_run(
    mut env: JNIEnv,
    _: JClass,
    source: JString,
    file: JString,
    base_dir: JString,
    args: JObjectArray,
    virtual_time: jboolean,
    listener: JObject,
    cancel: jlong,
) -> jstring {
    let src = jstr(&mut env, &source);
    let file = jstr(&mut env, &file);
    let base = jstr(&mut env, &base_dir);
    let mut argv = Vec::new();
    if !args.is_null() {
        let n = env.get_array_length(&args).unwrap_or(0);
        for k in 0..n {
            if let Ok(o) = env.get_object_array_element(&args, k) {
                argv.push(jstr(&mut env, &JString::from(o)));
            }
        }
    }
    let Some(sink) = make_sink(&mut env, &listener) else {
        return out(&mut env, "internal: cannot reference listener");
    };
    let cfg = config(&sink, base, argv, virtual_time != 0, cancel_flag(cancel));
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        embed::run(&src, &file, cfg)
    }));
    sink.flush();
    match r {
        Ok(Ok(())) => std::ptr::null_mut(),
        Ok(Err(e)) => out(&mut env, &e),
        Err(_) => out(&mut env, "internal error: the Tulpa runtime panicked"),
    }
}

#[no_mangle]
pub extern "system" fn Java_dev_hermitech_tulpafield_core_TulpaNative_tests(
    mut env: JNIEnv,
    _: JClass,
    source: JString,
    file: JString,
    base_dir: JString,
    listener: JObject,
    cancel: jlong,
) -> jstring {
    let src = jstr(&mut env, &source);
    let file = jstr(&mut env, &file);
    let base = jstr(&mut env, &base_dir);
    let Some(sink) = make_sink(&mut env, &listener) else {
        return out(&mut env, "{\"error\":\"listener\"}");
    };
    let cfg = config(&sink, base, vec![], false, cancel_flag(cancel));
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        embed::tests(&src, &file, cfg)
    }));
    sink.flush();
    let s = match r {
        Ok(Ok(j)) => j,
        Ok(Err(e)) => serde_json::json!({ "error": e }).to_string(),
        Err(_) => "{\"error\":\"internal error: runtime panicked\"}".into(),
    };
    out(&mut env, &s)
}

#[no_mangle]
pub extern "system" fn Java_dev_hermitech_tulpafield_core_TulpaNative_verify(
    mut env: JNIEnv,
    _: JClass,
    source: JString,
    base_dir: JString,
    trials: jint,
    cancel: jlong,
) -> jstring {
    let src = jstr(&mut env, &source);
    let base = jstr(&mut env, &base_dir);
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        embed::verify(
            &src,
            base.into(),
            trials.max(1) as usize,
            cancel_flag(cancel),
        )
    }));
    let s = match r {
        Ok(Ok(j)) => j,
        Ok(Err(e)) => serde_json::json!({ "error": e }).to_string(),
        Err(_) => "{\"error\":\"internal error: runtime panicked\"}".into(),
    };
    out(&mut env, &s)
}

// ---------------------------------------------------------------------------- REPL sessions

struct SessionBox {
    session: embed::Session,
    sink: Rc<Sink>,
    cancel: Arc<AtomicBool>,
}

#[no_mangle]
pub extern "system" fn Java_dev_hermitech_tulpafield_core_TulpaNative_sessionNew(
    mut env: JNIEnv,
    _: JClass,
    listener: JObject,
    base_dir: JString,
) -> jlong {
    let base = jstr(&mut env, &base_dir);
    enter_dir(&base);
    let Some(sink) = make_sink(&mut env, &listener) else {
        return 0;
    };
    let cancel = Arc::new(AtomicBool::new(false));
    let (s1, s2) = (sink.clone(), sink.clone());
    let mut session = embed::Session::new(
        Rc::new(move |t: &str| s1.write(t)),
        Some(Rc::new(move |p: &str| s2.input(p))),
        cancel.clone(),
    );
    session.set_base_dir(base.into());
    Box::into_raw(Box::new(SessionBox {
        session,
        sink,
        cancel,
    })) as jlong
}

#[no_mangle]
pub extern "system" fn Java_dev_hermitech_tulpafield_core_TulpaNative_sessionFeed(
    mut env: JNIEnv,
    _: JClass,
    h: jlong,
    line: JString,
) -> jstring {
    if h == 0 {
        return out(&mut env, "{\"kind\":\"error\",\"text\":\"no session\"}");
    }
    let line = jstr(&mut env, &line);
    let sb = unsafe { &mut *(h as *mut SessionBox) };
    sb.cancel.store(false, Ordering::Relaxed);
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| sb.session.feed(&line)));
    sb.sink.flush();
    let j = match r {
        Ok(Feed::More) => serde_json::json!({"kind": "more"}),
        Ok(Feed::Value(v)) => serde_json::json!({"kind": "value", "text": v}),
        Ok(Feed::Error(e)) => serde_json::json!({"kind": "error", "text": e}),
        Err(_) => serde_json::json!({"kind": "error", "text": "internal error: runtime panicked"}),
    };
    out(&mut env, &j.to_string())
}

#[no_mangle]
pub extern "system" fn Java_dev_hermitech_tulpafield_core_TulpaNative_sessionInterrupt(
    _: JNIEnv,
    _: JClass,
    h: jlong,
) {
    if h != 0 {
        unsafe {
            (*(h as *const SessionBox))
                .cancel
                .store(true, Ordering::Relaxed)
        }
    }
}

#[no_mangle]
pub extern "system" fn Java_dev_hermitech_tulpafield_core_TulpaNative_sessionCancelEntry(
    _: JNIEnv,
    _: JClass,
    h: jlong,
) {
    if h != 0 {
        unsafe { (*(h as *mut SessionBox)).session.cancel_entry() }
    }
}

#[no_mangle]
pub extern "system" fn Java_dev_hermitech_tulpafield_core_TulpaNative_sessionNames(
    mut env: JNIEnv,
    _: JClass,
    h: jlong,
) -> jstring {
    let names = if h == 0 {
        vec![]
    } else {
        unsafe { (*(h as *const SessionBox)).session.names() }
    };
    out(&mut env, &serde_json::json!(names).to_string())
}

#[no_mangle]
pub extern "system" fn Java_dev_hermitech_tulpafield_core_TulpaNative_sessionFree(
    _: JNIEnv,
    _: JClass,
    h: jlong,
) {
    if h != 0 {
        drop(unsafe { Box::from_raw(h as *mut SessionBox) });
    }
}

// ---------------------------------------------------------------------------- analysis

#[no_mangle]
pub extern "system" fn Java_dev_hermitech_tulpafield_core_TulpaNative_diagnostics(
    mut env: JNIEnv,
    _: JClass,
    src: JString,
) -> jstring {
    let s = jstr(&mut env, &src);
    let r = std::panic::catch_unwind(|| embed::diagnostics(&s)).unwrap_or_else(|_| "[]".into());
    out(&mut env, &r)
}

#[no_mangle]
pub extern "system" fn Java_dev_hermitech_tulpafield_core_TulpaNative_completion(
    mut env: JNIEnv,
    _: JClass,
    src: JString,
    line: jint,
    col: jint,
) -> jstring {
    let s = jstr(&mut env, &src);
    let r = std::panic::catch_unwind(|| {
        embed::completion(&s, line.max(0) as usize, col.max(0) as usize)
    })
    .unwrap_or_else(|_| "[]".into());
    out(&mut env, &r)
}

#[no_mangle]
pub extern "system" fn Java_dev_hermitech_tulpafield_core_TulpaNative_hover(
    mut env: JNIEnv,
    _: JClass,
    src: JString,
    line: jint,
    col: jint,
) -> jstring {
    let s = jstr(&mut env, &src);
    let r =
        std::panic::catch_unwind(|| embed::hover(&s, line.max(0) as usize, col.max(0) as usize))
            .unwrap_or(None);
    out_opt(&mut env, r)
}

#[no_mangle]
pub extern "system" fn Java_dev_hermitech_tulpafield_core_TulpaNative_symbols(
    mut env: JNIEnv,
    _: JClass,
    src: JString,
) -> jstring {
    let s = jstr(&mut env, &src);
    let r = std::panic::catch_unwind(|| embed::symbols(&s)).unwrap_or_else(|_| "[]".into());
    out(&mut env, &r)
}

#[no_mangle]
pub extern "system" fn Java_dev_hermitech_tulpafield_core_TulpaNative_definition(
    mut env: JNIEnv,
    _: JClass,
    src: JString,
    line: jint,
    col: jint,
) -> jstring {
    let s = jstr(&mut env, &src);
    let r = std::panic::catch_unwind(|| {
        embed::definition(&s, line.max(0) as usize, col.max(0) as usize)
    })
    .unwrap_or_else(|_| "null".into());
    out(&mut env, &r)
}

#[no_mangle]
pub extern "system" fn Java_dev_hermitech_tulpafield_core_TulpaNative_machines(
    mut env: JNIEnv,
    _: JClass,
    src: JString,
) -> jstring {
    let s = jstr(&mut env, &src);
    let r = std::panic::catch_unwind(|| embed::machines(&s)).unwrap_or_else(|_| "[]".into());
    out(&mut env, &r)
}

#[no_mangle]
pub extern "system" fn Java_dev_hermitech_tulpafield_core_TulpaNative_bytecode(
    mut env: JNIEnv,
    _: JClass,
    src: JString,
) -> jstring {
    let s = jstr(&mut env, &src);
    let r = match std::panic::catch_unwind(|| embed::bytecode(&s)) {
        Ok(Ok(t)) => t,
        Ok(Err(e)) => e,
        Err(_) => "internal error".into(),
    };
    out(&mut env, &r)
}

#[no_mangle]
pub extern "system" fn Java_dev_hermitech_tulpafield_core_TulpaNative_modules(
    mut env: JNIEnv,
    _: JClass,
) -> jstring {
    out(&mut env, &embed::modules())
}

// ---------------------------------------------------------------------------- sensors

#[no_mangle]
pub extern "system" fn Java_dev_hermitech_tulpafield_core_TulpaNative_pushSensor(
    env: JNIEnv,
    _: JClass,
    kind: jint,
    values: JFloatArray,
) {
    let n = env.get_array_length(&values).unwrap_or(0).max(0) as usize;
    let mut buf = vec![0f32; n.min(16)];
    if env.get_float_array_region(&values, 0, &mut buf).is_ok() {
        phone::push(kind, &buf);
    }
}

#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn Java_dev_hermitech_tulpafield_core_TulpaNative_pushLocation(
    _: JNIEnv,
    _: JClass,
    lat: jdouble,
    lon: jdouble,
    alt: jdouble,
    accuracy: jdouble,
    speed: jdouble,
    bearing: jdouble,
) {
    phone::push_location([lat, lon, alt, accuracy, speed, bearing]);
}

#[no_mangle]
pub extern "system" fn Java_dev_hermitech_tulpafield_core_TulpaNative_resetSensors(
    _: JNIEnv,
    _: JClass,
) {
    phone::reset();
}

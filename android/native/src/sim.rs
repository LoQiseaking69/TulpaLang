//! On-device physics: MuJoCo (through `csrc/hsim.c`) plus a co-simulation server that speaks
//! .RAV IDE's protocol, so `cosim::connect()` programs run unchanged on the phone.
//!
//! Protocol (one JSON object per line over TCP, identical to .RAV IDE's `cosim_server.py`):
//!   request `{"id", "method", "params"}` → `{"id", "result"}` | `{"id", "error"}`;
//!   notification `{"method", "params"}` (no reply). Methods: info, reset, observe, step,
//!   set_ctrl, set_state, apply_force.

use serde_json::{json, Map, Value as J};
use std::collections::HashMap;
use std::ffi::{c_char, c_double, c_float, c_int, c_long, c_void, CStr, CString};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

#[allow(dead_code)]
extern "C" {
    fn hs_load(path: *const c_char, err: *mut c_char, errsz: c_int) -> *mut c_void;
    fn hs_free(s: *mut c_void);
    fn hs_version() -> *const c_char;
    fn hs_count(s: *mut c_void, what: c_int) -> c_long;
    fn hs_name(s: *mut c_void, ty: c_int, id: c_int) -> *const c_char;
    fn hs_id(s: *mut c_void, ty: c_int, name: *const c_char) -> c_int;
    fn hs_dptr(s: *mut c_void, f: c_int) -> *mut c_double;
    fn hs_iptr(s: *mut c_void, f: c_int) -> *mut c_int;
    fn hs_ctrllimited(s: *mut c_void, i: c_int) -> c_int;
    fn hs_gear(s: *mut c_void, i: c_int) -> c_double;
    fn hs_timestep(s: *mut c_void) -> c_double;
    fn hs_gravity(s: *mut c_void, out: *mut c_double);
    fn hs_time(s: *mut c_void) -> c_double;
    fn hs_stat(s: *mut c_void, out: *mut c_double);
    fn hs_energy(s: *mut c_void, out: *mut c_double) -> c_int;
    fn hs_step(s: *mut c_void);
    fn hs_forward(s: *mut c_void);
    fn hs_subtree_vel(s: *mut c_void);
    fn hs_reset(s: *mut c_void, key: c_int);
    fn hs_contact(
        s: *mut c_void,
        i: c_int,
        b1: *mut c_int,
        b2: *mut c_int,
        f: *mut c_double,
        pos: *mut c_double,
    );
    fn hs_geom_rgba(s: *mut c_void, g: c_int, out: *mut c_float);
    fn hs_mesh_tris(s: *mut c_void, mesh: c_int, out: *mut c_float) -> c_long;
}

// counts
const NQ: c_int = 0;
const NV: c_int = 1;
const NU: c_int = 2;
const NBODY: c_int = 3;
const NJNT: c_int = 4;
const NSENSOR: c_int = 5;
const NKEY: c_int = 6;
const NGEOM: c_int = 7;
const NMESH: c_int = 8;
const NCON: c_int = 9;
// object types
const O_BODY: c_int = 1;
const O_JOINT: c_int = 3;
const O_ACTUATOR: c_int = 19;
const O_SENSOR: c_int = 22;
const O_KEY: c_int = 23;
// double arrays
const D_QPOS: c_int = 0;
const D_QVEL: c_int = 1;
const D_CTRL: c_int = 2;
const D_ACTFORCE: c_int = 3;
const D_SENSORDATA: c_int = 4;
const D_XPOS: c_int = 5;
const D_XQUAT: c_int = 6;
const D_SUBCOM: c_int = 7;
const D_SUBLINVEL: c_int = 8;
const D_CTRLRANGE: c_int = 9;
const D_JNTRANGE: c_int = 10;
const D_XFRC: c_int = 11;
const D_GEOMXPOS: c_int = 12;
const D_GEOMXMAT: c_int = 13;
const D_GEOMSIZE: c_int = 14;
const D_BODYMASS: c_int = 15;
// int arrays
const I_TRNID: c_int = 0;
const I_JNTTYPE: c_int = 1;
const I_JNTQPOSADR: c_int = 2;
const I_JNTDOFADR: c_int = 3;
const I_SENSTYPE: c_int = 4;
const I_SENSDIM: c_int = 5;
const I_SENSADR: c_int = 6;
const I_GEOMTYPE: c_int = 7;
const I_GEOMDATAID: c_int = 8;
const I_GEOMBODY: c_int = 9;
const I_GEOMGROUP: c_int = 11;

/// Floats per geom in [`Model::frame`]: type, mesh id, size×3, pos×3, mat×9, rgba×4.
pub const GEOM_STRIDE: usize = 21;

pub fn version() -> String {
    unsafe { CStr::from_ptr(hs_version()).to_string_lossy().into_owned() }
}

/// A loaded MuJoCo model + data. Not thread-safe on its own: wrap in a Mutex.
pub struct Model {
    s: *mut c_void,
    pub path: String,
}

unsafe impl Send for Model {}

impl Drop for Model {
    fn drop(&mut self) {
        unsafe { hs_free(self.s) }
    }
}

impl Model {
    pub fn load(path: &str) -> Result<Model, String> {
        let c = CString::new(path).map_err(|_| "bad path".to_string())?;
        let mut err = vec![0 as c_char; 1024];
        let s = unsafe { hs_load(c.as_ptr(), err.as_mut_ptr(), err.len() as c_int) };
        if s.is_null() {
            let msg = unsafe { CStr::from_ptr(err.as_ptr()) }
                .to_string_lossy()
                .into_owned();
            return Err(if msg.is_empty() {
                format!("cannot load {path}")
            } else {
                msg
            });
        }
        Ok(Model {
            s,
            path: path.to_string(),
        })
    }

    fn n(&self, what: c_int) -> usize {
        unsafe { hs_count(self.s, what) as usize }
    }
    fn d(&self, f: c_int, len: usize) -> &[f64] {
        let p = unsafe { hs_dptr(self.s, f) };
        if p.is_null() || len == 0 {
            &[]
        } else {
            unsafe { std::slice::from_raw_parts(p, len) }
        }
    }
    fn dm(&mut self, f: c_int, len: usize) -> &mut [f64] {
        let p = unsafe { hs_dptr(self.s, f) };
        if p.is_null() || len == 0 {
            &mut []
        } else {
            unsafe { std::slice::from_raw_parts_mut(p, len) }
        }
    }
    fn i(&self, f: c_int, len: usize) -> &[i32] {
        let p = unsafe { hs_iptr(self.s, f) };
        if p.is_null() || len == 0 {
            &[]
        } else {
            unsafe { std::slice::from_raw_parts(p, len) }
        }
    }
    fn name(&self, ty: c_int, id: usize, prefix: &str) -> String {
        let p = unsafe { hs_name(self.s, ty, id as c_int) };
        if p.is_null() {
            format!("{prefix}{id}")
        } else {
            let s = unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned();
            if s.is_empty() {
                format!("{prefix}{id}")
            } else {
                s
            }
        }
    }
    fn id(&self, ty: c_int, name: &str) -> Option<usize> {
        let c = CString::new(name).ok()?;
        let i = unsafe { hs_id(self.s, ty, c.as_ptr()) };
        (i >= 0).then_some(i as usize)
    }

    pub fn time(&self) -> f64 {
        unsafe { hs_time(self.s) }
    }
    pub fn timestep(&self) -> f64 {
        unsafe { hs_timestep(self.s) }
    }
    pub fn step(&mut self) {
        unsafe { hs_step(self.s) }
    }
    pub fn forward(&mut self) {
        unsafe { hs_forward(self.s) }
    }

    /// `[center x, y, z, extent]` for camera framing.
    pub fn stat(&self) -> [f64; 4] {
        let mut o = [0.0; 4];
        unsafe { hs_stat(self.s, o.as_mut_ptr()) };
        o
    }

    pub fn info(&self) -> J {
        let nu = self.n(NU);
        let njnt = self.n(NJNT);
        let trn = self.i(I_TRNID, nu * 2);
        let qadr = self.i(I_JNTQPOSADR, njnt);
        let dadr = self.i(I_JNTDOFADR, njnt);
        let jtype = self.i(I_JNTTYPE, njnt);
        let jrange = self.d(D_JNTRANGE, njnt * 2);
        let crange = self.d(D_CTRLRANGE, nu * 2);
        let actuators: Vec<J> = (0..nu)
            .map(|i| {
                let j = trn[2 * i];
                let jok = j >= 0 && (j as usize) < njnt;
                json!({
                    "name": self.name(O_ACTUATOR, i, "act"),
                    "joint": if jok { J::from(self.name(O_JOINT, j as usize, "joint")) } else { J::Null },
                    "ctrlrange": [crange[2 * i], crange[2 * i + 1]],
                    "gear": unsafe { hs_gear(self.s, i as c_int) },
                    "qposadr": if jok { J::from(qadr[j as usize]) } else { J::Null },
                    "dofadr": if jok { J::from(dadr[j as usize]) } else { J::Null },
                })
            })
            .collect();
        let joints: Vec<J> = (0..njnt)
            .map(|j| {
                json!({
                    "name": self.name(O_JOINT, j, "joint"),
                    "type": match jtype[j] { 0 => "free", 1 => "ball", 2 => "slide", 3 => "hinge", _ => "?" },
                    "range": [jrange[2 * j], jrange[2 * j + 1]],
                    "qposadr": qadr[j],
                    "dofadr": dadr[j],
                })
            })
            .collect();
        let ns = self.n(NSENSOR);
        let st = self.i(I_SENSTYPE, ns);
        let sd = self.i(I_SENSDIM, ns);
        let sensors: Vec<J> = (0..ns)
            .map(|s| json!({"name": self.name(O_SENSOR, s, "sensor"), "type": st[s], "dim": sd[s]}))
            .collect();
        let nb = self.n(NBODY);
        let mut g = [0.0; 3];
        unsafe { hs_gravity(self.s, g.as_mut_ptr()) };
        json!({
            "model": std::path::Path::new(&self.path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
            "timestep": self.timestep(),
            "gravity": g,
            "nq": self.n(NQ), "nv": self.n(NV), "nu": nu,
            "total_mass": self.d(D_BODYMASS, nb).iter().sum::<f64>(),
            "actuators": actuators, "joints": joints, "sensors": sensors,
            "bodies": (0..nb).map(|b| self.name(O_BODY, b, "body")).collect::<Vec<_>>(),
            "keyframes": (0..self.n(NKEY)).map(|k| self.name(O_KEY, k, "key")).collect::<Vec<_>>(),
        })
    }

    pub fn observe(&mut self) -> J {
        unsafe { hs_subtree_vel(self.s) };
        let (nq, nv, nu, nb, ns) = (
            self.n(NQ),
            self.n(NV),
            self.n(NU),
            self.n(NBODY),
            self.n(NSENSOR),
        );
        let sadr = self.i(I_SENSADR, ns).to_vec();
        let sdim = self.i(I_SENSDIM, ns).to_vec();
        let total: usize = sadr
            .iter()
            .zip(&sdim)
            .map(|(a, n)| (*a + *n) as usize)
            .max()
            .unwrap_or(0);
        let sdata = self.d(D_SENSORDATA, total).to_vec();
        let mut sensors = Map::new();
        for s in 0..ns {
            let a = sadr[s] as usize;
            sensors.insert(
                self.name(O_SENSOR, s, "sensor"),
                json!(sdata[a..a + sdim[s] as usize]),
            );
        }
        let mut contacts: HashMap<String, (f64, [f64; 3], usize)> = HashMap::new();
        for c in 0..self.n(NCON) {
            let (mut b1, mut b2, mut f) = (0, 0, 0.0);
            let mut pos = [0.0; 3];
            unsafe {
                hs_contact(
                    self.s,
                    c as c_int,
                    &mut b1,
                    &mut b2,
                    &mut f,
                    pos.as_mut_ptr(),
                )
            };
            if b1 != 0 && b2 != 0 {
                continue;
            }
            let body = if b1 == 0 { b2 } else { b1 } as usize;
            let e = contacts
                .entry(self.name(O_BODY, body, "body"))
                .or_insert((0.0, [0.0; 3], 0));
            e.0 += f;
            for (acc, p) in e.1.iter_mut().zip(pos) {
                *acc += f * p;
            }
            e.2 += 1;
        }
        let contacts: Map<String, J> = contacts
            .into_iter()
            .map(|(k, (f, mut cop, n))| {
                if f > 1e-9 {
                    for x in &mut cop {
                        *x /= f;
                    }
                }
                (k, json!({"force": f, "cop": cop, "n": n}))
            })
            .collect();
        let root = if nb > 1 { 1 } else { 0 };
        let xpos = self.d(D_XPOS, nb * 3);
        let xquat = self.d(D_XQUAT, nb * 4);
        let com = self.d(D_SUBCOM, nb * 3);
        let comv = self.d(D_SUBLINVEL, nb * 3);
        let mut bodies = Map::new();
        for b in 1..nb {
            bodies.insert(self.name(O_BODY, b, "body"), json!(xpos[3 * b..3 * b + 3]));
        }
        let mut en = [0.0; 2];
        let energy = if unsafe { hs_energy(self.s, en.as_mut_ptr()) } != 0 {
            json!(en)
        } else {
            J::Null
        };
        json!({
            "time": self.time(),
            "qpos": self.d(D_QPOS, nq),
            "qvel": self.d(D_QVEL, nv),
            "ctrl": self.d(D_CTRL, nu),
            "act_force": self.d(D_ACTFORCE, nu),
            "sensors": sensors,
            "com": com[3 * root..3 * root + 3],
            "com_vel": comv[3 * root..3 * root + 3],
            "root_pos": xpos[3 * root..3 * root + 3],
            "root_quat": xquat[4 * root..4 * root + 4],
            "bodies": bodies,
            "contacts": contacts,
            "energy": energy,
        })
    }

    pub fn reset(&mut self, key: &J) -> Result<(), String> {
        let k = match key {
            J::String(name) => {
                self.id(O_KEY, name)
                    .ok_or_else(|| format!("no keyframe named '{name}'"))? as c_int
            }
            J::Number(n) => n.as_i64().unwrap_or(0) as c_int,
            _ => {
                if self.n(NKEY) > 0 {
                    0
                } else {
                    -1
                }
            }
        };
        unsafe { hs_reset(self.s, k) };
        Ok(())
    }

    pub fn set_ctrl(&mut self, ctrl: &[f64]) -> Result<(), String> {
        let nu = self.n(NU);
        if ctrl.len() != nu {
            return Err(format!(
                "ctrl has {} values, model has {nu} actuators",
                ctrl.len()
            ));
        }
        if ctrl.iter().any(|x| !x.is_finite()) {
            return Err("ctrl contains NaN/inf".into());
        }
        let range = self.d(D_CTRLRANGE, nu * 2).to_vec();
        let lim: Vec<bool> = (0..nu)
            .map(|i| unsafe { hs_ctrllimited(self.s, i as c_int) } != 0)
            .collect();
        let c = self.dm(D_CTRL, nu);
        for i in 0..nu {
            c[i] = if lim[i] {
                ctrl[i].clamp(range[2 * i], range[2 * i + 1])
            } else {
                ctrl[i]
            };
        }
        Ok(())
    }

    pub fn set_state(&mut self, qpos: Option<&[f64]>, qvel: Option<&[f64]>) -> Result<(), String> {
        if let Some(q) = qpos {
            let n = self.n(NQ);
            if q.len() != n {
                return Err(format!("qpos has {} values, model has nq={n}", q.len()));
            }
            self.dm(D_QPOS, n).copy_from_slice(q);
        }
        if let Some(v) = qvel {
            let n = self.n(NV);
            if v.len() != n {
                return Err(format!("qvel has {} values, model has nv={n}", v.len()));
            }
            self.dm(D_QVEL, n).copy_from_slice(v);
        }
        self.forward();
        Ok(())
    }

    pub fn body(&self, name: &str) -> Result<usize, String> {
        self.id(O_BODY, name)
            .ok_or_else(|| format!("no body named '{name}'"))
    }

    pub fn set_xfrc(&mut self, body: usize, force: [f64; 3], torque: [f64; 3]) {
        let nb = self.n(NBODY);
        let x = self.dm(D_XFRC, nb * 6);
        x[6 * body..6 * body + 3].copy_from_slice(&force);
        x[6 * body + 3..6 * body + 6].copy_from_slice(&torque);
    }

    pub fn nbody(&self) -> usize {
        self.n(NBODY)
    }

    /// Visible geoms (group < 3, alpha > 0) as [`GEOM_STRIDE`] floats each.
    pub fn frame(&self) -> Vec<f32> {
        let ng = self.n(NGEOM);
        let ty = self.i(I_GEOMTYPE, ng);
        let data = self.i(I_GEOMDATAID, ng);
        let group = self.i(I_GEOMGROUP, ng);
        let size = self.d(D_GEOMSIZE, ng * 3);
        let pos = self.d(D_GEOMXPOS, ng * 3);
        let mat = self.d(D_GEOMXMAT, ng * 9);
        let mut out = Vec::with_capacity(ng * GEOM_STRIDE);
        let mut rgba = [0f32; 4];
        for g in 0..ng {
            unsafe { hs_geom_rgba(self.s, g as c_int, rgba.as_mut_ptr()) };
            if group[g] >= 3 || rgba[3] <= 0.0 {
                continue;
            }
            out.push(ty[g] as f32);
            out.push(data[g] as f32);
            out.extend(size[3 * g..3 * g + 3].iter().map(|&x| x as f32));
            out.extend(pos[3 * g..3 * g + 3].iter().map(|&x| x as f32));
            out.extend(mat[9 * g..9 * g + 9].iter().map(|&x| x as f32));
            out.extend_from_slice(&rgba);
        }
        out
    }

    pub fn nmesh(&self) -> usize {
        self.n(NMESH)
    }

    /// Mesh `id` as a triangle list (position + normal per vertex).
    pub fn mesh(&self, id: usize) -> Vec<f32> {
        if id >= self.nmesh() {
            return Vec::new();
        }
        let n = unsafe { hs_mesh_tris(self.s, id as c_int, std::ptr::null_mut()) } as usize;
        let mut v = vec![0f32; n];
        let w = unsafe { hs_mesh_tris(self.s, id as c_int, v.as_mut_ptr()) } as usize;
        v.truncate(w);
        v
    }

    #[allow(dead_code)]
    fn geom_body(&self, g: usize) -> usize {
        self.i(I_GEOMBODY, self.n(NGEOM))[g] as usize
    }
}

// ---------------------------------------------------------------------------- sessions

type Notify = Box<dyn Fn(&str, &J) + Send + Sync>;

struct Push {
    body: usize,
    force: [f64; 3],
    until: f64,
}

/// A model shared by the renderer, the free-running player and co-simulation clients.
pub struct Sim {
    pub model: Mutex<Model>,
    /// 0 = as fast as possible, 1 = real time, 2 = 2× …
    realtime: Mutex<f64>,
    pace: Mutex<Option<(Instant, f64)>>,
    pushes: Mutex<Vec<Push>>,
    pub steps: AtomicU64,
    pub clients: AtomicUsize,
    playing: AtomicBool,
    play_cv: (Mutex<()>, Condvar),
    stop: AtomicBool,
    notify: Mutex<Option<Notify>>,
    server: Mutex<Option<(String, Arc<AtomicBool>)>>,
}

impl Sim {
    pub fn load(path: &str) -> Result<Arc<Sim>, String> {
        let m = Model::load(path)?;
        let sim = Arc::new(Sim {
            model: Mutex::new(m),
            realtime: Mutex::new(1.0),
            pace: Mutex::new(None),
            pushes: Mutex::new(Vec::new()),
            steps: AtomicU64::new(0),
            clients: AtomicUsize::new(0),
            playing: AtomicBool::new(false),
            play_cv: (Mutex::new(()), Condvar::new()),
            stop: AtomicBool::new(false),
            notify: Mutex::new(None),
            server: Mutex::new(None),
        });
        let s2 = Arc::clone(&sim);
        thread::Builder::new()
            .name("sim-player".into())
            .spawn(move || s2.player())
            .ok();
        Ok(sim)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Model> {
        self.model.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn set_realtime(&self, r: f64) {
        *self.realtime.lock().unwrap() = r.max(0.0);
        *self.pace.lock().unwrap() = None;
    }

    pub fn set_notify(&self, f: Notify) {
        *self.notify.lock().unwrap() = Some(f);
    }

    /// Free-run the physics in real time (when no co-simulation client is driving it).
    pub fn set_playing(&self, on: bool) {
        self.playing.store(on, Ordering::SeqCst);
        *self.pace.lock().unwrap() = None;
        self.play_cv.1.notify_all();
    }

    pub fn playing(&self) -> bool {
        self.playing.load(Ordering::SeqCst)
    }

    fn player(self: Arc<Self>) {
        while !self.stop.load(Ordering::SeqCst) {
            if !self.playing() || self.clients.load(Ordering::SeqCst) > 0 {
                let g = self.play_cv.0.lock().unwrap();
                let _ = self.play_cv.1.wait_timeout(g, Duration::from_millis(100));
                continue;
            }
            let (dt, t) = {
                let mut m = self.lock();
                self.apply_pushes(&mut m);
                for _ in 0..4 {
                    m.step();
                }
                (m.timestep() * 4.0, m.time())
            };
            self.steps.fetch_add(4, Ordering::Relaxed);
            let _ = (dt, t);
            self.pace_wall(t, 1.0);
        }
    }

    fn pace_wall(&self, sim_t: f64, factor: f64) {
        if factor <= 0.0 {
            return;
        }
        let mut p = self.pace.lock().unwrap();
        let now = Instant::now();
        match *p {
            None => *p = Some((now, sim_t)),
            Some((w0, s0)) => {
                if sim_t < s0 {
                    *p = Some((now, sim_t));
                    return;
                }
                let target = w0 + Duration::from_secs_f64(((sim_t - s0) / factor).max(0.0));
                drop(p);
                if target > now {
                    thread::sleep((target - now).min(Duration::from_millis(250)));
                }
            }
        }
    }

    /// Schedule a push on `body` (or the root body) for `duration` seconds of sim time.
    pub fn push(&self, body: Option<&str>, force: [f64; 3], duration: f64) -> Result<(), String> {
        let m = self.lock();
        let b = match body {
            Some(n) if !n.is_empty() => m.body(n)?,
            _ => {
                if m.nbody() > 1 {
                    1
                } else {
                    return Err("model has no bodies".into());
                }
            }
        };
        let until = m.time() + duration;
        drop(m);
        self.pushes.lock().unwrap().push(Push {
            body: b,
            force,
            until,
        });
        Ok(())
    }

    fn apply_pushes(&self, m: &mut Model) {
        let mut p = self.pushes.lock().unwrap();
        if p.is_empty() {
            return;
        }
        let t = m.time();
        p.retain(|x| {
            if t < x.until {
                m.set_xfrc(x.body, x.force, [0.0; 3]);
                true
            } else {
                m.set_xfrc(x.body, [0.0; 3], [0.0; 3]);
                false
            }
        });
    }

    pub fn reset(&self) {
        let mut m = self.lock();
        let _ = m.reset(&J::Null);
        self.pushes.lock().unwrap().clear();
        *self.pace.lock().unwrap() = None;
    }

    pub fn frame(&self) -> (f64, Vec<f32>) {
        let m = self.lock();
        (m.time(), m.frame())
    }

    pub fn handle(&self, method: &str, params: &J) -> Result<J, String> {
        let p = params.as_object().cloned().unwrap_or_default();
        match method {
            "info" => Ok(self.lock().info()),
            "reset" => {
                let mut m = self.lock();
                m.reset(p.get("keyframe").unwrap_or(&J::Null))?;
                self.pushes.lock().unwrap().clear();
                *self.pace.lock().unwrap() = None;
                Ok(m.observe())
            }
            "observe" => Ok(self.lock().observe()),
            "step" | "set_ctrl" => {
                if let Some(c) = p.get("ctrl").filter(|c| !c.is_null()) {
                    let ctrl = floats(c, "ctrl")?;
                    self.lock().set_ctrl(&ctrl)?;
                }
                if method == "set_ctrl" {
                    return Ok(J::Null);
                }
                let n = p
                    .get("substeps")
                    .and_then(|x| x.as_u64())
                    .unwrap_or(1)
                    .max(1);
                let rt = *self.realtime.lock().unwrap();
                for _ in 0..n {
                    let t = {
                        let mut m = self.lock();
                        self.apply_pushes(&mut m);
                        m.step();
                        m.time()
                    };
                    self.steps.fetch_add(1, Ordering::Relaxed);
                    self.pace_wall(t, rt);
                }
                Ok(self.lock().observe())
            }
            "set_state" => {
                let q = p
                    .get("qpos")
                    .filter(|x| !x.is_null())
                    .map(|x| floats(x, "qpos"))
                    .transpose()?;
                let v = p
                    .get("qvel")
                    .filter(|x| !x.is_null())
                    .map(|x| floats(x, "qvel"))
                    .transpose()?;
                let mut m = self.lock();
                m.set_state(q.as_deref(), v.as_deref())?;
                Ok(m.observe())
            }
            "apply_force" => {
                let name = p.get("body").and_then(|b| b.as_str()).unwrap_or("");
                let f = vec3(p.get("force"))?;
                let t = vec3(p.get("torque"))?;
                let mut m = self.lock();
                let b = m.body(name)?;
                m.set_xfrc(b, f, t);
                Ok(J::Null)
            }
            _ => Err(format!("unknown method '{method}'")),
        }
    }

    /// Start (or return) the TCP co-simulation server; returns `host:port`.
    pub fn serve(self: &Arc<Self>, host: &str) -> Result<String, String> {
        let mut srv = self.server.lock().unwrap();
        if let Some((addr, _)) = srv.as_ref() {
            return Ok(addr.clone());
        }
        let l =
            TcpListener::bind((host, 0)).map_err(|e| format!("cannot open co-sim port: {e}"))?;
        let port = l.local_addr().map_err(|e| e.to_string())?.port();
        let addr = format!(
            "{}:{port}",
            if host == "0.0.0.0" { "127.0.0.1" } else { host }
        );
        let stop = Arc::new(AtomicBool::new(false));
        let me = Arc::clone(self);
        let st = Arc::clone(&stop);
        l.set_nonblocking(true).ok();
        thread::Builder::new()
            .name("cosim-accept".into())
            .spawn(move || {
                while !st.load(Ordering::SeqCst) {
                    match l.accept() {
                        Ok((sock, _)) => {
                            let me2 = Arc::clone(&me);
                            thread::spawn(move || me2.client(sock));
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(40))
                        }
                        Err(_) => thread::sleep(Duration::from_millis(100)),
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        *srv = Some((addr.clone(), stop));
        Ok(addr)
    }

    pub fn address(&self) -> Option<String> {
        self.server.lock().unwrap().as_ref().map(|(a, _)| a.clone())
    }

    fn client(&self, sock: TcpStream) {
        sock.set_nonblocking(false).ok();
        sock.set_nodelay(true).ok();
        self.clients.fetch_add(1, Ordering::SeqCst);
        *self.pace.lock().unwrap() = None;
        let mut w = match sock.try_clone() {
            Ok(w) => w,
            Err(_) => return,
        };
        let r = BufReader::new(sock);
        for line in r.lines() {
            let Ok(line) = line else { break };
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let msg: J = match serde_json::from_str(line) {
                Ok(m) => m,
                Err(e) => {
                    let _ = writeln!(
                        w,
                        "{}",
                        json!({"id": null, "error": format!("bad JSON: {e}")})
                    );
                    continue;
                }
            };
            let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");
            let params = msg.get("params").cloned().unwrap_or(J::Null);
            match msg.get("id") {
                None => {
                    if method == "bye" {
                        break;
                    }
                    if let Some(f) = self.notify.lock().unwrap().as_ref() {
                        f(method, &params);
                    }
                }
                Some(id) => {
                    let reply = match self.handle(method, &params) {
                        Ok(r) => json!({"id": id, "result": r}),
                        Err(e) => json!({"id": id, "error": e}),
                    };
                    if writeln!(w, "{reply}").is_err() {
                        break;
                    }
                }
            }
        }
        self.clients.fetch_sub(1, Ordering::SeqCst);
        self.play_cv.1.notify_all();
    }

    pub fn shutdown(&self) {
        self.stop.store(true, Ordering::SeqCst);
        self.play_cv.1.notify_all();
        if let Some((_, s)) = self.server.lock().unwrap().take() {
            s.store(true, Ordering::SeqCst);
        }
    }
}

fn floats(v: &J, what: &str) -> Result<Vec<f64>, String> {
    v.as_array()
        .ok_or_else(|| format!("{what} must be a list of numbers"))?
        .iter()
        .map(|x| {
            x.as_f64()
                .ok_or_else(|| format!("{what} must be a list of numbers"))
        })
        .collect()
}

fn vec3(v: Option<&J>) -> Result<[f64; 3], String> {
    match v {
        None | Some(J::Null) => Ok([0.0; 3]),
        Some(x) => {
            let f = floats(x, "vector")?;
            if f.len() != 3 {
                return Err("expected a 3-vector".into());
            }
            Ok([f[0], f[1], f[2]])
        }
    }
}

// ---------------------------------------------------------------------------- registry

pub fn registry() -> &'static Mutex<HashMap<u64, Arc<Sim>>> {
    static R: OnceLock<Mutex<HashMap<u64, Arc<Sim>>>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn get(h: u64) -> Option<Arc<Sim>> {
    registry().lock().unwrap().get(&h).cloned()
}

pub fn insert(sim: Arc<Sim>) -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let h = NEXT.fetch_add(1, Ordering::Relaxed);
    registry().lock().unwrap().insert(h, sim);
    h
}

pub fn remove(h: u64) {
    if let Some(s) = registry().lock().unwrap().remove(&h) {
        s.shutdown();
    }
}

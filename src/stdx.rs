//! Extended standard library: `pack::` (binary structs, CRC, COBS), `json::`, `tlm::`
//! (IDE telemetry) and `cosim::` (co-simulation client for HermiSim / any JSON-lines
//! physics server).

use crate::interp::Interpreter;
use crate::value::{Dict, Native, NativeFn, Value};
use serde_json::{json, Map, Number, Value as J};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;

type Res = Result<Value, String>;

pub fn path(m: &str, name: &str) -> Option<Value> {
    let table: &[(&str, NativeFn)] = match m {
        "pack" => PACK,
        "json" => JSON,
        "tlm" => TLM,
        "cosim" => COSIM,
        _ => return None,
    };
    table
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(n, f)| Value::Builtin(n, *f))
}

fn s_arg(a: &[Value], i: usize, what: &str) -> Result<String, String> {
    match a.get(i) {
        Some(Value::Str(s)) => Ok(s.to_string()),
        _ => Err(format!("{what}: argument {} must be a string", i + 1)),
    }
}

fn bytes_arg(a: &[Value], i: usize, what: &str) -> Result<Vec<u8>, String> {
    a.get(i)
        .ok_or_else(|| format!("{what}: missing bytes argument"))?
        .to_bytes()
        .map_err(|e| format!("{what}: {e}"))
}

fn num(v: &Value) -> Option<f64> {
    match v {
        Value::Quantity(q) => Some(q.si),
        o => o.as_f64(),
    }
}

// ---------------------------------------------------------------------------- pack

#[derive(Clone, Copy)]
enum Code {
    Pad,
    I(usize, bool),
    F32,
    F64,
    Bool,
}

fn parse_fmt(fmt: &str) -> Result<(bool, Vec<Code>), String> {
    let mut chars = fmt.chars().peekable();
    let mut little = true;
    if let Some(&c) = chars.peek() {
        match c {
            '<' | '=' | '@' => {
                chars.next();
            }
            '>' | '!' => {
                little = false;
                chars.next();
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    while let Some(c) = chars.next() {
        if c.is_whitespace() {
            continue;
        }
        let mut count = 1usize;
        if c.is_ascii_digit() {
            let mut n = c.to_digit(10).unwrap() as usize;
            while let Some(d) = chars.peek().and_then(|d| d.to_digit(10)) {
                n = n * 10 + d as usize;
                chars.next();
            }
            count = n;
            let Some(code) = chars.next() else {
                return Err(format!("pack: format '{fmt}' ends with a count"));
            };
            return parse_rest(fmt, little, out, count, code, chars.collect());
        }
        push_code(fmt, &mut out, count, c)?;
    }
    Ok((little, out))
}

fn parse_rest(
    fmt: &str,
    little: bool,
    mut out: Vec<Code>,
    count: usize,
    code: char,
    rest: String,
) -> Result<(bool, Vec<Code>), String> {
    push_code(fmt, &mut out, count, code)?;
    let (_, more) = parse_fmt(&format!("{}{}", if little { "<" } else { ">" }, rest))?;
    out.extend(more);
    Ok((little, out))
}

fn push_code(fmt: &str, out: &mut Vec<Code>, count: usize, c: char) -> Result<(), String> {
    let code = match c {
        'x' => Code::Pad,
        'b' => Code::I(1, true),
        'B' => Code::I(1, false),
        'h' => Code::I(2, true),
        'H' => Code::I(2, false),
        'i' | 'l' => Code::I(4, true),
        'I' | 'L' => Code::I(4, false),
        'q' => Code::I(8, true),
        'Q' => Code::I(8, false),
        'f' => Code::F32,
        'd' => Code::F64,
        '?' => Code::Bool,
        _ => {
            return Err(format!(
                "pack: unknown format code '{c}' in '{fmt}' (use x b B h H i I q Q f d ?)"
            ))
        }
    };
    out.extend(std::iter::repeat_n(code, count));
    Ok(())
}

fn size_of(c: Code) -> usize {
    match c {
        Code::Pad | Code::Bool => 1,
        Code::I(n, _) => n,
        Code::F32 => 4,
        Code::F64 => 8,
    }
}

fn pack_values(fmt: &str, vals: &[Value]) -> Result<Vec<u8>, String> {
    let (little, codes) = parse_fmt(fmt)?;
    let need = codes.iter().filter(|c| !matches!(c, Code::Pad)).count();
    if need != vals.len() {
        return Err(format!(
            "pack('{fmt}'): format needs {need} values, got {}",
            vals.len()
        ));
    }
    let mut out = Vec::new();
    let mut it = vals.iter();
    for c in codes {
        let mut b: Vec<u8> = match c {
            Code::Pad => {
                out.push(0);
                continue;
            }
            Code::Bool => vec![it.next().unwrap().truthy() as u8],
            Code::F32 => {
                let v = it.next().unwrap();
                (num(v)
                    .ok_or_else(|| format!("pack: 'f' expects a number, got {}", v.type_name()))?
                    as f32)
                    .to_le_bytes()
                    .to_vec()
            }
            Code::F64 => {
                let v = it.next().unwrap();
                num(v)
                    .ok_or_else(|| format!("pack: 'd' expects a number, got {}", v.type_name()))?
                    .to_le_bytes()
                    .to_vec()
            }
            Code::I(n, signed) => {
                let v = it.next().unwrap();
                let x = v.as_i64().ok_or_else(|| {
                    format!("pack: integer code expects an int, got {}", v.type_name())
                })?;
                let bits = (n * 8) as u32;
                let ok = if signed {
                    bits == 64 || (x >= -(1i64 << (bits - 1)) && x < (1i64 << (bits - 1)))
                } else {
                    x >= 0 && (bits == 64 || x < (1i64 << bits))
                };
                if !ok {
                    return Err(format!(
                        "pack: {x} does not fit in {} {}-bit",
                        if signed { "signed" } else { "unsigned" },
                        bits
                    ));
                }
                x.to_le_bytes()[..n].to_vec()
            }
        };
        if !little {
            b.reverse();
        }
        out.extend(b);
    }
    Ok(out)
}

fn unpack_bytes(fmt: &str, data: &[u8]) -> Res {
    let (little, codes) = parse_fmt(fmt)?;
    let size: usize = codes.iter().map(|c| size_of(*c)).sum();
    if data.len() != size {
        return Err(format!(
            "unpack('{fmt}'): needs {size} bytes, got {}",
            data.len()
        ));
    }
    let mut out = Vec::new();
    let mut off = 0;
    for c in codes {
        let n = size_of(c);
        let mut b = data[off..off + n].to_vec();
        off += n;
        if !little {
            b.reverse();
        }
        match c {
            Code::Pad => {}
            Code::Bool => out.push(Value::Bool(b[0] != 0)),
            Code::F32 => out.push(Value::Float(
                f32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64
            )),
            Code::F64 => out.push(Value::Float(f64::from_le_bytes(b.try_into().unwrap()))),
            Code::I(n, signed) => {
                let mut buf = [0u8; 8];
                buf[..n].copy_from_slice(&b);
                if signed && b[n - 1] & 0x80 != 0 {
                    for x in buf.iter_mut().skip(n) {
                        *x = 0xFF;
                    }
                }
                out.push(Value::Int(i64::from_le_bytes(buf)));
            }
        }
    }
    Ok(Value::list(out))
}

pub fn crc16_ccitt(data: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for &b in data {
        crc ^= (b as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}

pub fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

pub fn cobs_encode(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8];
    let mut code_idx = 0;
    let mut code = 1u8;
    for &b in data {
        if b == 0 {
            out[code_idx] = code;
            code_idx = out.len();
            out.push(0);
            code = 1;
        } else {
            out.push(b);
            code += 1;
            if code == 0xFF {
                out[code_idx] = code;
                code_idx = out.len();
                out.push(0);
                code = 1;
            }
        }
    }
    out[code_idx] = code;
    out
}

pub fn cobs_decode(data: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < data.len() {
        let code = data[i] as usize;
        if code == 0 {
            return Err("cobs_decode: unexpected zero byte".into());
        }
        i += 1;
        for _ in 1..code {
            if i >= data.len() {
                return Err("cobs_decode: truncated frame".into());
            }
            out.push(data[i]);
            i += 1;
        }
        if code < 0xFF && i < data.len() {
            out.push(0);
        }
    }
    Ok(out)
}

const PACK: &[(&str, NativeFn)] = &[
    ("pack", |_, a: Vec<Value>| {
        let fmt = s_arg(&a, 0, "pack::pack(fmt, values)")?;
        let vals: Vec<Value> = match a.get(1) {
            Some(Value::List(l)) if a.len() == 2 => l.borrow().clone(),
            _ => a[1..].to_vec(),
        };
        Ok(Value::from_bytes(&pack_values(&fmt, &vals)?))
    }),
    ("unpack", |_, a: Vec<Value>| {
        let fmt = s_arg(&a, 0, "pack::unpack(fmt, bytes)")?;
        unpack_bytes(&fmt, &bytes_arg(&a, 1, "pack::unpack")?)
    }),
    ("calcsize", |_, a: Vec<Value>| {
        let (_, codes) = parse_fmt(&s_arg(&a, 0, "pack::calcsize(fmt)")?)?;
        Ok(Value::Int(codes.iter().map(|c| size_of(*c) as i64).sum()))
    }),
    ("f32s", |_, a: Vec<Value>| {
        let Some(Value::List(l)) = a.first() else {
            return Err("pack::f32s(list)".into());
        };
        let l = l.borrow();
        Ok(Value::from_bytes(&pack_values(
            &format!("<{}f", l.len()),
            &l,
        )?))
    }),
    ("to_f32s", |_, a: Vec<Value>| {
        let b = bytes_arg(&a, 0, "pack::to_f32s")?;
        if b.len() % 4 != 0 {
            return Err(format!(
                "pack::to_f32s: {} bytes is not a multiple of 4",
                b.len()
            ));
        }
        unpack_bytes(&format!("<{}f", b.len() / 4), &b)
    }),
    ("crc16", |_, a: Vec<Value>| {
        Ok(Value::Int(
            crc16_ccitt(&bytes_arg(&a, 0, "pack::crc16")?) as i64
        ))
    }),
    ("crc32", |_, a: Vec<Value>| {
        Ok(Value::Int(crc32(&bytes_arg(&a, 0, "pack::crc32")?) as i64))
    }),
    ("cobs_encode", |_, a: Vec<Value>| {
        Ok(Value::from_bytes(&cobs_encode(&bytes_arg(
            &a,
            0,
            "pack::cobs_encode",
        )?)))
    }),
    ("cobs_decode", |_, a: Vec<Value>| {
        Ok(Value::from_bytes(&cobs_decode(&bytes_arg(
            &a,
            0,
            "pack::cobs_decode",
        )?)?))
    }),
];

// ---------------------------------------------------------------------------- json

pub fn to_json(v: &Value) -> Result<J, String> {
    Ok(match v {
        Value::Nil => J::Null,
        Value::Bool(b) => J::Bool(*b),
        Value::Int(i) => J::Number((*i).into()),
        Value::Float(f) => Number::from_f64(*f).map(J::Number).unwrap_or(J::Null),
        Value::Str(s) => J::String(s.to_string()),
        Value::Quantity(q) => Number::from_f64(q.si).map(J::Number).unwrap_or(J::Null),
        Value::Range(a, b, incl) => J::Array(
            (*a..if *incl { *b + 1 } else { *b })
                .map(|x| J::Number(x.into()))
                .collect(),
        ),
        Value::List(l) => J::Array(l.borrow().iter().map(to_json).collect::<Result<_, _>>()?),
        Value::Dict(d) => {
            let mut m = Map::new();
            for (k, v) in &d.borrow().entries {
                let key = match k {
                    Value::Str(s) => s.to_string(),
                    other => other.to_string(),
                };
                m.insert(key, to_json(v)?);
            }
            J::Object(m)
        }
        Value::Object(o) => {
            let o = o.borrow();
            let mut m = Map::new();
            for (k, v) in
                o.ty.def
                    .fields
                    .iter()
                    .map(|f| f.name.clone())
                    .filter_map(|n| o.get(&n).map(|v| (n, v)))
            {
                m.insert(k, to_json(&v)?);
            }
            J::Object(m)
        }
        Value::Native(n) => {
            let b = n.try_borrow().map_err(|_| "json: native object is busy")?;
            if let Some(t) = b
                .as_any()
                .and_then(|a| a.downcast_ref::<crate::nn::Tensor>())
            {
                let rows = t.0.clone();
                drop(b);
                let _ = &rows;
                return to_json(&crate::nn::wrap(rows).call_tolist()?);
            }
            J::String(b.describe())
        }
        Value::Signal(s) => to_json(&s.borrow().value.clone())?,
        other => J::String(other.to_string()),
    })
}

trait ToList {
    fn call_tolist(&self) -> Res;
}
impl ToList for Value {
    fn call_tolist(&self) -> Res {
        match self {
            Value::Native(n) => n.borrow_mut().call_method("tolist", &[]),
            _ => Ok(self.clone()),
        }
    }
}

pub fn from_json(j: &J) -> Value {
    match j {
        J::Null => Value::Nil,
        J::Bool(b) => Value::Bool(*b),
        J::Number(n) => match n.as_i64() {
            Some(i) => Value::Int(i),
            None => Value::Float(n.as_f64().unwrap_or(f64::NAN)),
        },
        J::String(s) => Value::str(s.as_str()),
        J::Array(a) => Value::list(a.iter().map(from_json).collect()),
        J::Object(m) => {
            let mut d = Dict::default();
            for (k, v) in m {
                let _ = d.set(Value::str(k.as_str()), from_json(v));
            }
            Value::dict(d)
        }
    }
}

const JSON: &[(&str, NativeFn)] = &[
    ("encode", |_, a: Vec<Value>| {
        let j = to_json(a.first().ok_or("json::encode(value, pretty=false)")?)?;
        let pretty = a.get(1).is_some_and(|v| v.truthy());
        Ok(Value::str(
            if pretty {
                serde_json::to_string_pretty(&j)
            } else {
                serde_json::to_string(&j)
            }
            .map_err(|e| e.to_string())?,
        ))
    }),
    ("decode", |_, a: Vec<Value>| {
        let s = s_arg(&a, 0, "json::decode(text)")?;
        let j: J = serde_json::from_str(&s).map_err(|e| format!("json::decode: {e}"))?;
        Ok(from_json(&j))
    }),
    ("load", |_, a: Vec<Value>| {
        let p = s_arg(&a, 0, "json::load(path)")?;
        let s = std::fs::read_to_string(&p).map_err(|e| format!("json::load({p}): {e}"))?;
        let j: J = serde_json::from_str(&s).map_err(|e| format!("json::load({p}): {e}"))?;
        Ok(from_json(&j))
    }),
    ("save", |_, a: Vec<Value>| {
        let p = s_arg(&a, 0, "json::save(path, value)")?;
        let j = to_json(a.get(1).ok_or("json::save(path, value)")?)?;
        std::fs::write(&p, serde_json::to_string_pretty(&j).unwrap())
            .map_err(|e| format!("json::save({p}): {e}"))?;
        Ok(Value::Nil)
    }),
];

// ---------------------------------------------------------------------------- tlm

/// Telemetry is emitted as `@@tlm {json}` lines when `TULPA_TELEMETRY` is set (HermiSim sets
/// it); otherwise every `tlm::` call is a cheap no-op so scripts run unchanged elsewhere.
pub fn telemetry_on() -> bool {
    std::env::var_os("TULPA_TELEMETRY").is_some_and(|v| v != "0")
}

fn emit(i: &mut Interpreter, obj: J) {
    i.write_out(&format!("@@tlm {}\n", obj));
}

fn now_s() -> f64 {
    use std::sync::OnceLock;
    static START: OnceLock<std::time::Instant> = OnceLock::new();
    START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_secs_f64()
}

const TLM: &[(&str, NativeFn)] = &[
    ("enabled", |_, _| Ok(Value::Bool(telemetry_on()))),
    ("plot", |i, a: Vec<Value>| {
        let name = s_arg(&a, 0, "tlm::plot(name, value, t=nil)")?;
        if !telemetry_on() {
            return Ok(Value::Nil);
        }
        let v = a.get(1).ok_or("tlm::plot(name, value)")?;
        let t = a.get(2).and_then(num).unwrap_or_else(now_s);
        let unit = match v {
            Value::Quantity(q) => q.unit.as_ref().map(|u| u.name.to_string()),
            _ => None,
        };
        let val = match v {
            Value::Quantity(q) => json!(q.unit.as_ref().map_or(q.si, |u| q.si / u.scale)),
            other => to_json(other)?,
        };
        emit(
            i,
            json!({"k": "plot", "name": name, "t": t, "v": val, "unit": unit}),
        );
        Ok(Value::Nil)
    }),
    ("event", |i, a: Vec<Value>| {
        let name = s_arg(&a, 0, "tlm::event(name, data=nil)")?;
        if telemetry_on() {
            let data = a.get(1).map(to_json).transpose()?.unwrap_or(J::Null);
            emit(
                i,
                json!({"k": "event", "name": name, "t": now_s(), "data": data}),
            );
        }
        Ok(Value::Nil)
    }),
    ("status", |i, a: Vec<Value>| {
        let text = a.first().map(|v| v.to_string()).unwrap_or_default();
        if telemetry_on() {
            emit(i, json!({"k": "status", "text": text}));
        }
        Ok(Value::Nil)
    }),
    ("artifact", |i, a: Vec<Value>| {
        // Tells the IDE a file was produced (VCD waveform, netlist, BOM, weights ...).
        let p = s_arg(&a, 0, "tlm::artifact(path, kind=nil)")?;
        if telemetry_on() {
            let abs = std::fs::canonicalize(&p)
                .map(|x| x.display().to_string())
                .unwrap_or(p);
            let kind = a.get(1).map(|v| v.to_string());
            emit(i, json!({"k": "artifact", "path": abs, "kind": kind}));
        }
        Ok(Value::Nil)
    }),
];

// ---------------------------------------------------------------------------- cosim

/// JSON-lines RPC client to a physics co-simulator.
///
/// Request:  `{"id": n, "method": "step", "params": {...}}\n`
/// Response: `{"id": n, "result": ...}\n` or `{"id": n, "error": "msg"}\n`
pub struct CoSim {
    addr: String,
    reader: BufReader<TcpStream>,
    writer: TcpStream,
    next_id: u64,
}

impl CoSim {
    fn connect(addr: &str) -> Result<CoSim, String> {
        let s = TcpStream::connect(addr)
            .map_err(|e| format!("cosim: cannot reach co-simulator at {addr}: {e}"))?;
        let _ = s.set_nodelay(true);
        let w = s.try_clone().map_err(|e| e.to_string())?;
        Ok(CoSim {
            addr: addr.to_string(),
            reader: BufReader::new(s),
            writer: w,
            next_id: 1,
        })
    }

    fn rpc(&mut self, method: &str, params: J) -> Result<J, String> {
        let id = self.next_id;
        self.next_id += 1;
        let mut line =
            serde_json::to_string(&json!({"id": id, "method": method, "params": params})).unwrap();
        line.push('\n');
        self.writer
            .write_all(line.as_bytes())
            .map_err(|e| format!("cosim: connection lost: {e}"))?;
        let mut resp = String::new();
        loop {
            resp.clear();
            let n = self
                .reader
                .read_line(&mut resp)
                .map_err(|e| format!("cosim: connection lost: {e}"))?;
            if n == 0 {
                return Err("cosim: simulator closed the connection".into());
            }
            let j: J =
                serde_json::from_str(resp.trim()).map_err(|e| format!("cosim: bad reply: {e}"))?;
            if j.get("id").and_then(|x| x.as_u64()) != Some(id) {
                continue; // unsolicited notification
            }
            if let Some(e) = j.get("error") {
                return Err(format!(
                    "cosim.{method}: {}",
                    e.as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| e.to_string())
                ));
            }
            return Ok(j.get("result").cloned().unwrap_or(J::Null));
        }
    }

    fn notify(&mut self, method: &str, params: J) -> Result<(), String> {
        let mut line = serde_json::to_string(&json!({"method": method, "params": params})).unwrap();
        line.push('\n');
        self.writer
            .write_all(line.as_bytes())
            .map_err(|e| format!("cosim: connection lost: {e}"))
    }
}

fn list_param(v: Option<&Value>, what: &str) -> Result<J, String> {
    match v {
        None | Some(Value::Nil) => Ok(J::Null),
        Some(v) => to_json(v).map_err(|e| format!("{what}: {e}")),
    }
}

impl Native for CoSim {
    fn type_name(&self) -> &'static str {
        "CoSim"
    }
    fn describe(&self) -> String {
        format!("<CoSim {}>", self.addr)
    }
    fn call_method(&mut self, m: &str, a: &[Value]) -> Res {
        let r = match m {
            "info" => self.rpc("info", J::Null)?,
            "reset" => self.rpc("reset", json!({"keyframe": list_param(a.first(), "reset")?}))?,
            "observe" | "obs" => self.rpc("observe", J::Null)?,
            "step" => {
                let ctrl = list_param(a.first(), "CoSim.step(ctrl)")?;
                let n = a.get(1).and_then(|v| v.as_i64()).unwrap_or(1);
                self.rpc("step", json!({"ctrl": ctrl, "substeps": n}))?
            }
            "set_ctrl" => self.rpc("set_ctrl", json!({"ctrl": list_param(a.first(), "set_ctrl")?}))?,
            "set_state" => self.rpc(
                "set_state",
                json!({"qpos": list_param(a.first(), "set_state")?, "qvel": list_param(a.get(1), "set_state")?}),
            )?,
            "apply_force" => self.rpc(
                "apply_force",
                json!({"body": a.first().map(|v| v.to_string()), "force": list_param(a.get(1), "apply_force")?,
                       "torque": list_param(a.get(2), "apply_force")?}),
            )?,
            "call" => {
                let method = s_arg(a, 0, "CoSim.call(method, params)")?;
                self.rpc(&method, list_param(a.get(1), "call")?)?
            }
            "plot" => {
                let name = s_arg(a, 0, "CoSim.plot(name, value)")?;
                self.notify("plot", json!({"name": name, "v": list_param(a.get(1), "plot")?}))?;
                J::Null
            }
            "log" => {
                self.notify("log", json!({"text": a.first().map(|v| v.to_string()).unwrap_or_default()}))?;
                J::Null
            }
            "close" => {
                let _ = self.notify("bye", J::Null);
                let _ = self.writer.shutdown(std::net::Shutdown::Both);
                J::Null
            }
            "addr" => J::String(self.addr.clone()),
            _ => return Err(format!("CoSim has no method '{m}'")),
        };
        Ok(from_json(&r))
    }
}

const COSIM: &[(&str, NativeFn)] = &[
    ("connect", |_, a: Vec<Value>| {
        let addr = match a.first() {
            Some(Value::Str(s)) => s.to_string(),
            _ => std::env::var("TULPA_COSIM").map_err(|_| {
                "cosim::connect: no co-simulator. Run this program from HermiSim (Run ▸ Run in Simulator), \
                 with `hermisim run FILE --model MODEL`, or set TULPA_COSIM=host:port"
                    .to_string()
            })?,
        };
        Ok(Value::native(CoSim::connect(
            addr.trim_start_matches("tcp://"),
        )?))
    }),
    ("available", |_, _| {
        Ok(Value::Bool(std::env::var_os("TULPA_COSIM").is_some()))
    }),
];

//! Tulpa Link: the transport core of Tulpa's robotics middleware (`tlink::`, wrapped by
//! `std/link`).
//!
//! Nodes find each other without a broker, exchange *unit-typed* topics, call services and
//! watch each other's liveliness. What makes it Tulpa-native rather than a DDS clone:
//!   - Schemas carry physical dimensions. A `[m/s]` publisher and a `[m]` subscriber are
//!     reported as incompatible at discovery, and quantities travel in SI, so `[cm]` and `[m]`
//!     endpoints interoperate exactly.
//!   - The same program runs on a deterministic in-process network (`transport: "sim"`, with
//!     seeded latency, jitter, loss and partitions on the virtual clock) and on UDP.
//!   - There is one small binary wire format (documented in docs/link.md), not IDL + codegen.
//!
//! Reliability is per topic: best-effort drops late packets; `reliable` topics deliver in
//! order with NACK-based retransmission from a bounded history; `latched` topics hand their
//! last value to late joiners.

use crate::interp::Interpreter;
use crate::units::{Dim, Quantity, Unit};
use crate::value::{Dict, Native, NativeFn, Value};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::net::{SocketAddr, UdpSocket};
use std::rc::Rc;

type Res = Result<Value, String>;

const MAGIC: &[u8; 2] = b"TL";
const VERSION: u8 = 1;
const ANNOUNCE: u8 = 1;
const DATA: u8 = 2;
const NACK: u8 = 3;
const REQUEST: u8 = 4;
const REPLY: u8 = 5;
const BYE: u8 = 6;

const ANNOUNCE_PERIOD: f64 = 0.25;
const PEER_TIMEOUT: f64 = 1.5;
const PORT_BASE: u16 = 7400;
const PORTS_PER_DOMAIN: u16 = 16;

pub fn path(name: &str) -> Option<Value> {
    TABLE
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(n, f)| Value::Builtin(n, *f))
}

pub fn names() -> Vec<&'static str> {
    TABLE.iter().map(|(n, _)| *n).collect()
}

// ------------------------------------------------------------------ schemas

#[derive(Clone, Debug)]
enum Schema {
    Any,
    Int,
    Float,
    Bool,
    Str,
    Unit(Rc<Unit>),
    List(Box<Schema>),
    Record(Vec<(String, Schema)>),
}

impl Schema {
    fn parse(v: &Value) -> Result<Schema, String> {
        match v {
            Value::Nil => Ok(Schema::Any),
            Value::Str(s) => match &**s {
                "any" => Ok(Schema::Any),
                "int" => Ok(Schema::Int),
                "float" => Ok(Schema::Float),
                "bool" => Ok(Schema::Bool),
                "str" => Ok(Schema::Str),
                u => {
                    let inner = u.strip_prefix('[').and_then(|x| x.strip_suffix(']')).ok_or_else(|| {
                        format!("link: unknown schema '{u}' (use any, int, float, bool, str, \"[unit]\", [schema] or {{field: schema}})")
                    })?;
                    Ok(Schema::Unit(Rc::new(crate::units::parse(inner)?)))
                }
            },
            Value::List(l) => match l.borrow().as_slice() {
                [one] => Ok(Schema::List(Box::new(Schema::parse(one)?))),
                _ => {
                    Err("link: a list schema has exactly one element schema, e.g. [\"[m]\"]".into())
                }
            },
            Value::Dict(d) => {
                let mut fields = Vec::new();
                for (k, v) in &d.borrow().entries {
                    fields.push((k.to_string(), Schema::parse(v)?));
                }
                fields.sort_by(|a, b| a.0.cmp(&b.0));
                Ok(Schema::Record(fields))
            }
            other => Err(format!("link: a schema cannot be a {}", other.type_name())),
        }
    }

    /// Canonical text. Units are written by dimension, so `[cm]` and `[m]` are the same type.
    fn canon(&self) -> String {
        match self {
            Schema::Any => "any".into(),
            Schema::Int => "int".into(),
            Schema::Float => "float".into(),
            Schema::Bool => "bool".into(),
            Schema::Str => "str".into(),
            Schema::Unit(u) => format!("[{}]", crate::units::dim_name(&u.dim)),
            Schema::List(s) => format!("[{}]", s.canon()),
            Schema::Record(fs) => format!(
                "{{{}}}",
                fs.iter()
                    .map(|(k, s)| format!("{k}:{}", s.canon()))
                    .collect::<Vec<_>>()
                    .join(";")
            ),
        }
    }

    /// 0 for `any`, which is compatible with every schema.
    fn hash(&self) -> u32 {
        if matches!(self, Schema::Any) {
            return 0;
        }
        self.canon().bytes().fold(0x811c_9dc5u32, |h, b| {
            (h ^ b as u32).wrapping_mul(0x0100_0193)
        })
    }

    /// Checks `v` against the schema. `at` names the place for error messages.
    fn check(&self, v: &Value, at: &str) -> Result<(), String> {
        let bad = |want: &str| Err(format!("{at}: expected {want}, got {}", describe(v)));
        match self {
            Schema::Any => Ok(()),
            Schema::Int => match v {
                Value::Int(_) => Ok(()),
                _ => bad("int"),
            },
            Schema::Float => match v {
                Value::Float(_) | Value::Int(_) => Ok(()),
                _ => bad("float"),
            },
            Schema::Bool => match v {
                Value::Bool(_) => Ok(()),
                _ => bad("bool"),
            },
            Schema::Str => match v {
                Value::Str(_) => Ok(()),
                _ => bad("str"),
            },
            Schema::Unit(u) => match v {
                Value::Quantity(q) if q.dim == u.dim => Ok(()),
                _ => bad(&format!("a quantity in [{}]", u.name)),
            },
            Schema::List(s) => match v {
                Value::List(l) => {
                    for (i, x) in l.borrow().iter().enumerate() {
                        s.check(x, &format!("{at}[{i}]"))?;
                    }
                    Ok(())
                }
                _ => bad("a list"),
            },
            Schema::Record(fs) => match v {
                Value::Dict(d) => {
                    let d = d.borrow();
                    for (k, s) in fs {
                        match d.get(&Value::str(k.as_str()))? {
                            Some(x) => s.check(&x, &format!("{at}.{k}"))?,
                            None => return Err(format!("{at}: missing field '{k}'")),
                        }
                    }
                    if let Some((k, _)) = d
                        .entries
                        .iter()
                        .find(|(k, _)| !fs.iter().any(|(f, _)| *f == k.to_string()))
                    {
                        return Err(format!("{at}: unexpected field '{k}'"));
                    }
                    Ok(())
                }
                _ => bad("a record"),
            },
        }
    }

    /// Gives received quantities the display unit this endpoint declared.
    fn shape(&self, v: Value) -> Value {
        match (self, v) {
            (Schema::Unit(u), Value::Quantity(q)) => Value::Quantity(Quantity {
                si: q.si,
                dim: q.dim,
                unit: Some(u.clone()),
            }),
            (Schema::Float, Value::Int(i)) => Value::Float(i as f64),
            (Schema::List(s), Value::List(l)) => {
                let items: Vec<Value> = l.borrow().iter().cloned().map(|x| s.shape(x)).collect();
                Value::list(items)
            }
            (Schema::Record(fs), Value::Dict(d)) => {
                let mut out = Dict::default();
                for (k, x) in &d.borrow().entries {
                    let s = fs.iter().find(|(f, _)| *f == k.to_string()).map(|(_, s)| s);
                    let x = match s {
                        Some(s) => s.shape(x.clone()),
                        None => x.clone(),
                    };
                    let _ = out.set(k.clone(), x);
                }
                Value::dict(out)
            }
            (_, v) => v,
        }
    }
}

fn describe(v: &Value) -> String {
    match v {
        Value::Quantity(q) => format!("a quantity in [{}]", crate::units::dim_name(&q.dim)),
        other => other.type_name().to_string(),
    }
}

// ------------------------------------------------------------------ wire format

#[derive(Default)]
struct W(Vec<u8>);

impl W {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.0.extend(v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.0.extend(v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.0.extend(v.to_le_bytes());
    }
    fn f64(&mut self, v: f64) {
        self.0.extend(v.to_le_bytes());
    }
    fn str(&mut self, s: &str) {
        self.u16(s.len().min(65535) as u16);
        self.0.extend(&s.as_bytes()[..s.len().min(65535)]);
    }
    fn value(&mut self, v: &Value) -> Result<(), String> {
        match v {
            Value::Nil => self.u8(0),
            Value::Bool(false) => self.u8(1),
            Value::Bool(true) => self.u8(2),
            Value::Int(i) => {
                self.u8(3);
                self.u64(*i as u64);
            }
            Value::Float(f) => {
                self.u8(4);
                self.f64(*f);
            }
            Value::Str(s) => {
                self.u8(5);
                self.u32(s.len() as u32);
                self.0.extend(s.as_bytes());
            }
            Value::List(l) => {
                self.u8(6);
                let l = l.borrow();
                self.u32(l.len() as u32);
                for x in l.iter() {
                    self.value(x)?;
                }
            }
            Value::Dict(d) => {
                self.u8(7);
                let d = d.borrow();
                self.u32(d.entries.len() as u32);
                for (k, x) in &d.entries {
                    self.value(k)?;
                    self.value(x)?;
                }
            }
            Value::Quantity(q) => {
                self.u8(8);
                self.f64(q.si);
                self.0.extend(q.dim.iter().map(|d| *d as u8));
            }
            other => {
                return Err(format!(
                    "link: a {} cannot be sent over a topic",
                    other.type_name()
                ))
            }
        }
        Ok(())
    }
}

struct R<'a>(&'a [u8]);

impl R<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], String> {
        if self.0.len() < n {
            return Err("link: truncated packet".into());
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Ok(a)
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, String> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn f64(&mut self) -> Result<f64, String> {
        Ok(f64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn str(&mut self) -> Result<String, String> {
        let n = self.u16()? as usize;
        String::from_utf8(self.take(n)?.to_vec())
            .map_err(|_| "link: invalid text in packet".to_string())
    }
    fn value(&mut self, depth: usize) -> Result<Value, String> {
        if depth > 32 {
            return Err("link: value nested too deeply".into());
        }
        Ok(match self.u8()? {
            0 => Value::Nil,
            1 => Value::Bool(false),
            2 => Value::Bool(true),
            3 => Value::Int(self.u64()? as i64),
            4 => Value::Float(self.f64()?),
            5 => {
                let n = self.u32()? as usize;
                Value::str(
                    String::from_utf8(self.take(n)?.to_vec())
                        .map_err(|_| "link: invalid text in packet".to_string())?,
                )
            }
            6 => {
                let n = self.u32()? as usize;
                let mut items = Vec::with_capacity(n.min(4096));
                for _ in 0..n {
                    items.push(self.value(depth + 1)?);
                }
                Value::list(items)
            }
            7 => {
                let n = self.u32()? as usize;
                let mut d = Dict::default();
                for _ in 0..n {
                    let k = self.value(depth + 1)?;
                    let v = self.value(depth + 1)?;
                    d.set(k, v)?;
                }
                Value::dict(d)
            }
            8 => {
                let si = self.f64()?;
                let mut dim: Dim = [0; 8];
                for (i, b) in self.take(8)?.iter().enumerate() {
                    dim[i] = *b as i8;
                }
                Value::Quantity(Quantity {
                    si,
                    dim,
                    unit: None,
                })
            }
            t => return Err(format!("link: unknown value tag {t}")),
        })
    }
}

// ------------------------------------------------------------------ transports

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Addr {
    Sim(u32),
    Udp(SocketAddr),
}

struct SimPacket {
    to: u32,
    from: u32,
    at: f64,
    order: u64,
    data: Rc<Vec<u8>>,
}

/// A deterministic in-process network shared by every `sim` node of the interpreter.
#[derive(Default)]
struct SimNet {
    nodes: BTreeMap<u32, u16>,
    next: u32,
    queue: Vec<SimPacket>,
    order: u64,
    latency: f64,
    jitter: f64,
    loss: f64,
    rng: u64,
    cut: HashSet<(u32, u32)>,
    sent: u64,
    dropped: u64,
}

impl SimNet {
    fn rand(&mut self) -> f64 {
        // xorshift64*: reproducible across platforms
        let mut x = if self.rng == 0 {
            0x9e37_79b9_7f4a_7c15
        } else {
            self.rng
        };
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        (x.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 11) as f64 / (1u64 << 53) as f64
    }

    fn send(&mut self, from: u32, to: Option<u32>, domain: u16, now: f64, data: Vec<u8>) {
        let data = Rc::new(data);
        let targets: Vec<u32> = match to {
            Some(t) => vec![t],
            None => self
                .nodes
                .iter()
                .filter(|(a, d)| **a != from && **d == domain)
                .map(|(a, _)| *a)
                .collect(),
        };
        for t in targets {
            self.sent += 1;
            let pair = (from.min(t), from.max(t));
            if self.cut.contains(&pair) || (self.loss > 0.0 && self.rand() < self.loss) {
                self.dropped += 1;
                continue;
            }
            let delay = self.latency
                + if self.jitter > 0.0 {
                    self.rand() * self.jitter
                } else {
                    0.0
                };
            self.order += 1;
            self.queue.push(SimPacket {
                to: t,
                from,
                at: now + delay,
                order: self.order,
                data: data.clone(),
            });
        }
    }

    fn recv(&mut self, addr: u32, now: f64) -> Vec<(u32, Rc<Vec<u8>>)> {
        let mut due: Vec<SimPacket> = Vec::new();
        let mut rest = Vec::new();
        for p in self.queue.drain(..) {
            if p.to == addr && p.at <= now + 1e-12 {
                due.push(p);
            } else {
                rest.push(p);
            }
        }
        self.queue = rest;
        due.sort_by(|a, b| a.at.partial_cmp(&b.at).unwrap().then(a.order.cmp(&b.order)));
        due.into_iter().map(|p| (p.from, p.data)).collect()
    }
}

thread_local! {
    static SIM: RefCell<SimNet> = RefCell::new(SimNet::default());
}

enum Transport {
    Sim(u32),
    Udp {
        sock: UdpSocket,
        base: u16,
        port: u16,
        extra: Vec<std::net::IpAddr>,
    },
}

// ------------------------------------------------------------------ node state

#[derive(Clone)]
struct Endpoint {
    role: u8, // 0 publisher, 1 subscriber, 2 service
    topic: String,
    hash: u32,
    flags: u8, // 1 reliable, 2 latched
    last_seq: u64,
}

struct Peer {
    name: String,
    addr: Addr,
    last_seen: f64,
    endpoints: Vec<Endpoint>,
}

struct Publisher {
    schema: Schema,
    seq: u64,
    reliable: bool,
    latched: bool,
    keep: usize,
    history: VecDeque<(u64, Vec<u8>)>,
    sent: u64,
}

struct Stream {
    expected: u64,
    held: BTreeMap<u64, (f64, Value)>,
}

struct Subscriber {
    schema: Schema,
    reliable: bool,
    deadline: Option<f64>,
    last_rx: Option<f64>,
    missed: bool,
    streams: HashMap<u64, Stream>,
    received: u64,
    lost: u64,
}

struct Node {
    name: String,
    id: u64,
    domain: u16,
    transport: Transport,
    peers: BTreeMap<u64, Peer>,
    pubs: BTreeMap<String, Publisher>,
    subs: BTreeMap<String, Subscriber>,
    services: BTreeMap<String, ()>,
    events: Vec<Value>,
    last_announce: f64,
    dirty: bool,
    next_req: u64,
    /// Requests awaiting a reply: id -> (server, packet, last sent, sends so far).
    pending: HashMap<u64, (Addr, Vec<u8>, f64, u32)>,
    /// Requests being handled, and recent replies kept for retransmission.
    in_progress: HashSet<(u64, u64)>,
    answered: VecDeque<((u64, u64), Vec<u8>)>,
    warned: HashSet<(u64, String)>,
    closed: bool,
}

fn event(kind: &str, fields: Vec<(&str, Value)>) -> Value {
    let mut d = Dict::default();
    let _ = d.set(Value::str("kind"), Value::str(kind));
    for (k, v) in fields {
        let _ = d.set(Value::str(k), v);
    }
    Value::dict(d)
}

impl Node {
    fn header(&self, kind: u8) -> W {
        let mut w = W::default();
        w.0.extend(MAGIC);
        w.u8(VERSION);
        w.u8(kind);
        w.u16(self.domain);
        w.u64(self.id);
        w
    }

    fn send(&self, to: Option<Addr>, now: f64, data: Vec<u8>) {
        match (&self.transport, to) {
            (Transport::Sim(me), Some(Addr::Sim(t))) => {
                SIM.with(|s| s.borrow_mut().send(*me, Some(t), self.domain, now, data))
            }
            (Transport::Sim(me), None) => {
                SIM.with(|s| s.borrow_mut().send(*me, None, self.domain, now, data))
            }
            (Transport::Udp { sock, .. }, Some(Addr::Udp(a))) => {
                let _ = sock.send_to(&data, a);
            }
            (
                Transport::Udp {
                    sock,
                    base,
                    port,
                    extra,
                },
                None,
            ) => {
                for k in 0..PORTS_PER_DOMAIN {
                    let p = base + k;
                    if p != *port {
                        let _ = sock.send_to(&data, ("127.0.0.1", p));
                    }
                    let _ = sock.send_to(&data, ("255.255.255.255", p));
                    for ip in extra {
                        let _ = sock.send_to(&data, (*ip, p));
                    }
                }
            }
            _ => {}
        }
    }

    fn announce(&mut self, now: f64) {
        let mut w = self.header(ANNOUNCE);
        w.str(&self.name);
        w.u16((self.pubs.len() + self.subs.len() + self.services.len()) as u16);
        for (t, p) in &self.pubs {
            w.u8(0);
            w.str(t);
            w.u32(p.schema.hash());
            w.u8(p.reliable as u8 | (p.latched as u8) << 1);
            w.u64(p.seq);
        }
        for (t, s) in &self.subs {
            w.u8(1);
            w.str(t);
            w.u32(s.schema.hash());
            w.u8(s.reliable as u8);
            w.u64(0);
        }
        for t in self.services.keys() {
            w.u8(2);
            w.str(t);
            w.u32(0);
            w.u8(0);
            w.u64(0);
        }
        self.send(None, now, w.0);
        self.last_announce = now;
        self.dirty = false;
    }

    fn data_packet(&self, topic: &str, seq: u64, stamp: f64, payload: &[u8]) -> Vec<u8> {
        let mut w = self.header(DATA);
        w.str(topic);
        w.u64(seq);
        w.f64(stamp);
        w.0.extend(payload);
        w.0
    }

    /// Peers subscribed to `topic` with a compatible schema.
    fn listeners(&self, topic: &str, hash: u32) -> Vec<Addr> {
        self.peers
            .values()
            .filter(|p| {
                p.endpoints.iter().any(|e| {
                    e.role == 1 && e.topic == topic && (e.hash == hash || e.hash == 0 || hash == 0)
                })
            })
            .map(|p| p.addr)
            .collect()
    }

    fn publish(&mut self, topic: &str, v: &Value, now: f64) -> Result<(), String> {
        let p = self
            .pubs
            .get_mut(topic)
            .ok_or_else(|| format!("link: '{topic}' is not advertised by node '{}'", self.name))?;
        p.schema.check(v, topic)?;
        let mut w = W::default();
        w.value(v)?;
        p.seq += 1;
        p.sent += 1;
        let seq = p.seq;
        p.history.push_back((seq, w.0.clone()));
        let keep = p.keep.max(1);
        while p.history.len() > keep {
            p.history.pop_front();
        }
        let hash = p.schema.hash();
        let packet = self.data_packet(topic, seq, now, &w.0);
        for a in self.listeners(topic, hash) {
            self.send(Some(a), now, packet.clone());
        }
        // subscribers on this node get it directly
        let (id, name) = (self.id, self.name.clone());
        if self.subs.contains_key(topic) {
            self.deliver(topic, id, &name, seq, now, v.clone(), now, true);
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn deliver(
        &mut self,
        topic: &str,
        from: u64,
        from_name: &str,
        seq: u64,
        stamp: f64,
        v: Value,
        now: f64,
        in_order: bool,
    ) {
        let Some(s) = self.subs.get_mut(topic) else {
            return;
        };
        if let Err(e) = s.schema.check(&v, topic) {
            self.events.push(event(
                "incompatible",
                vec![
                    ("topic", Value::str(topic)),
                    ("peer", Value::str(from_name)),
                    ("reason", Value::str(e)),
                ],
            ));
            return;
        }
        let reliable = s.reliable && !in_order;
        let st = s.streams.entry(from).or_insert(Stream {
            expected: seq,
            held: BTreeMap::new(),
        });
        let mut ready: Vec<(u64, f64, Value)> = Vec::new();
        if seq < st.expected {
            return; // duplicate or stale
        }
        if !reliable {
            if seq > st.expected {
                s.lost += seq - st.expected;
            }
            st.expected = seq + 1;
            ready.push((seq, stamp, v));
        } else {
            st.held.insert(seq, (stamp, v));
            while let Some((stamp, v)) = st.held.remove(&st.expected) {
                ready.push((st.expected, stamp, v));
                st.expected += 1;
            }
        }
        for (seq, stamp, v) in ready {
            s.received += 1;
            s.last_rx = Some(now);
            s.missed = false;
            let v = s.schema.shape(v);
            self.events.push(event(
                "msg",
                vec![
                    ("topic", Value::str(topic)),
                    ("value", v),
                    ("from", Value::str(from_name)),
                    ("seq", Value::Int(seq as i64)),
                    ("stamp", Value::Float(stamp)),
                    ("latency", Value::Float(now - stamp)),
                ],
            ));
        }
    }

    /// Asks `from` to resend what a reliable subscription is missing up to `last`.
    fn request_missing(&mut self, topic: &str, from: u64, last: u64, now: f64) {
        let Some(s) = self.subs.get(topic) else {
            return;
        };
        if !s.reliable {
            return;
        }
        let Some(st) = s.streams.get(&from) else {
            return;
        };
        let missing: Vec<u64> = (st.expected..=last)
            .filter(|q| !st.held.contains_key(q))
            .take(64)
            .collect();
        if missing.is_empty() {
            return;
        }
        let Some(addr) = self.peers.get(&from).map(|p| p.addr) else {
            return;
        };
        let mut w = self.header(NACK);
        w.str(topic);
        w.u16(missing.len() as u16);
        for q in missing {
            w.u64(q);
        }
        self.send(Some(addr), now, w.0);
    }

    fn handle(&mut self, from_addr: Addr, data: &[u8], now: f64) -> Result<(), String> {
        let mut r = R(data);
        if r.take(2)? != MAGIC || r.u8()? != VERSION {
            return Ok(());
        }
        let kind = r.u8()?;
        let domain = r.u16()?;
        let id = r.u64()?;
        if domain != self.domain || id == self.id {
            return Ok(());
        }
        match kind {
            ANNOUNCE => {
                let name = r.str()?;
                let n = r.u16()?;
                let mut endpoints = Vec::new();
                for _ in 0..n {
                    endpoints.push(Endpoint {
                        role: r.u8()?,
                        topic: r.str()?,
                        hash: r.u32()?,
                        flags: r.u8()?,
                        last_seq: r.u64()?,
                    });
                }
                let known = self.peers.get(&id).map(|p| p.endpoints.clone());
                if known.is_none() {
                    self.events.push(event(
                        "peer_joined",
                        vec![("peer", Value::str(name.as_str()))],
                    ));
                    self.dirty = true; // introduce ourselves promptly
                }
                let old = known.unwrap_or_default();
                for e in &endpoints {
                    let is_new = !old.iter().any(|o| o.role == e.role && o.topic == e.topic);
                    // schema agreement
                    let mine = match e.role {
                        0 => self.subs.get(&e.topic).map(|s| s.schema.hash()),
                        1 => self.pubs.get(&e.topic).map(|p| p.schema.hash()),
                        _ => None,
                    };
                    if let Some(h) = mine {
                        if h != e.hash && h != 0 && e.hash != 0 {
                            if self.warned.insert((id, e.topic.clone())) {
                                let ours = match e.role {
                                    0 => self.subs[&e.topic].schema.canon(),
                                    _ => self.pubs[&e.topic].schema.canon(),
                                };
                                self.events.push(event(
                                    "incompatible",
                                    vec![
                                        ("topic", Value::str(e.topic.as_str())),
                                        ("peer", Value::str(name.as_str())),
                                        (
                                            "reason",
                                            Value::str(format!(
                                                "schema mismatch: this node uses {ours}"
                                            )),
                                        ),
                                    ],
                                ));
                            }
                            continue;
                        }
                    }
                    // a new subscriber to a latched topic gets the last value
                    if e.role == 1 && is_new {
                        if let Some(p) = self.pubs.get(&e.topic) {
                            if p.latched {
                                if let Some((seq, payload)) = p.history.back() {
                                    let pkt = self.data_packet(&e.topic, *seq, now, payload);
                                    self.send(Some(from_addr), now, pkt);
                                }
                            }
                        }
                    }
                    // a reliable subscription catches up on anything published since it joined
                    if e.role == 0 && e.flags & 1 != 0 {
                        if let Some(s) = self.subs.get_mut(&e.topic) {
                            if s.reliable {
                                let latched = e.flags & 2 != 0;
                                s.streams.entry(id).or_insert(Stream {
                                    expected: if latched {
                                        e.last_seq.max(1)
                                    } else {
                                        e.last_seq + 1
                                    },
                                    held: BTreeMap::new(),
                                });
                            }
                        }
                    }
                }
                self.peers.insert(
                    id,
                    Peer {
                        name,
                        addr: from_addr,
                        last_seen: now,
                        endpoints: endpoints.clone(),
                    },
                );
                for e in endpoints.iter().filter(|e| e.role == 0 && e.flags & 1 != 0) {
                    self.request_missing(&e.topic.clone(), id, e.last_seq, now);
                }
            }
            DATA => {
                let topic = r.str()?;
                let seq = r.u64()?;
                let stamp = r.f64()?;
                let v = r.value(0)?;
                let name = self
                    .peers
                    .get(&id)
                    .map(|p| p.name.clone())
                    .unwrap_or_else(|| format!("{id:016x}"));
                if let Some(p) = self.peers.get_mut(&id) {
                    p.last_seen = now;
                }
                self.deliver(&topic, id, &name, seq, stamp, v, now, false);
                // a gap on a reliable stream: ask again
                let gap = self
                    .subs
                    .get(&topic)
                    .and_then(|s| s.streams.get(&id))
                    .and_then(|st| st.held.keys().next_back().copied());
                if let Some(last) = gap {
                    self.request_missing(&topic, id, last, now);
                }
            }
            NACK => {
                let topic = r.str()?;
                let n = r.u16()?;
                let mut wanted = Vec::new();
                for _ in 0..n {
                    wanted.push(r.u64()?);
                }
                if let Some(p) = self.pubs.get(&topic) {
                    let packets: Vec<Vec<u8>> = p
                        .history
                        .iter()
                        .filter(|(q, _)| wanted.contains(q))
                        .map(|(q, payload)| self.data_packet(&topic, *q, now, payload))
                        .collect();
                    for pkt in packets {
                        self.send(Some(from_addr), now, pkt);
                    }
                }
            }
            REQUEST => {
                let service = r.str()?;
                let req = r.u64()?;
                let v = r.value(0)?;
                // requests are retransmitted until answered: handle each one once
                if let Some((_, pkt)) = self.answered.iter().find(|(k, _)| *k == (id, req)) {
                    let pkt = pkt.clone();
                    self.send(Some(from_addr), now, pkt);
                    return Ok(());
                }
                if !self.in_progress.insert((id, req)) {
                    return Ok(());
                }
                let name = self
                    .peers
                    .get(&id)
                    .map(|p| p.name.clone())
                    .unwrap_or_default();
                self.events.push(event(
                    "request",
                    vec![
                        ("service", Value::str(service)),
                        ("id", Value::Int(req as i64)),
                        ("from", Value::Int(id as i64)),
                        ("peer", Value::str(name)),
                        ("value", v),
                    ],
                ));
            }
            REPLY => {
                let req = r.u64()?;
                let ok = r.u8()? != 0;
                let v = r.value(0)?;
                if self.pending.remove(&req).is_some() {
                    self.events.push(event(
                        "reply",
                        vec![
                            ("id", Value::Int(req as i64)),
                            ("ok", Value::Bool(ok)),
                            ("value", v),
                        ],
                    ));
                }
            }
            BYE => {
                if let Some(p) = self.peers.remove(&id) {
                    self.events.push(event(
                        "peer_lost",
                        vec![("peer", Value::str(p.name)), ("reason", Value::str("left"))],
                    ));
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn pump(&mut self, now: f64) -> Vec<Value> {
        if self.closed {
            return Vec::new();
        }
        let packets: Vec<(Addr, Rc<Vec<u8>>)> = match &self.transport {
            Transport::Sim(me) => SIM
                .with(|s| s.borrow_mut().recv(*me, now))
                .into_iter()
                .map(|(f, d)| (Addr::Sim(f), d))
                .collect(),
            Transport::Udp { sock, .. } => {
                let mut out = Vec::new();
                let mut buf = [0u8; 65536];
                while let Ok((n, from)) = sock.recv_from(&mut buf) {
                    out.push((Addr::Udp(from), Rc::new(buf[..n].to_vec())));
                    if out.len() >= 4096 {
                        break;
                    }
                }
                out
            }
        };
        for (from, data) in packets {
            let _ = self.handle(from, &data, now);
        }
        if self.dirty || now - self.last_announce >= ANNOUNCE_PERIOD {
            self.announce(now);
        }
        let mut resend = Vec::new();
        for (addr, pkt, last, tries) in self.pending.values_mut() {
            if now - *last >= 0.05 && *tries < 40 {
                *last = now;
                *tries += 1;
                resend.push((*addr, pkt.clone()));
            }
        }
        for (addr, pkt) in resend {
            self.send(Some(addr), now, pkt);
        }
        let lost: Vec<u64> = self
            .peers
            .iter()
            .filter(|(_, p)| now - p.last_seen > PEER_TIMEOUT)
            .map(|(id, _)| *id)
            .collect();
        for id in lost {
            if let Some(p) = self.peers.remove(&id) {
                self.events.push(event(
                    "peer_lost",
                    vec![
                        ("peer", Value::str(p.name)),
                        ("reason", Value::str("timeout")),
                    ],
                ));
            }
        }
        let mut late = Vec::new();
        for (t, s) in self.subs.iter_mut() {
            if let (Some(d), Some(rx)) = (s.deadline, s.last_rx) {
                if !s.missed && now - rx > d {
                    s.missed = true;
                    late.push((t.clone(), now - rx));
                }
            }
        }
        for (t, age) in late {
            self.events.push(event(
                "deadline",
                vec![("topic", Value::str(t)), ("age", Value::Float(age))],
            ));
        }
        std::mem::take(&mut self.events)
    }

    fn info(&self) -> Value {
        let mut d = Dict::default();
        fn s(x: impl Into<Rc<str>>) -> Value {
            Value::str(x)
        }
        let _ = d.set(s("name"), s(self.name.as_str()));
        let _ = d.set(s("domain"), Value::Int(self.domain as i64));
        let _ = d.set(
            s("transport"),
            s(match &self.transport {
                Transport::Sim(_) => "sim".to_string(),
                Transport::Udp { port, .. } => format!("udp:{port}"),
            }),
        );
        let ends = |pubs: Vec<String>, subs: Vec<String>, svcs: Vec<String>| {
            let mut e = Dict::default();
            let list = |v: Vec<String>| Value::list(v.into_iter().map(Value::str).collect());
            let _ = e.set(Value::str("publishes"), list(pubs));
            let _ = e.set(Value::str("subscribes"), list(subs));
            let _ = e.set(Value::str("services"), list(svcs));
            e
        };
        let mut me = ends(
            self.pubs.keys().cloned().collect(),
            self.subs.keys().cloned().collect(),
            self.services.keys().cloned().collect(),
        );
        let _ = me.set(s("name"), s(self.name.as_str()));
        let mut nodes = vec![Value::dict(me)];
        for p in self.peers.values() {
            let pick = |role: u8| {
                p.endpoints
                    .iter()
                    .filter(|e| e.role == role)
                    .map(|e| e.topic.clone())
                    .collect::<Vec<_>>()
            };
            let mut e = ends(pick(0), pick(1), pick(2));
            let _ = e.set(s("name"), s(p.name.as_str()));
            nodes.push(Value::dict(e));
        }
        let _ = d.set(s("nodes"), Value::list(nodes));
        let mut topics = Vec::new();
        for (t, p) in &self.pubs {
            topics.push(event(
                "publisher",
                vec![
                    ("topic", s(t.as_str())),
                    ("schema", s(p.schema.canon())),
                    ("sent", Value::Int(p.sent as i64)),
                    ("reliable", Value::Bool(p.reliable)),
                    ("latched", Value::Bool(p.latched)),
                ],
            ));
        }
        for (t, sub) in &self.subs {
            topics.push(event(
                "subscriber",
                vec![
                    ("topic", s(t.as_str())),
                    ("schema", s(sub.schema.canon())),
                    ("received", Value::Int(sub.received as i64)),
                    ("lost", Value::Int(sub.lost as i64)),
                    ("reliable", Value::Bool(sub.reliable)),
                ],
            ));
        }
        let _ = d.set(s("topics"), Value::list(topics));
        Value::dict(d)
    }
}

struct Handle(Rc<RefCell<Node>>);

impl Native for Handle {
    fn type_name(&self) -> &'static str {
        "LinkNode"
    }
    fn call_method(&mut self, method: &str, _: &[Value]) -> Result<Value, String> {
        Err(format!("LinkNode has no method '{method}' (use std/link)"))
    }
    fn describe(&self) -> String {
        format!("<LinkNode {}>", self.0.borrow().name)
    }
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
}

fn node(a: &[Value]) -> Result<Rc<RefCell<Node>>, String> {
    if let Some(Value::Native(n)) = a.first() {
        if let Some(h) = n.borrow().as_any().and_then(|x| x.downcast_ref::<Handle>()) {
            return Ok(h.0.clone());
        }
    }
    Err("link: expected a node handle".into())
}

fn text(a: &[Value], i: usize, what: &str) -> Result<String, String> {
    match a.get(i) {
        Some(Value::Str(s)) => Ok(s.to_string()),
        _ => Err(format!("link: {what} must be a string")),
    }
}

fn opt(o: Option<&Value>, key: &str) -> Option<Value> {
    match o {
        Some(Value::Dict(d)) => d.borrow().get(&Value::str(key)).ok().flatten(),
        _ => None,
    }
}

fn secs(v: &Value) -> Option<f64> {
    match v {
        Value::Quantity(q) => Some(q.si),
        o => o.as_f64(),
    }
}

fn fnv64(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

const TABLE: &[(&str, NativeFn)] = &[
    // tlink::node(name, {domain, transport: "sim"|"udp", peers: ["host", ...]})
    ("node", |i: &mut Interpreter, a: Vec<Value>| -> Res {
        let name = text(&a, 0, "the node name")?;
        let o = a.get(1);
        let domain = opt(o, "domain").and_then(|v| v.as_f64()).unwrap_or(0.0) as u16;
        let kind = match opt(o, "transport") {
            Some(Value::Str(s)) => s.to_string(),
            _ => std::env::var("TULPA_LINK").unwrap_or_else(|_| "sim".into()),
        };
        let (transport, id) = match kind.as_str() {
            "sim" => {
                let addr = SIM.with(|s| {
                    let mut s = s.borrow_mut();
                    s.next += 1;
                    let a = s.next;
                    s.nodes.insert(a, domain);
                    a
                });
                (Transport::Sim(addr), fnv64(&name) ^ (addr as u64) << 48)
            }
            "udp" => {
                if domain > 200 {
                    return Err("link: UDP domains are 0..=200".into());
                }
                let base = PORT_BASE + domain * PORTS_PER_DOMAIN;
                let mut bound = None;
                for k in 0..PORTS_PER_DOMAIN {
                    if let Ok(s) = UdpSocket::bind(("0.0.0.0", base + k)) {
                        bound = Some((s, base + k));
                        break;
                    }
                }
                let (sock, port) = bound.ok_or_else(|| {
                    format!(
                        "link: no free UDP port in {base}..{} (too many nodes on this host)",
                        base + PORTS_PER_DOMAIN
                    )
                })?;
                sock.set_nonblocking(true).map_err(|e| e.to_string())?;
                let _ = sock.set_broadcast(true);
                let mut extra = Vec::new();
                if let Some(Value::List(l)) = opt(o, "peers") {
                    for p in l.borrow().iter() {
                        let host = p.to_string();
                        let ip = host
                            .parse()
                            .map_err(|_| format!("link: peer '{host}' is not an IP address"))?;
                        extra.push(ip);
                    }
                }
                let unique = std::process::id() as u64 ^ (port as u64) << 32;
                (
                    Transport::Udp {
                        sock,
                        base,
                        port,
                        extra,
                    },
                    fnv64(&name) ^ unique.wrapping_mul(0x9e37_79b9_7f4a_7c15),
                )
            }
            other => {
                return Err(format!(
                    "link: unknown transport '{other}' (use \"sim\" or \"udp\")"
                ))
            }
        };
        let mut n = Node {
            name,
            id,
            domain,
            transport,
            peers: BTreeMap::new(),
            pubs: BTreeMap::new(),
            subs: BTreeMap::new(),
            services: BTreeMap::new(),
            events: Vec::new(),
            last_announce: -1.0,
            dirty: true,
            next_req: 0,
            pending: HashMap::new(),
            in_progress: HashSet::new(),
            answered: VecDeque::new(),
            warned: HashSet::new(),
            closed: false,
        };
        n.announce(i.now());
        Ok(Value::native(Handle(Rc::new(RefCell::new(n)))))
    }),
    // tlink::advertise(node, topic, schema, {reliable, latched, keep})
    ("advertise", |_, a| {
        let n = node(&a)?;
        let topic = text(&a, 1, "the topic name")?;
        let schema = Schema::parse(a.get(2).unwrap_or(&Value::Nil))?;
        let o = a.get(3);
        let flag = |k: &str| opt(o, k).map(|v| v.truthy()).unwrap_or(false);
        let mut n = n.borrow_mut();
        if let Some(p) = n.pubs.get(&topic) {
            if p.schema.hash() != schema.hash() {
                return Err(format!(
                    "link: '{topic}' is already advertised as {}",
                    p.schema.canon()
                ));
            }
            return Ok(Value::Nil);
        }
        n.pubs.insert(
            topic,
            Publisher {
                schema,
                seq: 0,
                reliable: flag("reliable"),
                latched: flag("latched"),
                keep: opt(o, "keep").and_then(|v| v.as_f64()).unwrap_or(32.0) as usize,
                history: VecDeque::new(),
                sent: 0,
            },
        );
        n.dirty = true;
        Ok(Value::Nil)
    }),
    // tlink::subscribe(node, topic, schema, {reliable, deadline})
    ("subscribe", |_, a| {
        let n = node(&a)?;
        let topic = text(&a, 1, "the topic name")?;
        let schema = Schema::parse(a.get(2).unwrap_or(&Value::Nil))?;
        let o = a.get(3);
        let mut n = n.borrow_mut();
        if let Some(s) = n.subs.get(&topic) {
            if s.schema.hash() != schema.hash() {
                return Err(format!(
                    "link: '{topic}' is already subscribed as {}",
                    s.schema.canon()
                ));
            }
            return Ok(Value::Nil);
        }
        n.subs.insert(
            topic,
            Subscriber {
                schema,
                reliable: opt(o, "reliable").map(|v| v.truthy()).unwrap_or(false),
                deadline: opt(o, "deadline").as_ref().and_then(secs),
                last_rx: None,
                missed: false,
                streams: HashMap::new(),
                received: 0,
                lost: 0,
            },
        );
        n.dirty = true;
        Ok(Value::Nil)
    }),
    ("publish", |i, a| {
        let n = node(&a)?;
        let topic = text(&a, 1, "the topic name")?;
        let v = a.get(2).cloned().unwrap_or(Value::Nil);
        let now = i.now();
        n.borrow_mut().publish(&topic, &v, now)?;
        Ok(Value::Nil)
    }),
    ("serve", |_, a| {
        let n = node(&a)?;
        let name = text(&a, 1, "the service name")?;
        let mut n = n.borrow_mut();
        n.services.insert(name, ());
        n.dirty = true;
        Ok(Value::Nil)
    }),
    // tlink::request(node, service, value) -> request id, or nil if nobody serves it yet
    ("request", |i, a| {
        let n = node(&a)?;
        let service = text(&a, 1, "the service name")?;
        let v = a.get(2).cloned().unwrap_or(Value::Nil);
        let now = i.now();
        let mut n = n.borrow_mut();
        n.next_req += 1;
        let req = n.next_req;
        if n.services.contains_key(&service) {
            let (id, name) = (n.id, n.name.clone());
            n.events.push(event(
                "request",
                vec![
                    ("service", Value::str(service)),
                    ("id", Value::Int(req as i64)),
                    ("from", Value::Int(id as i64)),
                    ("peer", Value::str(name)),
                    ("value", v),
                ],
            ));
            return Ok(Value::Int(req as i64));
        }
        let target = n
            .peers
            .values()
            .find(|p| {
                p.endpoints
                    .iter()
                    .any(|e| e.role == 2 && e.topic == service)
            })
            .map(|p| p.addr);
        let Some(addr) = target else {
            return Ok(Value::Nil);
        };
        let mut w = n.header(REQUEST);
        w.str(&service);
        w.u64(req);
        w.value(&v)?;
        n.pending.insert(req, (addr, w.0.clone(), now, 1));
        n.send(Some(addr), now, w.0);
        Ok(Value::Int(req as i64))
    }),
    // tlink::reply(node, from, id, ok, value)
    ("reply", |i, a| {
        let n = node(&a)?;
        let (Some(Value::Int(from)), Some(Value::Int(req))) = (a.get(1), a.get(2)) else {
            return Err("link: reply(node, from, id, ok, value)".into());
        };
        let ok = a.get(3).map(|v| v.truthy()).unwrap_or(true);
        let v = a.get(4).cloned().unwrap_or(Value::Nil);
        let now = i.now();
        let mut n = n.borrow_mut();
        if *from as u64 == n.id {
            n.events.push(event(
                "reply",
                vec![
                    ("id", Value::Int(*req)),
                    ("ok", Value::Bool(ok)),
                    ("value", v),
                ],
            ));
            return Ok(Value::Nil);
        }
        let Some(addr) = n.peers.get(&(*from as u64)).map(|p| p.addr) else {
            return Ok(Value::Nil);
        };
        let mut w = n.header(REPLY);
        w.u64(*req as u64);
        w.u8(ok as u8);
        w.value(&v)?;
        let key = (*from as u64, *req as u64);
        n.in_progress.remove(&key);
        n.answered.push_back((key, w.0.clone()));
        if n.answered.len() > 64 {
            n.answered.pop_front();
        }
        n.send(Some(addr), now, w.0);
        Ok(Value::Nil)
    }),
    // tlink::poll(node) -> list of events since the last poll
    ("poll", |i, a| {
        let n = node(&a)?;
        let now = i.now();
        let ev = n.borrow_mut().pump(now);
        Ok(Value::list(ev))
    }),
    ("info", |_, a| Ok(node(&a)?.borrow().info())),
    ("close", |i, a| {
        let n = node(&a)?;
        let now = i.now();
        let mut n = n.borrow_mut();
        if !n.closed {
            let w = n.header(BYE);
            n.send(None, now, w.0);
            n.closed = true;
            if let Transport::Sim(addr) = n.transport {
                SIM.with(|s| s.borrow_mut().nodes.remove(&addr));
            }
        }
        Ok(Value::Nil)
    }),
    // tlink::sim({latency, jitter, loss, seed}) configures the in-process network
    ("sim", |_, a| {
        let o = a.first();
        SIM.with(|s| {
            let mut s = s.borrow_mut();
            if let Some(v) = opt(o, "latency").as_ref().and_then(secs) {
                s.latency = v.max(0.0);
            }
            if let Some(v) = opt(o, "jitter").as_ref().and_then(secs) {
                s.jitter = v.max(0.0);
            }
            if let Some(v) = opt(o, "loss").and_then(|v| v.as_f64()) {
                s.loss = v.clamp(0.0, 1.0);
            }
            if let Some(v) = opt(o, "seed").and_then(|v| v.as_f64()) {
                s.rng = v as u64 ^ 0x9e37_79b9_7f4a_7c15;
            }
            let mut d = Dict::default();
            let _ = d.set(Value::str("sent"), Value::Int(s.sent as i64));
            let _ = d.set(Value::str("dropped"), Value::Int(s.dropped as i64));
            let _ = d.set(Value::str("in_flight"), Value::Int(s.queue.len() as i64));
            Ok(Value::dict(d))
        })
    }),
    // tlink::partition(node_a, node_b, cut = true): drop everything between two sim nodes
    ("partition", |_, a| {
        let x = node(&a)?;
        let y = node(&a[1.min(a.len())..])?;
        let cut = a.get(2).map(|v| v.truthy()).unwrap_or(true);
        let sim_addr = |n: &Rc<RefCell<Node>>| match n.borrow().transport {
            Transport::Sim(a) => Ok(a),
            _ => Err("link: partition() only applies to sim nodes".to_string()),
        };
        let (p, q) = (sim_addr(&x)?, sim_addr(&y)?);
        let pair = (p.min(q), p.max(q));
        SIM.with(|s| {
            let mut s = s.borrow_mut();
            if cut {
                s.cut.insert(pair);
            } else {
                s.cut.remove(&pair);
            }
        });
        Ok(Value::Nil)
    }),
];

//! Bluetooth Low Energy: a GATT client speaking ATT over L2CAP (Linux, no BlueZ D-Bus needed),
//! HCI scanning, and a simulated GATT server that speaks the same ATT protocol, so the client's
//! discovery/read/write/notify paths are exercised in tests without hardware.

use super::tape::Taped;
use super::{Backend, SimBus};
use crate::builtins::int;
use crate::interp::Interpreter;
use crate::value::{Dict, Native, Value};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;

// ---------------------------------------------------------------------------- UUIDs

const BASE_SUFFIX: &str = "-0000-1000-8000-00805f9b34fb";

const NAMES: &[(&str, &str)] = &[
    ("generic_access", "1800"),
    ("device_name", "2a00"),
    ("appearance", "2a01"),
    ("device_information", "180a"),
    ("manufacturer_name", "2a29"),
    ("model_number", "2a24"),
    ("firmware_revision", "2a26"),
    ("battery_service", "180f"),
    ("battery_level", "2a19"),
    ("heart_rate", "180d"),
    ("heart_rate_measurement", "2a37"),
    ("environmental_sensing", "181a"),
    ("temperature", "2a6e"),
    ("humidity", "2a6f"),
    ("pressure", "2a6d"),
    ("nordic_uart", "6e400001-b5a3-f393-e0a9-e50e24dcca9e"),
    ("nordic_uart_rx", "6e400002-b5a3-f393-e0a9-e50e24dcca9e"),
    ("nordic_uart_tx", "6e400003-b5a3-f393-e0a9-e50e24dcca9e"),
];

/// Canonical form: 4 hex digits for SIG base UUIDs, otherwise the full lowercase 128-bit form.
pub fn normalize_uuid(s: &str) -> Result<String, String> {
    let s = s.trim().to_lowercase();
    if let Some((_, u)) = NAMES.iter().find(|(n, _)| *n == s) {
        return Ok(u.to_string());
    }
    let s = s.trim_start_matches("0x").to_string();
    if s.len() == 4 && s.chars().all(|c| c.is_ascii_hexdigit()) {
        return Ok(s);
    }
    if s.len() == 8 && s.chars().all(|c| c.is_ascii_hexdigit()) && s.starts_with("0000") {
        return Ok(s[4..].to_string());
    }
    if s.len() == 36 && s.chars().filter(|c| *c == '-').count() == 4 {
        if s.starts_with("0000") && s.ends_with(BASE_SUFFIX) {
            return Ok(s[4..8].to_string());
        }
        return Ok(s);
    }
    Err(format!("invalid BLE UUID '{s}' (use e.g. \"2a19\", a 128-bit UUID, or a name like \"battery_level\")"))
}

fn uuid_from_bytes(b: &[u8]) -> String {
    match b.len() {
        2 => format!("{:04x}", u16::from_le_bytes([b[0], b[1]])),
        16 => {
            let r: Vec<u8> = b.iter().rev().copied().collect();
            let h: String = r.iter().map(|x| format!("{x:02x}")).collect();
            let full = format!(
                "{}-{}-{}-{}-{}",
                &h[0..8],
                &h[8..12],
                &h[12..16],
                &h[16..20],
                &h[20..32]
            );
            normalize_uuid(&full).unwrap_or(full)
        }
        _ => "?".into(),
    }
}

fn uuid_to_bytes(u: &str) -> Vec<u8> {
    if u.len() == 4 {
        return u16::from_str_radix(u, 16)
            .unwrap_or(0)
            .to_le_bytes()
            .to_vec();
    }
    let h: String = u.chars().filter(|c| *c != '-').collect();
    let mut b: Vec<u8> = (0..16)
        .map(|i| u8::from_str_radix(&h[i * 2..i * 2 + 2], 16).unwrap_or(0))
        .collect();
    b.reverse();
    b
}

fn le16(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}

fn att_error_name(code: u8) -> &'static str {
    match code {
        0x01 => "invalid handle",
        0x02 => "read not permitted",
        0x03 => "write not permitted",
        0x05 => "insufficient authentication",
        0x06 => "request not supported",
        0x07 => "invalid offset",
        0x0A => "attribute not found",
        0x0D => "invalid attribute value length",
        0x0F => "insufficient encryption",
        _ => "ATT error",
    }
}

// ---------------------------------------------------------------------------- transport + client

pub trait AttTransport {
    fn send(&mut self, pdu: &[u8]) -> Result<(), String>;
    fn recv(&mut self, timeout_ms: u64) -> Result<Option<Vec<u8>>, String>;
}

#[derive(Clone, Debug)]
pub struct Characteristic {
    pub uuid: String,
    pub service: String,
    pub decl: u16,
    pub value: u16,
    pub props: u8,
    pub cccd: Option<u16>,
}

pub struct Gatt {
    t: Box<dyn AttTransport>,
    pub mtu: u16,
    pub services: Vec<(String, u16, u16)>,
    pub chars: Vec<Characteristic>,
    notes: VecDeque<(u16, Vec<u8>)>,
}

type AttResult = Result<Result<Vec<u8>, u8>, String>;

impl Gatt {
    pub fn new(t: Box<dyn AttTransport>) -> Result<Self, String> {
        let mut g = Gatt {
            t,
            mtu: 23,
            services: vec![],
            chars: vec![],
            notes: VecDeque::new(),
        };
        let mut req = vec![0x02];
        req.extend_from_slice(&247u16.to_le_bytes());
        if let Ok(rsp) = g.request(&req)? {
            if rsp.len() >= 3 && rsp[0] == 0x03 {
                g.mtu = le16(&rsp, 1).clamp(23, 247);
            }
        }
        g.discover()?;
        Ok(g)
    }

    /// Sends a request and waits for its response, queueing notifications that arrive meanwhile.
    fn request(&mut self, pdu: &[u8]) -> AttResult {
        self.t.send(pdu)?;
        loop {
            let Some(rsp) = self.t.recv(3000)? else {
                return Err(format!(
                    "BLE: no response to ATT request 0x{:02x} (timeout)",
                    pdu[0]
                ));
            };
            match rsp.first() {
                Some(0x1B) if rsp.len() >= 3 => {
                    self.notes.push_back((le16(&rsp, 1), rsp[3..].to_vec()))
                }
                Some(0x1D) if rsp.len() >= 3 => {
                    self.notes.push_back((le16(&rsp, 1), rsp[3..].to_vec()));
                    self.t.send(&[0x1E])?;
                }
                Some(0x01) if rsp.len() >= 5 => return Ok(Err(rsp[4])),
                Some(_) => return Ok(Ok(rsp)),
                None => {}
            }
        }
    }

    fn discover(&mut self) -> Result<(), String> {
        // primary services (Read By Group Type, 0x2800)
        let mut start = 1u16;
        loop {
            let mut req = vec![0x10];
            req.extend_from_slice(&start.to_le_bytes());
            req.extend_from_slice(&0xFFFFu16.to_le_bytes());
            req.extend_from_slice(&0x2800u16.to_le_bytes());
            let rsp = match self.request(&req)? {
                Ok(r) if r.first() == Some(&0x11) && r.len() > 2 => r,
                _ => break,
            };
            let len = rsp[1] as usize;
            let mut last = start;
            for e in rsp[2..].chunks(len).filter(|c| c.len() == len) {
                let (s, end) = (le16(e, 0), le16(e, 2));
                self.services.push((uuid_from_bytes(&e[4..]), s, end));
                last = end;
            }
            if last == 0xFFFF || last < start {
                break;
            }
            start = last + 1;
        }
        // characteristics per service (Read By Type, 0x2803)
        for (suuid, sstart, send) in self.services.clone() {
            let mut start = sstart;
            while start <= send {
                let mut req = vec![0x08];
                req.extend_from_slice(&start.to_le_bytes());
                req.extend_from_slice(&send.to_le_bytes());
                req.extend_from_slice(&0x2803u16.to_le_bytes());
                let rsp = match self.request(&req)? {
                    Ok(r) if r.first() == Some(&0x09) && r.len() > 2 => r,
                    _ => break,
                };
                let len = rsp[1] as usize;
                let mut last = start;
                for e in rsp[2..].chunks(len).filter(|c| c.len() == len && len >= 7) {
                    let decl = le16(e, 0);
                    self.chars.push(Characteristic {
                        uuid: uuid_from_bytes(&e[5..]),
                        service: suuid.clone(),
                        decl,
                        props: e[2],
                        value: le16(e, 3),
                        cccd: None,
                    });
                    last = decl;
                }
                if last == u16::MAX || last < start {
                    break;
                }
                start = last + 1;
            }
        }
        // client characteristic configuration descriptors (Find Information)
        for i in 0..self.chars.len() {
            let c = self.chars[i].clone();
            if c.props & 0x30 == 0 {
                continue; // no notify/indicate
            }
            let svc_end = self
                .services
                .iter()
                .find(|s| s.0 == c.service && s.1 <= c.decl && c.decl <= s.2)
                .map(|s| s.2)
                .unwrap_or(0xFFFF);
            let next_decl = self
                .chars
                .iter()
                .filter(|o| o.decl > c.decl)
                .map(|o| o.decl)
                .min()
                .unwrap_or(svc_end + 1);
            let (s, e) = (c.value + 1, (next_decl - 1).min(svc_end));
            if s > e {
                continue;
            }
            let mut req = vec![0x04];
            req.extend_from_slice(&s.to_le_bytes());
            req.extend_from_slice(&e.to_le_bytes());
            if let Ok(rsp) = self.request(&req)? {
                if rsp.first() == Some(&0x05) && rsp.len() > 2 {
                    let step = if rsp[1] == 1 { 4 } else { 18 };
                    for d in rsp[2..].chunks(step).filter(|d| d.len() == step) {
                        if step == 4 && le16(d, 2) == 0x2902 {
                            self.chars[i].cccd = Some(le16(d, 0));
                        }
                    }
                }
            }
        }
        Ok(())
    }

    pub fn find(&self, uuid: &str) -> Result<Characteristic, String> {
        let u = normalize_uuid(uuid)?;
        self.chars
            .iter()
            .find(|c| c.uuid == u)
            .cloned()
            .ok_or_else(|| format!("BLE: device has no characteristic {u}"))
    }

    pub fn read(&mut self, uuid: &str) -> Result<Vec<u8>, String> {
        let c = self.find(uuid)?;
        let mut req = vec![0x0A];
        req.extend_from_slice(&c.value.to_le_bytes());
        let mut out = match self.request(&req)? {
            Ok(r) if r.first() == Some(&0x0B) => r[1..].to_vec(),
            Ok(r) => return Err(format!("BLE: unexpected response 0x{:02x}", r[0])),
            Err(code) => return Err(format!("BLE read {}: {}", c.uuid, att_error_name(code))),
        };
        // long values: Read Blob until a short chunk
        while out.len() >= (self.mtu - 1) as usize && out.len() < 512 {
            let mut req = vec![0x0C];
            req.extend_from_slice(&c.value.to_le_bytes());
            req.extend_from_slice(&(out.len() as u16).to_le_bytes());
            match self.request(&req)? {
                Ok(r) if r.first() == Some(&0x0D) && r.len() > 1 => {
                    let short = r.len() - 1 < (self.mtu - 1) as usize;
                    out.extend_from_slice(&r[1..]);
                    if short {
                        break;
                    }
                }
                _ => break,
            }
        }
        Ok(out)
    }

    pub fn write(&mut self, uuid: &str, data: &[u8], response: bool) -> Result<(), String> {
        let c = self.find(uuid)?;
        self.write_handle(c.value, data, response)
            .map_err(|e| format!("BLE write {}: {e}", c.uuid))
    }

    fn write_handle(&mut self, handle: u16, data: &[u8], response: bool) -> Result<(), String> {
        if data.len() > (self.mtu - 3) as usize {
            return Err(format!(
                "value of {} bytes exceeds MTU payload {}",
                data.len(),
                self.mtu - 3
            ));
        }
        let mut req = vec![if response { 0x12 } else { 0x52 }];
        req.extend_from_slice(&handle.to_le_bytes());
        req.extend_from_slice(data);
        if !response {
            return self.t.send(&req);
        }
        match self.request(&req)? {
            Ok(r) if r.first() == Some(&0x13) => Ok(()),
            Ok(r) => Err(format!("unexpected response 0x{:02x}", r[0])),
            Err(code) => Err(att_error_name(code).into()),
        }
    }

    pub fn subscribe(&mut self, uuid: &str, on: bool) -> Result<(), String> {
        let c = self.find(uuid)?;
        let cccd = c.cccd.ok_or_else(|| {
            format!(
                "BLE: characteristic {} does not support notifications",
                c.uuid
            )
        })?;
        let v: u16 = if !on {
            0
        } else if c.props & 0x10 != 0 {
            1
        } else {
            2
        };
        self.write_handle(cccd, &v.to_le_bytes(), true)
    }

    /// Notifications received so far plus any arriving within `timeout_ms`.
    pub fn poll(&mut self, timeout_ms: u64) -> Result<Vec<(String, Vec<u8>)>, String> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
        loop {
            let left = deadline
                .saturating_duration_since(std::time::Instant::now())
                .as_millis() as u64;
            match self.t.recv(if self.notes.is_empty() { left } else { 0 })? {
                Some(p) if p.first() == Some(&0x1B) && p.len() >= 3 => {
                    self.notes.push_back((le16(&p, 1), p[3..].to_vec()))
                }
                Some(p) if p.first() == Some(&0x1D) && p.len() >= 3 => {
                    self.notes.push_back((le16(&p, 1), p[3..].to_vec()));
                    self.t.send(&[0x1E])?;
                }
                Some(_) => {}
                None => break,
            }
        }
        let chars = self.chars.clone();
        Ok(self
            .notes
            .drain(..)
            .map(|(h, d)| {
                (
                    chars
                        .iter()
                        .find(|c| c.value == h)
                        .map(|c| c.uuid.clone())
                        .unwrap_or_else(|| format!("handle:{h}")),
                    d,
                )
            })
            .collect())
    }
}

// ---------------------------------------------------------------------------- simulated GATT server

struct Attr {
    handle: u16,
    typ: Vec<u8>,
    value: Vec<u8>,
}

/// (service uuid, [(characteristic uuid, initial value)])
pub type ServiceSpec = (String, Vec<(String, Vec<u8>)>);

#[derive(Default)]
pub struct SimGatt {
    pub name: String,
    pub rssi: i64,
    attrs: Vec<Attr>,
    services: Vec<(u16, u16)>,
    chars: HashMap<String, (u16, u16)>, // uuid -> (value handle, cccd handle)
    notify: HashSet<u16>,
    pub outbox: VecDeque<Vec<u8>>,
    pub written: HashMap<String, Vec<u8>>,
    mtu: u16,
}

impl SimGatt {
    /// services: [(service uuid, [(char uuid, initial value)])]
    pub fn new(name: &str, rssi: i64, services: Vec<ServiceSpec>) -> Self {
        let mut g = SimGatt {
            name: name.into(),
            rssi,
            mtu: 185,
            ..Default::default()
        };
        let mut h = 1u16;
        // generic access with the device name
        let mut all = vec![(
            "1800".to_string(),
            vec![("2a00".to_string(), name.as_bytes().to_vec())],
        )];
        all.extend(services);
        for (suuid, chars) in all {
            let start = h;
            g.attrs.push(Attr {
                handle: h,
                typ: 0x2800u16.to_le_bytes().to_vec(),
                value: uuid_to_bytes(&suuid),
            });
            h += 1;
            for (cuuid, val) in chars {
                let props: u8 = 0x02 | 0x04 | 0x08 | 0x10; // read, write w/o rsp, write, notify
                let mut decl = vec![props];
                decl.extend_from_slice(&(h + 1).to_le_bytes());
                decl.extend_from_slice(&uuid_to_bytes(&cuuid));
                g.attrs.push(Attr {
                    handle: h,
                    typ: 0x2803u16.to_le_bytes().to_vec(),
                    value: decl,
                });
                g.attrs.push(Attr {
                    handle: h + 1,
                    typ: uuid_to_bytes(&cuuid),
                    value: val,
                });
                g.attrs.push(Attr {
                    handle: h + 2,
                    typ: 0x2902u16.to_le_bytes().to_vec(),
                    value: vec![0, 0],
                });
                g.chars.insert(cuuid, (h + 1, h + 2));
                h += 3;
            }
            g.services.push((start, h - 1));
        }
        g
    }

    fn err(op: u8, handle: u16, code: u8) -> Vec<u8> {
        let mut v = vec![0x01, op];
        v.extend_from_slice(&handle.to_le_bytes());
        v.push(code);
        v
    }

    /// Handles one ATT PDU from the client; responses go to `outbox`.
    pub fn handle(&mut self, p: &[u8]) {
        if p.is_empty() {
            return;
        }
        let rsp = match p[0] {
            0x02 => {
                let mut v = vec![0x03];
                v.extend_from_slice(&self.mtu.to_le_bytes());
                if p.len() >= 3 {
                    self.mtu = self.mtu.min(le16(p, 1)).max(23);
                }
                v
            }
            0x10 | 0x08 | 0x04 if p.len() >= 5 => {
                let (s, e) = (le16(p, 1), le16(p, 3));
                let room = self.mtu as usize - 2;
                match p[0] {
                    0x10 => {
                        let mut out = vec![0x11, 0];
                        let mut len = 0;
                        for &(gs, ge) in &self.services {
                            if gs < s || gs > e {
                                continue;
                            }
                            let u = &self.attrs.iter().find(|a| a.handle == gs).unwrap().value;
                            let l = 4 + u.len();
                            if len == 0 {
                                len = l;
                            }
                            if l != len || out.len() - 2 + l > room {
                                break;
                            }
                            out.extend_from_slice(&gs.to_le_bytes());
                            out.extend_from_slice(&ge.to_le_bytes());
                            out.extend_from_slice(u);
                        }
                        if len == 0 {
                            Self::err(0x10, s, 0x0A)
                        } else {
                            out[1] = len as u8;
                            out
                        }
                    }
                    0x08 => {
                        let want = &p[5..];
                        let mut out = vec![0x09, 0];
                        let mut len = 0;
                        for a in self
                            .attrs
                            .iter()
                            .filter(|a| a.handle >= s && a.handle <= e && a.typ == want)
                        {
                            let l = 2 + a.value.len();
                            if len == 0 {
                                len = l;
                            }
                            if l != len || out.len() - 2 + l > room {
                                break;
                            }
                            out.extend_from_slice(&a.handle.to_le_bytes());
                            out.extend_from_slice(&a.value);
                        }
                        if len == 0 {
                            Self::err(0x08, s, 0x0A)
                        } else {
                            out[1] = len as u8;
                            out
                        }
                    }
                    _ => {
                        let mut out = vec![0x05, 0];
                        let mut fmt = 0;
                        for a in self.attrs.iter().filter(|a| a.handle >= s && a.handle <= e) {
                            let f = if a.typ.len() == 2 { 1 } else { 2 };
                            if fmt == 0 {
                                fmt = f;
                            }
                            if f != fmt || out.len() + 2 + a.typ.len() > room {
                                break;
                            }
                            out.extend_from_slice(&a.handle.to_le_bytes());
                            out.extend_from_slice(&a.typ);
                        }
                        if fmt == 0 {
                            Self::err(0x04, s, 0x0A)
                        } else {
                            out[1] = fmt;
                            out
                        }
                    }
                }
            }
            0x0A | 0x0C if p.len() >= 3 => {
                let h = le16(p, 1);
                let off = if p[0] == 0x0C && p.len() >= 5 {
                    le16(p, 3) as usize
                } else {
                    0
                };
                match self.attrs.iter().find(|a| a.handle == h) {
                    Some(a) if off <= a.value.len() => {
                        let mut v = vec![if p[0] == 0x0A { 0x0B } else { 0x0D }];
                        let end = (off + self.mtu as usize - 1).min(a.value.len());
                        v.extend_from_slice(&a.value[off..end]);
                        v
                    }
                    Some(_) => Self::err(p[0], h, 0x07),
                    None => Self::err(p[0], h, 0x01),
                }
            }
            0x12 | 0x52 if p.len() >= 3 => {
                let h = le16(p, 1);
                let data = p[3..].to_vec();
                let cccd_of = self.chars.values().find(|(_, c)| *c == h).map(|(v, _)| *v);
                match self.attrs.iter_mut().find(|a| a.handle == h) {
                    Some(a) => {
                        a.value = data.clone();
                        if let Some(vh) = cccd_of {
                            if data.first().copied().unwrap_or(0) & 0x03 != 0 {
                                self.notify.insert(vh);
                            } else {
                                self.notify.remove(&vh);
                            }
                        } else if let Some((u, _)) = self.chars.iter().find(|(_, (v, _))| *v == h) {
                            self.written.insert(u.clone(), data);
                        }
                        if p[0] == 0x52 {
                            return;
                        }
                        vec![0x13]
                    }
                    None if p[0] == 0x52 => return,
                    None => Self::err(0x12, h, 0x01),
                }
            }
            0x1E => return,                 // indication confirmation
            op if op & 0x40 != 0 => return, // commands get no error response
            op => Self::err(op, 0, 0x06),
        };
        self.outbox.push_back(rsp);
    }

    /// Updates a characteristic and notifies the client if it subscribed.
    pub fn notify(&mut self, uuid: &str, data: Vec<u8>) -> Result<(), String> {
        let (vh, _) = *self
            .chars
            .get(uuid)
            .ok_or_else(|| format!("simulated device has no characteristic {uuid}"))?;
        if let Some(a) = self.attrs.iter_mut().find(|a| a.handle == vh) {
            a.value = data.clone();
        }
        if self.notify.contains(&vh) {
            let mut p = vec![0x1B];
            p.extend_from_slice(&vh.to_le_bytes());
            p.extend_from_slice(&data);
            self.outbox.push_back(p);
        }
        Ok(())
    }
}

struct SimAtt {
    bus: Rc<RefCell<SimBus>>,
    addr: String,
}

impl AttTransport for SimAtt {
    fn send(&mut self, pdu: &[u8]) -> Result<(), String> {
        let mut b = self.bus.borrow_mut();
        let dev = b
            .ble
            .get_mut(&self.addr)
            .ok_or_else(|| format!("BLE: simulated device {} disappeared", self.addr))?;
        dev.handle(pdu);
        Ok(())
    }
    fn recv(&mut self, _timeout_ms: u64) -> Result<Option<Vec<u8>>, String> {
        let mut b = self.bus.borrow_mut();
        Ok(b.ble.get_mut(&self.addr).and_then(|d| d.outbox.pop_front()))
    }
}

impl AttTransport for Taped<Box<dyn AttTransport>> {
    fn send(&mut self, pdu: &[u8]) -> Result<(), String> {
        self.tap_dyn("att", pdu, |d| d.send(pdu).map(|_| vec![]))
            .map(|_| ())
    }
    fn recv(&mut self, timeout_ms: u64) -> Result<Option<Vec<u8>>, String> {
        let r = self.tap_dyn("recv", &[], |d| {
            d.recv(timeout_ms).map(|o| o.unwrap_or_default())
        })?;
        Ok(if r.is_empty() { None } else { Some(r) })
    }
}

// ---------------------------------------------------------------------------- Linux transports

#[cfg(target_os = "linux")]
mod bluez {
    use super::AttTransport;
    use std::os::unix::io::RawFd;

    const AF_BLUETOOTH: i32 = 31;
    const BTPROTO_L2CAP: i32 = 0;
    const BTPROTO_HCI: i32 = 1;
    const ATT_CID: u16 = 4;
    const BDADDR_LE_PUBLIC: u8 = 1;
    const BDADDR_LE_RANDOM: u8 = 2;

    #[repr(C)]
    struct SockaddrL2 {
        family: u16,
        psm: u16,
        bdaddr: [u8; 6],
        cid: u16,
        bdaddr_type: u8,
    }

    #[repr(C)]
    struct SockaddrHci {
        family: u16,
        dev: u16,
        channel: u16,
    }

    #[repr(C)]
    struct HciFilter {
        type_mask: u32,
        event_mask: [u32; 2],
        opcode: u16,
    }

    fn err(what: &str) -> String {
        format!("{what}: {}", std::io::Error::last_os_error())
    }

    pub fn parse_addr(s: &str) -> Result<[u8; 6], String> {
        let parts: Vec<&str> = s.split(':').collect();
        if parts.len() != 6 {
            return Err(format!(
                "invalid Bluetooth address '{s}' (expected AA:BB:CC:DD:EE:FF)"
            ));
        }
        let mut a = [0u8; 6];
        for (i, p) in parts.iter().enumerate() {
            // bdaddr_t is little-endian: reverse the printed order
            a[5 - i] = u8::from_str_radix(p, 16)
                .map_err(|_| format!("invalid Bluetooth address '{s}'"))?;
        }
        Ok(a)
    }

    fn poll_in(fd: RawFd, timeout_ms: u64) -> Result<bool, String> {
        let mut pfd = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: single valid pollfd.
        let r = unsafe { libc::poll(&mut pfd, 1, timeout_ms.min(i32::MAX as u64) as i32) };
        if r < 0 {
            return Err(err("poll"));
        }
        Ok(r > 0)
    }

    /// ATT bearer: an L2CAP LE connection-oriented socket on the fixed ATT channel (CID 4).
    pub struct L2capAtt {
        fd: RawFd,
    }

    impl Drop for L2capAtt {
        fn drop(&mut self) {
            // SAFETY: fd owned by this struct.
            unsafe { libc::close(self.fd) };
        }
    }

    impl L2capAtt {
        pub fn connect(addr: &str, random: bool) -> Result<Self, String> {
            let bd = parse_addr(addr)?;
            // SAFETY: plain socket calls with correctly sized sockaddr_l2 structs.
            unsafe {
                let fd = libc::socket(
                    AF_BLUETOOTH,
                    libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC,
                    BTPROTO_L2CAP,
                );
                if fd < 0 {
                    return Err(err(
                        "Bluetooth socket (is the kernel built with Bluetooth support?)",
                    ));
                }
                let local = SockaddrL2 {
                    family: AF_BLUETOOTH as u16,
                    psm: 0,
                    bdaddr: [0; 6],
                    cid: ATT_CID.to_le(),
                    bdaddr_type: BDADDR_LE_PUBLIC,
                };
                let sz = std::mem::size_of::<SockaddrL2>() as u32;
                if libc::bind(fd, &local as *const SockaddrL2 as *const libc::sockaddr, sz) < 0 {
                    let e = err("bind L2CAP");
                    libc::close(fd);
                    return Err(e);
                }
                let remote = SockaddrL2 {
                    family: AF_BLUETOOTH as u16,
                    psm: 0,
                    bdaddr: bd,
                    cid: ATT_CID.to_le(),
                    bdaddr_type: if random {
                        BDADDR_LE_RANDOM
                    } else {
                        BDADDR_LE_PUBLIC
                    },
                };
                if libc::connect(
                    fd,
                    &remote as *const SockaddrL2 as *const libc::sockaddr,
                    sz,
                ) < 0
                {
                    let e = err(&format!("connect to {addr}"));
                    libc::close(fd);
                    return Err(e);
                }
                Ok(L2capAtt { fd })
            }
        }
    }

    impl AttTransport for L2capAtt {
        fn send(&mut self, pdu: &[u8]) -> Result<(), String> {
            // SAFETY: writes the PDU buffer.
            let n = unsafe { libc::write(self.fd, pdu.as_ptr() as *const libc::c_void, pdu.len()) };
            if n < 0 {
                return Err(err("BLE send"));
            }
            Ok(())
        }
        fn recv(&mut self, timeout_ms: u64) -> Result<Option<Vec<u8>>, String> {
            if !poll_in(self.fd, timeout_ms)? {
                return Ok(None);
            }
            let mut buf = [0u8; 512];
            // SAFETY: reads into a local buffer.
            let n =
                unsafe { libc::read(self.fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
            if n <= 0 {
                return Err(err("BLE connection lost"));
            }
            Ok(Some(buf[..n as usize].to_vec()))
        }
    }

    /// Active LE scan through a raw HCI socket (needs CAP_NET_RAW/CAP_NET_ADMIN, e.g. root).
    pub fn scan(seconds: f64) -> Result<Vec<(String, String, i64)>, String> {
        // SAFETY: raw HCI socket I/O with correctly laid-out structs and command packets.
        unsafe {
            let fd = libc::socket(
                AF_BLUETOOTH,
                libc::SOCK_RAW | libc::SOCK_CLOEXEC,
                BTPROTO_HCI,
            );
            if fd < 0 {
                return Err(err("HCI socket"));
            }
            let addr = SockaddrHci {
                family: AF_BLUETOOTH as u16,
                dev: 0,
                channel: 0,
            };
            if libc::bind(
                fd,
                &addr as *const SockaddrHci as *const libc::sockaddr,
                std::mem::size_of::<SockaddrHci>() as u32,
            ) < 0
            {
                let e = err("bind hci0 (scanning needs root or CAP_NET_RAW)");
                libc::close(fd);
                return Err(e);
            }
            let filter = HciFilter {
                type_mask: 1 << 4,
                event_mask: [(1 << 0x0E) | (1 << 0x0F), 1 << (0x3E - 32)],
                opcode: 0,
            };
            libc::setsockopt(
                fd,
                0,
                2,
                &filter as *const HciFilter as *const libc::c_void,
                std::mem::size_of::<HciFilter>() as u32,
            );
            let cmd = |opcode: u16, params: &[u8]| {
                let mut p = vec![0x01];
                p.extend_from_slice(&opcode.to_le_bytes());
                p.push(params.len() as u8);
                p.extend_from_slice(params);
                libc::write(fd, p.as_ptr() as *const libc::c_void, p.len());
            };
            cmd(0x200C, &[0x00, 0x00]); // scan off (in case it was on)
            cmd(0x200B, &[0x01, 0x10, 0x00, 0x10, 0x00, 0x00, 0x00]); // active, 10 ms interval/window
            cmd(0x200C, &[0x01, 0x00]); // scan on, keep duplicates (for RSSI updates)
            let mut found: Vec<(String, String, i64)> = Vec::new();
            let end =
                std::time::Instant::now() + std::time::Duration::from_secs_f64(seconds.max(0.1));
            loop {
                let left = end
                    .saturating_duration_since(std::time::Instant::now())
                    .as_millis() as u64;
                if left == 0 || !poll_in(fd, left)? {
                    break;
                }
                let mut buf = [0u8; 260];
                let n = libc::read(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len());
                if n < 4 || buf[0] != 0x04 || buf[1] != 0x3E || buf[3] != 0x02 {
                    continue;
                }
                let b = &buf[..n as usize];
                let mut i = 5; // after num_reports
                for _ in 0..b[4] {
                    if i + 9 > b.len() {
                        break;
                    }
                    let a = &b[i + 2..i + 8];
                    let address = format!(
                        "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
                        a[5], a[4], a[3], a[2], a[1], a[0]
                    );
                    let dlen = b[i + 8] as usize;
                    let data = &b[(i + 9).min(b.len())..(i + 9 + dlen).min(b.len())];
                    let rssi = b.get(i + 9 + dlen).map(|r| *r as i8 as i64).unwrap_or(0);
                    let mut name = String::new();
                    let mut k = 0;
                    while k + 1 < data.len() {
                        let l = data[k] as usize;
                        if l == 0 || k + 1 + l > data.len() {
                            break;
                        }
                        if data[k + 1] == 0x08 || data[k + 1] == 0x09 {
                            name = String::from_utf8_lossy(&data[k + 2..k + 1 + l]).to_string();
                        }
                        k += 1 + l;
                    }
                    match found.iter_mut().find(|f| f.0 == address) {
                        Some(f) => {
                            f.2 = rssi;
                            if f.1.is_empty() {
                                f.1 = name;
                            }
                        }
                        None => found.push((address, name, rssi)),
                    }
                    i += 10 + dlen;
                }
            }
            cmd(0x200C, &[0x00, 0x00]);
            libc::close(fd);
            Ok(found)
        }
    }
}

// ---------------------------------------------------------------------------- Tulpa object

pub struct Ble {
    addr: String,
    gatt: Option<Gatt>,
}

impl Ble {
    fn g(&mut self) -> Result<&mut Gatt, String> {
        self.gatt
            .as_mut()
            .ok_or_else(|| format!("BLE device {} is disconnected", self.addr))
    }
}

fn props_list(p: u8) -> Value {
    let names = [
        (0x02, "read"),
        (0x04, "write_without_response"),
        (0x08, "write"),
        (0x10, "notify"),
        (0x20, "indicate"),
    ];
    Value::list(
        names
            .iter()
            .filter(|(b, _)| p & b != 0)
            .map(|(_, n)| Value::str(*n))
            .collect(),
    )
}

impl Native for Ble {
    fn type_name(&self) -> &'static str {
        "BLE"
    }
    fn describe(&self) -> String {
        format!(
            "<BLE {}{}>",
            self.addr,
            if self.gatt.is_none() {
                " disconnected"
            } else {
                ""
            }
        )
    }
    fn call_method(&mut self, m: &str, a: &[Value]) -> Result<Value, String> {
        let uuid = |k: usize| {
            a.get(k)
                .map(|v| v.to_string())
                .ok_or_else(|| format!("{m}() needs a characteristic UUID"))
        };
        match m {
            "services" => Ok(Value::list(
                self.g()?
                    .services
                    .iter()
                    .map(|s| Value::str(s.0.as_str()))
                    .collect(),
            )),
            "characteristics" => {
                let chars = self.g()?.chars.clone();
                let mut out = Vec::new();
                for c in chars {
                    let mut d = Dict::default();
                    d.set(Value::str("uuid"), Value::str(c.uuid.as_str()))?;
                    d.set(Value::str("service"), Value::str(c.service.as_str()))?;
                    d.set(Value::str("properties"), props_list(c.props))?;
                    d.set(Value::str("handle"), Value::Int(c.value as i64))?;
                    out.push(Value::dict(d));
                }
                Ok(Value::list(out))
            }
            "read" => Ok(Value::from_bytes(&self.g()?.read(&uuid(0)?)?)),
            "read_text" => Ok(Value::str(
                String::from_utf8_lossy(&self.g()?.read(&uuid(0)?)?).to_string(),
            )),
            "write" => {
                let data = a.get(1).ok_or("write() needs (uuid, data)")?.to_bytes()?;
                let resp = a.get(2).map(|v| v.truthy()).unwrap_or(true);
                self.g()?.write(&uuid(0)?, &data, resp)?;
                Ok(Value::Nil)
            }
            "subscribe" | "unsubscribe" => {
                let u = uuid(0)?;
                self.g()?.subscribe(&u, m == "subscribe")?;
                Ok(Value::Nil)
            }
            "notifications" | "poll" => {
                let t = a
                    .first()
                    .map(|v| int(m, v))
                    .transpose()?
                    .unwrap_or(0)
                    .max(0) as u64;
                let notes = self.g()?.poll(t)?;
                let mut out = Vec::new();
                for (u, d) in notes {
                    let mut dict = Dict::default();
                    dict.set(Value::str("uuid"), Value::str(u))?;
                    dict.set(Value::str("data"), Value::from_bytes(&d))?;
                    out.push(Value::dict(dict));
                }
                Ok(Value::list(out))
            }
            "mtu" => Ok(Value::Int(self.g()?.mtu as i64)),
            "address" => Ok(Value::str(self.addr.as_str())),
            "connected" => Ok(Value::Bool(self.gatt.is_some())),
            "disconnect" => {
                self.gatt = None;
                Ok(Value::Nil)
            }
            _ => Err(format!("BLE has no method '{m}'")),
        }
    }
}

pub fn b_ble_connect(i: &mut Interpreter, a: Vec<Value>) -> Result<Value, String> {
    let addr = a
        .first()
        .map(|v| v.to_string())
        .ok_or("BLE::connect needs an address")?
        .to_uppercase();
    let random = a.get(1).map(|v| v.truthy()).unwrap_or(false);
    let tape = i.tape.clone();
    let replay = tape
        .as_ref()
        .is_some_and(|t| t.borrow().mode == super::TapeMode::Replay);
    let real = |i: &mut Interpreter| -> Result<Box<dyn AttTransport>, String> {
        Ok(match i.backend {
            Backend::Sim => {
                if !i.sim.borrow().ble.contains_key(&addr) {
                    return Err(format!(
                        "BLE: no simulated device at {addr} (create one with sim::ble_device)"
                    ));
                }
                Box::new(SimAtt {
                    bus: i.sim.clone(),
                    addr: addr.clone(),
                })
            }
            Backend::Linux => linux_connect(&addr, random)?,
        })
    };
    let transport: Box<dyn AttTransport> = match tape {
        Some(t) if replay => Box::new(Taped::<Box<dyn AttTransport>> {
            inner: None,
            tape: t,
            dev: format!("ble:{addr}"),
        }),
        Some(t) => Box::new(Taped::<Box<dyn AttTransport>> {
            inner: Some(real(i)?),
            tape: t,
            dev: format!("ble:{addr}"),
        }),
        None => real(i)?,
    };
    let gatt = Gatt::new(transport)?;
    i.sim.borrow_mut().note(format!(
        "ble connected {addr}: {} characteristics",
        gatt.chars.len()
    ));
    Ok(Value::native(Ble {
        addr,
        gatt: Some(gatt),
    }))
}

pub fn b_ble_scan(i: &mut Interpreter, a: Vec<Value>) -> Result<Value, String> {
    let secs = match a.first() {
        Some(v) => crate::ext::seconds_of(v).ok_or("BLE::scan expects seconds or a duration")?,
        None => 3.0,
    };
    let found: Vec<(String, String, i64)> = match i.backend {
        Backend::Sim => i
            .sim
            .borrow()
            .ble
            .iter()
            .map(|(k, d)| (k.clone(), d.name.clone(), d.rssi))
            .collect(),
        Backend::Linux => linux_scan(secs)?,
    };
    let mut found = found;
    found.sort_by_key(|f| std::cmp::Reverse(f.2));
    let mut out = Vec::new();
    for (addr, name, rssi) in found {
        let mut d = Dict::default();
        d.set(Value::str("address"), Value::str(addr))?;
        d.set(Value::str("name"), Value::str(name))?;
        d.set(Value::str("rssi"), Value::Int(rssi))?;
        out.push(Value::dict(d));
    }
    Ok(Value::list(out))
}

#[cfg(target_os = "linux")]
fn linux_connect(addr: &str, random: bool) -> Result<Box<dyn AttTransport>, String> {
    Ok(Box::new(bluez::L2capAtt::connect(addr, random)?))
}
#[cfg(target_os = "linux")]
fn linux_scan(secs: f64) -> Result<Vec<(String, String, i64)>, String> {
    bluez::scan(secs)
}
#[cfg(not(target_os = "linux"))]
fn linux_connect(_: &str, _: bool) -> Result<Box<dyn AttTransport>, String> {
    Err("BLE hardware access is only available on Linux".into())
}
#[cfg(not(target_os = "linux"))]
fn linux_scan(_: f64) -> Result<Vec<(String, String, i64)>, String> {
    Err("BLE hardware access is only available on Linux".into())
}

/// `sim::ble_device(address, name, {service: {characteristic: value}}, rssi = -60)`
pub fn sim_device(i: &mut Interpreter, a: Vec<Value>) -> Result<Value, String> {
    let addr = a
        .first()
        .map(|v| v.to_string())
        .ok_or("sim::ble_device needs an address")?
        .to_uppercase();
    let name = a.get(1).map(|v| v.to_string()).unwrap_or_default();
    let rssi = a.get(3).and_then(|v| v.as_i64()).unwrap_or(-60);
    let mut services = Vec::new();
    if let Some(Value::Dict(d)) = a.get(2) {
        for (su, chars) in d.borrow().entries.clone() {
            let mut cs = Vec::new();
            if let Value::Dict(cd) = chars {
                for (cu, val) in cd.borrow().entries.clone() {
                    cs.push((normalize_uuid(&cu.to_string())?, val.to_bytes()?));
                }
            }
            services.push((normalize_uuid(&su.to_string())?, cs));
        }
    }
    i.sim
        .borrow_mut()
        .ble
        .insert(addr, SimGatt::new(&name, rssi, services));
    Ok(Value::Nil)
}

pub fn sim_notify(i: &mut Interpreter, a: Vec<Value>) -> Result<Value, String> {
    let addr = a
        .first()
        .map(|v| v.to_string())
        .ok_or("sim::ble_notify(address, uuid, data)")?
        .to_uppercase();
    let u = normalize_uuid(
        &a.get(1)
            .ok_or("sim::ble_notify(address, uuid, data)")?
            .to_string(),
    )?;
    let data = a
        .get(2)
        .ok_or("sim::ble_notify(address, uuid, data)")?
        .to_bytes()?;
    let mut b = i.sim.borrow_mut();
    let dev = b
        .ble
        .get_mut(&addr)
        .ok_or_else(|| format!("no simulated BLE device at {addr}"))?;
    dev.notify(&u, data)?;
    Ok(Value::Nil)
}

pub fn sim_written(i: &mut Interpreter, a: Vec<Value>) -> Result<Value, String> {
    let addr = a
        .first()
        .map(|v| v.to_string())
        .ok_or("sim::ble_written(address, uuid)")?
        .to_uppercase();
    let u = normalize_uuid(
        &a.get(1)
            .ok_or("sim::ble_written(address, uuid)")?
            .to_string(),
    )?;
    let b = i.sim.borrow();
    let dev = b
        .ble
        .get(&addr)
        .ok_or_else(|| format!("no simulated BLE device at {addr}"))?;
    Ok(dev
        .written
        .get(&u)
        .map(|v| Value::from_bytes(v))
        .unwrap_or(Value::Nil))
}

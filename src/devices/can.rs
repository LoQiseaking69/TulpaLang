//! CAN bus: Linux SocketCAN (`can0`, `vcan0`, ...) and a simulated bus.

use super::tape::{Tape, Taped};
use super::{Backend, SimBus};
use crate::builtins::int;
use crate::interp::Interpreter;
use crate::value::{Dict, Native, Value};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub id: u32,
    pub data: Vec<u8>,
    pub extended: bool,
}

pub trait CanIo {
    fn send(&mut self, f: &Frame) -> Result<(), String>;
    fn recv(&mut self, timeout_ms: u64) -> Result<Option<Frame>, String>;
    fn set_filter(&mut self, id: u32, mask: u32) -> Result<(), String>;
}

fn encode(f: &Frame) -> Vec<u8> {
    let mut v = vec![f.extended as u8];
    v.extend_from_slice(&f.id.to_be_bytes());
    v.extend_from_slice(&f.data);
    v
}

fn decode(b: &[u8]) -> Option<Frame> {
    if b.len() < 5 {
        return None;
    }
    Some(Frame {
        extended: b[0] != 0,
        id: u32::from_be_bytes([b[1], b[2], b[3], b[4]]),
        data: b[5..].to_vec(),
    })
}

// ---------------------------------------------------------------------------- simulator

/// Per-interface virtual bus: every open socket has its own receive queue; a frame sent on one
/// socket is delivered to all other sockets on the same interface (CAN has no self-reception).
/// (interface, socket id, receive queue, filter (id, mask))
type SimSocket = (String, usize, VecDeque<Frame>, Option<(u32, u32)>);

#[derive(Default)]
pub struct SimCan {
    sockets: Vec<SimSocket>,
    next_id: usize,
    pub sent: Vec<(String, Frame)>,
}

impl SimCan {
    fn deliver(&mut self, iface: &str, from: Option<usize>, f: &Frame) {
        for (i, id, q, filter) in self.sockets.iter_mut() {
            if i == iface && Some(*id) != from {
                if let Some((fid, mask)) = filter {
                    if f.id & *mask != *fid & *mask {
                        continue;
                    }
                }
                q.push_back(f.clone());
            }
        }
    }
    pub fn inject(&mut self, iface: &str, f: Frame) {
        self.deliver(iface, None, &f);
    }
}

pub struct SimCanSocket {
    bus: Rc<RefCell<SimBus>>,
    iface: String,
    id: usize,
}

impl SimCanSocket {
    pub fn open(bus: Rc<RefCell<SimBus>>, iface: &str) -> Self {
        let id = {
            let mut b = bus.borrow_mut();
            let id = b.can.next_id;
            b.can.next_id += 1;
            b.can
                .sockets
                .push((iface.to_string(), id, VecDeque::new(), None));
            id
        };
        SimCanSocket {
            bus,
            iface: iface.to_string(),
            id,
        }
    }
}

impl CanIo for SimCanSocket {
    fn send(&mut self, f: &Frame) -> Result<(), String> {
        let mut b = self.bus.borrow_mut();
        b.can.deliver(&self.iface, Some(self.id), f);
        b.can.sent.push((self.iface.clone(), f.clone()));
        let data: Vec<String> = f.data.iter().map(|x| format!("{x:02x}")).collect();
        b.note(format!(
            "can {} tx {:03x}{} [{}]",
            self.iface,
            f.id,
            if f.extended { "x" } else { "" },
            data.join(" ")
        ));
        Ok(())
    }
    fn recv(&mut self, _timeout_ms: u64) -> Result<Option<Frame>, String> {
        let mut b = self.bus.borrow_mut();
        let id = self.id;
        Ok(b.can
            .sockets
            .iter_mut()
            .find(|s| s.1 == id)
            .and_then(|s| s.2.pop_front()))
    }
    fn set_filter(&mut self, fid: u32, mask: u32) -> Result<(), String> {
        let mut b = self.bus.borrow_mut();
        let id = self.id;
        if let Some(s) = b.can.sockets.iter_mut().find(|s| s.1 == id) {
            s.3 = Some((fid, mask));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------- SocketCAN

#[cfg(target_os = "linux")]
mod socketcan {
    use super::{CanIo, Frame};
    use std::os::unix::io::RawFd;

    const CAN_EFF_FLAG: u32 = 0x8000_0000;
    const CAN_RTR_FLAG: u32 = 0x4000_0000;
    const CAN_EFF_MASK: u32 = 0x1FFF_FFFF;
    const SOL_CAN_RAW: i32 = 101;
    const CAN_RAW_FILTER: i32 = 1;

    #[repr(C)]
    struct CanFrame {
        can_id: u32,
        len: u8,
        pad: u8,
        res0: u8,
        len8_dlc: u8,
        data: [u8; 8],
    }

    #[repr(C)]
    struct SockaddrCan {
        family: u16,
        ifindex: i32,
        addr: [u8; 16],
    }

    #[repr(C)]
    struct CanFilter {
        id: u32,
        mask: u32,
    }

    const _: () = assert!(std::mem::size_of::<CanFrame>() == 16);

    pub struct SocketCan {
        fd: RawFd,
        iface: String,
    }

    impl Drop for SocketCan {
        fn drop(&mut self) {
            // SAFETY: fd is owned by this struct.
            unsafe { libc::close(self.fd) };
        }
    }

    fn err(what: &str) -> String {
        format!("{what}: {}", std::io::Error::last_os_error())
    }

    impl SocketCan {
        pub fn open(iface: &str) -> Result<Self, String> {
            let name =
                std::ffi::CString::new(iface).map_err(|_| "bad interface name".to_string())?;
            // SAFETY: plain libc calls with valid arguments; fd ownership moves into SocketCan.
            unsafe {
                let idx = libc::if_nametoindex(name.as_ptr());
                if idx == 0 {
                    return Err(format!("CAN interface '{iface}' not found (try `ip link set {iface} up type can bitrate 500000`)"));
                }
                let fd = libc::socket(
                    libc::AF_CAN,
                    libc::SOCK_RAW | libc::SOCK_CLOEXEC,
                    libc::CAN_RAW,
                );
                if fd < 0 {
                    return Err(err("CAN socket"));
                }
                let addr = SockaddrCan {
                    family: libc::AF_CAN as u16,
                    ifindex: idx as i32,
                    addr: [0; 16],
                };
                if libc::bind(
                    fd,
                    &addr as *const SockaddrCan as *const libc::sockaddr,
                    std::mem::size_of::<SockaddrCan>() as u32,
                ) < 0
                {
                    let e = err(&format!("bind {iface}"));
                    libc::close(fd);
                    return Err(e);
                }
                Ok(SocketCan {
                    fd,
                    iface: iface.into(),
                })
            }
        }
    }

    impl CanIo for SocketCan {
        fn send(&mut self, f: &Frame) -> Result<(), String> {
            let mut data = [0u8; 8];
            data[..f.data.len()].copy_from_slice(&f.data);
            let id = if f.extended {
                (f.id & CAN_EFF_MASK) | CAN_EFF_FLAG
            } else {
                f.id & 0x7FF
            };
            let fr = CanFrame {
                can_id: id,
                len: f.data.len() as u8,
                pad: 0,
                res0: 0,
                len8_dlc: 0,
                data,
            };
            // SAFETY: writes one complete can_frame.
            let n =
                unsafe { libc::write(self.fd, &fr as *const CanFrame as *const libc::c_void, 16) };
            if n != 16 {
                return Err(err(&format!("{} send", self.iface)));
            }
            Ok(())
        }
        fn recv(&mut self, timeout_ms: u64) -> Result<Option<Frame>, String> {
            let mut pfd = libc::pollfd {
                fd: self.fd,
                events: libc::POLLIN,
                revents: 0,
            };
            // SAFETY: single valid pollfd; then reads exactly one can_frame.
            unsafe {
                let r = libc::poll(&mut pfd, 1, timeout_ms.min(i32::MAX as u64) as i32);
                if r < 0 {
                    return Err(err("CAN poll"));
                }
                if r == 0 {
                    return Ok(None);
                }
                let mut fr = std::mem::zeroed::<CanFrame>();
                let n = libc::read(self.fd, &mut fr as *mut CanFrame as *mut libc::c_void, 16);
                if n != 16 {
                    return Err(err(&format!("{} receive", self.iface)));
                }
                if fr.can_id & CAN_RTR_FLAG != 0 {
                    return Ok(Some(Frame {
                        id: fr.can_id & CAN_EFF_MASK,
                        data: vec![],
                        extended: fr.can_id & CAN_EFF_FLAG != 0,
                    }));
                }
                let ext = fr.can_id & CAN_EFF_FLAG != 0;
                let id = if ext {
                    fr.can_id & CAN_EFF_MASK
                } else {
                    fr.can_id & 0x7FF
                };
                Ok(Some(Frame {
                    id,
                    data: fr.data[..(fr.len as usize).min(8)].to_vec(),
                    extended: ext,
                }))
            }
        }
        fn set_filter(&mut self, id: u32, mask: u32) -> Result<(), String> {
            let f = CanFilter { id, mask };
            // SAFETY: passes one can_filter struct.
            let r = unsafe {
                libc::setsockopt(
                    self.fd,
                    SOL_CAN_RAW,
                    CAN_RAW_FILTER,
                    &f as *const CanFilter as *const libc::c_void,
                    8,
                )
            };
            if r < 0 {
                return Err(err("CAN filter"));
            }
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------- tape support

impl CanIo for Taped<Box<dyn CanIo>> {
    fn send(&mut self, f: &Frame) -> Result<(), String> {
        let enc = encode(f);
        self.tap_dyn("send", &enc, |d| d.send(f).map(|_| vec![]))
            .map(|_| ())
    }
    fn recv(&mut self, timeout_ms: u64) -> Result<Option<Frame>, String> {
        let r = self.tap_dyn("recv", &[], |d| {
            d.recv(timeout_ms)
                .map(|o| o.map(|f| encode(&f)).unwrap_or_default())
        })?;
        Ok(decode(&r))
    }
    fn set_filter(&mut self, id: u32, mask: u32) -> Result<(), String> {
        let mut input = id.to_be_bytes().to_vec();
        input.extend_from_slice(&mask.to_be_bytes());
        self.tap_dyn("filter", &input, |d| d.set_filter(id, mask).map(|_| vec![]))
            .map(|_| ())
    }
}

// ---------------------------------------------------------------------------- Tulpa object

pub struct Can {
    iface: String,
    io: Box<dyn CanIo>,
}

fn frame_value(f: Frame) -> Result<Value, String> {
    let mut d = Dict::default();
    d.set(Value::str("id"), Value::Int(f.id as i64))?;
    d.set(Value::str("data"), Value::from_bytes(&f.data))?;
    d.set(Value::str("extended"), Value::Bool(f.extended))?;
    Ok(Value::dict(d))
}

impl Native for Can {
    fn type_name(&self) -> &'static str {
        "CAN"
    }
    fn describe(&self) -> String {
        format!("<CAN {}>", self.iface)
    }
    fn call_method(&mut self, m: &str, a: &[Value]) -> Result<Value, String> {
        match m {
            "send" => {
                let id = int("send", a.first().ok_or("send() needs an id")?)?;
                let data = a
                    .get(1)
                    .map(|v| v.to_bytes())
                    .transpose()?
                    .unwrap_or_default();
                if data.len() > 8 {
                    return Err(format!(
                        "CAN frames carry at most 8 bytes, got {}",
                        data.len()
                    ));
                }
                let extended = a.get(2).map(|v| v.truthy()).unwrap_or(id > 0x7FF);
                let max = if extended { 0x1FFF_FFFF } else { 0x7FF };
                if id < 0 || id > max {
                    return Err(format!(
                        "CAN id 0x{id:x} out of range for {} frames",
                        if extended { "extended" } else { "standard" }
                    ));
                }
                self.io.send(&Frame {
                    id: id as u32,
                    data,
                    extended,
                })?;
                Ok(Value::Nil)
            }
            "receive" | "recv" => {
                let t = a
                    .first()
                    .map(|v| int("receive", v))
                    .transpose()?
                    .unwrap_or(100)
                    .max(0) as u64;
                match self.io.recv(t)? {
                    Some(f) => frame_value(f),
                    None => Ok(Value::Nil),
                }
            }
            "receive_all" => {
                let mut out = Vec::new();
                while let Some(f) = self.io.recv(0)? {
                    out.push(frame_value(f)?);
                    if out.len() >= 4096 {
                        break;
                    }
                }
                Ok(Value::list(out))
            }
            "set_filter" | "filter" => {
                let id = int(
                    "set_filter",
                    a.first().ok_or("set_filter() needs (id, mask)")?,
                )? as u32;
                let mask = a
                    .get(1)
                    .map(|v| int("set_filter", v))
                    .transpose()?
                    .unwrap_or(0x7FF) as u32;
                self.io.set_filter(id, mask)?;
                Ok(Value::Nil)
            }
            "interface" => Ok(Value::str(self.iface.as_str())),
            _ => Err(format!("CAN has no method '{m}'")),
        }
    }
}

pub fn b_can_new(i: &mut Interpreter, a: Vec<Value>) -> Result<Value, String> {
    let iface = a
        .first()
        .map(|v| v.to_string())
        .unwrap_or_else(|| "can0".into());
    let tape: Option<Rc<RefCell<Tape>>> = i.tape.clone();
    let replay = tape
        .as_ref()
        .is_some_and(|t| t.borrow().mode == super::TapeMode::Replay);
    let real = || -> Result<Box<dyn CanIo>, String> {
        Ok(match i.backend {
            Backend::Sim => Box::new(SimCanSocket::open(i.sim.clone(), &iface)),
            Backend::Linux => linux_can(&iface)?,
        })
    };
    let io: Box<dyn CanIo> = match tape {
        Some(t) if replay => Box::new(Taped::<Box<dyn CanIo>> {
            inner: None,
            tape: t,
            dev: format!("can:{iface}"),
        }),
        Some(t) => Box::new(Taped::<Box<dyn CanIo>> {
            inner: Some(real()?),
            tape: t,
            dev: format!("can:{iface}"),
        }),
        None => real()?,
    };
    Ok(Value::native(Can { iface, io }))
}

#[cfg(target_os = "linux")]
fn linux_can(iface: &str) -> Result<Box<dyn CanIo>, String> {
    Ok(Box::new(socketcan::SocketCan::open(iface)?))
}
#[cfg(not(target_os = "linux"))]
fn linux_can(_: &str) -> Result<Box<dyn CanIo>, String> {
    Err("SocketCAN is only available on Linux".into())
}

/// `sim::can_inject(iface, id, data)` / `sim::can_sent(iface)`
pub fn sim_inject(bus: &Rc<RefCell<SimBus>>, iface: &str, id: u32, data: Vec<u8>) {
    let extended = id > 0x7FF;
    bus.borrow_mut()
        .can
        .inject(iface, Frame { id, data, extended });
}

pub fn sim_sent(bus: &Rc<RefCell<SimBus>>, iface: &str) -> Result<Value, String> {
    let frames: Vec<Frame> = bus
        .borrow()
        .can
        .sent
        .iter()
        .filter(|(i, _)| i == iface)
        .map(|(_, f)| f.clone())
        .collect();
    Ok(Value::list(
        frames
            .into_iter()
            .map(frame_value)
            .collect::<Result<_, _>>()?,
    ))
}

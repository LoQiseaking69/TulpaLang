//! Record & replay of device traffic.
//!
//! `tulpa --record run.tape robot.rav` logs every device operation (with whatever backend is
//! active, usually real hardware). `tulpa --replay run.tape robot.rav` re-runs the script with
//! devices answering from the tape, and fails loudly at the first operation that differs from
//! the recording, which turns a field session into a regression test.

use super::{GpioIo, I2cIo, PwmIo, SpiIo, UartIo};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Record,
    Replay,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub dev: String,
    pub op: String,
    pub input: Vec<u8>,
    pub output: Vec<u8>,
}

pub struct Tape {
    pub mode: Mode,
    pub events: Vec<Event>,
    pub pos: usize,
    pub path: PathBuf,
}

fn hex(b: &[u8]) -> String {
    if b.is_empty() {
        "-".into()
    } else {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }
}

fn unhex(s: &str) -> Result<Vec<u8>, String> {
    if s == "-" {
        return Ok(vec![]);
    }
    if !s.len().is_multiple_of(2) {
        return Err(format!("bad hex field '{s}'"));
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|_| format!("bad hex field '{s}'")))
        .collect()
}

impl Tape {
    pub fn record(path: &Path) -> Self {
        Tape {
            mode: Mode::Record,
            events: vec![],
            pos: 0,
            path: path.to_path_buf(),
        }
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut events = Vec::new();
        for (n, line) in text.lines().enumerate() {
            if line.starts_with('#') || line.trim().is_empty() {
                continue;
            }
            let f: Vec<&str> = line.split('\t').collect();
            if f.len() != 4 {
                return Err(format!(
                    "{}:{}: expected 4 tab-separated fields",
                    path.display(),
                    n + 1
                ));
            }
            events.push(Event {
                dev: f[0].into(),
                op: f[1].into(),
                input: unhex(f[2])?,
                output: unhex(f[3])?,
            });
        }
        Ok(Tape {
            mode: Mode::Replay,
            events,
            pos: 0,
            path: path.to_path_buf(),
        })
    }

    pub fn save(&self) -> Result<(), String> {
        let mut out =
            String::from("# tulpa device tape v1: device<TAB>op<TAB>input-hex<TAB>output-hex\n");
        for e in &self.events {
            out.push_str(&format!(
                "{}\t{}\t{}\t{}\n",
                e.dev,
                e.op,
                hex(&e.input),
                hex(&e.output)
            ));
        }
        std::fs::write(&self.path, out).map_err(|e| format!("{}: {e}", self.path.display()))
    }

    /// Replay: returns the recorded output, or an error if the script diverged from the tape.
    fn replay(&mut self, dev: &str, op: &str, input: &[u8]) -> Result<Vec<u8>, String> {
        let k = self.pos;
        let Some(e) = self.events.get(k) else {
            return Err(format!(
                "replay tape exhausted at event #{k}: script did {dev} {op} [{}]",
                hex(input)
            ));
        };
        if e.dev != dev || e.op != op || e.input != input {
            return Err(format!(
                "replay diverged at event #{k}: tape has {} {} [{}], script did {dev} {op} [{}]",
                e.dev,
                e.op,
                hex(&e.input),
                hex(input)
            ));
        }
        self.pos += 1;
        Ok(e.output.clone())
    }

    pub fn remaining(&self) -> usize {
        self.events.len().saturating_sub(self.pos)
    }
}

/// Wraps a device I/O object. In record mode `inner` is the real (or simulated) device;
/// in replay mode it is `None` and answers come from the tape.
pub struct Taped<T> {
    pub inner: Option<T>,
    pub tape: Rc<RefCell<Tape>>,
    pub dev: String,
}

impl<T> Taped<T> {
    fn tap(
        &mut self,
        op: &str,
        input: &[u8],
        f: impl FnOnce(&mut T) -> Result<Vec<u8>, String>,
    ) -> Result<Vec<u8>, String> {
        let mode = self.tape.borrow().mode;
        match mode {
            Mode::Replay => self.tape.borrow_mut().replay(&self.dev, op, input),
            Mode::Record => {
                let out = f(self.inner.as_mut().expect("record mode needs a device"))?;
                self.tape.borrow_mut().events.push(Event {
                    dev: self.dev.clone(),
                    op: op.into(),
                    input: input.to_vec(),
                    output: out.clone(),
                });
                Ok(out)
            }
        }
    }
}

impl GpioIo for Taped<Box<dyn GpioIo>> {
    fn set_output(&mut self, out: bool) -> Result<(), String> {
        self.tap("dir", &[out as u8], |d| d.set_output(out).map(|_| vec![]))
            .map(|_| ())
    }
    fn write(&mut self, high: bool) -> Result<(), String> {
        self.tap("write", &[high as u8], |d| d.write(high).map(|_| vec![]))
            .map(|_| ())
    }
    fn read(&mut self) -> Result<bool, String> {
        let r = self.tap("read", &[], |d| d.read().map(|b| vec![b as u8]))?;
        Ok(r.first().copied().unwrap_or(0) != 0)
    }
}

impl I2cIo for Taped<Box<dyn I2cIo>> {
    fn write(&mut self, data: &[u8]) -> Result<(), String> {
        self.tap("write", data, |d| d.write(data).map(|_| vec![]))
            .map(|_| ())
    }
    fn read(&mut self, n: usize) -> Result<Vec<u8>, String> {
        self.tap(&format!("read:{n}"), &[], |d| d.read(n))
    }
    fn write_read(&mut self, data: &[u8], n: usize) -> Result<Vec<u8>, String> {
        self.tap(&format!("write_read:{n}"), data, |d| d.write_read(data, n))
    }
}

impl SpiIo for Taped<Box<dyn SpiIo>> {
    fn transfer(&mut self, data: &[u8]) -> Result<Vec<u8>, String> {
        self.tap("transfer", data, |d| d.transfer(data))
    }
    fn set_speed(&mut self, hz: u32) -> Result<(), String> {
        self.tap("speed", &hz.to_be_bytes(), |d| {
            d.set_speed(hz).map(|_| vec![])
        })
        .map(|_| ())
    }
    fn set_mode(&mut self, mode: u8) -> Result<(), String> {
        self.tap("mode", &[mode], |d| d.set_mode(mode).map(|_| vec![]))
            .map(|_| ())
    }
}

impl UartIo for Taped<Box<dyn UartIo>> {
    fn send(&mut self, data: &[u8]) -> Result<(), String> {
        self.tap("send", data, |d| d.send(data).map(|_| vec![]))
            .map(|_| ())
    }
    fn receive(&mut self, max: usize, timeout_ms: u64) -> Result<Vec<u8>, String> {
        self.tap(&format!("receive:{max}"), &[], |d| {
            d.receive(max, timeout_ms)
        })
    }
    fn available(&mut self) -> Result<usize, String> {
        let r = self.tap("available", &[], |d| {
            d.available().map(|n| (n as u32).to_be_bytes().to_vec())
        })?;
        Ok(r.try_into().map(u32::from_be_bytes).unwrap_or(0) as usize)
    }
}

impl PwmIo for Taped<Box<dyn PwmIo>> {
    fn set_period_ns(&mut self, ns: u64) -> Result<(), String> {
        self.tap("period", &ns.to_be_bytes(), |d| {
            d.set_period_ns(ns).map(|_| vec![])
        })
        .map(|_| ())
    }
    fn set_duty_ns(&mut self, ns: u64) -> Result<(), String> {
        self.tap("duty", &ns.to_be_bytes(), |d| {
            d.set_duty_ns(ns).map(|_| vec![])
        })
        .map(|_| ())
    }
    fn enable(&mut self, on: bool) -> Result<(), String> {
        self.tap("enable", &[on as u8], |d| d.enable(on).map(|_| vec![]))
            .map(|_| ())
    }
}

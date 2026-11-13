//! Record & replay of device traffic.
//!
//! `tulpa --record run.tape robot.rav` logs every device operation with a timestamp.
//! `tulpa --replay run.tape robot.rav` re-runs the script with devices answering from the tape:
//!
//! * **Commands** (GPIO writes/direction, I2C writes, SPI transfers, UART sends, PWM settings)
//!   must happen in exactly the recorded order with the recorded data; the first difference is
//!   reported as a divergence.
//! * **Inputs** (GPIO reads, I2C reads/register reads, UART `available`) are treated as signals
//!   over time: a read returns the value the device had at that moment of the recorded run, so
//!   rules that poll inputs at a different rate still replay correctly.
//! * **UART receive** delivers recorded bytes once they have "arrived" on the timeline.
//!
//! Replay runs on a virtual clock that advances to each recorded timestamp, so `when … for` and
//! `after … within` rules are judged against the timing that actually happened.

use super::{GpioIo, I2cIo, PwmIo, SpiIo, UartIo};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Record,
    Replay,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Must match in order.
    Command,
    /// Value sampled over time.
    Input,
    /// Bytes that arrive over time and are consumed once.
    Stream,
}

fn kind_of(dev: &str, op: &str) -> Kind {
    if (op == "read" && dev.starts_with("gpio"))
        || op.starts_with("read:")
        || op.starts_with("write_read:")
        || op == "available"
    {
        Kind::Input
    } else if op.starts_with("receive:") || op == "recv" {
        Kind::Stream
    } else {
        Kind::Command
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub dev: String,
    pub op: String,
    pub input: Vec<u8>,
    pub output: Vec<u8>,
    /// Seconds since the start of the recorded run (tape v2); None for v1 tapes.
    pub t: Option<f64>,
}

type Key = (String, String, Vec<u8>);

pub struct Tape {
    pub mode: Mode,
    pub events: Vec<Event>,
    /// Index into the command list of the next expected command.
    pub pos: usize,
    pub path: PathBuf,
    /// Interpreter clock (seconds), updated before every device call.
    pub clock: f64,
    /// Set when a replayed command was recorded later than the current clock: the interpreter
    /// advances its virtual clock to this time after the call.
    pub lag_until: Option<f64>,
    commands: Vec<usize>,
    inputs: HashMap<Key, Vec<usize>>,
    input_cursor: HashMap<Key, usize>,
    streams: HashMap<String, (Vec<usize>, usize)>,
    last_input: HashMap<Key, Vec<u8>>,
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
    fn empty(mode: Mode, path: &Path) -> Self {
        Tape {
            mode,
            events: vec![],
            pos: 0,
            path: path.to_path_buf(),
            clock: 0.0,
            lag_until: None,
            commands: vec![],
            inputs: HashMap::new(),
            input_cursor: HashMap::new(),
            streams: HashMap::new(),
            last_input: HashMap::new(),
        }
    }

    pub fn record(path: &Path) -> Self {
        Self::empty(Mode::Record, path)
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut tape = Self::empty(Mode::Replay, path);
        for (n, line) in text.lines().enumerate() {
            if line.starts_with('#') || line.trim().is_empty() {
                continue;
            }
            let f: Vec<&str> = line.split('\t').collect();
            if f.len() != 4 && f.len() != 5 {
                return Err(format!(
                    "{}:{}: expected 4 or 5 tab-separated fields",
                    path.display(),
                    n + 1
                ));
            }
            let t = match f.get(4) {
                Some(ms) => Some(
                    ms.parse::<f64>().map_err(|_| {
                        format!("{}:{}: bad timestamp '{ms}'", path.display(), n + 1)
                    })? / 1000.0,
                ),
                None => None,
            };
            tape.events.push(Event {
                dev: f[0].into(),
                op: f[1].into(),
                input: unhex(f[2])?,
                output: unhex(f[3])?,
                t,
            });
        }
        for (i, e) in tape.events.iter().enumerate() {
            match kind_of(&e.dev, &e.op) {
                Kind::Command => tape.commands.push(i),
                Kind::Input => tape
                    .inputs
                    .entry((e.dev.clone(), e.op.clone(), e.input.clone()))
                    .or_default()
                    .push(i),
                Kind::Stream => {
                    if !e.output.is_empty() {
                        tape.streams.entry(e.dev.clone()).or_default().0.push(i);
                    }
                }
            }
        }
        Ok(tape)
    }

    pub fn save(&self) -> Result<(), String> {
        let mut out = String::from(
            "# tulpa device tape v2: device<TAB>op<TAB>input-hex<TAB>output-hex<TAB>time-ms\n",
        );
        for e in &self.events {
            let t =
                e.t.map(|t| format!("\t{:.3}", t * 1000.0))
                    .unwrap_or_default();
            out.push_str(&format!(
                "{}\t{}\t{}\t{}{t}\n",
                e.dev,
                e.op,
                hex(&e.input),
                hex(&e.output)
            ));
        }
        std::fs::write(&self.path, out).map_err(|e| format!("{}: {e}", self.path.display()))
    }

    fn record_event(&mut self, dev: &str, op: &str, input: &[u8], output: &[u8]) {
        let kind = kind_of(dev, op);
        // Inputs are stored as changes: an unchanged value adds nothing to the timeline.
        if kind == Kind::Input {
            let key = (dev.to_string(), op.to_string(), input.to_vec());
            if self.last_input.get(&key).map(|v| v.as_slice()) == Some(output) {
                return;
            }
            self.last_input.insert(key, output.to_vec());
        }
        if kind == Kind::Stream && output.is_empty() {
            return;
        }
        let t = Some(self.clock);
        self.events.push(Event {
            dev: dev.into(),
            op: op.into(),
            input: input.to_vec(),
            output: output.to_vec(),
            t,
        });
    }

    fn replay(&mut self, dev: &str, op: &str, input: &[u8]) -> Result<Vec<u8>, String> {
        match kind_of(dev, op) {
            Kind::Command => {
                let Some(&idx) = self.commands.get(self.pos) else {
                    return Err(format!(
                        "replay tape exhausted at command #{}: script did {dev} {op} [{}]",
                        self.pos,
                        hex(input)
                    ));
                };
                let e = &self.events[idx];
                if e.dev != dev || e.op != op || e.input != input {
                    return Err(format!(
                        "replay diverged at command #{}: tape has {} {} [{}], script did {dev} {op} [{}]",
                        self.pos,
                        e.dev,
                        e.op,
                        hex(&e.input),
                        hex(input)
                    ));
                }
                if let Some(t) = e.t {
                    if t > self.clock {
                        self.lag_until = Some(t);
                    }
                }
                self.pos += 1;
                Ok(e.output.clone())
            }
            Kind::Input => {
                let key = (dev.to_string(), op.to_string(), input.to_vec());
                let Some(list) = self.inputs.get(&key) else {
                    return Err(format!(
                        "replay: the recording never read {dev} {op} [{}]",
                        hex(input)
                    ));
                };
                let timed = list.iter().all(|&i| self.events[i].t.is_some());
                let idx = if timed {
                    // the value at the current time: the last change at or before now (else the
                    // first); 1 ms of tolerance absorbs recording jitter (e.g. a read issued
                    // microseconds after the write that caused it)
                    let now = self.clock + 0.001;
                    list.iter()
                        .rev()
                        .find(|&&i| self.events[i].t.unwrap() <= now)
                        .copied()
                        .unwrap_or(list[0])
                } else {
                    let c = self.input_cursor.entry(key.clone()).or_insert(0);
                    let i = list[(*c).min(list.len() - 1)];
                    *c += 1;
                    i
                };
                Ok(self.events[idx].output.clone())
            }
            Kind::Stream => {
                let Some((list, cur)) = self.streams.get_mut(dev) else {
                    return Ok(vec![]);
                };
                if op == "recv" {
                    // framed streams (CAN frames, BLE ATT PDUs): one frame per call.
                    // BLE responses are request-driven, so they are delivered in order; CAN frames
                    // are delivered once they have arrived on the recorded timeline.
                    let Some(&idx) = list.get(*cur) else {
                        return Ok(vec![]);
                    };
                    let e = &self.events[idx];
                    if !dev.starts_with("ble:") && e.t.is_some_and(|t| t > self.clock + 0.001) {
                        return Ok(vec![]);
                    }
                    *cur += 1;
                    return Ok(e.output.clone());
                }
                let max: usize = op
                    .trim_start_matches("receive:")
                    .parse()
                    .unwrap_or(usize::MAX);
                let now = self.clock + 0.001;
                let mut out = Vec::new();
                while *cur < list.len() && out.len() < max {
                    let e = &self.events[list[*cur]];
                    if e.t.is_some_and(|t| t > now) {
                        break;
                    }
                    out.extend_from_slice(&e.output);
                    *cur += 1;
                }
                out.truncate(max);
                Ok(out)
            }
        }
    }

    /// Recorded commands not yet replayed.
    pub fn remaining(&self) -> usize {
        self.commands.len().saturating_sub(self.pos)
    }

    /// The next command to replay, as (device, op).
    pub fn next_command(&self) -> Option<(&str, &str)> {
        self.commands
            .get(self.pos)
            .map(|&i| (self.events[i].dev.as_str(), self.events[i].op.as_str()))
    }

    pub fn command_count(&self) -> usize {
        self.commands.len()
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
    pub(crate) fn tap_dyn(
        &mut self,
        op: &str,
        input: &[u8],
        f: impl FnOnce(&mut T) -> Result<Vec<u8>, String>,
    ) -> Result<Vec<u8>, String> {
        self.tap(op, input, f)
    }

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
                self.tape
                    .borrow_mut()
                    .record_event(&self.dev, op, input, &out);
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

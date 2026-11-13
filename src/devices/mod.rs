//! Device integration: GPIO, I2C, SPI, UART and a PID controller.
//!
//! Every device has two backends:
//! * `sim`   – an in-memory bus (default). Deterministic, scriptable via `sim::*`, safe to run anywhere.
//! * `linux` – real hardware through sysfs GPIO, `/dev/i2c-N`, `/dev/spidevB.C` and serial TTYs.

mod sim;
pub use sim::SimBus;
mod tape;
use tape::Taped;
pub use tape::{Mode as TapeMode, Tape};

#[cfg(target_os = "linux")]
mod linux;

use crate::builtins::{int, num};
use crate::interp::Interpreter;
use crate::value::{Native, NativeFn, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Sim,
    Linux,
}

type Res = Result<Value, String>;

// ---------------------------------------------------------------------------- I/O traits

pub trait GpioIo {
    fn set_output(&mut self, out: bool) -> Result<(), String>;
    fn write(&mut self, high: bool) -> Result<(), String>;
    fn read(&mut self) -> Result<bool, String>;
}

pub trait I2cIo {
    fn write(&mut self, data: &[u8]) -> Result<(), String>;
    fn read(&mut self, n: usize) -> Result<Vec<u8>, String>;
    /// Write then read with a repeated start (register reads).
    fn write_read(&mut self, data: &[u8], n: usize) -> Result<Vec<u8>, String>;
}

pub trait SpiIo {
    fn transfer(&mut self, data: &[u8]) -> Result<Vec<u8>, String>;
    fn set_speed(&mut self, hz: u32) -> Result<(), String>;
    fn set_mode(&mut self, mode: u8) -> Result<(), String>;
}

pub trait PwmIo {
    fn set_period_ns(&mut self, ns: u64) -> Result<(), String>;
    fn set_duty_ns(&mut self, ns: u64) -> Result<(), String>;
    fn enable(&mut self, on: bool) -> Result<(), String>;
}

/// Builds a device I/O object, wrapping it in a record/replay tape when one is active.
/// In replay mode the real backend is never opened.
macro_rules! make_io {
    ($i:expr, $dev:expr, $tr:ty, $real:expr) => {{
        let tape = $i.tape.clone();
        let boxed: Box<$tr> = match tape {
            Some(t) if t.borrow().mode == tape::Mode::Replay => Box::new(Taped::<Box<$tr>> {
                inner: None,
                tape: t,
                dev: $dev,
            }),
            Some(t) => {
                let real: Box<$tr> = $real;
                Box::new(Taped::<Box<$tr>> {
                    inner: Some(real),
                    tape: t,
                    dev: $dev,
                })
            }
            None => $real,
        };
        boxed
    }};
}

pub trait UartIo {
    fn send(&mut self, data: &[u8]) -> Result<(), String>;
    fn receive(&mut self, max: usize, timeout_ms: u64) -> Result<Vec<u8>, String>;
    fn available(&mut self) -> Result<usize, String>;
}

// ---------------------------------------------------------------------------- devices

fn arg<'a>(a: &'a [Value], i: usize, what: &str) -> Result<&'a Value, String> {
    a.get(i).ok_or_else(|| format!("missing argument: {what}"))
}

fn pad_args(name: &str, a: &[Value], min: usize, max: usize) -> Result<(), String> {
    if a.len() < min || a.len() > max {
        Err(format!(
            "{name}() takes {min}..{max} argument(s), got {}",
            a.len()
        ))
    } else {
        Ok(())
    }
}

pub struct Gpio {
    pin: u32,
    output: bool,
    io: Box<dyn GpioIo>,
}

impl Native for Gpio {
    fn type_name(&self) -> &'static str {
        "GPIO"
    }
    fn describe(&self) -> String {
        format!(
            "<GPIO {} {}>",
            self.pin,
            if self.output { "out" } else { "in" }
        )
    }
    fn call_method(&mut self, m: &str, a: &[Value]) -> Res {
        let need_out = |s: &Self| {
            if s.output {
                Ok(())
            } else {
                Err(format!(
                    "GPIO {} is an input; call set_direction(\"out\") first",
                    s.pin
                ))
            }
        };
        match m {
            "set_high" | "on" => {
                need_out(self)?;
                self.io.write(true)?;
                Ok(Value::Nil)
            }
            "set_low" | "off" => {
                need_out(self)?;
                self.io.write(false)?;
                Ok(Value::Nil)
            }
            "write" | "set" => {
                need_out(self)?;
                self.io.write(arg(a, 0, "level")?.truthy())?;
                Ok(Value::Nil)
            }
            "read" | "value" => Ok(Value::Int(self.io.read()? as i64)),
            "is_high" => Ok(Value::Bool(self.io.read()?)),
            "is_low" => Ok(Value::Bool(!self.io.read()?)),
            "toggle" => {
                need_out(self)?;
                let v = !self.io.read()?;
                self.io.write(v)?;
                Ok(Value::Int(v as i64))
            }
            "blink" => {
                need_out(self)?;
                let times = int("blink", arg(a, 0, "times")?)?;
                let ms = a
                    .get(1)
                    .map(|v| num("blink", v))
                    .transpose()?
                    .unwrap_or(250.0);
                let d = std::time::Duration::from_secs_f64(ms.max(0.0) / 1000.0);
                for _ in 0..times.max(0) {
                    self.io.write(true)?;
                    std::thread::sleep(d);
                    self.io.write(false)?;
                    std::thread::sleep(d);
                }
                Ok(Value::Nil)
            }
            "set_direction" => {
                let out = parse_direction(arg(a, 0, "direction")?)?;
                self.io.set_output(out)?;
                self.output = out;
                Ok(Value::Nil)
            }
            "pin" => Ok(Value::Int(self.pin as i64)),
            "direction" => Ok(Value::str(if self.output { "out" } else { "in" })),
            _ => Err(format!("GPIO has no method '{m}'")),
        }
    }
}

fn parse_direction(v: &Value) -> Result<bool, String> {
    match v.to_string().to_lowercase().as_str() {
        "out" | "output" => Ok(true),
        "in" | "input" => Ok(false),
        other => Err(format!(
            "invalid GPIO direction '{other}' (use \"in\" or \"out\")"
        )),
    }
}

pub struct I2c {
    bus: u32,
    addr: u16,
    io: Box<dyn I2cIo>,
}

impl Native for I2c {
    fn type_name(&self) -> &'static str {
        "I2C"
    }
    fn describe(&self) -> String {
        format!("<I2C bus {} @ 0x{:02x}>", self.bus, self.addr)
    }
    fn call_method(&mut self, m: &str, a: &[Value]) -> Res {
        let reg = |a: &[Value]| -> Result<u8, String> {
            crate::value::byte(int(m, arg(a, 0, "register")?)?)
        };
        match m {
            "write" => {
                self.io.write(&arg(a, 0, "bytes")?.to_bytes()?)?;
                Ok(Value::Nil)
            }
            "read" => {
                let n = int("read", arg(a, 0, "count")?)?.max(0) as usize;
                Ok(Value::from_bytes(&self.io.read(n)?))
            }
            "write_read" => {
                let data = arg(a, 0, "bytes")?.to_bytes()?;
                let n = int("write_read", arg(a, 1, "count")?)?.max(0) as usize;
                Ok(Value::from_bytes(&self.io.write_read(&data, n)?))
            }
            "write_reg" => {
                let mut data = vec![reg(a)?];
                data.extend(arg(a, 1, "value")?.to_bytes()?);
                self.io.write(&data)?;
                Ok(Value::Nil)
            }
            "read_reg" => Ok(Value::Int(self.io.write_read(&[reg(a)?], 1)?[0] as i64)),
            "read_regs" => {
                let n = int("read_regs", arg(a, 1, "count")?)?.max(0) as usize;
                Ok(Value::from_bytes(&self.io.write_read(&[reg(a)?], n)?))
            }
            "read_u16" | "read_i16" => {
                let b = self.io.write_read(&[reg(a)?], 2)?;
                let little = matches!(a.get(1), Some(Value::Str(s)) if s.as_ref() == "little");
                let raw = if little {
                    u16::from_le_bytes([b[0], b[1]])
                } else {
                    u16::from_be_bytes([b[0], b[1]])
                };
                Ok(Value::Int(if m == "read_i16" {
                    raw as i16 as i64
                } else {
                    raw as i64
                }))
            }
            "update_reg" => {
                // read-modify-write: update_reg(reg, mask, value)
                let r = reg(a)?;
                let mask = int("update_reg", arg(a, 1, "mask")?)? as u8;
                let val = int("update_reg", arg(a, 2, "value")?)? as u8;
                let cur = self.io.write_read(&[r], 1)?[0];
                let new = (cur & !mask) | (val & mask);
                self.io.write(&[r, new])?;
                Ok(Value::Int(new as i64))
            }
            "address" => Ok(Value::Int(self.addr as i64)),
            "bus" => Ok(Value::Int(self.bus as i64)),
            _ => Err(format!("I2C has no method '{m}'")),
        }
    }
}

pub struct Spi {
    bus: u32,
    cs: u32,
    io: Box<dyn SpiIo>,
}

impl Native for Spi {
    fn type_name(&self) -> &'static str {
        "SPI"
    }
    fn describe(&self) -> String {
        format!("<SPI {}.{}>", self.bus, self.cs)
    }
    fn call_method(&mut self, m: &str, a: &[Value]) -> Res {
        match m {
            "transfer" | "xfer" => Ok(Value::from_bytes(
                &self.io.transfer(&arg(a, 0, "bytes")?.to_bytes()?)?,
            )),
            "write" => {
                self.io.transfer(&arg(a, 0, "bytes")?.to_bytes()?)?;
                Ok(Value::Nil)
            }
            "read" => {
                let n = int("read", arg(a, 0, "count")?)?.max(0) as usize;
                Ok(Value::from_bytes(&self.io.transfer(&vec![0u8; n])?))
            }
            "set_speed" | "set_bus_speed" => {
                self.io
                    .set_speed(int("set_speed", arg(a, 0, "hz")?)? as u32)?;
                Ok(Value::Nil)
            }
            "set_mode" => {
                let mode = int("set_mode", arg(a, 0, "mode")?)?;
                if !(0..=3).contains(&mode) {
                    return Err("SPI mode must be 0..3".into());
                }
                self.io.set_mode(mode as u8)?;
                Ok(Value::Nil)
            }
            _ => Err(format!("SPI has no method '{m}'")),
        }
    }
}

pub struct Uart {
    port: String,
    baud: u32,
    io: Box<dyn UartIo>,
}

impl Native for Uart {
    fn type_name(&self) -> &'static str {
        "UART"
    }
    fn describe(&self) -> String {
        format!("<UART {} @ {}>", self.port, self.baud)
    }
    fn call_method(&mut self, m: &str, a: &[Value]) -> Res {
        match m {
            "send" | "write" => {
                let data = arg(a, 0, "data")?.to_bytes()?;
                self.io.send(&data)?;
                Ok(Value::Int(data.len() as i64))
            }
            "send_line" | "println" => {
                let mut data = arg(a, 0, "data")?.to_bytes()?;
                data.extend_from_slice(b"\r\n");
                self.io.send(&data)?;
                Ok(Value::Int(data.len() as i64))
            }
            "receive" | "read" => {
                let n = int("receive", arg(a, 0, "count")?)?.max(0) as usize;
                let t = a
                    .get(1)
                    .map(|v| int("receive", v))
                    .transpose()?
                    .unwrap_or(100)
                    .max(0) as u64;
                Ok(Value::from_bytes(&self.io.receive(n, t)?))
            }
            "read_text" => {
                let n = a
                    .first()
                    .map(|v| int("read_text", v))
                    .transpose()?
                    .unwrap_or(4096)
                    .max(0) as usize;
                let t = a
                    .get(1)
                    .map(|v| int("read_text", v))
                    .transpose()?
                    .unwrap_or(100)
                    .max(0) as u64;
                Ok(Value::str(
                    String::from_utf8_lossy(&self.io.receive(n, t)?).to_string(),
                ))
            }
            "read_line" | "readline" => {
                let t = a
                    .first()
                    .map(|v| int("read_line", v))
                    .transpose()?
                    .unwrap_or(1000)
                    .max(0) as u64;
                let deadline = std::time::Instant::now() + std::time::Duration::from_millis(t);
                let mut line = Vec::new();
                loop {
                    let left = deadline
                        .saturating_duration_since(std::time::Instant::now())
                        .as_millis() as u64;
                    let b = self.io.receive(1, left)?;
                    match b.first() {
                        Some(b'\n') => break,
                        Some(c) => line.push(*c),
                        None => {
                            if line.is_empty() {
                                return Ok(Value::Nil);
                            }
                            break;
                        }
                    }
                }
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                Ok(Value::str(String::from_utf8_lossy(&line).to_string()))
            }
            "available" => Ok(Value::Int(self.io.available()? as i64)),
            "port" => Ok(Value::str(self.port.as_str())),
            "baud" => Ok(Value::Int(self.baud as i64)),
            _ => Err(format!("UART has no method '{m}'")),
        }
    }
}

/// PID controller with output limits and integral anti-windup.
pub struct Pid {
    kp: f64,
    ki: f64,
    kd: f64,
    integral: f64,
    prev_err: Option<f64>,
    limits: Option<(f64, f64)>,
}

impl Native for Pid {
    fn type_name(&self) -> &'static str {
        "PID"
    }
    fn describe(&self) -> String {
        format!("<PID kp={} ki={} kd={}>", self.kp, self.ki, self.kd)
    }
    fn call_method(&mut self, m: &str, a: &[Value]) -> Res {
        match m {
            "update" | "step" => {
                pad_args("PID.update", a, 3, 3)?;
                let (sp, pv, dt) = (
                    num("update", &a[0])?,
                    num("update", &a[1])?,
                    num("update", &a[2])?,
                );
                if dt <= 0.0 {
                    return Err("PID.update: dt must be > 0".into());
                }
                let err = sp - pv;
                let deriv = self.prev_err.map_or(0.0, |p| (err - p) / dt);
                self.prev_err = Some(err);
                let candidate = self.integral + err * dt;
                let mut out = self.kp * err + self.ki * candidate + self.kd * deriv;
                match self.limits {
                    Some((lo, hi)) if out > hi || out < lo => {
                        out = out.clamp(lo, hi); // saturated: don't integrate (anti-windup)
                    }
                    _ => self.integral = candidate,
                }
                Ok(Value::Float(out))
            }
            "set_limits" => {
                pad_args("PID.set_limits", a, 2, 2)?;
                let (lo, hi) = (num("set_limits", &a[0])?, num("set_limits", &a[1])?);
                if lo > hi {
                    return Err("PID.set_limits: low > high".into());
                }
                self.limits = Some((lo, hi));
                Ok(Value::Nil)
            }
            "set_gains" => {
                pad_args("PID.set_gains", a, 3, 3)?;
                self.kp = num("set_gains", &a[0])?;
                self.ki = num("set_gains", &a[1])?;
                self.kd = num("set_gains", &a[2])?;
                Ok(Value::Nil)
            }
            "reset" => {
                self.integral = 0.0;
                self.prev_err = None;
                Ok(Value::Nil)
            }
            "integral" => Ok(Value::Float(self.integral)),
            _ => Err(format!("PID has no method '{m}'")),
        }
    }
}

// ---------------------------------------------------------------------------- constructors

fn b_gpio_new(i: &mut Interpreter, a: Vec<Value>) -> Res {
    pad_args("GPIO::new", &a, 1, 2)?;
    let pin = int("GPIO::new", &a[0])?;
    let pin = u32::try_from(pin).map_err(|_| format!("invalid GPIO pin {pin}"))?;
    let output = match a.get(1) {
        Some(d) => parse_direction(d)?,
        None => true,
    };
    let mut io = make_io!(
        i,
        format!("gpio{pin}"),
        dyn GpioIo,
        match i.backend {
            Backend::Sim => Box::new(sim::SimGpio::new(i.sim.clone(), pin)),
            Backend::Linux => linux_gpio(pin)?,
        }
    );
    io.set_output(output)?;
    Ok(Value::native(Gpio { pin, output, io }))
}

fn b_i2c_new(i: &mut Interpreter, a: Vec<Value>) -> Res {
    pad_args("I2C::new", &a, 1, 2)?;
    let (bus, addr) = if a.len() == 1 {
        (1, int("I2C::new", &a[0])?)
    } else {
        (int("I2C::new", &a[0])?, int("I2C::new", &a[1])?)
    };
    if !(0..=0x7f).contains(&addr) {
        return Err(format!("I2C address 0x{addr:x} out of 7-bit range"));
    }
    let (bus, addr) = (bus as u32, addr as u16);
    let io = make_io!(
        i,
        format!("i2c{bus}:0x{addr:02x}"),
        dyn I2cIo,
        match i.backend {
            Backend::Sim => Box::new(sim::SimI2c::new(i.sim.clone(), bus, addr)),
            Backend::Linux => linux_i2c(bus, addr)?,
        }
    );
    Ok(Value::native(I2c { bus, addr, io }))
}

fn b_spi_new(i: &mut Interpreter, a: Vec<Value>) -> Res {
    pad_args("SPI::new", &a, 0, 2)?;
    let bus = a
        .first()
        .map(|v| int("SPI::new", v))
        .transpose()?
        .unwrap_or(0) as u32;
    let cs = a
        .get(1)
        .map(|v| int("SPI::new", v))
        .transpose()?
        .unwrap_or(0) as u32;
    let io = make_io!(
        i,
        format!("spi{bus}.{cs}"),
        dyn SpiIo,
        match i.backend {
            Backend::Sim => Box::new(sim::SimSpi::new(i.sim.clone(), bus, cs)),
            Backend::Linux => linux_spi(bus, cs)?,
        }
    );
    Ok(Value::native(Spi { bus, cs, io }))
}

fn b_uart_new(i: &mut Interpreter, a: Vec<Value>) -> Res {
    pad_args("UART::new", &a, 1, 2)?;
    let port = a[0].to_string();
    let baud = a
        .get(1)
        .map(|v| int("UART::new", v))
        .transpose()?
        .unwrap_or(9600) as u32;
    let io = make_io!(
        i,
        format!("uart:{port}"),
        dyn UartIo,
        match i.backend {
            Backend::Sim => Box::new(sim::SimUart::new(i.sim.clone(), port.clone())),
            Backend::Linux => linux_uart(&port, baud)?,
        }
    );
    Ok(Value::native(Uart { port, baud, io }))
}

pub struct Pwm {
    chip: u32,
    ch: u32,
    period_ns: u64,
    duty_ns: u64,
    enabled: bool,
    io: Box<dyn PwmIo>,
}

impl Pwm {
    fn set_period(&mut self, ns: u64) -> Result<(), String> {
        if ns == 0 {
            return Err("PWM period must be > 0".into());
        }
        if self.duty_ns > ns {
            self.duty_ns = ns;
            self.io.set_duty_ns(ns)?;
        }
        self.io.set_period_ns(ns)?;
        self.period_ns = ns;
        Ok(())
    }
    fn set_duty(&mut self, ns: u64) -> Result<(), String> {
        if ns > self.period_ns {
            return Err(format!(
                "PWM pulse {ns} ns exceeds period {} ns",
                self.period_ns
            ));
        }
        self.io.set_duty_ns(ns)?;
        self.duty_ns = ns;
        Ok(())
    }
    fn enable(&mut self, on: bool) -> Result<(), String> {
        self.io.enable(on)?;
        self.enabled = on;
        Ok(())
    }
}

fn ns_of(v: &Value, unit_if_plain: f64, what: &str) -> Result<u64, String> {
    let secs = match v {
        Value::Quantity(_) => {
            crate::ext::seconds_of(v).ok_or_else(|| format!("{what} expects a time, got {v}"))?
        }
        other => num(what, other)? * unit_if_plain,
    };
    if secs < 0.0 {
        return Err(format!("{what} must be non-negative"));
    }
    Ok((secs * 1e9).round() as u64)
}

fn hz_of(v: &Value) -> Result<f64, String> {
    match v {
        Value::Quantity(q) if q.dim == [0, 0, -1, 0, 0, 0, 0, 0] => Ok(q.si),
        Value::Quantity(_) => Err(format!("frequency() expects Hz, got {v}")),
        other => num("frequency", other),
    }
}

impl Native for Pwm {
    fn type_name(&self) -> &'static str {
        "PWM"
    }
    fn describe(&self) -> String {
        format!(
            "<PWM {}.{} {} Hz duty {:.1}%>",
            self.chip,
            self.ch,
            1e9 / self.period_ns as f64,
            100.0 * self.duty_ns as f64 / self.period_ns as f64
        )
    }
    fn call_method(&mut self, m: &str, a: &[Value]) -> Res {
        match m {
            "frequency" | "set_frequency" => {
                let hz = hz_of(arg(a, 0, "frequency")?)?;
                if hz <= 0.0 {
                    return Err("frequency must be > 0".into());
                }
                let frac = self.duty_ns as f64 / self.period_ns as f64;
                let period = (1e9 / hz).round() as u64;
                self.set_period(period)?;
                self.set_duty((frac * period as f64).round() as u64)?;
                Ok(Value::Nil)
            }
            "period" => {
                self.set_period(ns_of(arg(a, 0, "period")?, 1e-9, "period")?)?;
                Ok(Value::Nil)
            }
            "duty" | "set_duty" => {
                let f = num("duty", arg(a, 0, "fraction")?)?;
                if !(0.0..=1.0).contains(&f) {
                    return Err(format!("duty fraction {f} outside 0..1"));
                }
                self.set_duty((f * self.period_ns as f64).round() as u64)?;
                Ok(Value::Nil)
            }
            "pulse" | "pulse_width" => {
                self.set_duty(ns_of(arg(a, 0, "pulse")?, 1e-6, "pulse")?)?; // plain numbers are µs
                Ok(Value::Nil)
            }
            "enable" | "on" => {
                self.enable(true)?;
                Ok(Value::Nil)
            }
            "disable" | "off" => {
                self.enable(false)?;
                Ok(Value::Nil)
            }
            "state" => {
                let mut d = crate::value::Dict::default();
                d.set(Value::str("period_ns"), Value::Int(self.period_ns as i64))?;
                d.set(Value::str("duty_ns"), Value::Int(self.duty_ns as i64))?;
                d.set(Value::str("enabled"), Value::Bool(self.enabled))?;
                Ok(Value::dict(d))
            }
            _ => Err(format!("PWM has no method '{m}'")),
        }
    }
}

/// Hobby servo on a PWM channel (50 Hz, pulse width proportional to angle).
pub struct Servo {
    pwm: Pwm,
    min_us: f64,
    max_us: f64,
    range_deg: f64,
    angle: Option<f64>,
}

impl Native for Servo {
    fn type_name(&self) -> &'static str {
        "Servo"
    }
    fn describe(&self) -> String {
        match self.angle {
            Some(a) => format!("<Servo {}.{} at {a} deg>", self.pwm.chip, self.pwm.ch),
            None => format!("<Servo {}.{} detached>", self.pwm.chip, self.pwm.ch),
        }
    }
    fn call_method(&mut self, m: &str, a: &[Value]) -> Res {
        match m {
            "angle" | "write" | "move_to" => {
                let Some(v) = a.first() else {
                    return Ok(match self.angle {
                        Some(x) => Value::Quantity(crate::units::Quantity::new(
                            x,
                            std::rc::Rc::new(crate::units::parse("deg").unwrap()),
                        )),
                        None => Value::Nil,
                    });
                };
                let deg = match v {
                    Value::Quantity(_) => crate::ext::radians_of(v)
                        .ok_or_else(|| format!("Servo.angle expects an angle, got {v}"))?
                        .to_degrees(),
                    other => num("angle", other)?,
                };
                if deg < -1e-9 || deg > self.range_deg + 1e-9 {
                    return Err(format!(
                        "servo angle {deg:.2} deg outside 0..{} deg",
                        self.range_deg
                    ));
                }
                let us = self.min_us + (self.max_us - self.min_us) * deg / self.range_deg;
                self.pwm.set_duty((us * 1000.0).round() as u64)?;
                if !self.pwm.enabled {
                    self.pwm.enable(true)?;
                }
                self.angle = Some(deg);
                Ok(Value::Nil)
            }
            "center" => self.call_method("angle", &[Value::Float(self.range_deg / 2.0)]),
            "detach" => {
                self.pwm.enable(false)?;
                self.angle = None;
                Ok(Value::Nil)
            }
            "pulse_us" => Ok(Value::Float(self.pwm.duty_ns as f64 / 1000.0)),
            _ => Err(format!("Servo has no method '{m}'")),
        }
    }
}

fn open_pwm(i: &mut Interpreter, chip: u32, ch: u32) -> Result<Pwm, String> {
    let io = make_io!(
        i,
        format!("pwm{chip}.{ch}"),
        dyn PwmIo,
        match i.backend {
            Backend::Sim => Box::new(sim::SimPwm::new(i.sim.clone(), chip, ch)),
            Backend::Linux => linux_pwm(chip, ch)?,
        }
    );
    let mut p = Pwm {
        chip,
        ch,
        period_ns: 20_000_000,
        duty_ns: 0,
        enabled: false,
        io,
    };
    p.io.set_period_ns(p.period_ns)?;
    Ok(p)
}

fn b_pwm_new(i: &mut Interpreter, a: Vec<Value>) -> Res {
    pad_args("PWM::new", &a, 0, 2)?;
    let chip = a
        .first()
        .map(|v| int("PWM::new", v))
        .transpose()?
        .unwrap_or(0) as u32;
    let ch = a
        .get(1)
        .map(|v| int("PWM::new", v))
        .transpose()?
        .unwrap_or(0) as u32;
    Ok(Value::native(open_pwm(i, chip, ch)?))
}

fn b_servo_new(i: &mut Interpreter, a: Vec<Value>) -> Res {
    pad_args("Servo::new", &a, 0, 5)?;
    let g = |k: usize, d: f64| {
        a.get(k)
            .map(|v| num("Servo::new", v))
            .transpose()
            .map(|x| x.unwrap_or(d))
    };
    let (chip, ch) = (g(0, 0.0)? as u32, g(1, 0.0)? as u32);
    let (min_us, max_us, range_deg) = (g(2, 500.0)?, g(3, 2500.0)?, g(4, 180.0)?);
    if min_us >= max_us || range_deg <= 0.0 {
        return Err("Servo::new: need min_us < max_us and range > 0".into());
    }
    let pwm = open_pwm(i, chip, ch)?;
    Ok(Value::native(Servo {
        pwm,
        min_us,
        max_us,
        range_deg,
        angle: None,
    }))
}

fn b_i2c_scan(i: &mut Interpreter, a: Vec<Value>) -> Res {
    pad_args("I2C::scan", &a, 0, 1)?;
    let bus = a
        .first()
        .map(|v| int("I2C::scan", v))
        .transpose()?
        .unwrap_or(1) as u32;
    let found: Vec<u16> = match i.backend {
        Backend::Sim => i.sim.borrow().i2c_addresses(bus),
        Backend::Linux => linux_scan(bus)?,
    };
    Ok(Value::list(
        found.into_iter().map(|x| Value::Int(x as i64)).collect(),
    ))
}

fn b_pid_new(_: &mut Interpreter, a: Vec<Value>) -> Res {
    pad_args("PID::new", &a, 3, 3)?;
    Ok(Value::native(Pid {
        kp: num("PID::new", &a[0])?,
        ki: num("PID::new", &a[1])?,
        kd: num("PID::new", &a[2])?,
        integral: 0.0,
        prev_err: None,
        limits: None,
    }))
}

#[cfg(target_os = "linux")]
fn linux_pwm(chip: u32, ch: u32) -> Result<Box<dyn PwmIo>, String> {
    Ok(Box::new(linux::SysfsPwm::open(chip, ch)?))
}
#[cfg(target_os = "linux")]
fn linux_scan(bus: u32) -> Result<Vec<u16>, String> {
    linux::scan(bus)
}
#[cfg(not(target_os = "linux"))]
fn linux_pwm(_: u32, _: u32) -> Result<Box<dyn PwmIo>, String> {
    unsupported()
}
#[cfg(not(target_os = "linux"))]
fn linux_scan(_: u32) -> Result<Vec<u16>, String> {
    unsupported()
}
#[cfg(target_os = "linux")]
fn linux_gpio(pin: u32) -> Result<Box<dyn GpioIo>, String> {
    Ok(Box::new(linux::SysfsGpio::open(pin)?))
}
#[cfg(target_os = "linux")]
fn linux_i2c(bus: u32, addr: u16) -> Result<Box<dyn I2cIo>, String> {
    Ok(Box::new(linux::LinuxI2c::open(bus, addr)?))
}
#[cfg(target_os = "linux")]
fn linux_spi(bus: u32, cs: u32) -> Result<Box<dyn SpiIo>, String> {
    Ok(Box::new(linux::LinuxSpi::open(bus, cs)?))
}
#[cfg(target_os = "linux")]
fn linux_uart(port: &str, baud: u32) -> Result<Box<dyn UartIo>, String> {
    Ok(Box::new(linux::LinuxUart::open(port, baud)?))
}

#[cfg(not(target_os = "linux"))]
fn unsupported<T>() -> Result<T, String> {
    Err(
        "the hardware backend is only available on Linux; run without --hw to use the simulator"
            .into(),
    )
}
#[cfg(not(target_os = "linux"))]
fn linux_gpio(_: u32) -> Result<Box<dyn GpioIo>, String> {
    unsupported()
}
#[cfg(not(target_os = "linux"))]
fn linux_i2c(_: u32, _: u16) -> Result<Box<dyn I2cIo>, String> {
    unsupported()
}
#[cfg(not(target_os = "linux"))]
fn linux_spi(_: u32, _: u32) -> Result<Box<dyn SpiIo>, String> {
    unsupported()
}
#[cfg(not(target_os = "linux"))]
fn linux_uart(_: &str, _: u32) -> Result<Box<dyn UartIo>, String> {
    unsupported()
}

// ---------------------------------------------------------------------------- sim:: module

fn sim_only(i: &Interpreter, f: &str) -> Result<(), String> {
    if i.backend == Backend::Sim {
        Ok(())
    } else {
        Err(format!(
            "sim::{f} is only available with the simulator backend"
        ))
    }
}

const SIM: &[(&str, NativeFn)] = &[
    ("pwm", |i, a| {
        sim_only(i, "pwm")?;
        pad_args("sim::pwm", &a, 2, 2)?;
        let (chip, ch) = (int("pwm", &a[0])? as u32, int("pwm", &a[1])? as u32);
        let (p, d, e) = i.sim.borrow().pwm_state(chip, ch);
        let mut dict = crate::value::Dict::default();
        dict.set(Value::str("period_ns"), Value::Int(p as i64))?;
        dict.set(Value::str("duty_ns"), Value::Int(d as i64))?;
        dict.set(Value::str("enabled"), Value::Bool(e))?;
        Ok(Value::dict(dict))
    }),
    ("backend", |i, _| {
        Ok(Value::str(if i.backend == Backend::Sim {
            "sim"
        } else {
            "linux"
        }))
    }),
    ("gpio_input", |i, a| {
        sim_only(i, "gpio_input")?;
        pad_args("sim::gpio_input", &a, 2, 2)?;
        i.sim
            .borrow_mut()
            .gpio_drive(int("gpio_input", &a[0])? as u32, a[1].truthy());
        Ok(Value::Nil)
    }),
    ("gpio_level", |i, a| {
        sim_only(i, "gpio_level")?;
        pad_args("sim::gpio_level", &a, 1, 1)?;
        Ok(Value::Int(
            i.sim.borrow().gpio_level(int("gpio_level", &a[0])? as u32) as i64,
        ))
    }),
    ("i2c_set", |i, a| {
        sim_only(i, "i2c_set")?;
        pad_args("sim::i2c_set", &a, 3, 4)?;
        let bus = a
            .get(3)
            .map(|v| int("i2c_set", v))
            .transpose()?
            .unwrap_or(1) as u32;
        let addr = int("i2c_set", &a[0])? as u16;
        let reg = crate::value::byte(int("i2c_set", &a[1])?)?;
        let data = a[2].to_bytes()?;
        i.sim.borrow_mut().i2c_poke(bus, addr, reg, &data);
        Ok(Value::Nil)
    }),
    ("i2c_get", |i, a| {
        sim_only(i, "i2c_get")?;
        pad_args("sim::i2c_get", &a, 2, 4)?;
        let addr = int("i2c_get", &a[0])? as u16;
        let reg = crate::value::byte(int("i2c_get", &a[1])?)?;
        let n = a
            .get(2)
            .map(|v| int("i2c_get", v))
            .transpose()?
            .unwrap_or(1)
            .max(0) as usize;
        let bus = a
            .get(3)
            .map(|v| int("i2c_get", v))
            .transpose()?
            .unwrap_or(1) as u32;
        Ok(Value::from_bytes(
            &i.sim.borrow_mut().i2c_peek(bus, addr, reg, n),
        ))
    }),
    ("uart_feed", |i, a| {
        sim_only(i, "uart_feed")?;
        pad_args("sim::uart_feed", &a, 2, 2)?;
        let data = a[1].to_bytes()?;
        i.sim.borrow_mut().uart_feed(&a[0].to_string(), &data);
        Ok(Value::Nil)
    }),
    ("uart_sent", |i, a| {
        sim_only(i, "uart_sent")?;
        pad_args("sim::uart_sent", &a, 1, 1)?;
        Ok(Value::from_bytes(
            &i.sim.borrow_mut().uart_take_sent(&a[0].to_string()),
        ))
    }),
    ("uart_loopback", |i, a| {
        sim_only(i, "uart_loopback")?;
        pad_args("sim::uart_loopback", &a, 2, 2)?;
        i.sim
            .borrow_mut()
            .uart_set_loopback(&a[0].to_string(), a[1].truthy());
        Ok(Value::Nil)
    }),
    ("spi_respond", |i, a| {
        sim_only(i, "spi_respond")?;
        pad_args("sim::spi_respond", &a, 3, 3)?;
        let (bus, cs) = (
            int("spi_respond", &a[0])? as u32,
            int("spi_respond", &a[1])? as u32,
        );
        let data = a[2].to_bytes()?;
        i.sim.borrow_mut().spi_queue(bus, cs, data);
        Ok(Value::Nil)
    }),
    ("log", |i, _| {
        Ok(Value::list(
            i.sim
                .borrow()
                .log
                .iter()
                .map(|s| Value::str(s.as_str()))
                .collect(),
        ))
    }),
    ("clear_log", |i, _| {
        i.sim.borrow_mut().log.clear();
        Ok(Value::Nil)
    }),
    ("reset", |i, _| {
        *i.sim.borrow_mut() = SimBus::default();
        Ok(Value::Nil)
    }),
];

/// Resolves device-related `Module::name` paths.
pub fn path(m: &str, name: &str) -> Option<Value> {
    let f: NativeFn = match (m, name) {
        ("GPIO" | "gpio", "new" | "open") => b_gpio_new,
        ("I2C" | "i2c", "scan") => b_i2c_scan,
        ("PWM" | "pwm", "new" | "open") => b_pwm_new,
        ("Servo" | "servo", "new" | "open") => b_servo_new,
        ("I2C" | "i2c", "new" | "open") => b_i2c_new,
        ("SPI" | "spi", "new" | "open") => b_spi_new,
        ("UART" | "uart", "new" | "open") => b_uart_new,
        ("PID" | "control", "new" | "pid") => b_pid_new,
        ("sim", _) => {
            return SIM
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(n, f)| Value::Builtin(n, *f))
        }
        _ => return None,
    };
    let label: &'static str = match m {
        _ if name == "scan" => "I2C::scan",
        "GPIO" | "gpio" => "GPIO::new",
        "I2C" | "i2c" => "I2C::new",
        "SPI" | "spi" => "SPI::new",
        "UART" | "uart" => "UART::new",
        "PWM" | "pwm" => "PWM::new",
        "Servo" | "servo" => "Servo::new",
        _ => "PID::new",
    };
    Some(Value::Builtin(label, f))
}

//! Device integration: GPIO, I2C, SPI, UART and a PID controller.
//!
//! Every device has two backends:
//! * `sim`   – an in-memory bus (default). Deterministic, scriptable via `sim::*`, safe to run anywhere.
//! * `linux` – real hardware through sysfs GPIO, `/dev/i2c-N`, `/dev/spidevB.C` and serial TTYs.

mod sim;
pub use sim::SimBus;

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
    let mut io: Box<dyn GpioIo> = match i.backend {
        Backend::Sim => Box::new(sim::SimGpio::new(i.sim.clone(), pin)),
        Backend::Linux => linux_gpio(pin)?,
    };
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
    let io: Box<dyn I2cIo> = match i.backend {
        Backend::Sim => Box::new(sim::SimI2c::new(i.sim.clone(), bus, addr)),
        Backend::Linux => linux_i2c(bus, addr)?,
    };
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
    let io: Box<dyn SpiIo> = match i.backend {
        Backend::Sim => Box::new(sim::SimSpi::new(i.sim.clone(), bus, cs)),
        Backend::Linux => linux_spi(bus, cs)?,
    };
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
    let io: Box<dyn UartIo> = match i.backend {
        Backend::Sim => Box::new(sim::SimUart::new(i.sim.clone(), port.clone())),
        Backend::Linux => linux_uart(&port, baud)?,
    };
    Ok(Value::native(Uart { port, baud, io }))
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
        "GPIO" | "gpio" => "GPIO::new",
        "I2C" | "i2c" => "I2C::new",
        "SPI" | "spi" => "SPI::new",
        "UART" | "uart" => "UART::new",
        _ => "PID::new",
    };
    Some(Value::Builtin(label, f))
}

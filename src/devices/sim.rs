//! In-memory simulated hardware bus.

use super::{GpioIo, I2cIo, PwmIo, SpiIo, UartIo};
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;

#[derive(Default)]
pub struct SimBus {
    gpio: HashMap<u32, (bool, bool)>, // pin -> (is_output, level)
    i2c: HashMap<(u32, u16), I2cRegs>,
    spi: HashMap<(u32, u32), VecDeque<Vec<u8>>>,
    uart_rx: HashMap<String, VecDeque<u8>>,
    uart_tx: HashMap<String, Vec<u8>>,
    uart_no_loop: HashMap<String, bool>,
    pwm: HashMap<(u32, u32), (u64, u64, bool)>,
    pub(crate) can: super::can::SimCan,
    pub(crate) ble: HashMap<String, super::ble::SimGatt>,
    /// Human-readable transaction log, readable from scripts via `sim::log()`.
    pub log: Vec<String>,
}

struct I2cRegs {
    regs: [u8; 256],
    ptr: u8,
}

impl Default for I2cRegs {
    fn default() -> Self {
        I2cRegs {
            regs: [0; 256],
            ptr: 0,
        }
    }
}

fn hex(b: &[u8]) -> String {
    b.iter()
        .map(|x| format!("{x:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

impl SimBus {
    pub(crate) fn note(&mut self, s: String) {
        if self.log.len() >= 10_000 {
            self.log.remove(0);
        }
        self.log.push(s);
    }
    pub fn gpio_drive(&mut self, pin: u32, level: bool) {
        let e = self.gpio.entry(pin).or_insert((false, false));
        e.1 = level;
        self.note(format!("gpio {pin} driven {}", level as u8));
    }
    pub fn gpio_level(&self, pin: u32) -> bool {
        self.gpio.get(&pin).is_some_and(|e| e.1)
    }
    pub fn i2c_poke(&mut self, bus: u32, addr: u16, reg: u8, data: &[u8]) {
        let d = self.i2c.entry((bus, addr)).or_default();
        for (k, b) in data.iter().enumerate() {
            d.regs[reg.wrapping_add(k as u8) as usize] = *b;
        }
    }
    pub fn i2c_peek(&mut self, bus: u32, addr: u16, reg: u8, n: usize) -> Vec<u8> {
        let d = self.i2c.entry((bus, addr)).or_default();
        (0..n)
            .map(|k| d.regs[reg.wrapping_add(k as u8) as usize])
            .collect()
    }
    pub fn uart_feed(&mut self, port: &str, data: &[u8]) {
        self.uart_rx
            .entry(port.to_string())
            .or_default()
            .extend(data.iter().copied());
    }
    pub fn uart_take_sent(&mut self, port: &str) -> Vec<u8> {
        self.uart_tx.remove(port).unwrap_or_default()
    }
    pub fn uart_set_loopback(&mut self, port: &str, on: bool) {
        self.uart_no_loop.insert(port.to_string(), !on);
    }
    pub fn i2c_addresses(&self, bus: u32) -> Vec<u16> {
        let mut v: Vec<u16> = self
            .i2c
            .keys()
            .filter(|(b, _)| *b == bus)
            .map(|(_, a)| *a)
            .collect();
        v.sort_unstable();
        v
    }
    pub fn pwm_state(&self, chip: u32, ch: u32) -> (u64, u64, bool) {
        self.pwm.get(&(chip, ch)).copied().unwrap_or((0, 0, false))
    }
    pub fn spi_queue(&mut self, bus: u32, cs: u32, data: Vec<u8>) {
        self.spi.entry((bus, cs)).or_default().push_back(data);
    }
}

pub struct SimGpio {
    bus: Rc<RefCell<SimBus>>,
    pin: u32,
}

impl SimGpio {
    pub fn new(bus: Rc<RefCell<SimBus>>, pin: u32) -> Self {
        SimGpio { bus, pin }
    }
}

impl GpioIo for SimGpio {
    fn set_output(&mut self, out: bool) -> Result<(), String> {
        let mut b = self.bus.borrow_mut();
        b.gpio.entry(self.pin).or_insert((out, false)).0 = out;
        b.note(format!(
            "gpio {} direction {}",
            self.pin,
            if out { "out" } else { "in" }
        ));
        Ok(())
    }
    fn write(&mut self, high: bool) -> Result<(), String> {
        let mut b = self.bus.borrow_mut();
        b.gpio.entry(self.pin).or_insert((true, false)).1 = high;
        b.note(format!("gpio {} <- {}", self.pin, high as u8));
        Ok(())
    }
    fn read(&mut self) -> Result<bool, String> {
        Ok(self.bus.borrow().gpio_level(self.pin))
    }
}

pub struct SimI2c {
    bus: Rc<RefCell<SimBus>>,
    id: (u32, u16),
}

impl SimI2c {
    pub fn new(bus: Rc<RefCell<SimBus>>, n: u32, addr: u16) -> Self {
        bus.borrow_mut().i2c.entry((n, addr)).or_default();
        SimI2c { bus, id: (n, addr) }
    }
}

/// Register-file semantics like most I2C sensors: the first written byte sets the register
/// pointer, following bytes are stored at successive registers; reads start at the pointer
/// and auto-increment.
impl I2cIo for SimI2c {
    fn write(&mut self, data: &[u8]) -> Result<(), String> {
        let mut b = self.bus.borrow_mut();
        let d = b.i2c.get_mut(&self.id).unwrap();
        if let Some((first, rest)) = data.split_first() {
            d.ptr = *first;
            for x in rest {
                d.regs[d.ptr as usize] = *x;
                d.ptr = d.ptr.wrapping_add(1);
            }
        }
        b.note(format!(
            "i2c {}:0x{:02x} write [{}]",
            self.id.0,
            self.id.1,
            hex(data)
        ));
        Ok(())
    }
    fn read(&mut self, n: usize) -> Result<Vec<u8>, String> {
        let mut b = self.bus.borrow_mut();
        let d = b.i2c.get_mut(&self.id).unwrap();
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            out.push(d.regs[d.ptr as usize]);
            d.ptr = d.ptr.wrapping_add(1);
        }
        b.note(format!(
            "i2c {}:0x{:02x} read [{}]",
            self.id.0,
            self.id.1,
            hex(&out)
        ));
        Ok(out)
    }
    fn write_read(&mut self, data: &[u8], n: usize) -> Result<Vec<u8>, String> {
        self.write(data)?;
        self.read(n)
    }
}

pub struct SimSpi {
    bus: Rc<RefCell<SimBus>>,
    id: (u32, u32),
}

impl SimSpi {
    pub fn new(bus: Rc<RefCell<SimBus>>, n: u32, cs: u32) -> Self {
        SimSpi { bus, id: (n, cs) }
    }
}

/// Returns the next queued response (see `sim::spi_respond`), otherwise loops MOSI back to MISO.
impl SpiIo for SimSpi {
    fn transfer(&mut self, data: &[u8]) -> Result<Vec<u8>, String> {
        let mut b = self.bus.borrow_mut();
        let mut out = match b.spi.get_mut(&self.id).and_then(|q| q.pop_front()) {
            Some(r) => r,
            None => data.to_vec(),
        };
        out.resize(data.len(), 0);
        b.note(format!(
            "spi {}.{} tx [{}] rx [{}]",
            self.id.0,
            self.id.1,
            hex(data),
            hex(&out)
        ));
        Ok(out)
    }
    fn set_speed(&mut self, hz: u32) -> Result<(), String> {
        self.bus
            .borrow_mut()
            .note(format!("spi {}.{} speed {hz} Hz", self.id.0, self.id.1));
        Ok(())
    }
    fn set_mode(&mut self, mode: u8) -> Result<(), String> {
        self.bus
            .borrow_mut()
            .note(format!("spi {}.{} mode {mode}", self.id.0, self.id.1));
        Ok(())
    }
}

pub struct SimUart {
    bus: Rc<RefCell<SimBus>>,
    port: String,
}

impl SimUart {
    pub fn new(bus: Rc<RefCell<SimBus>>, port: String) -> Self {
        SimUart { bus, port }
    }
}

/// Loopback by default: bytes sent come back on receive. `sim::uart_loopback(port, false)`
/// disables that so a script can play the remote end with `sim::uart_feed`.
impl UartIo for SimUart {
    fn send(&mut self, data: &[u8]) -> Result<(), String> {
        let mut b = self.bus.borrow_mut();
        b.uart_tx
            .entry(self.port.clone())
            .or_default()
            .extend_from_slice(data);
        if !b.uart_no_loop.get(&self.port).copied().unwrap_or(false) {
            b.uart_rx
                .entry(self.port.clone())
                .or_default()
                .extend(data.iter().copied());
        }
        let shown = String::from_utf8_lossy(data).escape_debug().to_string();
        b.note(format!("uart {} tx \"{shown}\"", self.port));
        Ok(())
    }
    fn receive(&mut self, max: usize, _timeout_ms: u64) -> Result<Vec<u8>, String> {
        let mut b = self.bus.borrow_mut();
        let q = b.uart_rx.entry(self.port.clone()).or_default();
        let n = max.min(q.len());
        let out: Vec<u8> = q.drain(..n).collect();
        if !out.is_empty() {
            let shown = String::from_utf8_lossy(&out).escape_debug().to_string();
            b.note(format!("uart {} rx \"{shown}\"", self.port));
        }
        Ok(out)
    }
    fn available(&mut self) -> Result<usize, String> {
        Ok(self
            .bus
            .borrow()
            .uart_rx
            .get(&self.port)
            .map_or(0, |q| q.len()))
    }
}

pub struct SimPwm {
    bus: Rc<RefCell<SimBus>>,
    id: (u32, u32),
}

impl SimPwm {
    pub fn new(bus: Rc<RefCell<SimBus>>, chip: u32, ch: u32) -> Self {
        bus.borrow_mut()
            .pwm
            .entry((chip, ch))
            .or_insert((0, 0, false));
        SimPwm {
            bus,
            id: (chip, ch),
        }
    }
}

impl PwmIo for SimPwm {
    fn set_period_ns(&mut self, ns: u64) -> Result<(), String> {
        let mut b = self.bus.borrow_mut();
        b.pwm.get_mut(&self.id).unwrap().0 = ns;
        b.note(format!("pwm {}.{} period {ns} ns", self.id.0, self.id.1));
        Ok(())
    }
    fn set_duty_ns(&mut self, ns: u64) -> Result<(), String> {
        let mut b = self.bus.borrow_mut();
        b.pwm.get_mut(&self.id).unwrap().1 = ns;
        b.note(format!("pwm {}.{} duty {ns} ns", self.id.0, self.id.1));
        Ok(())
    }
    fn enable(&mut self, on: bool) -> Result<(), String> {
        let mut b = self.bus.borrow_mut();
        b.pwm.get_mut(&self.id).unwrap().2 = on;
        b.note(format!(
            "pwm {}.{} {}",
            self.id.0,
            self.id.1,
            if on { "enabled" } else { "disabled" }
        ));
        Ok(())
    }
}

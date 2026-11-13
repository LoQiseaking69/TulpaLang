//! Real hardware on Linux: sysfs GPIO, i2c-dev, spidev and termios serial ports.

use super::{GpioIo, I2cIo, PwmIo, SpiIo, UartIo};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::io::AsRawFd;
use std::path::Path;

fn os_err(what: &str) -> String {
    format!("{what}: {}", std::io::Error::last_os_error())
}

// ---------------------------------------------------------------------------- GPIO (sysfs)

/// Sysfs GPIO. On kernels where the GPIO numbering is offset (e.g. Raspberry Pi 5 / kernel
/// 6.6+, where BCM17 is 588), set `TULPA_GPIO_BASE` to the chip base and keep using BCM numbers.
pub struct SysfsGpio {
    root: String,
}

impl SysfsGpio {
    pub fn open(pin: u32) -> Result<Self, String> {
        let base: u32 = std::env::var("TULPA_GPIO_BASE")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let n = pin + base;
        let root = format!("/sys/class/gpio/gpio{n}");
        if !Path::new(&root).exists() {
            std::fs::write("/sys/class/gpio/export", n.to_string()).map_err(|e| {
                format!("GPIO {pin}: cannot export via /sys/class/gpio/export ({e})")
            })?;
            // udev needs a moment to fix permissions on the new nodes.
            for _ in 0..50 {
                if std::fs::metadata(format!("{root}/value"))
                    .map(|m| !m.permissions().readonly())
                    .unwrap_or(false)
                {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
        Ok(SysfsGpio { root })
    }
}

impl GpioIo for SysfsGpio {
    fn set_output(&mut self, out: bool) -> Result<(), String> {
        std::fs::write(
            format!("{}/direction", self.root),
            if out { "out" } else { "in" },
        )
        .map_err(|e| format!("{}: set direction: {e}", self.root))
    }
    fn write(&mut self, high: bool) -> Result<(), String> {
        std::fs::write(format!("{}/value", self.root), if high { "1" } else { "0" })
            .map_err(|e| format!("{}: write: {e}", self.root))
    }
    fn read(&mut self) -> Result<bool, String> {
        let s = std::fs::read_to_string(format!("{}/value", self.root))
            .map_err(|e| format!("{}: read: {e}", self.root))?;
        Ok(s.trim() == "1")
    }
}

// ---------------------------------------------------------------------------- I2C

const I2C_SLAVE: u64 = 0x0703;
const I2C_RDWR: u64 = 0x0707;
const I2C_M_RD: u16 = 0x0001;

#[repr(C)]
struct I2cMsg {
    addr: u16,
    flags: u16,
    len: u16,
    buf: *mut u8,
}

#[repr(C)]
struct I2cRdwr {
    msgs: *mut I2cMsg,
    nmsgs: u32,
}

pub struct LinuxI2c {
    file: File,
    addr: u16,
}

impl LinuxI2c {
    pub fn open(bus: u32, addr: u16) -> Result<Self, String> {
        let path = format!("/dev/i2c-{bus}");
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|e| format!("{path}: {e}"))?;
        // SAFETY: valid fd, I2C_SLAVE takes the address as an integer argument.
        if unsafe { libc::ioctl(file.as_raw_fd(), I2C_SLAVE as _, addr as libc::c_ulong) } < 0 {
            return Err(os_err(&format!("{path}: select address 0x{addr:02x}")));
        }
        Ok(LinuxI2c { file, addr })
    }
}

impl I2cIo for LinuxI2c {
    fn write(&mut self, data: &[u8]) -> Result<(), String> {
        self.file
            .write_all(data)
            .map_err(|e| format!("i2c write: {e}"))
    }
    fn read(&mut self, n: usize) -> Result<Vec<u8>, String> {
        let mut buf = vec![0u8; n];
        self.file
            .read_exact(&mut buf)
            .map_err(|e| format!("i2c read: {e}"))?;
        Ok(buf)
    }
    fn write_read(&mut self, data: &[u8], n: usize) -> Result<Vec<u8>, String> {
        let mut wbuf = data.to_vec();
        let mut rbuf = vec![0u8; n];
        let mut msgs = [
            I2cMsg {
                addr: self.addr,
                flags: 0,
                len: wbuf.len() as u16,
                buf: wbuf.as_mut_ptr(),
            },
            I2cMsg {
                addr: self.addr,
                flags: I2C_M_RD,
                len: n as u16,
                buf: rbuf.as_mut_ptr(),
            },
        ];
        let mut req = I2cRdwr {
            msgs: msgs.as_mut_ptr(),
            nmsgs: 2,
        };
        // SAFETY: buffers outlive the call; struct layouts match <linux/i2c-dev.h>.
        if unsafe {
            libc::ioctl(
                self.file.as_raw_fd(),
                I2C_RDWR as _,
                &mut req as *mut I2cRdwr,
            )
        } < 0
        {
            return Err(os_err("i2c write_read"));
        }
        Ok(rbuf)
    }
}

// ---------------------------------------------------------------------------- SPI

const SPI_IOC_MESSAGE_1: u64 = 0x4020_6b00;
const SPI_IOC_WR_MODE: u64 = 0x4001_6b01;
const SPI_IOC_WR_MAX_SPEED_HZ: u64 = 0x4004_6b04;

#[repr(C)]
#[derive(Default)]
struct SpiIocTransfer {
    tx_buf: u64,
    rx_buf: u64,
    len: u32,
    speed_hz: u32,
    delay_usecs: u16,
    bits_per_word: u8,
    cs_change: u8,
    tx_nbits: u8,
    rx_nbits: u8,
    word_delay_usecs: u8,
    pad: u8,
}

pub struct LinuxSpi {
    file: File,
    speed: u32,
}

impl LinuxSpi {
    pub fn open(bus: u32, cs: u32) -> Result<Self, String> {
        let path = format!("/dev/spidev{bus}.{cs}");
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|e| format!("{path}: {e}"))?;
        Ok(LinuxSpi {
            file,
            speed: 1_000_000,
        })
    }
}

impl SpiIo for LinuxSpi {
    fn transfer(&mut self, data: &[u8]) -> Result<Vec<u8>, String> {
        let mut rx = vec![0u8; data.len()];
        let xfer = SpiIocTransfer {
            tx_buf: data.as_ptr() as u64,
            rx_buf: rx.as_mut_ptr() as u64,
            len: data.len() as u32,
            speed_hz: self.speed,
            bits_per_word: 8,
            ..Default::default()
        };
        // SAFETY: buffers outlive the call; layout matches <linux/spi/spidev.h>.
        if unsafe {
            libc::ioctl(
                self.file.as_raw_fd(),
                SPI_IOC_MESSAGE_1 as _,
                &xfer as *const SpiIocTransfer,
            )
        } < 0
        {
            return Err(os_err("spi transfer"));
        }
        Ok(rx)
    }
    fn set_speed(&mut self, hz: u32) -> Result<(), String> {
        // SAFETY: pointer to a live u32.
        if unsafe {
            libc::ioctl(
                self.file.as_raw_fd(),
                SPI_IOC_WR_MAX_SPEED_HZ as _,
                &hz as *const u32,
            )
        } < 0
        {
            return Err(os_err("spi set_speed"));
        }
        self.speed = hz;
        Ok(())
    }
    fn set_mode(&mut self, mode: u8) -> Result<(), String> {
        // SAFETY: pointer to a live u8.
        if unsafe {
            libc::ioctl(
                self.file.as_raw_fd(),
                SPI_IOC_WR_MODE as _,
                &mode as *const u8,
            )
        } < 0
        {
            return Err(os_err("spi set_mode"));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------- UART

pub struct LinuxUart {
    file: File,
}

fn baud_const(baud: u32) -> Result<libc::speed_t, String> {
    Ok(match baud {
        1200 => libc::B1200,
        2400 => libc::B2400,
        4800 => libc::B4800,
        9600 => libc::B9600,
        19200 => libc::B19200,
        38400 => libc::B38400,
        57600 => libc::B57600,
        115200 => libc::B115200,
        230400 => libc::B230400,
        460800 => libc::B460800,
        921600 => libc::B921600,
        1000000 => libc::B1000000,
        other => return Err(format!("unsupported baud rate {other}")),
    })
}

impl LinuxUart {
    pub fn open(port: &str, baud: u32) -> Result<Self, String> {
        use std::os::unix::fs::OpenOptionsExt;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NOCTTY)
            .open(port)
            .map_err(|e| format!("{port}: {e}"))?;
        let fd = file.as_raw_fd();
        let speed = baud_const(baud)?;
        // SAFETY: termios is plain data, fd is valid.
        unsafe {
            let mut t: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(fd, &mut t) != 0 {
                return Err(os_err(&format!("{port}: tcgetattr")));
            }
            libc::cfmakeraw(&mut t);
            t.c_cflag |= libc::CLOCAL | libc::CREAD;
            t.c_cc[libc::VMIN] = 0;
            t.c_cc[libc::VTIME] = 0;
            libc::cfsetispeed(&mut t, speed);
            libc::cfsetospeed(&mut t, speed);
            if libc::tcsetattr(fd, libc::TCSANOW, &t) != 0 {
                return Err(os_err(&format!("{port}: tcsetattr")));
            }
            libc::tcflush(fd, libc::TCIOFLUSH);
        }
        Ok(LinuxUart { file })
    }
}

impl UartIo for LinuxUart {
    fn send(&mut self, data: &[u8]) -> Result<(), String> {
        self.file
            .write_all(data)
            .map_err(|e| format!("uart send: {e}"))?;
        self.file.flush().map_err(|e| format!("uart flush: {e}"))
    }
    fn receive(&mut self, max: usize, timeout_ms: u64) -> Result<Vec<u8>, String> {
        let mut out = Vec::with_capacity(max);
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
        while out.len() < max {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            let mut pfd = libc::pollfd {
                fd: self.file.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            // SAFETY: single valid pollfd.
            let r = unsafe { libc::poll(&mut pfd, 1, left.as_millis() as libc::c_int) };
            if r < 0 {
                return Err(os_err("uart poll"));
            }
            if r == 0 {
                break; // timeout
            }
            let mut buf = vec![0u8; max - out.len()];
            let n = self
                .file
                .read(&mut buf)
                .map_err(|e| format!("uart read: {e}"))?;
            if n == 0 {
                break;
            }
            out.extend_from_slice(&buf[..n]);
        }
        Ok(out)
    }
    fn available(&mut self) -> Result<usize, String> {
        let mut n: libc::c_int = 0;
        // SAFETY: FIONREAD writes an int.
        if unsafe {
            libc::ioctl(
                self.file.as_raw_fd(),
                libc::FIONREAD as _,
                &mut n as *mut libc::c_int,
            )
        } < 0
        {
            return Err(os_err("uart available"));
        }
        Ok(n.max(0) as usize)
    }
}

// ---------------------------------------------------------------------------- PWM (sysfs)

pub struct SysfsPwm {
    root: String,
}

impl SysfsPwm {
    pub fn open(chip: u32, ch: u32) -> Result<Self, String> {
        let chip_dir = format!("/sys/class/pwm/pwmchip{chip}");
        let root = format!("{chip_dir}/pwm{ch}");
        if !Path::new(&root).exists() {
            std::fs::write(format!("{chip_dir}/export"), ch.to_string()).map_err(|e| {
                format!("PWM {chip}.{ch}: cannot export via {chip_dir}/export ({e})")
            })?;
            for _ in 0..50 {
                if Path::new(&format!("{root}/period")).exists() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
        Ok(SysfsPwm { root })
    }
    fn put(&self, file: &str, v: impl ToString) -> Result<(), String> {
        std::fs::write(format!("{}/{file}", self.root), v.to_string())
            .map_err(|e| format!("{}/{file}: {e}", self.root))
    }
}

impl PwmIo for SysfsPwm {
    fn set_period_ns(&mut self, ns: u64) -> Result<(), String> {
        self.put("period", ns)
    }
    fn set_duty_ns(&mut self, ns: u64) -> Result<(), String> {
        self.put("duty_cycle", ns)
    }
    fn enable(&mut self, on: bool) -> Result<(), String> {
        self.put("enable", if on { 1 } else { 0 })
    }
}

/// Probes 7-bit addresses 0x03..=0x77 with a 1-byte read (like `i2cdetect -r`).
pub fn scan(bus: u32) -> Result<Vec<u16>, String> {
    let path = format!("/dev/i2c-{bus}");
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|e| format!("{path}: {e}"))?;
    let mut found = Vec::new();
    for addr in 0x03u16..=0x77 {
        // SAFETY: valid fd; I2C_SLAVE takes the address as an integer argument.
        if unsafe { libc::ioctl(file.as_raw_fd(), I2C_SLAVE as _, addr as libc::c_ulong) } < 0 {
            continue;
        }
        let mut b = [0u8; 1];
        if file.read(&mut b).is_ok() {
            found.push(addr);
        }
    }
    Ok(found)
}

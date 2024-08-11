use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::Path;
use std::thread;
use std::time::Duration;

/// Represents an SPI device with advanced functionality.
pub struct SPI {
    bus: u8,
    cs_pin: u8,
    device_file: Option<File>,
    log_enabled: bool,
}

impl SPI {
    /// Creates a new SPI instance with the specified bus and chip select pin.
    pub fn new(bus: u8, cs_pin: u8) -> io::Result<Self> {
        let mut spi = SPI {
            bus,
            cs_pin,
            device_file: None,
            log_enabled: false,
        };
        spi.open_device_file()?;
        spi.log(&format!("SPI device initialized on bus {} with chip select pin {}", bus, cs_pin));
        Ok(spi)
    }

    /// Opens the SPI device file for reading and writing.
    fn open_device_file(&mut self) -> io::Result<()> {
        let device_path = format!("/dev/spidev{}.{}", self.bus, self.cs_pin);
        let file = OpenOptions::new().read(true).write(true).open(&device_path)?;
        self.device_file = Some(file);
        self.log("Device file opened");
        Ok(())
    }

    /// Transfers data to and from the SPI device.
    pub fn transfer(&mut self, data: &[u8]) -> io::Result<Vec<u8>> {
        let mut response = vec![0; data.len()];
        if let Some(ref mut device_file) = self.device_file {
            device_file.write_all(data)?;
            device_file.read_exact(&mut response)?;
            self.log(&format!("Data transferred on bus {}: {:?}", self.bus, response));
        }
        Ok(response)
    }

    /// Enables or disables logging for SPI operations.
    pub fn enable_logging(&mut self, enable: bool) {
        self.log_enabled = enable;
        self.log(&format!("Logging {}", if enable { "enabled" } else { "disabled" }));
    }

    /// Logs a message if logging is enabled.
    fn log(&self, message: &str) {
        if self.log_enabled {
            println!("SPI {}: {}", self.bus, message);
        }
    }

    /// Transfers data with a delay between each byte.
    pub fn transfer_with_delay(&mut self, data: &[u8], delay: Duration) -> io::Result<Vec<u8>> {
        let mut response = vec![0; data.len()];
        for (i, &byte) in data.iter().enumerate() {
            let mut buffer = [byte];
            let mut read_buffer = [0u8; 1];
            if let Some(ref mut device_file) = self.device_file {
                device_file.write_all(&buffer)?;
                thread::sleep(delay);
                device_file.read_exact(&mut read_buffer)?;
                response[i] = read_buffer[0];
            }
        }
        self.log(&format!("Data transferred with delay on bus {}: {:?}", self.bus, response));
        Ok(response)
    }

    /// Configures the SPI bus speed.
    pub fn set_bus_speed(&self, speed_hz: u32) -> io::Result<()> {
        let spi_speed_file = format!("/sys/class/spi_master/spi{}/spi{}/speed_hz", self.bus, self.cs_pin);
        let mut file = OpenOptions::new().write(true).open(spi_speed_file)?;
        writeln!(file, "{}", speed_hz)?;
        self.log(&format!("SPI bus speed set to {} Hz", speed_hz));
        Ok(())
    }

    /// Configures the SPI mode (0, 1, 2, 3).
    pub fn set_mode(&self, mode: u8) -> io::Result<()> {
        if mode > 3 {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "Invalid SPI mode"));
        }
        let spi_mode_file = format!("/sys/class/spi_master/spi{}/spi{}/mode", self.bus, self.cs_pin);
        let mut file = OpenOptions::new().write(true).open(spi_mode_file)?;
        writeln!(file, "{}", mode)?;
        self.log(&format!("SPI mode set to {}", mode));
        Ok(())
    }

    /// Sets the chip select pin to active (low) or inactive (high).
    pub fn set_chip_select(&self, active: bool) -> io::Result<()> {
        let cs_path = format!("/sys/class/gpio/gpio{}/value", self.cs_pin);
        let mut file = OpenOptions::new().write(true).open(cs_path)?;
        if active {
            writeln!(file, "0")?;
            self.log("Chip select set to active (low)");
        } else {
            writeln!(file, "1")?;
            self.log("Chip select set to inactive (high)");
        }
        Ok(())
    }
}

fn main() {
    // Example usage of the SPI library
    let mut spi = SPI::new(0, 0).expect("Failed to initialize SPI");
    spi.enable_logging(true);

    let data_to_transfer = [0xAA, 0xBB, 0xCC];
    let response = spi.transfer(&data_to_transfer).expect("Failed to transfer data");
    println!("Received data: {:?}", response);

    spi.set_bus_speed(1_000_000).expect("Failed to set bus speed");
    spi.set_mode(0).expect("Failed to set SPI mode");
    spi.set_chip_select(true).expect("Failed to set chip select");

    let delayed_data = [0x10, 0x20, 0x30];
    let delayed_response = spi.transfer_with_delay(&delayed_data, Duration::from_millis(100)).expect("Failed to transfer with delay");
    println!("Received data with delay: {:?}", delayed_response);
}


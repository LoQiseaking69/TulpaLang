use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::Path;
use std::thread;
use std::time::Duration;

/// Represents an I2C device with advanced functionality.
pub struct I2C {
    address: u16,
    bus: u8,
    device_file: Option<File>,
    log_enabled: bool,
}

impl I2C {
    /// Creates a new I2C instance with the specified bus and address.
    pub fn new(bus: u8, address: u16) -> io::Result<Self> {
        let mut i2c = I2C {
            address,
            bus,
            device_file: None,
            log_enabled: false,
        };
        i2c.open_device_file()?;
        i2c.log(&format!("I2C device initialized on bus {} with address 0x{:02x}", bus, address));
        Ok(i2c)
    }

    /// Opens the I2C device file for reading and writing.
    fn open_device_file(&mut self) -> io::Result<()> {
        let device_path = format!("/dev/i2c-{}", self.bus);
        let file = OpenOptions::new().read(true).write(true).open(&device_path)?;
        self.device_file = Some(file);
        self.log("Device file opened");
        Ok(())
    }

    /// Writes data to the I2C device.
    pub fn write(&mut self, data: &[u8]) -> io::Result<()> {
        if let Some(ref mut device_file) = self.device_file {
            device_file.write_all(data)?;
            self.log(&format!("Data written to 0x{:02x}: {:?}", self.address, data));
        }
        Ok(())
    }

    /// Reads data from the I2C device into the provided buffer.
    pub fn read(&mut self, buffer: &mut [u8]) -> io::Result<()> {
        if let Some(ref mut device_file) = self.device_file {
            device_file.read_exact(buffer)?;
            self.log(&format!("Data read from 0x{:02x}: {:?}", self.address, buffer));
        }
        Ok(())
    }

    /// Enables or disables logging for I2C operations.
    pub fn enable_logging(&mut self, enable: bool) {
        self.log_enabled = enable;
        self.log(&format!("Logging {}", if enable { "enabled" } else { "disabled" }));
    }

    /// Logs a message if logging is enabled.
    fn log(&self, message: &str) {
        if self.log_enabled {
            println!("I2C {}: {}", self.address, message);
        }
    }

    /// Writes and then reads data from the I2C device in a single transaction.
    pub fn write_then_read(&mut self, write_data: &[u8], read_buffer: &mut [u8]) -> io::Result<()> {
        self.write(write_data)?;
        thread::sleep(Duration::from_millis(50)); // Add a delay between write and read if necessary
        self.read(read_buffer)?;
        self.log("Write-then-read transaction completed");
        Ok(())
    }

    /// Reads a single byte from the I2C device.
    pub fn read_byte(&mut self) -> io::Result<u8> {
        let mut buffer = [0u8; 1];
        self.read(&mut buffer)?;
        self.log(&format!("Byte read from 0x{:02x}: 0x{:02x}", self.address, buffer[0]));
        Ok(buffer[0])
    }

    /// Writes a single byte to the I2C device.
    pub fn write_byte(&mut self, byte: u8) -> io::Result<()> {
        self.write(&[byte])?;
        self.log(&format!("Byte written to 0x{:02x}: 0x{:02x}", self.address, byte));
        Ok(())
    }

    /// Reads multiple bytes from the I2C device with a delay between each byte.
    pub fn read_with_delay(&mut self, buffer: &mut [u8], delay: Duration) -> io::Result<()> {
        for byte in buffer.iter_mut() {
            *byte = self.read_byte()?;
            thread::sleep(delay);
        }
        self.log(&format!("Data read with delay from 0x{:02x}: {:?}", self.address, buffer));
        Ok(())
    }

    /// Writes multiple bytes to the I2C device with a delay between each byte.
    pub fn write_with_delay(&mut self, data: &[u8], delay: Duration) -> io::Result<()> {
        for &byte in data.iter() {
            self.write_byte(byte)?;
            thread::sleep(delay);
        }
        self.log(&format!("Data written with delay to 0x{:02x}: {:?}", self.address, data));
        Ok(())
    }
}

fn main() {
    // Example usage of the I2C library
    let mut i2c = I2C::new(1, 0x68).expect("Failed to initialize I2C");
    i2c.enable_logging(true);

    let write_data = [0x00, 0x01, 0x02];
    i2c.write(&write_data).expect("Failed to write data");

    let mut read_buffer = [0u8; 3];
    i2c.read(&mut read_buffer).expect("Failed to read data");
    println!("Read data: {:?}", read_buffer);

    let single_byte = i2c.read_byte().expect("Failed to read byte");
    println!("Read single byte: 0x{:02x}", single_byte);

    i2c.write_byte(0xFF).expect("Failed to write byte");

    let mut delayed_buffer = [0u8; 3];
    i2c.read_with_delay(&mut delayed_buffer, Duration::from_millis(500)).expect("Failed to read with delay");

    let delayed_write_data = [0x10, 0x20, 0x30];
    i2c.write_with_delay(&delayed_write_data, Duration::from_millis(500)).expect("Failed to write with delay");
}


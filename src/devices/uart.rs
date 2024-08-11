use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::Path;
use std::time::Duration;
use std::thread;

/// Represents a UART device with enhanced functionality.
pub struct UART {
    port: String,
    baud_rate: u32,
    device_file: Option<File>,
    log_enabled: bool,
}

impl UART {
    /// Creates a new UART instance with the specified port and baud rate.
    pub fn new(port: &str, baud_rate: u32) -> io::Result<Self> {
        let mut uart = UART {
            port: port.to_string(),
            baud_rate,
            device_file: None,
            log_enabled: false,
        };
        uart.open_device_file()?;
        uart.configure_baud_rate()?;
        uart.log(&format!("UART initialized on port {} with baud rate {}", port, baud_rate));
        Ok(uart)
    }

    /// Opens the UART device file for reading and writing.
    fn open_device_file(&mut self) -> io::Result<()> {
        let device_path = Path::new(&self.port);
        let file = OpenOptions::new().read(true).write(true).open(device_path)?;
        self.device_file = Some(file);
        self.log("Device file opened");
        Ok(())
    }

    /// Configures the baud rate for the UART device.
    fn configure_baud_rate(&self) -> io::Result<()> {
        // Configuration is typically handled by the OS, but a placeholder is provided here.
        self.log(&format!("Baud rate configured to {}", self.baud_rate));
        Ok(())
    }

    /// Sends data to the UART device.
    pub fn send(&mut self, data: &[u8]) -> io::Result<()> {
        if let Some(ref mut device_file) = self.device_file {
            device_file.write_all(data)?;
            self.log(&format!("Data sent: {:?}", data));
        }
        Ok(())
    }

    /// Receives data from the UART device into the provided buffer.
    pub fn receive(&mut self, buffer: &mut [u8]) -> io::Result<()> {
        if let Some(ref mut device_file) = self.device_file {
            device_file.read_exact(buffer)?;
            self.log(&format!("Data received: {:?}", buffer));
        }
        Ok(())
    }

    /// Enables or disables logging for UART operations.
    pub fn enable_logging(&mut self, enable: bool) {
        self.log_enabled = enable;
        self.log(&format!("Logging {}", if enable { "enabled" } else { "disabled" }));
    }

    /// Logs a message if logging is enabled.
    fn log(&self, message: &str) {
        if self.log_enabled {
            println!("UART {}: {}", self.port, message);
        }
    }

    /// Sends data with a delay between each byte.
    pub fn send_with_delay(&mut self, data: &[u8], delay: Duration) -> io::Result<()> {
        for &byte in data.iter() {
            self.send(&[byte])?;
            thread::sleep(delay);
        }
        self.log(&format!("Data sent with delay: {:?}", data));
        Ok(())
    }

    /// Configures the UART parity (e.g., even, odd, none).
    pub fn set_parity(&self, parity: &str) -> io::Result<()> {
        // Example implementation (pseudo-code).
        self.log(&format!("UART parity set to {}", parity));
        Ok(())
    }

    /// Configures the UART stop bits (e.g., 1, 2).
    pub fn set_stop_bits(&self, stop_bits: u8) -> io::Result<()> {
        if stop_bits != 1 && stop_bits != 2 {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "Invalid stop bits configuration"));
        }
        // Example implementation (pseudo-code).
        self.log(&format!("UART stop bits set to {}", stop_bits));
        Ok(())
    }
}

fn main() {
    // Example usage of the UART library
    let mut uart = UART::new("/dev/ttyS0", 9600).expect("Failed to initialize UART");
    uart.enable_logging(true);

    let data_to_send = b"Hello, UART!";
    uart.send(data_to_send).expect("Failed to send data");

    let mut buffer = [0u8; 12];
    uart.receive(&mut buffer).expect("Failed to receive data");
    println!("Received data: {:?}", buffer);

    uart.set_parity("none").expect("Failed to set parity");
    uart.set_stop_bits(1).expect("Failed to set stop bits");

    let delayed_data = b"Delayed UART!";
    uart.send_with_delay(delayed_data, Duration::from_millis(100)).expect("Failed to send with delay");
}


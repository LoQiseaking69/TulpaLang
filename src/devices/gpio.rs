use std::fs::{File, OpenOptions};
use std::io::{self, Write, Read};
use std::path::Path;
use std::thread;
use std::time::Duration;

/// Represents the direction of a GPIO pin.
#[derive(Debug)]
pub enum Direction {
    In,
    Out,
}

/// A structure representing a GPIO pin, including its configuration and state.
pub struct GPIO {
    pin: u8,
    direction: Direction,
    value_file: Option<File>,
    log_enabled: bool,
}

impl GPIO {
    /// Creates a new GPIO instance with the specified pin number and direction.
    pub fn new(pin: u8, direction: Direction) -> io::Result<Self> {
        let mut gpio = GPIO {
            pin,
            direction,
            value_file: None,
            log_enabled: false,
        };
        gpio.export()?;
        gpio.set_direction()?;
        gpio.open_value_file()?;
        gpio.log("GPIO initialized");
        Ok(gpio)
    }

    /// Exports the GPIO pin to make it available for use in the system.
    fn export(&self) -> io::Result<()> {
        let export_path = Path::new("/sys/class/gpio/export");
        let mut export_file = OpenOptions::new().write(true).open(export_path)?;
        writeln!(export_file, "{}", self.pin)?;
        self.log("Pin exported");
        Ok(())
    }

    /// Unexports the GPIO pin to free it for other uses.
    fn unexport(&self) -> io::Result<()> {
        let unexport_path = Path::new("/sys/class/gpio/unexport");
        let mut unexport_file = OpenOptions::new().write(true).open(unexport_path)?;
        writeln!(unexport_file, "{}", self.pin)?;
        self.log("Pin unexported");
        Ok(())
    }

    /// Sets the direction of the GPIO pin (in or out).
    fn set_direction(&self) -> io::Result<()> {
        let direction_path = Path::new(&format!("/sys/class/gpio/gpio{}/direction", self.pin));
        let mut direction_file = OpenOptions::new().write(true).open(direction_path)?;
        match self.direction {
            Direction::In => writeln!(direction_file, "in")?,
            Direction::Out => writeln!(direction_file, "out")?,
        }
        self.log("Direction set");
        Ok(())
    }

    /// Opens the value file associated with the GPIO pin for reading or writing.
    fn open_value_file(&mut self) -> io::Result<()> {
        let value_path = Path::new(&format!("/sys/class/gpio/gpio{}/value", self.pin));
        let file = OpenOptions::new().read(true).write(true).open(value_path)?;
        self.value_file = Some(file);
        self.log("Value file opened");
        Ok(())
    }

    /// Sets the GPIO pin to a high value (1).
    pub fn set_high(&mut self) -> io::Result<()> {
        if let Some(ref mut value_file) = self.value_file {
            value_file.write_all(b"1")?;
            self.log("Pin set high");
        }
        Ok(())
    }

    /// Sets the GPIO pin to a low value (0).
    pub fn set_low(&mut self) -> io::Result<()> {
        if let Some(ref mut value_file) = self.value_file {
            value_file.write_all(b"0")?;
            self.log("Pin set low");
        }
        Ok(())
    }

    /// Reads the current value of the GPIO pin (true for high, false for low).
    pub fn read(&mut self) -> io::Result<bool> {
        let mut buffer = [0; 1];
        if let Some(ref mut value_file) = self.value_file {
            value_file.read_exact(&mut buffer)?;
        }
        let value = buffer[0] == b'1';
        self.log(&format!("Pin read: {}", value));
        Ok(value)
    }

    /// Enables or disables logging for GPIO operations.
    pub fn enable_logging(&mut self, enable: bool) {
        self.log_enabled = enable;
        self.log(&format!("Logging {}", if enable { "enabled" } else { "disabled" }));
    }

    /// Logs a message if logging is enabled.
    fn log(&self, message: &str) {
        if self.log_enabled {
            println!("GPIO {}: {}", self.pin, message);
        }
    }

    /// Toggles the GPIO pin between high and low states with a delay between each toggle.
    pub fn toggle(&mut self, delay: Duration) -> io::Result<()> {
        self.set_high()?;
        thread::sleep(delay);
        self.set_low()?;
        thread::sleep(delay);
        self.log("Pin toggled");
        Ok(())
    }

    /// Blinks the GPIO pin a specified number of times with a given delay between blinks.
    pub fn blink(&mut self, times: u8, delay: Duration) -> io::Result<()> {
        for _ in 0..times {
            self.toggle(delay)?;
        }
        self.log(&format!("Pin blinked {} times", times));
        Ok(())
    }
}

impl Drop for GPIO {
    /// Ensures the GPIO pin is unexported when the GPIO instance is dropped.
    fn drop(&mut self) {
        if let Err(e) = self.unexport() {
            eprintln!("Failed to unexport GPIO {}: {}", self.pin, e);
        }
    }
}

fn main() {
    // Example usage of the GPIO library
    let mut gpio = GPIO::new(17, Direction::Out).expect("Failed to initialize GPIO");
    gpio.enable_logging(true);

    gpio.set_high().expect("Failed to set GPIO high");
    thread::sleep(Duration::from_secs(1));

    gpio.set_low().expect("Failed to set GPIO low");
    thread::sleep(Duration::from_secs(1));

    gpio.toggle(Duration::from_millis(500)).expect("Failed to toggle GPIO");
    gpio.blink(5, Duration::from_millis(200)).expect("Failed to blink GPIO");

    let value = gpio.read().expect("Failed to read GPIO");
    println!("Final GPIO value: {}", value);
}


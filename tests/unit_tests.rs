#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    // Unit test for the GPIO module
    #[test]
    fn test_gpio_set_high() {
        let mut gpio = GPIO::new(17, Direction::Out).expect("Failed to initialize GPIO");
        gpio.set_high().expect("Failed to set GPIO high");
        let value = gpio.read().expect("Failed to read GPIO");
        assert!(value, "GPIO pin should be high");
    }

    #[test]
    fn test_gpio_set_low() {
        let mut gpio = GPIO::new(17, Direction::Out).expect("Failed to initialize GPIO");
        gpio.set_low().expect("Failed to set GPIO low");
        let value = gpio.read().expect("Failed to read GPIO");
        assert!(!value, "GPIO pin should be low");
    }

    #[test]
    fn test_gpio_toggle() {
        let mut gpio = GPIO::new(17, Direction::Out).expect("Failed to initialize GPIO");
        gpio.toggle(Duration::from_millis(100)).expect("Failed to toggle GPIO");
        let value = gpio.read().expect("Failed to read GPIO");
        assert!(value, "GPIO pin should be high after toggle");
    }

    // Unit test for the I2C module
    #[test]
    fn test_i2c_write() {
        let mut i2c = I2C::new(1, 0x68).expect("Failed to initialize I2C");
        let data = [0x00, 0x01, 0x02];
        assert!(i2c.write(&data).is_ok(), "Failed to write to I2C");
    }

    #[test]
    fn test_i2c_read() {
        let mut i2c = I2C::new(1, 0x68).expect("Failed to initialize I2C");
        let mut buffer = [0u8; 3];
        assert!(i2c.read(&mut buffer).is_ok(), "Failed to read from I2C");
        assert_eq!(buffer.len(), 3, "Read buffer should have length 3");
    }

    // Unit test for the SPI module
    #[test]
    fn test_spi_transfer() {
        let mut spi = SPI::new(0, 0).expect("Failed to initialize SPI");
        let data = [0xAA, 0xBB, 0xCC];
        let response = spi.transfer(&data).expect("Failed to transfer data over SPI");
        assert_eq!(response.len(), 3, "SPI response should have length 3");
    }

    #[test]
    fn test_spi_transfer_with_delay() {
        let mut spi = SPI::new(0, 0).expect("Failed to initialize SPI");
        let data = [0x10, 0x20, 0x30];
        let response = spi.transfer_with_delay(&data, Duration::from_millis(100)).expect("Failed to transfer with delay over SPI");
        assert_eq!(response.len(), 3, "SPI response should have length 3");
    }

    // Unit test for the UART module
    #[test]
    fn test_uart_send() {
        let mut uart = UART::new("/dev/ttyS0", 9600).expect("Failed to initialize UART");
        let data = b"Hello, UART!";
        assert!(uart.send(data).is_ok(), "Failed to send data over UART");
    }

    #[test]
    fn test_uart_receive() {
        let mut uart = UART::new("/dev/ttyS0", 9600).expect("Failed to initialize UART");
        let mut buffer = [0u8; 12];
        assert!(uart.receive(&mut buffer).is_ok(), "Failed to receive data over UART");
        assert_eq!(buffer.len(), 12, "UART buffer should have length 12");
    }
}


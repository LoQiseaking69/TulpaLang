#[cfg(test)]
mod integration_tests {
    use super::*;
    use std::time::Duration;

    // Integration test for GPIO and I2C modules
    #[test]
    fn integration_test_gpio_i2c() {
        let mut gpio = GPIO::new(22, Direction::Out).expect("Failed to initialize GPIO");
        gpio.set_high().expect("Failed to set GPIO high");

        let mut i2c = I2C::new(1, 0x68).expect("Failed to initialize I2C");
        let write_data = [0x01];
        let mut read_buffer = [0u8; 1];
        i2c.write_then_read(&write_data, &mut read_buffer).expect("Failed to write-then-read on I2C");

        assert!(read_buffer[0] != 0, "Expected non-zero read from I2C");
        gpio.set_low().expect("Failed to set GPIO low");
    }

    // Integration test for SPI and UART modules
    #[test]
    fn integration_test_spi_uart() {
        let mut spi = SPI::new(0, 0).expect("Failed to initialize SPI");
        let data_to_transfer = [0x55, 0xAA];
        let response = spi.transfer(&data_to_transfer).expect("Failed to transfer data over SPI");

        let mut uart = UART::new("/dev/ttyS0", 9600).expect("Failed to initialize UART");
        uart.send(&response).expect("Failed to send SPI response over UART");

        let mut buffer = [0u8; 2];
        uart.receive(&mut buffer).expect("Failed to receive data over UART");

        assert_eq!(buffer, response, "UART received data does not match SPI response");
    }
}


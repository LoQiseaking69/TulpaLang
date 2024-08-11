#[cfg(test)]
mod hardware_tests {
    use super::*;
    use std::time::Duration;

    // Hardware test for GPIO module
    #[test]
    fn hardware_test_gpio_blink() {
        let mut gpio = GPIO::new(18, Direction::Out).expect("Failed to initialize GPIO");
        assert!(gpio.blink(10, Duration::from_millis(100)).is_ok(), "Failed to blink GPIO");
    }

    // Hardware test for I2C module
    #[test]
    fn hardware_test_i2c_write_then_read() {
        let mut i2c = I2C::new(1, 0x68).expect("Failed to initialize I2C");
        let write_data = [0x00, 0x01, 0x02];
        let mut read_buffer = [0u8; 3];
        assert!(i2c.write_then_read(&write_data, &mut read_buffer).is_ok(), "Failed to perform write-then-read on I2C");
    }

    // Hardware test for SPI module
    #[test]
    fn hardware_test_spi_transfer() {
        let mut spi = SPI::new(0, 1).expect("Failed to initialize SPI");
        let data = [0x01, 0x02, 0x03];
        let response = spi.transfer(&data).expect("Failed to transfer data over SPI");
        assert_eq!(response.len(), 3, "SPI response should have length 3");
    }

    // Hardware test for UART module
    #[test]
    fn hardware_test_uart_send_receive() {
        let mut uart = UART::new("/dev/ttyS1", 115200).expect("Failed to initialize UART");
        let data = b"Test UART Communication";
        uart.send(data).expect("Failed to send data over UART");

        let mut buffer = [0u8; 24];
        uart.receive(&mut buffer).expect("Failed to receive data over UART");
        assert_eq!(&buffer[..], data, "UART received data does not match sent data");
    }
}


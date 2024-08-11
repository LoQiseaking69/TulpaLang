# Device Integration with Tulpa

Tulpa allows for seamless integration with hardware devices using modules such as GPIO, I2C, SPI, and UART. This guide covers how to use these modules.

## GPIO Module

The GPIO module allows you to control GPIO pins on your device. Here's how to set a pin high:

```rust
let gpio = GPIO::new(17);
gpio.set_high();
```

You can also read the state of a pin:

```rust
let state = gpio.read();
println!("Pin state: {}", state);
```

## I2C Module

The I2C module enables communication with I2C devices. Here's how to write to an I2C device:

```rust
let i2c = I2C::new(0x40);
i2c.write(&[0x01, 0x02, 0x03]);
```

To read data from an I2C device:

```rust
let mut buffer = [0; 3];
i2c.read(&mut buffer);
println!("Read data: {:?}", buffer);
```

## SPI Module

The SPI module is used for high-speed communication with SPI devices. Here's an example:

```rust
let spi = SPI::new(0, 10);
let response = spi.transfer(&[0xAA, 0xBB, 0xCC]);
println!("SPI response: {:?}", response);
```

## UART Module

The UART module facilitates serial communication. You can send data over UART like this:

```rust
let uart = UART::new("COM1".to_string(), 9600);
uart.send(b"Hello, UART!");
```

To receive data over UART:

```rust
let mut buffer = [0; 10];
uart.receive(&mut buffer);
println!("Received data: {:?}", buffer);
```

## Putting It All Together

You can combine these modules to create powerful device integration applications in Tulpa. Refer to the [examples](../examples) for more details.

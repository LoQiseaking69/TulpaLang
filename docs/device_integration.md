# Device Integration with Tulpa

Every device is a built-in object. The same script runs against one of two backends:

| Backend | Select with | What it talks to |
|---------|-------------|------------------|
| **sim** (default) | `--sim` or nothing | an in-memory bus: deterministic, scriptable, logs every transaction |
| **linux** | `--hw` or `TULPA_BACKEND=linux` | sysfs GPIO, `/dev/i2c-N`, `/dev/spidevB.C`, serial TTYs |

`sim::backend()` returns `"sim"` or `"linux"`, so a script can preload fake sensor data only when
it's simulated. Add `--trace` to print every device call and its result to stderr.

## GPIO

```tulpa
let led = GPIO::new(17)            # output by default
let button = GPIO::new(4, "in")
led.set_high(); led.set_low(); led.write(1); led.toggle()
led.blink(3, 200)                  # 3 times, 200 ms on/off
if button.read() == HIGH { print("pressed") }
button.is_high(); led.set_direction("in"); led.pin(); led.direction()
```

Writing to a pin that's configured as an input is an error, not a silent no-op.

**Hardware notes:**
- Uses `/sys/class/gpio`, so you need to be root or in the `gpio` group.
- On kernels where the sysfs numbers are offset, set `TULPA_GPIO_BASE` to the chip's base and keep using BCM numbers in your scripts. Raspberry Pi 5 and kernels 6.6+ are like this; there, `cat /sys/class/gpio/gpiochip*/base` shows e.g. `571`.

## I2C

```tulpa
let imu = I2C::new(1, 0x68)        # bus 1, 7-bit address (I2C::new(0x68) defaults to bus 1)
imu.write_reg(0x6B, 0x00)          # wake MPU-6050
let ax = imu.read_i16(0x3B)        # signed big-endian 16-bit register pair
let raw = imu.read_regs(0x3B, 6)   # burst read
imu.update_reg(0x1C, 0x18, 0x08)   # read-modify-write under a mask
imu.write([0x10, 0xAA]); imu.read(2); imu.write_read([0x0F], 1)
```

All register reads (`read_reg`, `read_regs`, `read_u16`, `read_i16`, `write_read`) use a single
`I2C_RDWR` transaction with a **repeated start**, which is what most sensors expect.
`read_u16(reg, "little")` / `read_i16(reg, "little")` handle little-endian devices.

## SPI

```tulpa
let adc = SPI::new(0, 0)           # /dev/spidev0.0
adc.set_mode(0); adc.set_speed(1_000_000)
let r = adc.transfer([0x01, 0x80, 0x00])          # full duplex, returns MISO bytes
let value = ((r[1] & 0x03) << 8) | r[2]           # MCP3008 channel 0
adc.write([0xFF]); adc.read(4)
```

## UART

```tulpa
let gps = UART::new("/dev/serial0", 9600)
gps.send("raw bytes or a string")
gps.send_line("AT")               # appends \r\n
let line = gps.read_line(1000)     # str, or nil on timeout (ms)
let bytes = gps.receive(16, 100)   # up to 16 bytes, 100 ms timeout
gps.read_text(); gps.available()
```

Supported baud rates: 1200 through 1,000,000, in the standard steps. The port is put into raw 8N1 mode.

## PID controller

```tulpa
let pid = PID::new(kp, ki, kd)
pid.set_limits(-255, 255)          # clamp output; integral stops accumulating while saturated
let out = pid.update(setpoint, measured, dt_seconds)
pid.reset(); pid.set_gains(kp, ki, kd); pid.integral()
```

## PWM and servos

```tulpa
let pwm = PWM::new(0, 1)               # /sys/class/pwm/pwmchip0/pwm1
pwm.frequency(1[kHz])                  # or a plain number in Hz
pwm.duty(0.25)                         # fraction 0..1
pwm.pulse(1500[us])                    # or a plain number in µs
pwm.enable(); pwm.disable(); pwm.state()

let s = Servo::new(0, 0)               # chip, channel, min_us=500, max_us=2500, range=180
s.angle(45[deg])                       # or a plain number in degrees; out of range is an error
s.center(); s.detach(); s.angle(); s.pulse_us()
```

`sim::pwm(chip, channel)` returns `{period_ns, duty_ns, enabled}` in the simulator.

## Finding devices

`I2C::scan(bus = 1)` returns the addresses that respond.
- On hardware it probes 0x03 to 0x77 with a 1-byte read, as `i2cdetect -r` does.
- In the simulator it returns every address that has been opened or preloaded.

## Record and replay

```bash
tulpa --hw --record bench.tape robot.rav   # capture a real session
tulpa --replay bench.tape robot.rav        # rerun anywhere, no hardware needed
```

**Recording.** Every device operation is logged to a plain-text tape, along with what the device
returned: GPIO direction, writes and reads; I2C writes, reads and register reads; SPI transfers;
UART traffic; PWM settings. Each line holds the device, the operation, the input and the output, in hex.

**Replaying.** No real or simulated device is opened. Each operation must match the next one on the
tape, and gets the recorded answer back. The run fails in either of two cases:
- At the first operation that differs, with a message like `replay diverged at event #2: tape has gpio17 write [01], script did gpio17 write [00]`.
- If the script ends before using the whole tape.

This turns a field session into a regression test for your control logic.

## The simulator

Each device type behaves the way the real thing usually does:

- **GPIO.** Outputs remember their level. Inputs read whatever `sim::gpio_input(pin, level)` drives.
- **I2C.** Every address has a 256-byte register file. The first byte you write sets the register pointer, the bytes after it are stored at successive registers, and reads auto-increment from the pointer.
- **SPI.** By default, MOSI is looped back to MISO. Use `sim::spi_respond(bus, cs, bytes)` to queue the next response.
- **UART.** By default, whatever you send comes back on receive. To play the remote device instead, turn loopback off with `sim::uart_loopback(port, false)` and inject replies with `sim::uart_feed`.

The `sim::` helpers:

| Function | Effect |
|----------|--------|
| `sim::gpio_input(pin, level)` / `sim::gpio_level(pin)` | drive / inspect a pin |
| `sim::i2c_set(addr, reg, bytes[, bus])` / `sim::i2c_get(addr, reg[, n, bus])` | preload / inspect registers |
| `sim::spi_respond(bus, cs, bytes)` | queue the next SPI response |
| `sim::uart_feed(port, data)` / `sim::uart_sent(port)` | inject RX / take everything sent |
| `sim::uart_loopback(port, on)` | toggle loopback |
| `sim::pwm(chip, ch)` | inspect a PWM channel |
| `sim::log()` / `sim::clear_log()` / `sim::reset()` | transaction log and reset |

This makes hardware logic unit-testable:

```tulpa
define_test!(overheat_turns_fan_on, {
    sim::i2c_set(0x48, 0x00, [0x50, 0x00])     # 80 °C
    control_step()
    assert_eq!(sim::gpio_level(18), 1)
})
```

Run it with `tulpa test file.rav`.

## Embedding from Rust

```rust
let (mut interp, out) = tulpa::Interpreter::captured();
interp.backend = tulpa::Backend::Sim;
let prog = tulpa::parse(r#"let s = SPI::new(); print(s.transfer([1, 2]))"#).unwrap();
interp.run(&prog).unwrap();
assert_eq!(out.borrow().as_str(), "[1, 2]\n");
```

The evaluator recurses on the native stack. Run deep scripts on a thread with a generous stack;
`tulpa::run_captured` does that for you.

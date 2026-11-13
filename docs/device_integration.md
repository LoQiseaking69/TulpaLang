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
- **Character device (default).** When `/dev/gpiochip0` exists, lines are requested through the
  kernel's GPIO v2 uAPI. This is the modern interface: no exporting, and no number offsets.
  - Pin numbers are line offsets on the chip. On a Raspberry Pi those are the BCM numbers.
  - Pick another chip with `TULPA_GPIO_CHIP=gpiochip4`.
  - You need read/write access to the chip device, usually through the `gpio` group.
- **sysfs fallback.** Used when no chip device exists, or forced with `TULPA_GPIO=sysfs`.
  It goes through `/sys/class/gpio`. On kernels where sysfs numbers are offset, set
  `TULPA_GPIO_BASE` to the chip's base; `cat /sys/class/gpio/gpiochip*/base` shows it.

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

Tapes are plain text. Each line holds the device, the operation, the input and output in hex, and a timestamp in ms.

**Commands** must match in order: GPIO direction and writes, I2C writes, SPI transfers, UART sends,
PWM settings, CAN sends and BLE requests. The first difference fails the run:
`replay diverged at command #1: tape has gpio17 write [01], script did gpio17 write [00]`.
The run also fails if the script ends with recorded commands unused.

**Inputs** replay as signals over time: GPIO reads, I2C reads and register reads, and UART `available`.
A read returns the value the device had at that moment of the recorded run. Rules can therefore
poll at any rate, and only changes are stored. UART bytes and CAN frames are delivered once they
have arrived on the recorded timeline.

**Timing.** A timed tape replays on the virtual clock. After each command, the clock advances to
that command's recorded time. `when … for` debounces and `after … within` deadlines are therefore
judged against the timing that actually happened on the hardware, and a 10-second field run replays
in milliseconds.

Tapes without timestamps, written by version 2.0, still replay in order.

## CAN bus

```tulpa
let bus = CAN::new("can0")                 # SocketCAN interface (vcan0 works for testing)
bus.send(0x201, [0xB0, 0x04])              # ids above 0x7FF are sent as extended frames
bus.send(0x18FF50E5, [1, 2, 3], true)      # force the extended (29-bit) format
let f = bus.receive(100)                   # {id, data, extended}, or nil after 100 ms
bus.receive_all()                          # drain everything queued
bus.set_filter(0x180, 0x780)               # kernel-side id/mask filter
```

Bring an interface up with `sudo ip link set can0 up type can bitrate 500000`, or create a virtual
one with `sudo ip link add vcan0 type vcan && sudo ip link set vcan0 up`.

In the simulator, sockets on the same interface see each other's frames, but not their own.
`sim::can_inject(iface, id, data)` plays another node and `sim::can_sent(iface)` lists what was sent.

## Bluetooth LE

```tulpa
let found = BLE::scan(3[s])                # [{address, name, rssi}], strongest first
let hrm = BLE::connect("AA:BB:CC:DD:EE:01")   # pass true as the second argument for random addresses
hrm.services(); hrm.characteristics()      # discovered on connect
hrm.read("battery_level")                  # names, 16-bit ("2a19") or 128-bit UUIDs
hrm.read_text("device_name")
hrm.write("nordic_uart_rx", "GO\n")         # with response; pass false for write-without-response
hrm.subscribe("heart_rate_measurement")
for n in hrm.notifications(500) { print(n.uuid, n.data) }   # {uuid, data}
hrm.disconnect()
```

Tulpa speaks ATT directly over an L2CAP LE socket on channel 4, so it doesn't need BlueZ's D-Bus API.
- Long values are read with Read Blob.
- Indications are confirmed automatically.
- Scanning uses a raw HCI socket and needs root, or `CAP_NET_RAW` and `CAP_NET_ADMIN`. If `bluetoothd` is scanning at the same time, stop it.

The simulator is a real GATT server: the client runs the same discovery, read, write and notification
protocol against it.
- `sim::ble_device(address, name, {service: {characteristic: value}}, rssi)` creates a device.
- `sim::ble_notify(address, uuid, data)` pushes a notification.
- `sim::ble_written(address, uuid)` returns the last value the script wrote.

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

# Driver SDK

Tulpa 3.3 lets you write hardware drivers entirely in Tulpa. You can use the same driver in
three places:

- the simulator, for tests and CI;
- a Linux board (`--hw`);
- native firmware for a microcontroller (`tulpa build --target rp2040`).

The SDK has three parts:

1. **Interfaces.** These are named sets of method signatures. A struct declares
   `impl Imu`, and Tulpa checks it when the struct is defined.
2. **`std/driver`.** This module has the standard driver interfaces, plus bit and byte helpers
   that compile natively.
3. **`std/drivers/*`.** These are reference drivers for the MPU-6050, PCA9685 and MCP3008.
   Copy one when you start a new driver.

## Interfaces

```tulpa
interface Rangefinder: Driver {          # extends Driver (probe, init)
    fn distance(self) -> [m]
}

struct HcSr04 impl Rangefinder {
    trig: GPIO
    echo: GPIO
    fn probe(self) -> bool { true }
    fn init(self) { self.trig.write(0) }
    fn distance(self) -> [m] { ... }
}
```

- An interface method has a signature but no body. Parameter and return types are optional.
  A unit return type such as `-> [m]` is part of the contract.
- `struct S impl A, B` is checked when `S` is defined. A missing method, a different number of
  parameters, or a return unit that doesn't match is reported. For example:
  `struct Bad implements Thermometer but has no method temperature(self)`.
- An interface name works as a parameter type. `fn log(s: Rangefinder)` accepts any struct that
  implements it, including through an interface that extends it.
- `implements(x, "Rangefinder")` tests conformance at run time. In native code it is folded at
  compile time.
- Interfaces are exported by modules like any other name:
  `from "std/driver" import Imu`.

## `std/driver`

| Interface | Methods |
|---|---|
| `Driver` | `probe(self) -> bool`, `init(self)` |
| `Imu: Driver` | `update`, `accel_x/y/z -> [m/s^2]`, `gyro_x/y/z -> [rad/s]` |
| `Thermometer: Driver` | `temperature -> [K]` |
| `Rangefinder: Driver` | `distance -> [m]` |
| `Adc: Driver` | `read_raw(channel) -> int`, `read_volts(channel) -> [V]` |
| `PwmController: Driver` | `set_frequency(f: [Hz])`, `set_pulse(channel, width: [s])` |

The helpers work on plain integers and have contracts, so they compile natively:

| Helper | Meaning |
|---|---|
| `bits(v, shift, width)` | extract a bit field |
| `with_bits(v, shift, width, x)` | replace a bit field |
| `bit(v, n)` | test one bit |
| `twos(v, nbits)` | sign-extend a two's-complement value |
| `u16_be/u16_le(a, b)`, `i16_be/i16_le(a, b)` | combine two bytes |
| `lo8(v)`, `hi8(v)` | split a 16-bit value |

`Regmap(bus)` wraps an I2C or SPI device as a register file, with `read`, `write`, `field`,
`set_field` and `update`. It runs on the interpreter only, because its `dev` field can hold
either bus type. Native drivers type their bus field as `bus: I2C` and call the bus directly.

## Writing a driver that compiles natively

The bundled drivers follow these rules, so they build for every target:

- **Type every field.** Write `bus: I2C` and `ax: [m/s^2] = 0[m/s^2]`, not `ax = 0`. Native
  structs have a fixed layout, and the compiler infers what it can, but explicit types also give
  better error messages.
- **Use device methods.** `read_reg`, `write_reg`, `read_regs`, `write`, `read`, `transfer`
  work the same on every backend.
- **Use fixed-size arrays.** Use literals like `[0, 0, 0]` and `[0] * 6`, and index into the
  result of `read_regs`. Dicts, closures and `try/catch` don't compile natively.
- **Put contracts on the public API.** `require angle >= 0[deg] and angle <= self.range`
  becomes a check on every target. On a microcontroller, a violation calls `tulpa_hal_panic`.

```tulpa
from "std/driver" import Imu, Thermometer, i16_be

struct Mpu6050 impl Imu, Thermometer {
    bus: I2C
    ax: [m/s^2] = 0[m/s^2]
    ...
    fn probe(self) -> bool { self.bus.read_reg(0x75) == 0x68 }
    fn update(self) {
        let r = self.bus.read_regs(0x3B, 14)          # one burst read
        self.ax = float(i16_be(r[0], r[1])) * self.accel_scale()
        ...
    }
    fn accel_x(self) -> [m/s^2] { self.ax }
}
```

## Testing a driver on the simulator

Load the device's registers with `sim::` hooks, run the driver, and check the result. Then
build the same file natively and compare the two outputs:

```tulpa
sim::i2c_set(0x68, 0x75, [0x68])                          # WHO_AM_I
sim::i2c_set(0x68, 0x3B, [0, 0, 0, 0, 0x40, 0, 0, 0, 0, 131, 0, 0, 0, 0])
let imu = Mpu6050(I2C::new(1, 0x68))
imu.init()
imu.update()
print(f"az {imu.accel_z():.4f}  gx {imu.gyro_x().to("deg/s"):.3f}")
```

```
tulpa --virtual-time main.rav > want.txt
tulpa build main.rav --target sim --exe -o drv && ./drv > got.txt
diff want.txt got.txt
```

`examples/drivers` does exactly this, and `cargo test` checks it on every build. The `sim::`
hooks only compile for `--target sim`, or with `--sim` for a firmware target that runs under
QEMU. On real hardware they are rejected.

## Reference drivers

| Module | Part | Implements |
|---|---|---|
| `std/drivers/mpu6050` | InvenSense MPU-6050/6500 IMU (I2C) | `Imu`, `Thermometer` |
| `std/drivers/pca9685` | NXP PCA9685 16-channel PWM/servo (I2C) | `PwmController` |
| `std/drivers/mcp3008` | Microchip MCP3008 8-channel 10-bit ADC (SPI) | `Adc` |

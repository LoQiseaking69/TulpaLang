//! Device semantics on the simulator backend (no hardware required).

use tulpa::run_captured;

fn run(src: &str) -> String {
    run_captured(src).unwrap_or_else(|e| panic!("program failed: {e}\n{src}"))
}

#[test]
fn gpio_output_and_input() {
    let src = r#"
let led = GPIO::new(17)
led.set_high()
let a = led.read()
led.toggle()
let button = GPIO::new(4, "in")
sim::gpio_input(4, 1)
print(a, led.read(), button.read(), button.is_high(), sim::gpio_level(17))
try { button.set_high() } catch e { print(e) }
"#;
    assert_eq!(
        run(src),
        "1 0 1 true 0\nGPIO 4 is an input; call set_direction(\"out\") first\n"
    );
}

#[test]
fn i2c_register_file() {
    let src = r#"
let dev = I2C::new(0x68)
dev.write_reg(0x6B, 0x00)
sim::i2c_set(0x68, 0x3B, [0xFF, 0x38])      # accel X = -200
print(dev.read_i16(0x3B), dev.read_u16(0x3B), dev.read_reg(0x6B), dev.read_regs(0x3B, 2))
dev.update_reg(0x6B, 0x0F, 0x05)
print(sim::i2c_get(0x68, 0x6B), dev.address())
"#;
    assert_eq!(run(src), "-200 65336 0 [255, 56]\n[5] 104\n");
}

#[test]
fn i2c_rejects_bad_addresses_and_bytes() {
    assert!(run_captured("I2C::new(0x80)")
        .unwrap_err()
        .contains("out of 7-bit range"));
    assert!(run_captured("I2C::new(0x40).write([300])")
        .unwrap_err()
        .contains("byte value 300 out of range"));
}

#[test]
fn spi_loopback_and_scripted_responses() {
    let src = r#"
let adc = SPI::new(0, 1)
print(adc.transfer([1, 2, 3]))
sim::spi_respond(0, 1, [0x00, 0x03, 0xFF])
let r = adc.transfer([0x01, 0x80, 0x00])
print(((r[1] & 0x03) << 8) | r[2])
"#;
    assert_eq!(run(src), "[1, 2, 3]\n1023\n");
}

#[test]
fn uart_loopback_and_remote_end() {
    let src = r#"
let u = UART::new("/dev/ttyAMA0", 115200)
u.send("ping\n")
print(u.available(), u.read_line())
sim::uart_loopback("/dev/ttyAMA0", false)
u.send_line("AT")
sim::uart_feed("/dev/ttyAMA0", "OK\r\n")
print(u.read_line(), decode(sim::uart_sent("/dev/ttyAMA0")).strip(), u.read_line(10))
"#;
    assert_eq!(run(src), "5 ping\nOK ping\nAT nil\n");
}

#[test]
fn transaction_log() {
    let src = r#"
let p = GPIO::new(5)
p.set_high()
I2C::new(2, 0x10).write([1, 2])
for line in sim::log() { print(line) }
"#;
    assert_eq!(
        run(src),
        "gpio 5 direction out\ngpio 5 <- 1\ni2c 2:0x10 write [01 02]\n"
    );
}

#[test]
fn pid_converges_and_respects_limits() {
    let src = r#"
let pid = PID::new(1.5, 0.8, 0.0)
pid.set_limits(-10, 10)
let x = 0.0
for _ in 0..2000 { x += pid.update(5.0, x, 0.01) * 0.01 }
print(abs(x - 5.0) < 0.01, pid.update(1000, 0, 0.01))
"#;
    assert_eq!(run(src), "true 10.0\n");
}

#[test]
fn device_objects_print_nicely() {
    assert_eq!(
        run("print(GPIO::new(3), I2C::new(1, 0x40), SPI::new(), UART::new(\"COM1\"), PID::new(1, 0, 0))"),
        "<GPIO 3 out> <I2C bus 1 @ 0x40> <SPI 0.0> <UART COM1 @ 9600> <PID kp=1 ki=0 kd=0>\n"
    );
}

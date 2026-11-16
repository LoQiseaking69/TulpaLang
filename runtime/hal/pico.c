/* hal/pico.c: Tulpa native code on the Raspberry Pi Pico / RP2040 (Pico SDK 2.x).
 *
 * Mapping from Tulpa's device model to the RP2040:
 *
 *   GPIO::new(pin)          any GPIO 0..29
 *   I2C::new(bus, addr)     bus 0 = i2c0, bus 1 = i2c1, 400 kHz, internal pull-ups
 *   SPI::new(bus, cs)       bus 0 = spi0, bus 1 = spi1; `cs` is the chip-select GPIO
 *   UART::new("uart1", b)   "uart0" / "uart1" (or "0" / "1")
 *   PWM::new(chip, pin)     the second argument is the GPIO; `chip` is ignored
 *   Servo::new(chip, pin)   same, at 50 Hz
 *
 * Pins default to the Pico's usual ones and can be changed at build time, e.g.
 * -DTULPA_I2C1_SDA=2 -DTULPA_I2C1_SCL=3 (see the defines below).
 *
 * Part of Tulpa (Business Source License 1.1; see LICENSE). */
#include "tulpa_hal.h"

#include <stdio.h>
#include <string.h>

#include "hardware/clocks.h"
#include "hardware/gpio.h"
#include "hardware/i2c.h"
#include "hardware/pwm.h"
#include "hardware/spi.h"
#include "hardware/uart.h"
#include "pico/stdlib.h"

#ifndef TULPA_I2C0_SDA
#define TULPA_I2C0_SDA 4
#endif
#ifndef TULPA_I2C0_SCL
#define TULPA_I2C0_SCL 5
#endif
#ifndef TULPA_I2C1_SDA
#define TULPA_I2C1_SDA 6
#endif
#ifndef TULPA_I2C1_SCL
#define TULPA_I2C1_SCL 7
#endif
#ifndef TULPA_I2C_HZ
#define TULPA_I2C_HZ 400000
#endif
#ifndef TULPA_SPI0_SCK
#define TULPA_SPI0_SCK 18
#endif
#ifndef TULPA_SPI0_TX
#define TULPA_SPI0_TX 19
#endif
#ifndef TULPA_SPI0_RX
#define TULPA_SPI0_RX 16
#endif
#ifndef TULPA_SPI1_SCK
#define TULPA_SPI1_SCK 10
#endif
#ifndef TULPA_SPI1_TX
#define TULPA_SPI1_TX 11
#endif
#ifndef TULPA_SPI1_RX
#define TULPA_SPI1_RX 12
#endif
#ifndef TULPA_UART0_TX
#define TULPA_UART0_TX 0
#endif
#ifndef TULPA_UART0_RX
#define TULPA_UART0_RX 1
#endif
#ifndef TULPA_UART1_TX
#define TULPA_UART1_TX 4
#endif
#ifndef TULPA_UART1_RX
#define TULPA_UART1_RX 5
#endif
#ifndef TULPA_UART_TIMEOUT_US
#define TULPA_UART_TIMEOUT_US 1000000
#endif

/* ------------------------------------------------------------------ GPIO */

int tulpa_hal_gpio_init(tulpa_gpio_t *g) {
    if (g->pin >= (int)NUM_BANK0_GPIOS) return -1;
    gpio_init((uint)g->pin);
    gpio_set_dir((uint)g->pin, g->output ? GPIO_OUT : GPIO_IN);
    return 0;
}

int tulpa_hal_gpio_set_output(tulpa_gpio_t *g, int output) {
    gpio_set_dir((uint)g->pin, output ? GPIO_OUT : GPIO_IN);
    return 0;
}

int tulpa_hal_gpio_write(tulpa_gpio_t *g, int level) {
    gpio_put((uint)g->pin, level != 0);
    return 0;
}

int tulpa_hal_gpio_read(tulpa_gpio_t *g) {
    /* outputs read back the driven level, like the interpreter */
    return g->output ? (int)gpio_get_out_level((uint)g->pin) : (int)gpio_get((uint)g->pin);
}

/* ------------------------------------------------------------------ I2C */

static bool i2c_ready[2];

int tulpa_hal_i2c_init(tulpa_i2c_t *d) {
    if (d->bus < 0 || d->bus > 1) return -1;
    i2c_inst_t *inst = d->bus ? i2c1 : i2c0;
    if (!i2c_ready[d->bus]) {
        uint sda = d->bus ? TULPA_I2C1_SDA : TULPA_I2C0_SDA;
        uint scl = d->bus ? TULPA_I2C1_SCL : TULPA_I2C0_SCL;
        i2c_init(inst, TULPA_I2C_HZ);
        gpio_set_function(sda, GPIO_FUNC_I2C);
        gpio_set_function(scl, GPIO_FUNC_I2C);
        gpio_pull_up(sda);
        gpio_pull_up(scl);
        i2c_ready[d->bus] = true;
    }
    d->hw = inst;
    return 0;
}

int tulpa_hal_i2c_write(tulpa_i2c_t *d, const uint8_t *data, size_t n) {
    int r = i2c_write_blocking((i2c_inst_t *)d->hw, (uint8_t)d->addr, data, n, false);
    return r == (int)n ? 0 : -1;
}

int tulpa_hal_i2c_write_read(tulpa_i2c_t *d, const uint8_t *w, size_t wn, uint8_t *r, size_t rn) {
    i2c_inst_t *inst = (i2c_inst_t *)d->hw;
    if (wn && i2c_write_blocking(inst, (uint8_t)d->addr, w, wn, true) != (int)wn) return -1;
    return i2c_read_blocking(inst, (uint8_t)d->addr, r, rn, false) == (int)rn ? 0 : -1;
}

/* ------------------------------------------------------------------ SPI */

static bool spi_ready[2];

int tulpa_hal_spi_init(tulpa_spi_t *d) {
    if (d->bus < 0 || d->bus > 1 || d->cs < 0 || d->cs >= (int)NUM_BANK0_GPIOS) return -1;
    spi_inst_t *inst = d->bus ? spi1 : spi0;
    if (!spi_ready[d->bus]) {
        spi_init(inst, (uint)d->hz);
        gpio_set_function(d->bus ? TULPA_SPI1_SCK : TULPA_SPI0_SCK, GPIO_FUNC_SPI);
        gpio_set_function(d->bus ? TULPA_SPI1_TX : TULPA_SPI0_TX, GPIO_FUNC_SPI);
        gpio_set_function(d->bus ? TULPA_SPI1_RX : TULPA_SPI0_RX, GPIO_FUNC_SPI);
        spi_ready[d->bus] = true;
    }
    gpio_init((uint)d->cs);
    gpio_set_dir((uint)d->cs, GPIO_OUT);
    gpio_put((uint)d->cs, 1);
    d->hw = inst;
    return tulpa_hal_spi_configure(d);
}

int tulpa_hal_spi_configure(tulpa_spi_t *d) {
    spi_inst_t *inst = (spi_inst_t *)d->hw;
    spi_set_baudrate(inst, (uint)d->hz);
    spi_set_format(inst, 8, (d->mode & 2) ? SPI_CPOL_1 : SPI_CPOL_0, (d->mode & 1) ? SPI_CPHA_1 : SPI_CPHA_0,
                   SPI_MSB_FIRST);
    return 0;
}

int tulpa_hal_spi_transfer(tulpa_spi_t *d, const uint8_t *tx, uint8_t *rx, size_t n) {
    spi_inst_t *inst = (spi_inst_t *)d->hw;
    /* the bus is shared: apply this device's format before selecting it */
    tulpa_hal_spi_configure(d);
    gpio_put((uint)d->cs, 0);
    int r = spi_write_read_blocking(inst, tx, rx, n);
    gpio_put((uint)d->cs, 1);
    return r == (int)n ? 0 : -1;
}

/* ------------------------------------------------------------------ UART */

int tulpa_hal_uart_init(tulpa_uart_t *u) {
    int idx;
    if (!strcmp(u->port, "uart0") || !strcmp(u->port, "0"))
        idx = 0;
    else if (!strcmp(u->port, "uart1") || !strcmp(u->port, "1"))
        idx = 1;
    else
        return -1;
    uart_inst_t *inst = idx ? uart1 : uart0;
    uart_init(inst, (uint)u->baud);
    gpio_set_function(idx ? TULPA_UART1_TX : TULPA_UART0_TX, GPIO_FUNC_UART);
    gpio_set_function(idx ? TULPA_UART1_RX : TULPA_UART0_RX, GPIO_FUNC_UART);
    u->hw = inst;
    return 0;
}

int tulpa_hal_uart_write(tulpa_uart_t *u, const uint8_t *data, size_t n) {
    uart_write_blocking((uart_inst_t *)u->hw, data, n);
    return 0;
}

int tulpa_hal_uart_read(tulpa_uart_t *u, uint8_t *out, size_t n) {
    uart_inst_t *inst = (uart_inst_t *)u->hw;
    size_t got = 0;
    while (got < n && uart_is_readable_within_us(inst, TULPA_UART_TIMEOUT_US)) out[got++] = (uint8_t)uart_getc(inst);
    return (int)got;
}

int tulpa_hal_uart_available(tulpa_uart_t *u) { return uart_is_readable((uart_inst_t *)u->hw) ? 1 : 0; }

/* ------------------------------------------------------------------ PWM */

int tulpa_hal_pwm_init(tulpa_pwm_t *p) {
    if (p->channel < 0 || p->channel >= (int)NUM_BANK0_GPIOS) return -1;
    gpio_set_function((uint)p->channel, GPIO_FUNC_PWM);
    return 0;
}

/* Picks the smallest clock divider that fits the period into the 16-bit counter. */
int tulpa_hal_pwm_apply(tulpa_pwm_t *p) {
    uint pin = (uint)p->channel;
    uint slice = pwm_gpio_to_slice_num(pin);
    double sys = (double)clock_get_hz(clk_sys);
    double cycles = (double)p->period_ns * sys / 1e9;
    double div = cycles / 65536.0;
    if (div < 1.0) div = 1.0;
    if (div > 255.9375) return -1; /* slower than ~7.5 Hz at 125 MHz */
    div = (double)((uint32_t)(div * 16.0 + 0.999)) / 16.0; /* 4 fractional bits, rounded up */
    uint32_t wrap = (uint32_t)(cycles / div + 0.5);
    if (wrap < 1) wrap = 1;
    uint32_t level = (uint32_t)((double)p->duty_ns * sys / 1e9 / div + 0.5);
    if (level > wrap) level = wrap;
    pwm_set_clkdiv(slice, (float)div);
    pwm_set_wrap(slice, (uint16_t)(wrap - 1));
    pwm_set_gpio_level(pin, (uint16_t)level);
    pwm_set_enabled(slice, p->enabled != 0);
    return 0;
}

/* ------------------------------------------------------------------ time and console */

uint64_t tulpa_hal_micros(void) { return time_us_64(); }

void tulpa_hal_delay_us(uint64_t us) { sleep_us(us); }

void tulpa_hal_console_write(const char *s, size_t n) {
    fwrite(s, 1, n, stdout);
    fflush(stdout);
}

void tulpa_hal_panic(const char *msg) { panic("%s", msg); }

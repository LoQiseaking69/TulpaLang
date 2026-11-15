/* hal/sim.c: a deterministic simulated board for Tulpa native code.
 *
 * Same behaviour as the interpreter's default `sim` backend: GPIO levels in memory, I2C
 * devices with register files (write sets the register pointer, reads auto-increment), SPI
 * that returns queued responses or loops back, PWM state, and a virtual clock: delays advance
 * time instantly, like `tulpa run --virtual-time`. Uses no OS services except the console, so
 * it also runs bare-metal (tests on QEMU build it with -DTULPA_SIM_NO_STDIO and provide
 * tulpa_hal_console_write / tulpa_hal_panic themselves).
 *
 * Part of Tulpa (BSD 3-Clause). */
#include "tulpa_hal.h"

#include <string.h>

#ifndef SIM_PINS
#define SIM_PINS 256
#endif
#ifndef SIM_I2C
#define SIM_I2C 16
#endif
#ifndef SIM_SPI_Q
#define SIM_SPI_Q 8
#endif
#ifndef SIM_SPI_LEN
#define SIM_SPI_LEN 64
#endif

static uint8_t gpio_level[SIM_PINS];
static uint64_t now_us;

struct sim_i2c {
    int used, bus, addr;
    uint8_t regs[256];
    uint8_t ptr;
};
static struct sim_i2c i2c[SIM_I2C];

struct sim_spi_resp {
    int used, bus, cs, len;
    uint8_t data[SIM_SPI_LEN];
};
static struct sim_spi_resp spi_q[SIM_SPI_Q];

static struct sim_i2c *i2c_dev(int bus, int addr) {
    for (int i = 0; i < SIM_I2C; i++)
        if (i2c[i].used && i2c[i].bus == bus && i2c[i].addr == addr) return &i2c[i];
    for (int i = 0; i < SIM_I2C; i++)
        if (!i2c[i].used) {
            memset(&i2c[i], 0, sizeof i2c[i]);
            i2c[i].used = 1;
            i2c[i].bus = bus;
            i2c[i].addr = addr;
            return &i2c[i];
        }
    return NULL;
}

/* ------------------------------------------------------------------ GPIO */

int tulpa_hal_gpio_init(tulpa_gpio_t *g) { return g->pin < SIM_PINS ? 0 : -1; }
int tulpa_hal_gpio_set_output(tulpa_gpio_t *g, int output) {
    (void)g;
    (void)output;
    return 0;
}
int tulpa_hal_gpio_write(tulpa_gpio_t *g, int level) {
    gpio_level[g->pin] = (uint8_t)level;
    return 0;
}
int tulpa_hal_gpio_read(tulpa_gpio_t *g) { return gpio_level[g->pin]; }

void tulpa_sim_gpio_input(int64_t pin, int64_t level) {
    if (pin >= 0 && pin < SIM_PINS) gpio_level[pin] = level ? 1 : 0;
}
int64_t tulpa_sim_gpio_level(int64_t pin) { return (pin >= 0 && pin < SIM_PINS) ? gpio_level[pin] : 0; }

/* ------------------------------------------------------------------ I2C */

int tulpa_hal_i2c_init(tulpa_i2c_t *d) { return i2c_dev(d->bus, d->addr) ? 0 : -1; }

int tulpa_hal_i2c_write(tulpa_i2c_t *d, const uint8_t *data, size_t n) {
    struct sim_i2c *s = i2c_dev(d->bus, d->addr);
    if (!s) return -1;
    if (n == 0) return 0;
    s->ptr = data[0];
    for (size_t i = 1; i < n; i++) s->regs[s->ptr++] = data[i];
    return 0;
}

int tulpa_hal_i2c_write_read(tulpa_i2c_t *d, const uint8_t *w, size_t wn, uint8_t *r, size_t rn) {
    struct sim_i2c *s = i2c_dev(d->bus, d->addr);
    if (!s) return -1;
    if (wn && tulpa_hal_i2c_write(d, w, wn) < 0) return -1;
    for (size_t i = 0; i < rn; i++) r[i] = s->regs[s->ptr++];
    return 0;
}

void tulpa_sim_i2c_set(int64_t bus, int64_t addr, int64_t reg, const int64_t *data, int64_t n) {
    struct sim_i2c *s = i2c_dev((int)bus, (int)addr);
    if (!s) return;
    for (int64_t i = 0; i < n; i++) s->regs[(uint8_t)(reg + i)] = (uint8_t)data[i];
}

void tulpa_sim_i2c_get(int64_t bus, int64_t addr, int64_t reg, int64_t *out, int64_t n) {
    struct sim_i2c *s = i2c_dev((int)bus, (int)addr);
    for (int64_t i = 0; i < n; i++) out[i] = s ? s->regs[(uint8_t)(reg + i)] : 0;
}

/* ------------------------------------------------------------------ SPI */

int tulpa_hal_spi_init(tulpa_spi_t *d) {
    (void)d;
    return 0;
}
int tulpa_hal_spi_configure(tulpa_spi_t *d) {
    (void)d;
    return 0;
}

int tulpa_hal_spi_transfer(tulpa_spi_t *d, const uint8_t *tx, uint8_t *rx, size_t n) {
    for (int i = 0; i < SIM_SPI_Q; i++) {
        struct sim_spi_resp *q = &spi_q[i];
        if (q->used && q->bus == d->bus && q->cs == d->cs) {
            for (size_t k = 0; k < n; k++) rx[k] = (int)k < q->len ? q->data[k] : 0;
            q->used = 0;
            /* keep FIFO order: shift the rest down */
            for (int j = i; j + 1 < SIM_SPI_Q; j++) spi_q[j] = spi_q[j + 1];
            spi_q[SIM_SPI_Q - 1].used = 0;
            return 0;
        }
    }
    memcpy(rx, tx, n); /* loopback */
    return 0;
}

void tulpa_sim_spi_respond(int64_t bus, int64_t cs, const int64_t *data, int64_t n) {
    for (int i = 0; i < SIM_SPI_Q; i++)
        if (!spi_q[i].used) {
            spi_q[i].used = 1;
            spi_q[i].bus = (int)bus;
            spi_q[i].cs = (int)cs;
            spi_q[i].len = n > SIM_SPI_LEN ? SIM_SPI_LEN : (int)n;
            for (int k = 0; k < spi_q[i].len; k++) spi_q[i].data[k] = (uint8_t)data[k];
            return;
        }
}

/* ------------------------------------------------------------------ UART (loopback) */

static uint8_t uart_buf[256];
static size_t uart_len;

int tulpa_hal_uart_init(tulpa_uart_t *u) {
    (void)u;
    return 0;
}
int tulpa_hal_uart_write(tulpa_uart_t *u, const uint8_t *data, size_t n) {
    (void)u;
    for (size_t i = 0; i < n && uart_len < sizeof uart_buf; i++) uart_buf[uart_len++] = data[i];
    return 0;
}
int tulpa_hal_uart_read(tulpa_uart_t *u, uint8_t *out, size_t n) {
    (void)u;
    size_t k = n < uart_len ? n : uart_len;
    memcpy(out, uart_buf, k);
    memmove(uart_buf, uart_buf + k, uart_len - k);
    uart_len -= k;
    return (int)k;
}
int tulpa_hal_uart_available(tulpa_uart_t *u) {
    (void)u;
    return (int)uart_len;
}

/* ------------------------------------------------------------------ PWM */

int tulpa_hal_pwm_init(tulpa_pwm_t *p) {
    (void)p;
    return 0;
}
int tulpa_hal_pwm_apply(tulpa_pwm_t *p) {
    (void)p;
    return 0;
}

/* ------------------------------------------------------------------ virtual clock */

uint64_t tulpa_hal_micros(void) { return now_us; }
void tulpa_hal_delay_us(uint64_t us) { now_us += us; }

/* ------------------------------------------------------------------ console */

#ifndef TULPA_SIM_NO_STDIO
#include <stdio.h>
#include <stdlib.h>

void tulpa_hal_console_write(const char *s, size_t n) { fwrite(s, 1, n, stdout); }

void tulpa_hal_panic(const char *msg) {
    fflush(stdout);
    fprintf(stderr, "%s\n", msg);
    exit(70);
}
#endif

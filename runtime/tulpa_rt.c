/* tulpa_rt.c: the portable runtime behind Tulpa native code.
 *
 * Implements the tulpa_* ABI that generated code calls (device methods, print, timing) on top
 * of the board HAL in tulpa_hal.h. Checks mirror the interpreter's, with the same messages, so
 * a program fails the same way on a microcontroller as it does under `tulpa run`.
 *
 * Needs a C99 libc with snprintf/strtod and libm (newlib, picolibc, glibc, musl all work).
 *
 * Part of Tulpa (Business Source License 1.1; see LICENSE). */
#include "tulpa_hal.h"

#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#ifndef TULPA_MAX_XFER
#define TULPA_MAX_XFER 256 /* longest bus transfer, in bytes */
#endif

static void fail(const char *fmt, ...) __attribute__((noreturn, format(printf, 1, 2)));

#include <stdarg.h>
static void fail(const char *fmt, ...) {
    char buf[200];
    int n = snprintf(buf, sizeof buf, "tulpa: ");
    va_list ap;
    va_start(ap, fmt);
    vsnprintf(buf + n, sizeof buf - (size_t)n, fmt, ap);
    va_end(ap);
    tulpa_violation(buf);
    tulpa_hal_panic(buf); /* in case tulpa_violation was overridden and returned */
}

static uint8_t to_byte(int64_t v) {
    if (v < 0 || v > 255) fail("byte value %lld out of range 0..255", (long long)v);
    return (uint8_t)v;
}

static size_t xfer_len(int64_t n) {
    if (n < 0 || n > TULPA_MAX_XFER) fail("transfer of %lld bytes exceeds %d", (long long)n, TULPA_MAX_XFER);
    return (size_t)n;
}

/* ------------------------------------------------------------------ output */

static tulpa_uart_t *sink;

static void out(const char *s, size_t n) {
    if (n == 0) return;
    if (sink) {
        if (tulpa_hal_uart_write(sink, (const uint8_t *)s, n) < 0) fail("UART %s write failed", sink->port);
    } else {
        tulpa_hal_console_write(s, n);
    }
}

void tulpa_out_uart(tulpa_uart_t *u) { sink = u; }
void tulpa_out_console(void) { sink = NULL; }

void tulpa_print_str(const char *s) { out(s, strlen(s)); }
void tulpa_print_nl(void) { out("\n", 1); }
void tulpa_print_bool(int32_t v) { tulpa_print_str(v ? "true" : "false"); }

void tulpa_print_i64(int64_t v) {
    char b[24];
    int n = snprintf(b, sizeof b, "%lld", (long long)v);
    out(b, (size_t)n);
}

/* Rust's `{}` for f64: the shortest decimal that round-trips, never in exponent notation. */
static int shortest(double v, char *buf, size_t cap) {
    if (isnan(v)) return snprintf(buf, cap, "NaN");
    if (isinf(v)) return snprintf(buf, cap, v < 0 ? "-inf" : "inf");
    char g[40];
    int p;
    for (p = 1; p <= 17; p++) {
        snprintf(g, sizeof g, "%.*e", p - 1, v);
        if (strtod(g, NULL) == v) break;
    }
    /* g = [-]d.ddde[+-]xx  ->  plain decimal */
    const char *s = g;
    int neg = 0;
    if (*s == '-') { neg = 1; s++; }
    char digits[24];
    int nd = 0;
    for (; *s && *s != 'e'; s++)
        if (*s >= '0' && *s <= '9') digits[nd++] = *s;
    int exp = (*s == 'e') ? atoi(s + 1) : 0;
    while (nd > 1 && digits[nd - 1] == '0') nd--;
    char *o = buf;
    char *end = buf + cap - 1;
    if (neg && o < end) *o++ = '-';
    int point = exp + 1; /* digits before the decimal point */
    if (point <= 0) {
        if (o < end) *o++ = '0';
        if (o < end) *o++ = '.';
        for (int i = 0; i < -point && o < end; i++) *o++ = '0';
        for (int i = 0; i < nd && o < end; i++) *o++ = digits[i];
    } else {
        for (int i = 0; i < point && o < end; i++) *o++ = i < nd ? digits[i] : '0';
        if (nd > point) {
            if (o < end) *o++ = '.';
            for (int i = point; i < nd && o < end; i++) *o++ = digits[i];
        }
    }
    *o = 0;
    return (int)(o - buf);
}

/* Matches fmt_float(): whole numbers print with one decimal ("2.0"). */
void tulpa_print_f64(double v) {
    char b[64];
    int n;
    if (isfinite(v) && v == floor(v) && fabs(v) < 1e15)
        n = snprintf(b, sizeof b, "%.1f", v);
    else
        n = shortest(v, b, sizeof b);
    out(b, (size_t)n);
}

/* Matches Quantity's Display: rounded to 12 significant digits, whole numbers without
 * decimals, then the unit. Native code holds SI values, so the unit is the SI one. */
void tulpa_print_q(double v, const char *unit) {
    double r = v;
    if (v != 0.0 && isfinite(v)) {
        int digits = 12 - (int)ceil(log10(fabs(v)));
        if (digits < 0) digits = 0;
        if (digits > 15) digits = 15;
        double p = pow(10.0, digits);
        r = round(v * p) / p;
    }
    char b[64];
    int n;
    if (isfinite(r) && r == floor(r) && fabs(r) < 1e15)
        n = snprintf(b, sizeof b, "%lld", (long long)r);
    else
        n = shortest(r, b, sizeof b);
    out(b, (size_t)n);
    out(" ", 1);
    tulpa_print_str(unit);
}

void tulpa_print_arr(const int64_t *a, int64_t n) {
    out("[", 1);
    for (int64_t i = 0; i < n; i++) {
        if (i) out(", ", 2);
        tulpa_print_i64(a[i]);
    }
    out("]", 1);
}

void tulpa_print_fmt_i64(int64_t v, const char *fmt) {
    char b[80];
    int n = snprintf(b, sizeof b, fmt, (long long)v);
    out(b, (size_t)n);
}

void tulpa_print_fmt_f64(double v, const char *fmt) {
    char b[96];
    int n = snprintf(b, sizeof b, fmt, v);
    out(b, (size_t)n);
}

void tulpa_print_bin(int64_t v, int32_t width, int32_t zero) {
    char b[70];
    int n = 0;
    uint64_t u = (uint64_t)v;
    char tmp[65];
    int k = 0;
    do {
        tmp[k++] = (char)('0' + (u & 1));
        u >>= 1;
    } while (u && k < 64);
    while (n + k < width && n < 64) b[n++] = zero ? '0' : ' ';
    while (k) b[n++] = tmp[--k];
    out(b, (size_t)n);
}

/* ------------------------------------------------------------------ time */

int64_t tulpa_micros(void) { return (int64_t)tulpa_hal_micros(); }
int64_t tulpa_millis(void) { return (int64_t)(tulpa_hal_micros() / 1000u); }

void tulpa_sleep(double seconds) {
    if (!(seconds > 0)) return;
    tulpa_hal_delay_us((uint64_t)llround(seconds * 1e6));
}

void tulpa_every_start(int64_t *next_us) { *next_us = (int64_t)tulpa_hal_micros(); }

/* every!: the next tick is one period after the previous one; an overrun restarts the
 * schedule from now instead of trying to catch up (as in the interpreter). */
void tulpa_every_wait(int64_t *next_us, int64_t period_us) {
    if (period_us < 0) period_us = 0;
    *next_us += period_us;
    int64_t now = (int64_t)tulpa_hal_micros();
    if (*next_us > now)
        tulpa_hal_delay_us((uint64_t)(*next_us - now));
    else
        *next_us = now;
}

/* ------------------------------------------------------------------ GPIO */

void tulpa_gpio_open(tulpa_gpio_t *g, int32_t pin, int32_t output) {
    if (pin < 0) fail("invalid GPIO pin %d", (int)pin);
    g->pin = pin;
    g->output = output ? 1 : 0;
    if (tulpa_hal_gpio_init(g) < 0) fail("cannot open GPIO %d", (int)pin);
}

static void need_out(tulpa_gpio_t *g) {
    if (!g->output) fail("GPIO %d is an input; call set_direction(\"out\") first", (int)g->pin);
}

void tulpa_gpio_write(tulpa_gpio_t *g, int32_t level) {
    need_out(g);
    if (tulpa_hal_gpio_write(g, level ? 1 : 0) < 0) fail("GPIO %d write failed", (int)g->pin);
}

int64_t tulpa_gpio_read(tulpa_gpio_t *g) {
    int v = tulpa_hal_gpio_read(g);
    if (v < 0) fail("GPIO %d read failed", (int)g->pin);
    return v ? 1 : 0;
}

void tulpa_gpio_toggle(tulpa_gpio_t *g) {
    need_out(g);
    tulpa_gpio_write(g, tulpa_gpio_read(g) ? 0 : 1);
}

void tulpa_gpio_set_direction(tulpa_gpio_t *g, int32_t output) {
    g->output = output ? 1 : 0;
    if (tulpa_hal_gpio_set_output(g, g->output) < 0) fail("GPIO %d: cannot change direction", (int)g->pin);
}

/* ------------------------------------------------------------------ I2C */

void tulpa_i2c_open(tulpa_i2c_t *d, int32_t bus, int32_t addr) {
    if (addr < 0 || addr > 0x7f) fail("I2C address 0x%x out of 7-bit range", (unsigned)addr);
    d->bus = bus;
    d->addr = addr;
    if (tulpa_hal_i2c_init(d) < 0) fail("cannot open I2C bus %d", (int)bus);
}

static void i2c_w(tulpa_i2c_t *d, const uint8_t *b, size_t n) {
    if (tulpa_hal_i2c_write(d, b, n) < 0) fail("I2C %d@0x%02x write failed (no ACK?)", (int)d->bus, (unsigned)d->addr);
}

static void i2c_wr(tulpa_i2c_t *d, const uint8_t *w, size_t wn, uint8_t *r, size_t rn) {
    if (tulpa_hal_i2c_write_read(d, w, wn, r, rn) < 0)
        fail("I2C %d@0x%02x read failed (no ACK?)", (int)d->bus, (unsigned)d->addr);
}

void tulpa_i2c_write_reg(tulpa_i2c_t *d, int64_t reg, int64_t value) {
    uint8_t b[2] = {to_byte(reg), to_byte(value)};
    i2c_w(d, b, 2);
}

void tulpa_i2c_write_reg_n(tulpa_i2c_t *d, int64_t reg, const int64_t *data, int64_t n) {
    uint8_t b[TULPA_MAX_XFER + 1];
    size_t k = xfer_len(n);
    b[0] = to_byte(reg);
    for (size_t i = 0; i < k; i++) b[i + 1] = to_byte(data[i]);
    i2c_w(d, b, k + 1);
}

int64_t tulpa_i2c_read_reg(tulpa_i2c_t *d, int64_t reg) {
    uint8_t r = to_byte(reg), v = 0;
    i2c_wr(d, &r, 1, &v, 1);
    return v;
}

void tulpa_i2c_read_regs(tulpa_i2c_t *d, int64_t reg, int64_t *out_, int64_t n) {
    uint8_t r = to_byte(reg), b[TULPA_MAX_XFER];
    size_t k = xfer_len(n);
    i2c_wr(d, &r, 1, b, k);
    for (size_t i = 0; i < k; i++) out_[i] = b[i];
}

int64_t tulpa_i2c_read_u16(tulpa_i2c_t *d, int64_t reg, int32_t little, int32_t is_signed) {
    uint8_t r = to_byte(reg), b[2];
    i2c_wr(d, &r, 1, b, 2);
    uint16_t raw = little ? (uint16_t)(b[0] | (b[1] << 8)) : (uint16_t)((b[0] << 8) | b[1]);
    return is_signed ? (int64_t)(int16_t)raw : (int64_t)raw;
}

void tulpa_i2c_update_reg(tulpa_i2c_t *d, int64_t reg, int64_t mask, int64_t value) {
    uint8_t r = to_byte(reg), cur = 0;
    i2c_wr(d, &r, 1, &cur, 1);
    uint8_t m = (uint8_t)mask, v = (uint8_t)value;
    uint8_t b[2] = {r, (uint8_t)((cur & ~m) | (v & m))};
    i2c_w(d, b, 2);
}

void tulpa_i2c_write(tulpa_i2c_t *d, const int64_t *data, int64_t n) {
    uint8_t b[TULPA_MAX_XFER];
    size_t k = xfer_len(n);
    for (size_t i = 0; i < k; i++) b[i] = to_byte(data[i]);
    i2c_w(d, b, k);
}

void tulpa_i2c_read(tulpa_i2c_t *d, int64_t *out_, int64_t n) {
    uint8_t b[TULPA_MAX_XFER];
    size_t k = xfer_len(n);
    i2c_wr(d, NULL, 0, b, k);
    for (size_t i = 0; i < k; i++) out_[i] = b[i];
}

void tulpa_i2c_write_read(tulpa_i2c_t *d, const int64_t *data, int64_t n, int64_t *out_, int64_t m) {
    uint8_t w[TULPA_MAX_XFER], r[TULPA_MAX_XFER];
    size_t wn = xfer_len(n), rn = xfer_len(m);
    for (size_t i = 0; i < wn; i++) w[i] = to_byte(data[i]);
    i2c_wr(d, w, wn, r, rn);
    for (size_t i = 0; i < rn; i++) out_[i] = r[i];
}

/* ------------------------------------------------------------------ SPI */

void tulpa_spi_open(tulpa_spi_t *d, int32_t bus, int32_t cs) {
    d->bus = bus;
    d->cs = cs;
    d->hz = 1000000;
    d->mode = 0;
    if (tulpa_hal_spi_init(d) < 0) fail("cannot open SPI %d.%d", (int)bus, (int)cs);
}

void tulpa_spi_transfer(tulpa_spi_t *d, const int64_t *tx, int64_t *rx, int64_t n) {
    uint8_t t[TULPA_MAX_XFER], r[TULPA_MAX_XFER];
    size_t k = xfer_len(n);
    for (size_t i = 0; i < k; i++) t[i] = to_byte(tx[i]);
    memset(r, 0, k);
    if (tulpa_hal_spi_transfer(d, t, r, k) < 0) fail("SPI %d.%d transfer failed", (int)d->bus, (int)d->cs);
    if (rx)
        for (size_t i = 0; i < k; i++) rx[i] = r[i];
}

void tulpa_spi_write(tulpa_spi_t *d, const int64_t *tx, int64_t n) { tulpa_spi_transfer(d, tx, NULL, n); }

void tulpa_spi_read(tulpa_spi_t *d, int64_t *rx, int64_t n) {
    int64_t zeros[TULPA_MAX_XFER] = {0};
    tulpa_spi_transfer(d, zeros, rx, n);
}

void tulpa_spi_set_speed(tulpa_spi_t *d, int64_t hz) {
    if (hz <= 0 || hz > 200000000) fail("SPI speed %lld Hz out of range", (long long)hz);
    d->hz = (int32_t)hz;
    if (tulpa_hal_spi_configure(d) < 0) fail("SPI %d.%d: cannot set speed", (int)d->bus, (int)d->cs);
}

void tulpa_spi_set_mode(tulpa_spi_t *d, int64_t mode) {
    if (mode < 0 || mode > 3) fail("SPI mode %lld is not 0..3", (long long)mode);
    d->mode = (int32_t)mode;
    if (tulpa_hal_spi_configure(d) < 0) fail("SPI %d.%d: cannot set mode", (int)d->bus, (int)d->cs);
}

/* ------------------------------------------------------------------ UART */

void tulpa_uart_open(tulpa_uart_t *u, const char *port, int32_t baud) {
    u->port = port;
    u->baud = baud;
    if (tulpa_hal_uart_init(u) < 0) fail("cannot open UART %s", port);
}

void tulpa_uart_write(tulpa_uart_t *u, const int64_t *data, int64_t n) {
    uint8_t b[TULPA_MAX_XFER];
    size_t k = xfer_len(n);
    for (size_t i = 0; i < k; i++) b[i] = to_byte(data[i]);
    if (tulpa_hal_uart_write(u, b, k) < 0) fail("UART %s write failed", u->port);
}

void tulpa_uart_read(tulpa_uart_t *u, int64_t *out_, int64_t n) {
    uint8_t b[TULPA_MAX_XFER];
    size_t k = xfer_len(n);
    int got = tulpa_hal_uart_read(u, b, k);
    if (got < 0) fail("UART %s read failed", u->port);
    for (size_t i = 0; i < k; i++) out_[i] = (int)i < got ? b[i] : 0;
}

int64_t tulpa_uart_available(tulpa_uart_t *u) {
    int n = tulpa_hal_uart_available(u);
    return n < 0 ? 0 : n;
}

/* ------------------------------------------------------------------ PWM and servos */

static void pwm_apply(tulpa_pwm_t *p) {
    if (tulpa_hal_pwm_apply(p) < 0) fail("PWM %d.%d update failed", (int)p->chip, (int)p->channel);
}

void tulpa_pwm_open(tulpa_pwm_t *p, int32_t chip, int32_t channel) {
    p->chip = chip;
    p->channel = channel;
    p->period_ns = 20000000; /* 50 Hz, as in the interpreter */
    p->duty_ns = 0;
    p->enabled = 0;
    if (tulpa_hal_pwm_init(p) < 0) fail("cannot open PWM %d.%d", (int)chip, (int)channel);
    pwm_apply(p);
}

static void set_period(tulpa_pwm_t *p, int64_t ns) {
    if (ns <= 0) fail("PWM period must be > 0");
    if (p->duty_ns > ns) p->duty_ns = ns;
    p->period_ns = ns;
}

static void set_duty(tulpa_pwm_t *p, int64_t ns) {
    if (ns > p->period_ns)
        fail("PWM pulse %lld ns exceeds period %lld ns", (long long)ns, (long long)p->period_ns);
    p->duty_ns = ns;
}

void tulpa_pwm_frequency(tulpa_pwm_t *p, double hz) {
    if (!(hz > 0)) fail("frequency must be > 0");
    double frac = (double)p->duty_ns / (double)p->period_ns;
    int64_t period = llround(1e9 / hz);
    set_period(p, period);
    set_duty(p, llround(frac * (double)period));
    pwm_apply(p);
}

void tulpa_pwm_period(tulpa_pwm_t *p, double seconds) {
    if (seconds < 0) fail("period must be non-negative");
    set_period(p, llround(seconds * 1e9));
    pwm_apply(p);
}

void tulpa_pwm_duty(tulpa_pwm_t *p, double f) {
    if (!(f >= 0.0 && f <= 1.0)) fail("duty fraction %g outside 0..1", f);
    set_duty(p, llround(f * (double)p->period_ns));
    pwm_apply(p);
}

void tulpa_pwm_pulse(tulpa_pwm_t *p, double seconds) {
    if (seconds < 0) fail("pulse must be non-negative");
    set_duty(p, llround(seconds * 1e9));
    pwm_apply(p);
}

void tulpa_pwm_enable(tulpa_pwm_t *p, int32_t on) {
    p->enabled = on ? 1 : 0;
    pwm_apply(p);
}

void tulpa_servo_open(tulpa_servo_t *s, int32_t chip, int32_t channel, double min_us, double max_us,
                      double range_deg) {
    if (!(min_us < max_us) || !(range_deg > 0)) fail("Servo::new: need min_us < max_us and range > 0");
    s->min_us = min_us;
    s->max_us = max_us;
    s->range_deg = range_deg;
    s->angle_deg = 0;
    s->attached = 0;
    tulpa_pwm_open(&s->pwm, chip, channel);
}

void tulpa_servo_angle(tulpa_servo_t *s, double radians) {
    double deg = radians * (180.0 / 3.14159265358979323846);
    if (deg < -1e-9 || deg > s->range_deg + 1e-9) fail("servo angle %.2f deg outside 0..%g deg", deg, s->range_deg);
    double us = s->min_us + (s->max_us - s->min_us) * deg / s->range_deg;
    set_duty(&s->pwm, llround(us * 1000.0));
    s->pwm.enabled = 1;
    pwm_apply(&s->pwm);
    s->angle_deg = deg;
    s->attached = 1;
}

double tulpa_servo_get_angle(tulpa_servo_t *s) { return s->angle_deg * (3.14159265358979323846 / 180.0); }

void tulpa_servo_center(tulpa_servo_t *s) {
    tulpa_servo_angle(s, s->range_deg / 2.0 * (3.14159265358979323846 / 180.0));
}

void tulpa_servo_detach(tulpa_servo_t *s) {
    s->pwm.enabled = 0;
    pwm_apply(&s->pwm);
    s->attached = 0;
}

double tulpa_servo_pulse_us(tulpa_servo_t *s) { return (double)s->pwm.duty_ns / 1000.0; }

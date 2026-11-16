/* tulpa_hal.h: the contract between Tulpa native code and a board.
 *
 * Generated code (`tulpa build --target ...`) calls the tulpa_* runtime ABI below, which is
 * implemented once, portably, in tulpa_rt.c. tulpa_rt.c in turn calls a small board HAL (the
 * tulpa_hal_* functions at the end of this file) that each port implements:
 *
 *   hal/sim.c        deterministic simulated bus and virtual clock (tests, CI, any host)
 *   hal/linux.c      Linux: gpiochip, i2c-dev, spidev, sysfs PWM, termios (Raspberry Pi, ...)
 *   hal/pico.c       Raspberry Pi Pico / RP2040 with the Pico SDK
 *   hal/esp_idf.c    ESP32, ESP32-S3, ESP32-C3, ESP32-C6 with ESP-IDF 5.x
 *
 * Porting to a new board means implementing the ~20 tulpa_hal_* functions. The device structs
 * are allocated by the generated code, so their layout is fixed; `hw` is free for the port.
 *
 * Part of Tulpa (Business Source License 1.1; see LICENSE). */
#ifndef TULPA_HAL_H
#define TULPA_HAL_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ------------------------------------------------------------------ device handles */

typedef struct { int32_t pin; int32_t output; void *hw; } tulpa_gpio_t;
typedef struct { int32_t bus; int32_t addr; void *hw; } tulpa_i2c_t;
typedef struct { int32_t bus; int32_t cs; int32_t hz; int32_t mode; void *hw; } tulpa_spi_t;
typedef struct { const char *port; int32_t baud; void *hw; } tulpa_uart_t;
typedef struct {
    int32_t chip;
    int32_t channel;
    int64_t period_ns;
    int64_t duty_ns;
    int32_t enabled;
    void *hw;
} tulpa_pwm_t;
typedef struct {
    tulpa_pwm_t pwm;
    double min_us, max_us, range_deg, angle_deg;
    int32_t attached;
} tulpa_servo_t;

/* ------------------------------------------------------------------ program entry */

/* Generated from the program's top-level statements (when it has any). */
void tulpa_entry(void);

/* Called when a contract, invariant, `always` monitor, bounds or overflow check fails.
 * Generated code defines a weak version that calls tulpa_hal_panic(); override it to stop
 * actuators first. */
void tulpa_violation(const char *msg);

/* ------------------------------------------------------------------ runtime ABI (tulpa_rt.c) */

void tulpa_gpio_open(tulpa_gpio_t *g, int32_t pin, int32_t output);
void tulpa_gpio_write(tulpa_gpio_t *g, int32_t level);
int64_t tulpa_gpio_read(tulpa_gpio_t *g);
void tulpa_gpio_toggle(tulpa_gpio_t *g);
void tulpa_gpio_set_direction(tulpa_gpio_t *g, int32_t output);

void tulpa_i2c_open(tulpa_i2c_t *d, int32_t bus, int32_t addr);
void tulpa_i2c_write_reg(tulpa_i2c_t *d, int64_t reg, int64_t value);
void tulpa_i2c_write_reg_n(tulpa_i2c_t *d, int64_t reg, const int64_t *data, int64_t n);
int64_t tulpa_i2c_read_reg(tulpa_i2c_t *d, int64_t reg);
void tulpa_i2c_read_regs(tulpa_i2c_t *d, int64_t reg, int64_t *out, int64_t n);
int64_t tulpa_i2c_read_u16(tulpa_i2c_t *d, int64_t reg, int32_t little, int32_t is_signed);
void tulpa_i2c_update_reg(tulpa_i2c_t *d, int64_t reg, int64_t mask, int64_t value);
void tulpa_i2c_write(tulpa_i2c_t *d, const int64_t *data, int64_t n);
void tulpa_i2c_read(tulpa_i2c_t *d, int64_t *out, int64_t n);
void tulpa_i2c_write_read(tulpa_i2c_t *d, const int64_t *data, int64_t n, int64_t *out, int64_t m);

void tulpa_spi_open(tulpa_spi_t *d, int32_t bus, int32_t cs);
void tulpa_spi_transfer(tulpa_spi_t *d, const int64_t *tx, int64_t *rx, int64_t n);
void tulpa_spi_write(tulpa_spi_t *d, const int64_t *tx, int64_t n);
void tulpa_spi_read(tulpa_spi_t *d, int64_t *rx, int64_t n);
void tulpa_spi_set_speed(tulpa_spi_t *d, int64_t hz);
void tulpa_spi_set_mode(tulpa_spi_t *d, int64_t mode);

void tulpa_uart_open(tulpa_uart_t *u, const char *port, int32_t baud);
void tulpa_uart_write(tulpa_uart_t *u, const int64_t *data, int64_t n);
void tulpa_uart_read(tulpa_uart_t *u, int64_t *out, int64_t n);
int64_t tulpa_uart_available(tulpa_uart_t *u);

void tulpa_pwm_open(tulpa_pwm_t *p, int32_t chip, int32_t channel);
void tulpa_pwm_frequency(tulpa_pwm_t *p, double hz);
void tulpa_pwm_period(tulpa_pwm_t *p, double seconds);
void tulpa_pwm_duty(tulpa_pwm_t *p, double fraction);
void tulpa_pwm_pulse(tulpa_pwm_t *p, double seconds);
void tulpa_pwm_enable(tulpa_pwm_t *p, int32_t on);

void tulpa_servo_open(tulpa_servo_t *s, int32_t chip, int32_t channel, double min_us, double max_us,
                      double range_deg);
void tulpa_servo_angle(tulpa_servo_t *s, double radians);
double tulpa_servo_get_angle(tulpa_servo_t *s);
void tulpa_servo_center(tulpa_servo_t *s);
void tulpa_servo_detach(tulpa_servo_t *s);
double tulpa_servo_pulse_us(tulpa_servo_t *s);

void tulpa_sleep(double seconds);
int64_t tulpa_millis(void);
int64_t tulpa_micros(void);
void tulpa_every_start(int64_t *next_us);
void tulpa_every_wait(int64_t *next_us, int64_t period_us);

/* print() and f-strings; output goes to the console, or to a UART during UART.send(f"...") */
void tulpa_print_str(const char *s);
void tulpa_print_i64(int64_t v);
void tulpa_print_f64(double v);
void tulpa_print_bool(int32_t v);
void tulpa_print_q(double si, const char *unit);
void tulpa_print_arr(const int64_t *a, int64_t n);
void tulpa_print_fmt_i64(int64_t v, const char *fmt);
void tulpa_print_fmt_f64(double v, const char *fmt);
void tulpa_print_bin(int64_t v, int32_t width, int32_t zero);
void tulpa_print_nl(void);
void tulpa_out_uart(tulpa_uart_t *u);
void tulpa_out_console(void);

/* simulation-only hooks (hal/sim.c), used by sim::* in Tulpa */
void tulpa_sim_i2c_set(int64_t bus, int64_t addr, int64_t reg, const int64_t *data, int64_t n);
void tulpa_sim_i2c_get(int64_t bus, int64_t addr, int64_t reg, int64_t *out, int64_t n);
void tulpa_sim_gpio_input(int64_t pin, int64_t level);
int64_t tulpa_sim_gpio_level(int64_t pin);
void tulpa_sim_spi_respond(int64_t bus, int64_t cs, const int64_t *data, int64_t n);
void tulpa_sim_uart_feed(const uint8_t *data, size_t n);

/* ------------------------------------------------------------------ board HAL (ports implement)
 * Return 0 (or a non-negative value where noted) on success and a negative number on failure;
 * tulpa_rt.c turns failures into a panic with a readable message. */

int tulpa_hal_gpio_init(tulpa_gpio_t *g);                 /* g->pin, g->output are set */
int tulpa_hal_gpio_set_output(tulpa_gpio_t *g, int output);
int tulpa_hal_gpio_write(tulpa_gpio_t *g, int level);
int tulpa_hal_gpio_read(tulpa_gpio_t *g);                 /* 0 or 1 */

int tulpa_hal_i2c_init(tulpa_i2c_t *d);                   /* d->bus, d->addr are set */
int tulpa_hal_i2c_write(tulpa_i2c_t *d, const uint8_t *data, size_t n);
/* write `wn` bytes, repeated start, read `rn` bytes */
int tulpa_hal_i2c_write_read(tulpa_i2c_t *d, const uint8_t *w, size_t wn, uint8_t *r, size_t rn);

int tulpa_hal_spi_init(tulpa_spi_t *d);                   /* d->bus, d->cs, d->hz, d->mode */
int tulpa_hal_spi_configure(tulpa_spi_t *d);              /* d->hz or d->mode changed */
int tulpa_hal_spi_transfer(tulpa_spi_t *d, const uint8_t *tx, uint8_t *rx, size_t n);

int tulpa_hal_uart_init(tulpa_uart_t *u);                 /* u->port, u->baud */
int tulpa_hal_uart_write(tulpa_uart_t *u, const uint8_t *data, size_t n);
int tulpa_hal_uart_read(tulpa_uart_t *u, uint8_t *out, size_t n);   /* returns bytes read */
int tulpa_hal_uart_available(tulpa_uart_t *u);            /* returns bytes waiting */

int tulpa_hal_pwm_init(tulpa_pwm_t *p);                   /* p->chip, p->channel */
int tulpa_hal_pwm_apply(tulpa_pwm_t *p);                  /* period_ns, duty_ns, enabled */

uint64_t tulpa_hal_micros(void);                          /* monotonic, since boot */
void tulpa_hal_delay_us(uint64_t us);
void tulpa_hal_console_write(const char *s, size_t n);
void tulpa_hal_panic(const char *msg)
#if defined(__GNUC__) || defined(__clang__)
    __attribute__((noreturn))
#endif
    ;

#ifdef __cplusplus
}
#endif

#endif /* TULPA_HAL_H */

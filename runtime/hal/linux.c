/* hal/linux.c: Tulpa native code on Linux single-board computers (Raspberry Pi, BeagleBone,
 * Jetson, ...).
 *
 *   GPIO   /dev/gpiochipN character device (v2 uAPI, kernel 5.10+). The pin number is the line
 *          offset on TULPA_GPIOCHIP (default /dev/gpiochip0; on a Pi 5 use /dev/gpiochip4).
 *   I2C    /dev/i2c-<bus>
 *   SPI    /dev/spidev<bus>.<cs>
 *   UART   the port path, e.g. "/dev/ttyAMA0" or "/dev/ttyUSB0" (raw 8N1)
 *   PWM    /sys/class/pwm/pwmchip<chip>/pwm<channel>
 *
 * Part of Tulpa (BSD 3-Clause). */
#define _GNU_SOURCE
#include "tulpa_hal.h"

#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <termios.h>
#include <time.h>
#include <unistd.h>

#include <linux/gpio.h>
#include <linux/i2c-dev.h>
#include <linux/i2c.h>
#include <linux/spi/spidev.h>

#define FD(p) ((int)(intptr_t)(p))
#define HW(fd) ((void *)(intptr_t)(fd))

/* ------------------------------------------------------------------ GPIO */

static int chip_fd = -1;

static int gpio_request(tulpa_gpio_t *g) {
    if (chip_fd < 0) {
        const char *path = getenv("TULPA_GPIOCHIP");
        chip_fd = open(path ? path : "/dev/gpiochip0", O_RDWR | O_CLOEXEC);
        if (chip_fd < 0) return -1;
    }
    if (g->hw) close(FD(g->hw));
    struct gpio_v2_line_request req;
    memset(&req, 0, sizeof req);
    req.offsets[0] = (uint32_t)g->pin;
    req.num_lines = 1;
    req.config.flags = g->output ? GPIO_V2_LINE_FLAG_OUTPUT : GPIO_V2_LINE_FLAG_INPUT;
    snprintf(req.consumer, sizeof req.consumer, "tulpa");
    if (ioctl(chip_fd, GPIO_V2_GET_LINE_IOCTL, &req) < 0) return -1;
    g->hw = HW(req.fd);
    return 0;
}

int tulpa_hal_gpio_init(tulpa_gpio_t *g) {
    g->hw = NULL;
    return gpio_request(g);
}

int tulpa_hal_gpio_set_output(tulpa_gpio_t *g, int output) {
    g->output = output;
    return gpio_request(g);
}

int tulpa_hal_gpio_write(tulpa_gpio_t *g, int level) {
    struct gpio_v2_line_values v = {.bits = level ? 1 : 0, .mask = 1};
    return ioctl(FD(g->hw), GPIO_V2_LINE_SET_VALUES_IOCTL, &v) < 0 ? -1 : 0;
}

int tulpa_hal_gpio_read(tulpa_gpio_t *g) {
    struct gpio_v2_line_values v = {.bits = 0, .mask = 1};
    if (ioctl(FD(g->hw), GPIO_V2_LINE_GET_VALUES_IOCTL, &v) < 0) return -1;
    return (int)(v.bits & 1);
}

/* ------------------------------------------------------------------ I2C */

int tulpa_hal_i2c_init(tulpa_i2c_t *d) {
    char path[32];
    snprintf(path, sizeof path, "/dev/i2c-%d", (int)d->bus);
    int fd = open(path, O_RDWR | O_CLOEXEC);
    if (fd < 0) return -1;
    d->hw = HW(fd);
    return 0;
}

int tulpa_hal_i2c_write(tulpa_i2c_t *d, const uint8_t *data, size_t n) {
    struct i2c_msg m = {.addr = (uint16_t)d->addr, .flags = 0, .len = (uint16_t)n, .buf = (uint8_t *)data};
    struct i2c_rdwr_ioctl_data x = {.msgs = &m, .nmsgs = 1};
    return ioctl(FD(d->hw), I2C_RDWR, &x) < 0 ? -1 : 0;
}

int tulpa_hal_i2c_write_read(tulpa_i2c_t *d, const uint8_t *w, size_t wn, uint8_t *r, size_t rn) {
    struct i2c_msg m[2];
    int k = 0;
    if (wn) m[k++] = (struct i2c_msg){.addr = (uint16_t)d->addr, .flags = 0, .len = (uint16_t)wn, .buf = (uint8_t *)w};
    m[k++] = (struct i2c_msg){.addr = (uint16_t)d->addr, .flags = I2C_M_RD, .len = (uint16_t)rn, .buf = r};
    struct i2c_rdwr_ioctl_data x = {.msgs = m, .nmsgs = (uint32_t)k};
    return ioctl(FD(d->hw), I2C_RDWR, &x) < 0 ? -1 : 0;
}

/* ------------------------------------------------------------------ SPI */

int tulpa_hal_spi_init(tulpa_spi_t *d) {
    char path[40];
    snprintf(path, sizeof path, "/dev/spidev%d.%d", (int)d->bus, (int)d->cs);
    int fd = open(path, O_RDWR | O_CLOEXEC);
    if (fd < 0) return -1;
    d->hw = HW(fd);
    return tulpa_hal_spi_configure(d);
}

int tulpa_hal_spi_configure(tulpa_spi_t *d) {
    uint8_t mode = (uint8_t)d->mode, bits = 8;
    uint32_t hz = (uint32_t)d->hz;
    if (ioctl(FD(d->hw), SPI_IOC_WR_MODE, &mode) < 0) return -1;
    if (ioctl(FD(d->hw), SPI_IOC_WR_BITS_PER_WORD, &bits) < 0) return -1;
    if (ioctl(FD(d->hw), SPI_IOC_WR_MAX_SPEED_HZ, &hz) < 0) return -1;
    return 0;
}

int tulpa_hal_spi_transfer(tulpa_spi_t *d, const uint8_t *tx, uint8_t *rx, size_t n) {
    struct spi_ioc_transfer t;
    memset(&t, 0, sizeof t);
    t.tx_buf = (uintptr_t)tx;
    t.rx_buf = (uintptr_t)rx;
    t.len = (uint32_t)n;
    t.speed_hz = (uint32_t)d->hz;
    t.bits_per_word = 8;
    return ioctl(FD(d->hw), SPI_IOC_MESSAGE(1), &t) < 0 ? -1 : 0;
}

/* ------------------------------------------------------------------ UART */

static speed_t baud_const(int b) {
    switch (b) {
    case 1200: return B1200;
    case 2400: return B2400;
    case 4800: return B4800;
    case 9600: return B9600;
    case 19200: return B19200;
    case 38400: return B38400;
    case 57600: return B57600;
    case 115200: return B115200;
    case 230400: return B230400;
    case 460800: return B460800;
    case 921600: return B921600;
    default: return 0;
    }
}

int tulpa_hal_uart_init(tulpa_uart_t *u) {
    speed_t sp = baud_const(u->baud);
    if (!sp) return -1;
    int fd = open(u->port, O_RDWR | O_NOCTTY | O_CLOEXEC);
    if (fd < 0) return -1;
    struct termios t;
    if (tcgetattr(fd, &t) < 0) {
        close(fd);
        return -1;
    }
    cfmakeraw(&t);
    cfsetispeed(&t, sp);
    cfsetospeed(&t, sp);
    t.c_cflag |= CLOCAL | CREAD;
    t.c_cc[VMIN] = 0;
    t.c_cc[VTIME] = 10; /* reads wait up to 1 s */
    if (tcsetattr(fd, TCSANOW, &t) < 0) {
        close(fd);
        return -1;
    }
    u->hw = HW(fd);
    return 0;
}

int tulpa_hal_uart_write(tulpa_uart_t *u, const uint8_t *data, size_t n) {
    size_t done = 0;
    while (done < n) {
        ssize_t k = write(FD(u->hw), data + done, n - done);
        if (k < 0) {
            if (errno == EINTR) continue;
            return -1;
        }
        done += (size_t)k;
    }
    return 0;
}

int tulpa_hal_uart_read(tulpa_uart_t *u, uint8_t *out, size_t n) {
    size_t got = 0;
    while (got < n) {
        ssize_t k = read(FD(u->hw), out + got, n - got);
        if (k < 0) {
            if (errno == EINTR) continue;
            return -1;
        }
        if (k == 0) break; /* timeout */
        got += (size_t)k;
    }
    return (int)got;
}

int tulpa_hal_uart_available(tulpa_uart_t *u) {
    int n = 0;
    return ioctl(FD(u->hw), FIONREAD, &n) < 0 ? -1 : n;
}

/* ------------------------------------------------------------------ PWM (sysfs) */

static int sysfs_write(const char *path, long long v) {
    FILE *f = fopen(path, "w");
    if (!f) return -1;
    int ok = fprintf(f, "%lld", v) > 0;
    return (fclose(f) == 0 && ok) ? 0 : -1;
}

int tulpa_hal_pwm_init(tulpa_pwm_t *p) {
    char path[96];
    snprintf(path, sizeof path, "/sys/class/pwm/pwmchip%d/pwm%d", (int)p->chip, (int)p->channel);
    if (access(path, F_OK) != 0) {
        char exp[96];
        snprintf(exp, sizeof exp, "/sys/class/pwm/pwmchip%d/export", (int)p->chip);
        if (sysfs_write(exp, p->channel) < 0) return -1;
        for (int i = 0; i < 100 && access(path, W_OK) != 0; i++) usleep(10000); /* udev */
    }
    return 0;
}

int tulpa_hal_pwm_apply(tulpa_pwm_t *p) {
    char base[96], path[128];
    snprintf(base, sizeof base, "/sys/class/pwm/pwmchip%d/pwm%d", (int)p->chip, (int)p->channel);
    /* duty must never exceed period, so order the writes */
    snprintf(path, sizeof path, "%s/duty_cycle", base);
    sysfs_write(path, 0);
    snprintf(path, sizeof path, "%s/period", base);
    if (sysfs_write(path, p->period_ns) < 0) return -1;
    snprintf(path, sizeof path, "%s/duty_cycle", base);
    if (sysfs_write(path, p->duty_ns) < 0) return -1;
    snprintf(path, sizeof path, "%s/enable", base);
    return sysfs_write(path, p->enabled ? 1 : 0);
}

/* ------------------------------------------------------------------ time and console */

uint64_t tulpa_hal_micros(void) {
    static struct timespec t0;
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    if (t0.tv_sec == 0 && t0.tv_nsec == 0) t0 = t;
    return (uint64_t)(t.tv_sec - t0.tv_sec) * 1000000u + (uint64_t)((t.tv_nsec - t0.tv_nsec) / 1000);
}

void tulpa_hal_delay_us(uint64_t us) {
    struct timespec t = {.tv_sec = (time_t)(us / 1000000u), .tv_nsec = (long)(us % 1000000u) * 1000};
    while (nanosleep(&t, &t) < 0 && errno == EINTR) {
    }
}

void tulpa_hal_console_write(const char *s, size_t n) { fwrite(s, 1, n, stdout); }

void tulpa_hal_panic(const char *msg) {
    fflush(stdout);
    fprintf(stderr, "%s\n", msg);
    exit(70);
}

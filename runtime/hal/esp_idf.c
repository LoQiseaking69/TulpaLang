/* hal/esp_idf.c: Tulpa native code on Espressif chips with ESP-IDF 5.x
 * (ESP32, ESP32-S2, ESP32-S3, ESP32-C3, ESP32-C6, ...).
 *
 * Mapping from Tulpa's device model:
 *
 *   GPIO::new(pin)          any GPIO
 *   I2C::new(bus, addr)     I2C port `bus` (0 or 1), new i2c_master driver, 400 kHz
 *   SPI::new(bus, cs)       bus 0 = SPI2_HOST, bus 1 = SPI3_HOST; `cs` is the chip-select GPIO
 *   UART::new("uart1", b)   "uart0".."uart2" (or "0".."2")
 *   PWM::new(chip, pin)     LEDC, the second argument is the GPIO; channels and timers are
 *                           allocated as devices open (up to 8 / 4)
 *   Servo::new(chip, pin)   same, at 50 Hz
 *
 * Bus pins default to common dev-kit pins per chip and are set at build time, e.g.
 * -DTULPA_I2C0_SDA=21 -DTULPA_I2C0_SCL=22 (target_compile_definitions in main/CMakeLists.txt).
 *
 * Part of Tulpa (BSD 3-Clause). */
#include "tulpa_hal.h"

#include <stdio.h>
#include <string.h>

#include "driver/gpio.h"
#include "driver/i2c_master.h"
#include "driver/ledc.h"
#include "driver/spi_master.h"
#include "driver/uart.h"
#include "esp_rom_sys.h"
#include "esp_system.h"
#include "esp_timer.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "sdkconfig.h"

#if CONFIG_IDF_TARGET_ESP32
#define DEF_SDA 21
#define DEF_SCL 22
#define DEF_SCK 18
#define DEF_MOSI 23
#define DEF_MISO 19
#elif CONFIG_IDF_TARGET_ESP32S3 || CONFIG_IDF_TARGET_ESP32S2
#define DEF_SDA 8
#define DEF_SCL 9
#define DEF_SCK 12
#define DEF_MOSI 11
#define DEF_MISO 13
#else /* C3, C6, H2, ... */
#define DEF_SDA 5
#define DEF_SCL 6
#define DEF_SCK 4
#define DEF_MOSI 7
#define DEF_MISO 2
#endif

#ifndef TULPA_I2C0_SDA
#define TULPA_I2C0_SDA DEF_SDA
#endif
#ifndef TULPA_I2C0_SCL
#define TULPA_I2C0_SCL DEF_SCL
#endif
#ifndef TULPA_I2C1_SDA
#define TULPA_I2C1_SDA DEF_SDA
#endif
#ifndef TULPA_I2C1_SCL
#define TULPA_I2C1_SCL DEF_SCL
#endif
#ifndef TULPA_I2C_HZ
#define TULPA_I2C_HZ 400000
#endif
#ifndef TULPA_SPI_SCK
#define TULPA_SPI_SCK DEF_SCK
#endif
#ifndef TULPA_SPI_MOSI
#define TULPA_SPI_MOSI DEF_MOSI
#endif
#ifndef TULPA_SPI_MISO
#define TULPA_SPI_MISO DEF_MISO
#endif
#ifndef TULPA_UART_TX
#define TULPA_UART_TX UART_PIN_NO_CHANGE
#endif
#ifndef TULPA_UART_RX
#define TULPA_UART_RX UART_PIN_NO_CHANGE
#endif
#ifndef TULPA_TIMEOUT_MS
#define TULPA_TIMEOUT_MS 100
#endif

/* ------------------------------------------------------------------ GPIO */

int tulpa_hal_gpio_init(tulpa_gpio_t *g) {
    if (!GPIO_IS_VALID_GPIO(g->pin)) return -1;
    gpio_reset_pin((gpio_num_t)g->pin);
    return tulpa_hal_gpio_set_output(g, g->output);
}

int tulpa_hal_gpio_set_output(tulpa_gpio_t *g, int output) {
    /* outputs stay readable, so read() returns the driven level as in the interpreter */
    return gpio_set_direction((gpio_num_t)g->pin, output ? GPIO_MODE_INPUT_OUTPUT : GPIO_MODE_INPUT) == ESP_OK ? 0 : -1;
}

int tulpa_hal_gpio_write(tulpa_gpio_t *g, int level) {
    return gpio_set_level((gpio_num_t)g->pin, (uint32_t)level) == ESP_OK ? 0 : -1;
}

int tulpa_hal_gpio_read(tulpa_gpio_t *g) { return gpio_get_level((gpio_num_t)g->pin); }

/* ------------------------------------------------------------------ I2C */

static i2c_master_bus_handle_t i2c_bus[2];

int tulpa_hal_i2c_init(tulpa_i2c_t *d) {
    if (d->bus < 0 || d->bus >= SOC_I2C_NUM || d->bus > 1) return -1;
    if (!i2c_bus[d->bus]) {
        i2c_master_bus_config_t bc = {
            .i2c_port = d->bus,
            .sda_io_num = d->bus ? TULPA_I2C1_SDA : TULPA_I2C0_SDA,
            .scl_io_num = d->bus ? TULPA_I2C1_SCL : TULPA_I2C0_SCL,
            .clk_source = I2C_CLK_SRC_DEFAULT,
            .glitch_ignore_cnt = 7,
            .flags.enable_internal_pullup = true,
        };
        if (i2c_new_master_bus(&bc, &i2c_bus[d->bus]) != ESP_OK) return -1;
    }
    i2c_device_config_t dc = {
        .dev_addr_length = I2C_ADDR_BIT_LEN_7,
        .device_address = (uint16_t)d->addr,
        .scl_speed_hz = TULPA_I2C_HZ,
    };
    i2c_master_dev_handle_t h;
    if (i2c_master_bus_add_device(i2c_bus[d->bus], &dc, &h) != ESP_OK) return -1;
    d->hw = h;
    return 0;
}

int tulpa_hal_i2c_write(tulpa_i2c_t *d, const uint8_t *data, size_t n) {
    return i2c_master_transmit((i2c_master_dev_handle_t)d->hw, data, n, TULPA_TIMEOUT_MS) == ESP_OK ? 0 : -1;
}

int tulpa_hal_i2c_write_read(tulpa_i2c_t *d, const uint8_t *w, size_t wn, uint8_t *r, size_t rn) {
    i2c_master_dev_handle_t h = (i2c_master_dev_handle_t)d->hw;
    esp_err_t e = wn ? i2c_master_transmit_receive(h, w, wn, r, rn, TULPA_TIMEOUT_MS)
                     : i2c_master_receive(h, r, rn, TULPA_TIMEOUT_MS);
    return e == ESP_OK ? 0 : -1;
}

/* ------------------------------------------------------------------ SPI */

static bool spi_ready[2];

/* Tulpa SPI bus 0 is SPI2 (FSPI); bus 1 is SPI3 on chips that have it (ESP32, S2, S3). */
static spi_host_device_t spi_host(int bus) {
#if SOC_SPI_PERIPH_NUM >= 3
    return bus ? SPI3_HOST : SPI2_HOST;
#else
    (void)bus;
    return SPI2_HOST;
#endif
}

static int spi_add(tulpa_spi_t *d) {
    spi_device_interface_config_t dc = {
        .clock_speed_hz = d->hz,
        .mode = (uint8_t)d->mode,
        .spics_io_num = d->cs,
        .queue_size = 1,
    };
    spi_device_handle_t h;
    if (spi_bus_add_device(spi_host(d->bus), &dc, &h) != ESP_OK) return -1;
    d->hw = h;
    return 0;
}

int tulpa_hal_spi_init(tulpa_spi_t *d) {
    if (d->bus < 0 || d->bus > 1) return -1;
#if SOC_SPI_PERIPH_NUM < 3
    if (d->bus == 1) return -1; /* this chip has a single general-purpose SPI host */
#endif
    if (!spi_ready[d->bus]) {
        spi_bus_config_t bc = {
            .mosi_io_num = TULPA_SPI_MOSI,
            .miso_io_num = TULPA_SPI_MISO,
            .sclk_io_num = TULPA_SPI_SCK,
            .quadwp_io_num = -1,
            .quadhd_io_num = -1,
            .max_transfer_sz = 256,
        };
        if (spi_bus_initialize(spi_host(d->bus), &bc, SPI_DMA_CH_AUTO) != ESP_OK) return -1;
        spi_ready[d->bus] = true;
    }
    return spi_add(d);
}

int tulpa_hal_spi_configure(tulpa_spi_t *d) {
    /* speed and mode are per device in ESP-IDF: re-add it */
    if (d->hw) spi_bus_remove_device((spi_device_handle_t)d->hw);
    d->hw = NULL;
    return spi_add(d);
}

int tulpa_hal_spi_transfer(tulpa_spi_t *d, const uint8_t *tx, uint8_t *rx, size_t n) {
    spi_transaction_t t;
    memset(&t, 0, sizeof t);
    t.length = n * 8;
    t.tx_buffer = tx;
    t.rx_buffer = rx;
    return spi_device_polling_transmit((spi_device_handle_t)d->hw, &t) == ESP_OK ? 0 : -1;
}

/* ------------------------------------------------------------------ UART */

int tulpa_hal_uart_init(tulpa_uart_t *u) {
    const char *p = u->port;
    if (!strncmp(p, "uart", 4)) p += 4;
    if (p[0] < '0' || p[0] > '2' || p[1]) return -1;
    int port = p[0] - '0';
    if (port >= SOC_UART_NUM) return -1;
    uart_config_t c = {
        .baud_rate = u->baud,
        .data_bits = UART_DATA_8_BITS,
        .parity = UART_PARITY_DISABLE,
        .stop_bits = UART_STOP_BITS_1,
        .flow_ctrl = UART_HW_FLOWCTRL_DISABLE,
        .source_clk = UART_SCLK_DEFAULT,
    };
    if (!uart_is_driver_installed(port) && uart_driver_install(port, 512, 0, 0, NULL, 0) != ESP_OK) return -1;
    if (uart_param_config(port, &c) != ESP_OK) return -1;
    if (uart_set_pin(port, TULPA_UART_TX, TULPA_UART_RX, UART_PIN_NO_CHANGE, UART_PIN_NO_CHANGE) != ESP_OK) return -1;
    u->hw = (void *)(intptr_t)port;
    return 0;
}

int tulpa_hal_uart_write(tulpa_uart_t *u, const uint8_t *data, size_t n) {
    return uart_write_bytes((uart_port_t)(intptr_t)u->hw, data, n) == (int)n ? 0 : -1;
}

int tulpa_hal_uart_read(tulpa_uart_t *u, uint8_t *out, size_t n) {
    return uart_read_bytes((uart_port_t)(intptr_t)u->hw, out, (uint32_t)n, pdMS_TO_TICKS(1000));
}

int tulpa_hal_uart_available(tulpa_uart_t *u) {
    size_t n = 0;
    return uart_get_buffered_data_len((uart_port_t)(intptr_t)u->hw, &n) == ESP_OK ? (int)n : -1;
}

/* ------------------------------------------------------------------ PWM (LEDC) */

static int next_channel;

typedef struct {
    ledc_channel_t ch;
    ledc_timer_t timer;
    uint32_t bits;
    int64_t period_ns;
} esp_pwm_t;

static esp_pwm_t pwms[LEDC_CHANNEL_MAX];

int tulpa_hal_pwm_init(tulpa_pwm_t *p) {
    if (next_channel >= LEDC_CHANNEL_MAX || !GPIO_IS_VALID_OUTPUT_GPIO(p->channel)) return -1;
    esp_pwm_t *e = &pwms[next_channel];
    e->ch = (ledc_channel_t)next_channel;
    e->timer = (ledc_timer_t)(next_channel % LEDC_TIMER_MAX);
    e->bits = 0;
    e->period_ns = 0;
    next_channel++;
    p->hw = e;
    return 0;
}

int tulpa_hal_pwm_apply(tulpa_pwm_t *p) {
    esp_pwm_t *e = (esp_pwm_t *)p->hw;
    if (p->period_ns != e->period_ns) {
        uint32_t hz = (uint32_t)(1e9 / (double)p->period_ns + 0.5);
        uint32_t bits = ledc_find_suitable_duty_resolution(80000000, hz);
        if (bits > 14) bits = 14;
        if (bits < 1) return -1;
        ledc_timer_config_t tc = {
            .speed_mode = LEDC_LOW_SPEED_MODE,
            .duty_resolution = (ledc_timer_bit_t)bits,
            .timer_num = e->timer,
            .freq_hz = hz,
            .clk_cfg = LEDC_AUTO_CLK,
        };
        if (ledc_timer_config(&tc) != ESP_OK) return -1;
        ledc_channel_config_t cc = {
            .gpio_num = p->channel,
            .speed_mode = LEDC_LOW_SPEED_MODE,
            .channel = e->ch,
            .timer_sel = e->timer,
            .duty = 0,
            .hpoint = 0,
        };
        if (ledc_channel_config(&cc) != ESP_OK) return -1;
        e->bits = bits;
        e->period_ns = p->period_ns;
    }
    uint32_t max = (1u << e->bits);
    uint32_t duty = p->enabled ? (uint32_t)((double)p->duty_ns / (double)p->period_ns * max + 0.5) : 0;
    if (duty > max) duty = max;
    if (ledc_set_duty(LEDC_LOW_SPEED_MODE, e->ch, duty) != ESP_OK) return -1;
    return ledc_update_duty(LEDC_LOW_SPEED_MODE, e->ch) == ESP_OK ? 0 : -1;
}

/* ------------------------------------------------------------------ time and console */

uint64_t tulpa_hal_micros(void) { return (uint64_t)esp_timer_get_time(); }

void tulpa_hal_delay_us(uint64_t us) {
    /* yield to FreeRTOS for long waits (keeps the idle task and watchdog happy) */
    if (us >= 10000) {
        TickType_t ticks = pdMS_TO_TICKS(us / 1000);
        if (ticks) vTaskDelay(ticks);
        us -= (uint64_t)ticks * portTICK_PERIOD_MS * 1000u;
    }
    if (us) esp_rom_delay_us((uint32_t)us);
}

void tulpa_hal_console_write(const char *s, size_t n) {
    fwrite(s, 1, n, stdout);
    fflush(stdout);
}

void tulpa_hal_panic(const char *msg) { esp_system_abort(msg); }

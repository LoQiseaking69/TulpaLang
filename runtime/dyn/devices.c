/* ---------------------------------------------------------------- devices */

enum { N_GPIO, N_I2C, N_SPI, N_UART, N_PWM, N_SERVO, N_PID, N_LINK };

/* Simulation hooks exist only in the sim HAL; elsewhere these weak references are null. */
__attribute__((weak)) void tulpa_sim_i2c_set(int64_t bus, int64_t addr, int64_t reg, const int64_t *data, int64_t n);
__attribute__((weak)) void tulpa_sim_i2c_get(int64_t bus, int64_t addr, int64_t reg, int64_t *out, int64_t n);
__attribute__((weak)) void tulpa_sim_gpio_input(int64_t pin, int64_t level);
__attribute__((weak)) int64_t tulpa_sim_gpio_level(int64_t pin);
__attribute__((weak)) void tulpa_sim_spi_respond(int64_t bus, int64_t cs, const int64_t *data, int64_t n);
__attribute__((weak)) void tulpa_sim_uart_feed(const uint8_t *data, size_t n);

#define XFER 256
static int64_t TX[XFER + 2], RX[XFER + 2];

static Native *native_new(int kind) {
    Native *n = (Native *)alloc(T_NATIVE, sizeof(Native));
    n->kind = (uint8_t)kind;
    n->extra = NIL;
    return n;
}
static int64_t fill_tx(TV v, int64_t at) {
    size_t n;
    uint8_t *b = to_bytes(v, &n);
    if (n + (size_t)at > XFER) { free(b); fail("transfer of %u bytes exceeds %d", (unsigned)n, XFER); }
    for (size_t i = 0; i < n; i++) TX[at + (int64_t)i] = b[i];
    free(b);
    return (int64_t)n + at;
}
static int64_t count_arg(const char *name, TV v) {
    int64_t n = intarg(name, v);
    if (n < 0) n = 0;
    if (n > XFER) fail("transfer of %lld bytes exceeds %d", (long long)n, XFER);
    return n;
}
static TV rx_list(int64_t n) {
    List *l = list_new((uint32_t)n);
    for (int64_t i = 0; i < n; i++) l->v[i] = mk_int(RX[i]);
    l->n = (uint32_t)n;
    return mk_obj(&l->h);
}
static bool parse_direction(TV v) {
    char d[24];
    show_buf(v, d, sizeof d);
    for (char *c = d; *c; c++) if (*c >= 'A' && *c <= 'Z') *c = (char)(*c + 32);
    if (!strcmp(d, "out") || !strcmp(d, "output")) return true;
    if (!strcmp(d, "in") || !strcmp(d, "input")) return false;
    fail("invalid GPIO direction '%s' (use \"in\" or \"out\")", d);
}
#define ARG(k, what) (argc > (k) ? argv[k] : (fail("missing argument '%s'", what), NIL))

B(gpio_new) {
    arity("GPIO::new", argc, 1, 2);
    int64_t pin = intarg("GPIO::new", argv[0]);
    if (pin < 0) fail("invalid GPIO pin %lld", (long long)pin);
    bool out = argc > 1 ? parse_direction(argv[1]) : true;
    Native *n = native_new(N_GPIO);
    tulpa_gpio_open(&n->d.gpio, (int32_t)pin, out);
    return mk_obj(&n->h);
}
B(i2c_new) {
    arity("I2C::new", argc, 1, 2);
    int64_t bus = argc == 1 ? 1 : intarg("I2C::new", argv[0]), addr = intarg("I2C::new", argv[argc - 1]);
    if (addr < 0 || addr > 0x7f) fail("I2C address 0x%llx out of 7-bit range", (long long)addr);
    Native *n = native_new(N_I2C);
    tulpa_i2c_open(&n->d.i2c, (int32_t)bus, (int32_t)addr);
    return mk_obj(&n->h);
}
B(i2c_scan) {
    arity("I2C::scan", argc, 0, 1);
    int64_t bus = argc ? intarg("I2C::scan", argv[0]) : 1;
    List *l = list_new(0);
    if (tulpa_sim_gpio_level) return mk_obj(&l->h); /* the simulated bus answers at every address */
    for (int addr = 0x08; addr < 0x78; addr++) {
        tulpa_i2c_t d = {.bus = (int32_t)bus, .addr = addr};
        uint8_t byte;
        if (tulpa_hal_i2c_init(&d) >= 0 && tulpa_hal_i2c_write_read(&d, NULL, 0, &byte, 1) >= 0) list_push(l, mk_int(addr));
    }
    return mk_obj(&l->h);
}
B(spi_new) {
    arity("SPI::new", argc, 0, 2);
    Native *n = native_new(N_SPI);
    tulpa_spi_open(&n->d.spi, argc ? (int32_t)intarg("SPI::new", argv[0]) : 0, argc > 1 ? (int32_t)intarg("SPI::new", argv[1]) : 0);
    return mk_obj(&n->h);
}
B(uart_new) {
    arity("UART::new", argc, 1, 2);
    Native *n = native_new(N_UART);
    TV nv = mk_obj(&n->h);
    push(nv);
    n->extra = to_str(argv[0]);
    R.sp--;
    tulpa_uart_open(&n->d.uart, STR(n->extra)->s, argc > 1 ? (int32_t)intarg("UART::new", argv[1]) : 9600);
    return nv;
}
B(pwm_new) {
    arity("PWM::new", argc, 0, 2);
    Native *n = native_new(N_PWM);
    tulpa_pwm_open(&n->d.pwm, argc ? (int32_t)intarg("PWM::new", argv[0]) : 0, argc > 1 ? (int32_t)intarg("PWM::new", argv[1]) : 0);
    return mk_obj(&n->h);
}
B(servo_new) {
    arity("Servo::new", argc, 0, 5);
    double g[5] = {0, 0, 500, 2500, 180};
    for (int i = 0; i < argc; i++) g[i] = num("Servo::new", argv[i]);
    Native *n = native_new(N_SERVO);
    tulpa_servo_open(&n->d.servo, (int32_t)g[0], (int32_t)g[1], g[2], g[3], g[4]);
    return mk_obj(&n->h);
}
B(pid_new) {
    arity("PID::new", argc, 3, 3);
    Native *n = native_new(N_PID);
    for (int i = 0; i < 3; i++) n->d.pid[i] = num("PID::new", argv[i]);
    return mk_obj(&n->h);
}

static double pwm_seconds(TV v, double unit_if_plain, const char *what) {
    double s;
    if (v.t == T_QTY) {
        if (!seconds_of(v, &s)) {
            char shown[48];
            show_buf(v, shown, sizeof shown);
            fail("%s expects a time, got %s", what, shown);
        }
    } else s = num(what, v) * unit_if_plain;
    if (s < 0) fail("%s must be non-negative", what);
    return s;
}

static TV link_method(TV o, const char *name, int argc, TV *argv);

static TV native_method(TV o, const char *name, int argc, TV *argv) {
    Native *n = (Native *)o.u.o;
    switch (n->kind) {
    case N_GPIO: {
        tulpa_gpio_t *g = &n->d.gpio;
        if (IS("set_high") || IS("on")) { tulpa_gpio_write(g, 1); return NIL; }
        if (IS("set_low") || IS("off")) { tulpa_gpio_write(g, 0); return NIL; }
        if (IS("write") || IS("set")) { tulpa_gpio_write(g, truthy(ARG(0, "level"))); return NIL; }
        if (IS("read") || IS("value")) return mk_int(tulpa_gpio_read(g));
        if (IS("is_high")) return mk_bool(tulpa_gpio_read(g) != 0);
        if (IS("is_low")) return mk_bool(tulpa_gpio_read(g) == 0);
        if (IS("toggle")) { tulpa_gpio_toggle(g); return mk_int(tulpa_gpio_read(g)); }
        if (IS("blink")) {
            int64_t times = intarg("blink", ARG(0, "times"));
            double ms = argc > 1 ? num("blink", argv[1]) : 250.0;
            if (ms < 0) ms = 0;
            for (int64_t i = 0; i < times; i++) {
                tulpa_gpio_write(g, 1);
                tulpa_sleep(ms / 1000.0);
                tulpa_gpio_write(g, 0);
                tulpa_sleep(ms / 1000.0);
            }
            return NIL;
        }
        if (IS("set_direction")) { tulpa_gpio_set_direction(g, parse_direction(ARG(0, "direction"))); return NIL; }
        if (IS("pin")) return mk_int(g->pin);
        if (IS("direction")) return str_c(g->output ? "out" : "in");
        fail("GPIO has no method '%s'", name);
    }
    case N_I2C: {
        tulpa_i2c_t *d = &n->d.i2c;
        int64_t reg = 0;
        bool has_reg = strstr(name, "reg") || IS("read_u16") || IS("read_i16");
        if (has_reg) {
            reg = intarg(name, ARG(0, "register"));
            if (reg < 0 || reg > 255) fail("byte value %lld out of range 0..255", (long long)reg);
        }
        if (IS("write")) { tulpa_i2c_write(d, TX, fill_tx(ARG(0, "bytes"), 0)); return NIL; }
        if (IS("read")) { int64_t k = count_arg("read", ARG(0, "count")); tulpa_i2c_read(d, RX, k); return rx_list(k); }
        if (IS("write_read")) {
            int64_t w = fill_tx(ARG(0, "bytes"), 0), k = count_arg("write_read", ARG(1, "count"));
            tulpa_i2c_write_read(d, TX, w, RX, k);
            return rx_list(k);
        }
        if (IS("write_reg")) {
            TX[0] = reg;
            tulpa_i2c_write(d, TX, fill_tx(ARG(1, "value"), 1));
            return NIL;
        }
        if (IS("read_reg")) return mk_int(tulpa_i2c_read_reg(d, reg));
        if (IS("read_regs")) { int64_t k = count_arg("read_regs", ARG(1, "count")); tulpa_i2c_read_regs(d, reg, RX, k); return rx_list(k); }
        if (IS("read_u16") || IS("read_i16")) {
            bool little = argc > 1 && argv[1].t == T_STR && !strcmp(STR(argv[1])->s, "little");
            return mk_int(tulpa_i2c_read_u16(d, reg, little, name[5] == 'i'));
        }
        if (IS("update_reg")) {
            int64_t mask = intarg(name, ARG(1, "mask")) & 0xff, val = intarg(name, ARG(2, "value")) & 0xff;
            int64_t cur = tulpa_i2c_read_reg(d, reg), nv = (cur & ~mask) | (val & mask);
            tulpa_i2c_write_reg(d, reg, nv);
            return mk_int(nv & 0xff);
        }
        if (IS("address")) return mk_int(d->addr);
        if (IS("bus")) return mk_int(d->bus);
        fail("I2C has no method '%s'", name);
    }
    case N_SPI: {
        tulpa_spi_t *d = &n->d.spi;
        if (IS("transfer") || IS("xfer")) { int64_t k = fill_tx(ARG(0, "bytes"), 0); tulpa_spi_transfer(d, TX, RX, k); return rx_list(k); }
        if (IS("write")) { int64_t k = fill_tx(ARG(0, "bytes"), 0); tulpa_spi_transfer(d, TX, RX, k); return NIL; }
        if (IS("read")) {
            int64_t k = count_arg("read", ARG(0, "count"));
            memset(TX, 0, sizeof TX);
            tulpa_spi_transfer(d, TX, RX, k);
            return rx_list(k);
        }
        if (IS("set_speed") || IS("set_bus_speed")) { tulpa_spi_set_speed(d, intarg("set_speed", ARG(0, "hz"))); return NIL; }
        if (IS("set_mode")) {
            int64_t mode = intarg("set_mode", ARG(0, "mode"));
            if (mode < 0 || mode > 3) fail("SPI mode must be 0..3");
            tulpa_spi_set_mode(d, mode);
            return NIL;
        }
        fail("SPI has no method '%s'", name);
    }
    case N_UART: {
        tulpa_uart_t *u = &n->d.uart;
        if (IS("send") || IS("write") || IS("send_line") || IS("println")) {
            int64_t k = fill_tx(ARG(0, "data"), 0);
            if (name[0] == 'p' || name[4] == '_') { if (k + 2 > XFER) fail("transfer exceeds %d bytes", XFER); TX[k++] = '\r'; TX[k++] = '\n'; }
            tulpa_uart_write(u, TX, k);
            return mk_int(k);
        }
        if (IS("receive") || IS("read") || IS("read_text") || IS("read_line") || IS("readline")) {
            bool line = name[4] == '_' && name[5] == 'l' ? true : IS("readline");
            bool text = IS("read_text");
            int64_t want = line ? XFER : text ? (argc ? count_arg(name, argv[0]) : XFER) : count_arg(name, ARG(0, "count"));
            int ti = line ? 0 : 1;
            int64_t timeout = argc > ti ? intarg(name, argv[ti]) : line ? 1000 : 100;
            uint64_t deadline = tulpa_hal_micros() + (uint64_t)(timeout < 0 ? 0 : timeout) * 1000u;
            uint8_t buf[XFER];
            int64_t got = 0;
            bool ended = false;
            while (got < want && !ended) {
                if (tulpa_uart_available(u) > 0) {
                    int64_t one;
                    tulpa_uart_read(u, &one, 1);
                    if (line && one == '\n') { ended = true; break; }
                    buf[got++] = (uint8_t)one;
                } else {
                    if (tulpa_hal_micros() >= deadline) break;
                    tulpa_hal_delay_us(200);
                }
            }
            if (line) {
                if (!ended && !got) return NIL;
                if (got && buf[got - 1] == '\r') got--;
            }
            if (line || text) return str_n((const char *)buf, (size_t)got);
            return from_bytes(buf, (size_t)got);
        }
        if (IS("available")) return mk_int(tulpa_uart_available(u));
        if (IS("port")) return n->extra;
        if (IS("baud")) return mk_int(u->baud);
        fail("UART has no method '%s'", name);
    }
    case N_PWM: {
        tulpa_pwm_t *p = &n->d.pwm;
        if (IS("frequency") || IS("set_frequency")) {
            static const int8_t HZ[8] = {0, 0, -1};
            TV v = ARG(0, "frequency");
            double hz;
            if (v.t == T_QTY) {
                if (!dim_eq(QTY(v)->dim, HZ)) {
                    char shown[48];
                    show_buf(v, shown, sizeof shown);
                    fail("frequency() expects Hz, got %s", shown);
                }
                hz = QTY(v)->si;
            } else hz = num("frequency", v);
            if (hz <= 0) fail("frequency must be > 0");
            tulpa_pwm_frequency(p, hz);
            return NIL;
        }
        if (IS("period")) { tulpa_pwm_period(p, pwm_seconds(ARG(0, "period"), 1e-9, "period")); return NIL; }
        if (IS("duty") || IS("set_duty")) {
            double f = num("duty", ARG(0, "fraction"));
            if (!(f >= 0 && f <= 1)) {
                SB b = {0};
                fmt_shortest(f, &b);
                char shown[40];
                snprintf(shown, sizeof shown, "%s", b.p);
                free(b.p);
                fail("duty fraction %s outside 0..1", shown);
            }
            tulpa_pwm_duty(p, f);
            return NIL;
        }
        if (IS("pulse") || IS("pulse_width")) { tulpa_pwm_pulse(p, pwm_seconds(ARG(0, "pulse"), 1e-6, "pulse")); return NIL; }
        if (IS("enable") || IS("on")) { tulpa_pwm_enable(p, 1); return NIL; }
        if (IS("disable") || IS("off")) { tulpa_pwm_enable(p, 0); return NIL; }
        if (IS("state")) {
            Dict *d = dict_new();
            push(mk_obj(&d->h));
            dict_set_c(d, "period_ns", mk_int(p->period_ns));
            dict_set_c(d, "duty_ns", mk_int(p->duty_ns));
            dict_set_c(d, "enabled", mk_bool(p->enabled));
            R.sp--;
            return mk_obj(&d->h);
        }
        fail("PWM has no method '%s'", name);
    }
    case N_SERVO: {
        tulpa_servo_t *s = &n->d.servo;
        if (IS("angle") || IS("write") || IS("move_to")) {
            if (!argc) {
                if (!s->attached) return NIL;
                int u = unit_find("deg", 3);
                return u >= 0 ? mk_qty(s->angle_deg * R.units[u].scale, R.units[u].dim, u) : mk_float(s->angle_deg);
            }
            double rad;
            if (argv[0].t == T_QTY) {
                if (!radians_of(argv[0], &rad)) {
                    char shown[48];
                    show_buf(argv[0], shown, sizeof shown);
                    fail("Servo.angle expects an angle, got %s", shown);
                }
            } else rad = RAD(num("angle", argv[0]));
            tulpa_servo_angle(s, rad);
            return NIL;
        }
        if (IS("center")) { tulpa_servo_center(s); return NIL; }
        if (IS("detach")) { tulpa_servo_detach(s); return NIL; }
        if (IS("pulse_us")) return mk_float(tulpa_servo_pulse_us(s));
        fail("Servo has no method '%s'", name);
    }
    case N_PID: {
        double *p = n->d.pid; /* kp ki kd integral prev_err has_prev lo hi has_limits */
        if (IS("update") || IS("step")) {
            arity("PID.update", argc, 3, 3);
            double sp = num("update", argv[0]), pv = num("update", argv[1]), dt = num("update", argv[2]);
            if (dt <= 0) fail("PID.update: dt must be > 0");
            double err = sp - pv, deriv = p[5] ? (err - p[4]) / dt : 0, cand = p[3] + err * dt;
            p[4] = err;
            p[5] = 1;
            double out = p[0] * err + p[1] * cand + p[2] * deriv;
            if (p[8] && (out > p[7] || out < p[6])) out = out > p[7] ? p[7] : p[6]; /* saturated: don't integrate */
            else p[3] = cand;
            return mk_float(out);
        }
        if (IS("set_limits")) {
            arity("PID.set_limits", argc, 2, 2);
            double lo = num("set_limits", argv[0]), hi = num("set_limits", argv[1]);
            if (lo > hi) fail("PID.set_limits: low > high");
            p[6] = lo; p[7] = hi; p[8] = 1;
            return NIL;
        }
        if (IS("set_gains")) {
            arity("PID.set_gains", argc, 3, 3);
            for (int i = 0; i < 3; i++) p[i] = num("set_gains", argv[i]);
            return NIL;
        }
        if (IS("reset")) { p[3] = 0; p[5] = 0; return NIL; }
        if (IS("integral")) return mk_float(p[3]);
        fail("PID has no method '%s'", name);
    }
    default: return link_method(o, name, argc, argv);
    }
}

/* ---------------------------------------------------------------- sim:: */

static void sim_only(const char *name, bool hook) {
    if (!hook) fail("sim::%s is only available when the program is built for the simulator (--target sim)", name);
}
B(sim_virtual_time) { (void)argc; (void)argv; return NIL; } /* the sim HAL clock is always virtual */
B(sim_advance) {
    arity("sim::advance", argc, 1, 1);
    double s;
    if (!seconds_of(argv[0], &s)) fail("sim::advance expects a duration, e.g. 250[ms]");
    sleep_secs(s);
    return NIL;
}
B(sim_backend) { (void)argc; (void)argv; return str_c(tulpa_sim_gpio_level ? "sim" : "hardware"); }
B(sim_gpio_input) {
    arity("sim::gpio_input", argc, 2, 2);
    sim_only("gpio_input", tulpa_sim_gpio_input != NULL);
    tulpa_sim_gpio_input(intarg("gpio_input", argv[0]), truthy(argv[1]));
    react();
    return NIL;
}
B(sim_gpio_level) {
    arity("sim::gpio_level", argc, 1, 1);
    sim_only("gpio_level", tulpa_sim_gpio_level != NULL);
    return mk_int(tulpa_sim_gpio_level(intarg("gpio_level", argv[0])));
}
B(sim_i2c_set) {
    arity("sim::i2c_set", argc, 3, 4);
    sim_only("i2c_set", tulpa_sim_i2c_set != NULL);
    int64_t n = fill_tx(argv[2], 0);
    tulpa_sim_i2c_set(argc > 3 ? intarg("i2c_set", argv[3]) : 1, intarg("i2c_set", argv[0]), intarg("i2c_set", argv[1]), TX, n);
    react();
    return NIL;
}
B(sim_i2c_get) {
    arity("sim::i2c_get", argc, 2, 4);
    sim_only("i2c_get", tulpa_sim_i2c_get != NULL);
    int64_t n = argc > 2 ? count_arg("i2c_get", argv[2]) : 1;
    tulpa_sim_i2c_get(argc > 3 ? intarg("i2c_get", argv[3]) : 1, intarg("i2c_get", argv[0]), intarg("i2c_get", argv[1]), RX, n);
    return rx_list(n);
}
B(sim_spi_respond) {
    arity("sim::spi_respond", argc, 3, 3);
    sim_only("spi_respond", tulpa_sim_spi_respond != NULL);
    int64_t n = fill_tx(argv[2], 0);
    tulpa_sim_spi_respond(intarg("spi_respond", argv[0]), intarg("spi_respond", argv[1]), TX, n);
    return NIL;
}
B(sim_uart_feed) {
    arity("sim::uart_feed", argc, 2, 2);
    sim_only("uart_feed", tulpa_sim_uart_feed != NULL);
    size_t n;
    uint8_t *b = to_bytes(argv[1], &n);
    tulpa_sim_uart_feed(b, n);
    free(b);
    react();
    return NIL;
}

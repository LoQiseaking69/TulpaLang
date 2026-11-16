/* ---------------------------------------------------------------- builtin table */

static const struct { const char *name; TV (*fn)(int argc, TV *argv); } BUILTINS[] = {
    {"print", b_print}, {"println", b_print}, {"input", b_input}, {"len", b_len}, {"str", b_str}, {"repr", b_repr},
    {"int", b_int}, {"float", b_float}, {"bool", b_bool}, {"implements", b_implements}, {"type", b_type},
    {"list", b_list}, {"dict", b_dict}, {"range", b_range}, {"abs", b_abs}, {"min", b_min}, {"max", b_max},
    {"sum", b_sum}, {"round", b_round}, {"floor", b_floor}, {"ceil", b_ceil}, {"sqrt", b_sqrt}, {"sin", b_sin},
    {"cos", b_cos}, {"tan", b_tan}, {"asin", b_asin}, {"acos", b_acos}, {"atan", b_atan}, {"exp", b_exp},
    {"log", b_log}, {"log10", b_log10}, {"atan2", b_atan2}, {"hypot", b_hypot}, {"pow", b_pow},
    {"radians", b_radians}, {"degrees", b_degrees}, {"clamp", b_clamp}, {"lerp", b_lerp}, {"map", b_map},
    {"filter", b_filter}, {"reduce", b_reduce}, {"sorted", b_sorted}, {"reversed", b_reversed},
    {"enumerate", b_enumerate}, {"zip", b_zip}, {"any", b_any}, {"all", b_all}, {"keys", b_keys},
    {"values", b_values}, {"items", b_items}, {"sleep", b_sleep}, {"sleep_ms", b_sleep_ms},
    {"quantity", b_quantity}, {"signal", b_signal}, {"derive", b_derive}, {"check", b_check},
    {"millis", b_millis}, {"micros", b_micros}, {"now", b_now}, {"randint", b_randint}, {"random", b_random},
    {"choice", b_choice}, {"read_file", b_read_file}, {"write_file", b_write_file},
    {"append_file", b_append_file}, {"format", b_format}, {"hex", b_hex}, {"bin", b_bin}, {"ord", b_ord},
    {"chr", b_chr}, {"bytes", b_bytes}, {"decode", b_decode}, {"int_from_bytes", b_int_from_bytes},
    {"int_to_bytes", b_int_to_bytes}, {"error", b_error}, {"exit", b_exit}, {"join_all", b_join_all},
    {"await_all", b_await_all},
    /* names below this line are reached as module::name only */
    {"kin::velocity", b_kin_velocity}, {"kin::position", b_kin_position}, {"kin::acceleration", b_kin_acceleration},
    {"kin::time_of_flight", b_kin_time_of_flight}, {"kin::range", b_kin_range}, {"kin::max_height", b_kin_max_height},
    {"kin::fk2", b_kin_fk2}, {"kin::ik2", b_kin_ik2},
    {"GPIO::new", b_gpio_new}, {"I2C::new", b_i2c_new}, {"I2C::scan", b_i2c_scan}, {"SPI::new", b_spi_new},
    {"UART::new", b_uart_new}, {"PWM::new", b_pwm_new}, {"Servo::new", b_servo_new}, {"PID::new", b_pid_new},
    {"sim::virtual_time", b_sim_virtual_time}, {"sim::advance", b_sim_advance}, {"sim::backend", b_sim_backend},
    {"sim::gpio_input", b_sim_gpio_input}, {"sim::gpio_level", b_sim_gpio_level}, {"sim::i2c_set", b_sim_i2c_set},
    {"sim::i2c_get", b_sim_i2c_get}, {"sim::spi_respond", b_sim_spi_respond}, {"sim::uart_feed", b_sim_uart_feed},
    {"pack::pack", b_pack_pack}, {"pack::unpack", b_pack_unpack}, {"pack::calcsize", b_pack_calcsize},
    {"pack::f32s", b_pack_f32s}, {"pack::to_f32s", b_pack_to_f32s}, {"pack::crc16", b_pack_crc16},
    {"pack::crc32", b_pack_crc32}, {"pack::cobs_encode", b_pack_cobs_encode}, {"pack::cobs_decode", b_pack_cobs_decode},
    {"json::encode", b_json_encode}, {"json::decode", b_json_decode},
    {"tlm::enabled", b_tlm_enabled}, {"tlm::plot", b_tlm_off}, {"tlm::event", b_tlm_off}, {"tlm::status", b_tlm_off},
    {"tlm::artifact", b_tlm_off},
    {"tlink::node", b_tl_node}, {"tlink::advertise", b_tl_advertise}, {"tlink::subscribe", b_tl_subscribe},
    {"tlink::publish", b_tl_publish}, {"tlink::serve", b_tl_serve}, {"tlink::request", b_tl_request},
    {"tlink::reply", b_tl_reply}, {"tlink::poll", b_tl_poll}, {"tlink::info", b_tl_info}, {"tlink::close", b_tl_close},
    {"tlink::sim", b_tl_sim}, {"tlink::partition", b_tl_partition},
};
#define NBUILTINS ((int)(sizeof BUILTINS / sizeof BUILTINS[0]))

static const char *builtin_name(int ix) { return BUILTINS[ix].name; }
static TV builtin_call(int ix, int argc, TV *argv) { return BUILTINS[ix].fn(argc, argv); }
static int builtin_find(const char *name) {
    for (int i = 0; i < NBUILTINS; i++) if (!strcmp(BUILTINS[i].name, name)) return i;
    return -1;
}
/* A bare name: a builtin index, a constant code (-2..-6), or -1. */
static int global_find(const char *name) {
    if (strchr(name, ':')) return -1;
    static const char *const CONSTS[] = {"PI", "TAU", "E", "HIGH", "LOW"};
    for (int i = 0; i < 5; i++) if (!strcmp(CONSTS[i], name)) return -2 - i;
    return builtin_find(name);
}

static bool path_value(const char *m, const char *name, TV *out) {
    static const struct { const char *from, *to; } ALIAS[] = {
        {"kinematics", "kin"}, {"gpio", "GPIO"}, {"i2c", "I2C"}, {"spi", "SPI"}, {"uart", "UART"}, {"serial", "UART"},
        {"pwm", "PWM"}, {"servo", "Servo"}, {"control", "PID"}};
    for (size_t i = 0; i < sizeof ALIAS / sizeof ALIAS[0]; i++) if (!strcmp(m, ALIAS[i].from)) m = ALIAS[i].to;
    if (!strcmp(m, "math")) {
        if (!strcmp(name, "pi")) { *out = mk_float(3.14159265358979323846); return true; }
        if (!strcmp(name, "tau")) { *out = mk_float(6.28318530717958647692); return true; }
        if (!strcmp(name, "e")) { *out = mk_float(2.71828182845904523536); return true; }
        if (!strcmp(name, "inf")) { *out = mk_float(INFINITY); return true; }
        if (!strcmp(name, "nan")) { *out = mk_float(NAN); return true; }
    }
    char full[64];
    if (!strcmp(m, "math") || !strcmp(m, "std") || !strcmp(m, "io") || !strcmp(m, "time")) snprintf(full, sizeof full, "%s", name);
    else {
        if ((!strcmp(name, "open") || !strcmp(name, "pid")) && strcmp(m, "sim") && strcmp(m, "tlink") && strcmp(m, "kin") && strcmp(m, "pack") && strcmp(m, "json") && strcmp(m, "tlm")) name = "new";
        snprintf(full, sizeof full, "%s::%s", m, name);
    }
    int ix = strchr(full, ':') ? builtin_find(full) : global_find(full);
    if (ix < 0) return false;
    out->t = T_BUILTIN;
    out->u.i = ix;
    return true;
}

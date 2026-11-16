/* ---------------------------------------------------------------- builtin helpers */

static void arity(const char *name, int argc, int min, int max) {
    if (argc >= min && argc <= max) return;
    if (min == max) fail("%s() takes %d argument(s), got %d", name, min, argc);
    fail("%s() takes %d..%d argument(s), got %d", name, min, max, argc);
}
static double num(const char *name, TV v) {
    double d;
    if (!as_f64(v, &d)) fail("%s() expected a number, got %s", name, type_name(v));
    return d;
}
static int64_t intarg(const char *name, TV v) {
    int64_t i;
    if (!as_i64(v, &i)) fail("%s() expected an integer, got %s", name, type_name(v));
    return i;
}
static const char *sarg(const char *name, TV v) {
    if (v.t != T_STR) fail("%s() expected a string, got %s", name, type_name(v));
    return STR(v)->s;
}
/* bytes of a list or string, into a malloc'd buffer the caller frees */
static uint8_t *to_bytes(TV v, size_t *n) {
    if (v.t == T_STR) {
        uint8_t *b = xmalloc(STR(v)->len + 1);
        memcpy(b, STR(v)->s, STR(v)->len);
        *n = STR(v)->len;
        return b;
    }
    if (v.t == T_INT) {
        if (v.u.i < 0 || v.u.i > 255) fail("byte value %lld out of range 0..255", (long long)v.u.i);
        uint8_t *b = xmalloc(1);
        b[0] = (uint8_t)v.u.i;
        *n = 1;
        return b;
    }
    if (v.t != T_LIST) fail("expected bytes (list or str), got %s", type_name(v));
    for (uint32_t i = 0; i < LIST(v)->n; i++) {
        TV x = LIST(v)->v[i];
        int64_t k;
        if (!as_i64(x, &k)) fail("expected byte, got %s", type_name(x));
        if (k < 0 || k > 255) fail("byte value %lld out of range 0..255", (long long)k);
    }
    uint8_t *b = xmalloc(LIST(v)->n + 1);
    for (uint32_t i = 0; i < LIST(v)->n; i++) { int64_t k = 0; as_i64(LIST(v)->v[i], &k); b[i] = (uint8_t)k; }
    *n = LIST(v)->n;
    return b;
}
static TV from_bytes(const uint8_t *b, size_t n) {
    List *l = list_new((uint32_t)n);
    for (size_t i = 0; i < n; i++) l->v[i] = mk_int(b[i]);
    l->n = (uint32_t)n;
    return mk_obj(&l->h);
}
static uint64_t rand_u64(void) {
    uint64_t x = R.rng;
    x ^= x >> 12;
    x ^= x << 25;
    x ^= x >> 27;
    R.rng = x;
    return x * 0x2545F4914F6CDD1DULL;
}
static void no_host(const char *name) { fail("%s() needs a host computer; it is not available in firmware", name); }

/* Sorts list items (insertion sort: stable, and safe when the key function fails part-way). */
static TV sort_values(TV items, TV *key) {
    List *src = LIST(items);
    int base = R.sp;
    List *volatile keys = src;
    if (key) {
        List *k = list_new(src->n);
        push(mk_obj(&k->h));
        for (uint32_t i = 0; i < src->n; i++) list_push(k, call1(*key, src->v[i]));
        keys = k;
    }
    uint32_t n = src->n;
    uint32_t *idx = xmalloc((n + 1) * sizeof(uint32_t));
    for (uint32_t i = 0; i < n; i++) idx[i] = i;
    Try t = {.sp = R.sp, .depth = R.depth, .line = R.line, .reacting = R.reacting};
    if (R.ntry >= 32) { free(idx); fail("try blocks nested too deeply"); }
    R.tries[R.ntry++] = &t;
    if (setjmp(t.jb)) {
        R.ntry--;
        free(idx);
        char msg[240];
        snprintf(msg, sizeof msg, "%s", R.errmsg);
        fail("%s", msg);
    }
    /* merge sort keeps large lists fast */
    uint32_t *tmp = xmalloc((n + 1) * sizeof(uint32_t));
    for (uint32_t w = 1; w < n; w *= 2) {
        for (uint32_t lo = 0; lo < n; lo += 2 * w) {
            uint32_t mid = lo + w < n ? lo + w : n, hi = lo + 2 * w < n ? lo + 2 * w : n;
            uint32_t i = lo, j = mid, k = lo;
            while (i < mid && j < hi) tmp[k++] = compare(keys->v[idx[j]], keys->v[idx[i]]) < 0 ? idx[j++] : idx[i++];
            while (i < mid) tmp[k++] = idx[i++];
            while (j < hi) tmp[k++] = idx[j++];
        }
        memcpy(idx, tmp, n * sizeof(uint32_t));
    }
    free(tmp);
    R.ntry--;
    List *out = list_new(n);
    for (uint32_t i = 0; i < n; i++) out->v[i] = src->v[idx[i]];
    out->n = n;
    free(idx);
    R.sp = base;
    return mk_obj(&out->h);
}

/* ---------------------------------------------------------------- formatting */

static void pad_to(SB *out, const char *body, size_t width, char fill, char align) {
    size_t len = u8_count(body, strlen(body)), pad = width > len ? width - len : 0;
    size_t left = align == '>' ? pad : align == '^' ? pad / 2 : 0;
    for (size_t i = 0; i < left; i++) sb_putc(out, fill);
    sb_puts(out, body);
    for (size_t i = 0; i < pad - left; i++) sb_putc(out, fill);
}

static void radix_str(int64_t v, int radix, bool upper, const char *prefix, SB *b) {
    /* like Rust: negative numbers print as two's complement */
    uint64_t u = (uint64_t)v;
    char t[72];
    int n = 0;
    do {
        int d = (int)(u % (unsigned)radix);
        t[n++] = (char)(d < 10 ? '0' + d : (upper ? 'A' : 'a') + d - 10);
        u /= (unsigned)radix;
    } while (u);
    sb_puts(b, prefix);
    while (n) sb_putc(b, t[--n]);
}

static void format_spec(TV v, const char *spec, SB *out) {
    if (!*spec) { to_sb(v, out, false); return; }
    if (!strcmp(spec, "?")) { to_sb(v, out, true); return; }
    if (v.t == T_QTY) {
        Qty *q = QTY(v);
        format_spec(mk_float(q->unit >= 0 ? q->si / R.units[q->unit].scale : q->si), spec, out);
        sb_putc(out, ' ');
        if (q->unit >= 0) sb_puts(out, SYM(R.units[q->unit].canon)); else dim_name(q->dim, out);
        return;
    }
    const char *p = spec;
    char fill = ' ', align = 0;
    if (p[0] && (p[1] == '<' || p[1] == '>' || p[1] == '^')) { fill = p[0]; align = p[1]; p += 2; }
    else if (*p == '<' || *p == '>' || *p == '^') align = *p++;
    bool plus = false, alt = false, zero = false, has_prec = false;
    if (*p == '+') { plus = true; p++; }
    if (*p == '#') { alt = true; p++; }
    if (*p == '0') { zero = true; p++; }
    size_t width = 0, prec = 0;
    while (*p >= '0' && *p <= '9') width = width * 10 + (size_t)(*p++ - '0');
    if (*p == '.') {
        p++;
        if (!(*p >= '0' && *p <= '9')) fail("bad precision in format spec '%s'", spec);
        has_prec = true;
        while (*p >= '0' && *p <= '9') prec = prec * 10 + (size_t)(*p++ - '0');
    }
    const char *ty = p;
    bool numeric = is_num(v);
    double f = 0;
    int64_t iv = 0;
    SB body = {0};
    sb_puts(&body, "");
#define NEED_INT() do { if (!as_i64(v, &iv)) { free(body.p); fail("format type '%s' needs an integer, got %s", ty, type_name(v)); } } while (0)
#define NEED_NUM() do { if (!as_f64(v, &f)) { free(body.p); fail("format() expected a number, got %s", type_name(v)); } } while (0)
    if (!strcmp(ty, "x")) { NEED_INT(); radix_str(iv, 16, false, alt ? "0x" : "", &body); }
    else if (!strcmp(ty, "X")) { NEED_INT(); radix_str(iv, 16, true, alt ? "0x" : "", &body); }
    else if (!strcmp(ty, "b")) { NEED_INT(); radix_str(iv, 2, false, alt ? "0b" : "", &body); }
    else if (!strcmp(ty, "o")) { NEED_INT(); radix_str(iv, 8, false, alt ? "0o" : "", &body); }
    else if (!strcmp(ty, "d")) { NEED_INT(); fmt_i64(iv, &body); }
    else if (!strcmp(ty, "e")) {
        NEED_NUM();
        char t[64];
        snprintf(t, sizeof t, "%.*e", (int)(has_prec ? prec : 6), f);
        /* Rust writes 1.5e3 where C writes 1.5e+03 */
        char *ex = strchr(t, 'e');
        if (ex) {
            int en = atoi(ex + 1);
            *ex = 0;
            sb_printf(&body, "%se%d", t, en);
        } else sb_puts(&body, t);
    } else if (!strcmp(ty, "%")) { NEED_NUM(); sb_printf(&body, "%.*f%%", (int)(has_prec ? prec : 6), f * 100.0); }
    else if (!strcmp(ty, "f")) { NEED_NUM(); sb_printf(&body, "%.*f", (int)(has_prec ? prec : 6), f); }
    else if (!strcmp(ty, "s") || !*ty) {
        if (has_prec && numeric) { as_f64(v, &f); sb_printf(&body, "%.*f", (int)prec, f); }
        else {
            to_sb(v, &body, false);
            if (has_prec) {
                size_t i = 0, chars = 0;
                while (i < body.n && chars < prec) { i++; while (i < body.n && ((unsigned char)body.p[i] & 0xC0) == 0x80) i++; chars++; }
                body.n = i;
                body.p[i] = 0;
            }
        }
    } else if (!strcmp(ty, "?")) to_sb(v, &body, true);
    else { free(body.p); fail("unknown format type '%s'", ty); }
#undef NEED_INT
#undef NEED_NUM
    if (plus && numeric && as_f64(v, &f) && f >= 0) {
        SB s = {0};
        sb_putc(&s, '+');
        sb_puts(&s, body.p);
        free(body.p);
        body = s;
    }
    size_t len = u8_count(body.p, body.n);
    if (width > len) {
        if (zero && !align) {
            const char *rest = body.p;
            if (*rest == '-' || *rest == '+') sb_putc(out, *rest++);
            if (alt && strlen(rest) > 2 && rest[0] == '0') { sb_put(out, rest, 2); rest += 2; }
            for (size_t i = 0; i < width - len; i++) sb_putc(out, '0');
            sb_puts(out, rest);
        } else pad_to(out, body.p, width, fill, align ? align : numeric ? '>' : '<');
    } else sb_puts(out, body.p);
    free(body.p);
}

/* Rust-style format strings: {}, {:?}, {:.2}, {:#04x}, {0}, {{ and }}. */
static TV rust_format(const char *fmt, int argc, TV *argv) {
    SB out = {0};
    int next = 0;
    for (const char *p = fmt; *p;) {
        if (p[0] == '{' && p[1] == '{') { sb_putc(&out, '{'); p += 2; continue; }
        if (p[0] == '}' && p[1] == '}') { sb_putc(&out, '}'); p += 2; continue; }
        if (*p != '{') { sb_putc(&out, *p++); continue; }
        const char *end = strchr(p, '}');
        if (!end) { free(out.p); fail("unclosed '{' in format string"); }
        char inner[48];
        size_t n = (size_t)(end - p - 1);
        if (n >= sizeof inner) n = sizeof inner - 1;
        memcpy(inner, p + 1, n);
        inner[n] = 0;
        char *spec = strchr(inner, ':');
        if (spec) *spec++ = 0; else spec = inner + n;
        int idx;
        if (!*inner) idx = next++;
        else {
            if (strspn(inner, "0123456789") != strlen(inner)) { free(out.p); fail("named format arguments are not supported: {%s}", inner); }
            idx = atoi(inner);
        }
        if (idx >= argc) { free(out.p); fail("format string refers to argument %d, but only %d given", idx, argc); }
        format_spec(argv[idx], spec, &out);
        p = end + 1;
    }
    return sb_done(&out);
}

/* ---------------------------------------------------------------- global builtins */

#define B(name) static TV b_##name(int argc, TV *argv)
#define ITEMS(v) (push(iterate(v, 1)), LIST(R.stack[R.sp - 1]))

B(print) {
    SB b = {0};
    for (int i = 0; i < argc; i++) { if (i) sb_putc(&b, ' '); stringify(argv[i], &b); }
    sb_putc(&b, '\n');
    tulpa_print_str(b.p);
    free(b.p);
    return NIL;
}
B(input) { (void)argc; (void)argv; no_host("input"); return NIL; }
B(len) {
    arity("len", argc, 1, 1);
    TV v = argv[0];
    switch (v.t) {
    case T_STR: return mk_int((int64_t)u8_count(STR(v)->s, STR(v)->len));
    case T_LIST: return mk_int(LIST(v)->n);
    case T_DICT: return mk_int(DICT(v)->n);
    case T_RANGE: { Range *r = (Range *)v.u.o; int64_t n = r->incl ? r->b - r->a + 1 : r->b - r->a; return mk_int(n < 0 ? 0 : n); }
    default: fail("len() of %s", type_name(v));
    }
}
B(str) { arity("str", argc, 0, 1); return argc ? to_str(argv[0]) : str_c(""); }
B(repr) { arity("repr", argc, 1, 1); SB b = {0}; to_sb(argv[0], &b, true); return sb_done(&b); }
B(int) {
    arity("int", argc, 1, 2);
    TV v = argv[0];
    if (v.t == T_INT) return v;
    if (v.t == T_FLOAT) return mk_int((int64_t)trunc(v.u.f));
    if (v.t == T_BOOL) return mk_int(v.u.i);
    if (v.t != T_STR) fail("int() of %s", type_name(v));
    int radix = argc > 1 ? (int)intarg("int", argv[1]) : 10;
    char buf[80];
    size_t n = 0;
    const char *s = STR(v)->s;
    while (*s == ' ' || *s == '\t' || *s == '\n' || *s == '\r') s++;
    for (; *s && n < sizeof buf - 1; s++) if (*s != '_') buf[n++] = *s;
    while (n && (buf[n - 1] == ' ' || buf[n - 1] == '\n' || buf[n - 1] == '\t' || buf[n - 1] == '\r')) n--;
    buf[n] = 0;
    const char *t = buf;
    bool neg = *t == '-';
    if (neg) t++;
    if (radix == 10) {
        if (t[0] == '0' && (t[1] == 'x' || t[1] == 'X')) { radix = 16; t += 2; }
        else if (t[0] == '0' && t[1] == 'b') { radix = 2; t += 2; }
    }
    char *end;
    unsigned long long u = strtoull(t, &end, radix);
    if (!*t || *end || *t == '-' || *t == '+' || radix < 2 || radix > 36) {
        char shown[80];
        repr_buf(v, shown, sizeof shown);
        fail("int(): invalid literal %s", shown);
    }
    return mk_int(neg ? -(int64_t)u : (int64_t)u);
}
B(float) {
    arity("float", argc, 1, 1);
    TV v = argv[0];
    if (v.t == T_STR) {
        char *end;
        const char *s = STR(v)->s;
        while (*s == ' ') s++;
        double d = strtod(s, &end);
        while (*end == ' ' || *end == '\n') end++;
        if (end == s || *end) {
            char shown[80];
            repr_buf(v, shown, sizeof shown);
            fail("float(): invalid literal %s", shown);
        }
        return mk_float(d);
    }
    if (v.t == T_QTY) {
        SB b = {0};
        if (QTY(v)->unit >= 0) sb_puts(&b, SYM(R.units[QTY(v)->unit].canon)); else dim_name(QTY(v)->dim, &b);
        char unit[40];
        snprintf(unit, sizeof unit, "%s", b.p);
        free(b.p);
        fail("float() of a quantity is ambiguous; use .in(\"%s\") or .si", unit);
    }
    return mk_float(num("float", v));
}
B(bool) { arity("bool", argc, 1, 1); return mk_bool(truthy(argv[0])); }
B(implements) {
    arity("implements", argc, 2, 2);
    if (argv[1].t != T_STR) fail("implements() expects an interface name");
    int want = find_sym(STR(argv[1])->s);
    if (want < 0 || !is_iface(want)) fail("implements(): unknown interface '%s'", STR(argv[1])->s);
    return mk_bool(argv[0].t == T_OBJ && satisfies(KID(((Inst *)argv[0].u.o)->cls->def, 3), want));
}
B(type) { arity("type", argc, 1, 1); return str_c(type_name(argv[0])); }
B(list) { arity("list", argc, 0, 1); return argc ? iterate(argv[0], 1) : mk_obj(&list_new(0)->h); }
B(dict) {
    arity("dict", argc, 0, 1);
    Dict *d = dict_new();
    if (!argc) return mk_obj(&d->h);
    push(mk_obj(&d->h));
    if (argv[0].t == T_DICT) {
        for (uint32_t i = 0; i < DICT(argv[0])->n; i++) dict_set(d, DICT(argv[0])->k[i], DICT(argv[0])->v[i]);
    } else {
        List *l = ITEMS(argv[0]);
        for (uint32_t i = 0; i < l->n; i++) {
            if (l->v[i].t != T_LIST || LIST(l->v[i])->n != 2) fail("dict() expects a list of [key, value] pairs");
            dict_set(d, LIST(l->v[i])->v[0], LIST(l->v[i])->v[1]);
        }
        R.sp--;
    }
    R.sp--;
    return mk_obj(&d->h);
}
B(range) {
    arity("range", argc, 1, 3);
    int64_t x = intarg("range", argv[0]), y = argc > 1 ? intarg("range", argv[1]) : 0;
    if (argc < 3) {
        Range *r = (Range *)alloc(T_RANGE, sizeof(Range));
        r->a = argc == 1 ? 0 : x;
        r->b = argc == 1 ? x : y;
        return mk_obj(&r->h);
    }
    int64_t step = intarg("range", argv[2]);
    if (!step) fail("range() step must not be zero");
    List *l = list_new(0);
    for (int64_t k = x; (step > 0 && k < y) || (step < 0 && k > y); k += step) list_push(l, mk_int(k));
    return mk_obj(&l->h);
}
B(abs) {
    arity("abs", argc, 1, 1);
    if (argv[0].t == T_INT) return mk_int(argv[0].u.i < 0 ? -argv[0].u.i : argv[0].u.i);
    if (argv[0].t == T_QTY) return mk_qty(fabs(QTY(argv[0])->si), QTY(argv[0])->dim, QTY(argv[0])->unit);
    return mk_float(fabs(num("abs", argv[0])));
}
static TV extreme(const char *name, int argc, TV *argv, bool want_max) {
    int base = R.sp;
    TV *vals = argv;
    int n = argc;
    if (argc == 1) { List *l = ITEMS(argv[0]); vals = l->v; n = (int)l->n; }
    if (!n) fail("%s() of empty sequence", name);
    TV best = vals[0];
    for (int i = 1; i < n; i++) {
        int c = compare(vals[i], best);
        if (want_max ? c > 0 : c < 0) best = vals[i];
    }
    R.sp = base;
    return best;
}
B(min) { return extreme("min", argc, argv, false); }
B(max) { return extreme("max", argc, argv, true); }
B(sum) {
    arity("sum", argc, 1, 2);
    int base = R.sp;
    List *l = ITEMS(argv[0]);
    TV acc = argc > 1 ? argv[1] : mk_int(0);
    push(acc);
    for (uint32_t i = 0; i < l->n; i++) { acc = binary(acc, OP_ADD, l->v[i]); R.stack[base + 1] = acc; }
    R.sp = base;
    return acc;
}
static TV int_if_whole(double f) {
    if (f == floor(f) && fabs(f) < 9.0e15) return mk_int((int64_t)f);
    return mk_float(f);
}
B(round) {
    arity("round", argc, 1, 2);
    if (argv[0].t == T_QTY) return quantity_method(argv[0], "round", argc - 1, argv + 1);
    double x = num("round", argv[0]);
    if (argc == 1) return mk_int((int64_t)round(x));
    double p = pow(10, (double)intarg("round", argv[1]));
    return mk_float(round(x * p) / p);
}
B(floor) { arity("floor", argc, 1, 1); return int_if_whole(floor(num("floor", argv[0]))); }
B(ceil) { arity("ceil", argc, 1, 1); return int_if_whole(ceil(num("ceil", argv[0]))); }
B(sqrt) {
    arity("sqrt", argc, 1, 1);
    if (argv[0].t == T_QTY) {
        Qty *q = QTY(argv[0]);
        int8_t d[8];
        for (int i = 0; i < 8; i++) {
            if (q->dim[i] % 2) fail("sqrt() of %s has no unit", describe_dim(q->dim));
            d[i] = (int8_t)(q->dim[i] / 2);
        }
        return mk_qty(sqrt(q->si), d, -1);
    }
    return mk_float(sqrt(num("sqrt", argv[0])));
}
static TV trig(const char *name, int argc, TV *argv, double (*f)(double)) {
    arity(name, argc, 1, 1);
    double r;
    if (!radians_of(argv[0], &r)) {
        char shown[64];
        show_buf(argv[0], shown, sizeof shown);
        fail("%s() expects an angle (radians or e.g. 30[deg]), got %s", name, shown);
    }
    return mk_float(f(r));
}
B(sin) { return trig("sin", argc, argv, sin); }
B(cos) { return trig("cos", argc, argv, cos); }
B(tan) { return trig("tan", argc, argv, tan); }
#define M1(name, expr) B(name) { arity(#name, argc, 1, 1); double x = num(#name, argv[0]); return mk_float(expr); }
M1(asin, asin(x)) M1(acos, acos(x)) M1(atan, atan(x)) M1(exp, exp(x)) M1(log10, log10(x))
M1(radians, x * (3.14159265358979323846 / 180.0)) M1(degrees, x * (180.0 / 3.14159265358979323846))
B(log) {
    arity("log", argc, 1, 2);
    double x = num("log", argv[0]);
    return mk_float(argc > 1 ? log(x) / log(num("log", argv[1])) : log(x));
}
B(atan2) { arity("atan2", argc, 2, 2); return mk_float(atan2(num("atan2", argv[0]), num("atan2", argv[1]))); }
B(hypot) { arity("hypot", argc, 2, 2); return mk_float(hypot(num("hypot", argv[0]), num("hypot", argv[1]))); }
B(pow) { arity("pow", argc, 2, 2); return binary(argv[0], OP_POW, argv[1]); }
B(clamp) {
    arity("clamp", argc, 3, 3);
    if (compare(argv[0], argv[1]) < 0) return argv[1];
    if (compare(argv[0], argv[2]) > 0) return argv[2];
    return argv[0];
}
B(lerp) {
    arity("lerp", argc, 3, 3);
    double x = num("lerp", argv[0]), y = num("lerp", argv[1]), t = num("lerp", argv[2]);
    return mk_float(x + (y - x) * t);
}
static void fn_and_seq(const char *name, TV *argv, TV *f, TV *s) {
    if (callable(argv[0])) { *f = argv[0]; *s = argv[1]; }
    else if (callable(argv[1])) { *f = argv[1]; *s = argv[0]; }
    else fail("%s() expects a function and a sequence", name);
}
B(map) {
    arity("map", argc, 2, 2);
    TV f, s;
    int base = R.sp;
    fn_and_seq("map", argv, &f, &s);
    List *l = ITEMS(s), *out = list_new(l->n);
    push(mk_obj(&out->h));
    for (uint32_t i = 0; i < l->n; i++) list_push(out, call1(f, l->v[i]));
    R.sp = base;
    return mk_obj(&out->h);
}
B(filter) {
    arity("filter", argc, 2, 2);
    TV f, s;
    int base = R.sp;
    fn_and_seq("filter", argv, &f, &s);
    List *l = ITEMS(s), *out = list_new(0);
    push(mk_obj(&out->h));
    for (uint32_t i = 0; i < l->n; i++) if (truthy(call1(f, l->v[i]))) list_push(out, l->v[i]);
    R.sp = base;
    return mk_obj(&out->h);
}
B(reduce) {
    arity("reduce", argc, 2, 3);
    TV f, s;
    int base = R.sp;
    fn_and_seq("reduce", argv, &f, &s);
    List *l = ITEMS(s);
    uint32_t i = 0;
    TV acc;
    if (argc > 2) acc = argv[2];
    else {
        if (!l->n) fail("reduce() of empty sequence with no initial value");
        acc = l->v[i++];
    }
    push(acc);
    for (; i < l->n; i++) { acc = call2(f, acc, l->v[i]); R.stack[base + 1] = acc; }
    R.sp = base;
    return acc;
}
B(sorted) {
    arity("sorted", argc, 1, 2);
    int base = R.sp;
    push(iterate(argv[0], 1));
    TV r = sort_values(R.stack[base], argc > 1 ? &argv[1] : NULL);
    R.sp = base;
    return r;
}
B(reversed) {
    arity("reversed", argc, 1, 1);
    TV l = iterate(argv[0], 1);
    for (uint32_t i = 0, n = LIST(l)->n; i < n / 2; i++) { TV t = LIST(l)->v[i]; LIST(l)->v[i] = LIST(l)->v[n - 1 - i]; LIST(l)->v[n - 1 - i] = t; }
    return l;
}
B(enumerate) {
    arity("enumerate", argc, 1, 2);
    int base = R.sp;
    int64_t start = argc > 1 ? intarg("enumerate", argv[1]) : 0;
    List *l = ITEMS(argv[0]), *out = list_new(l->n);
    push(mk_obj(&out->h));
    for (uint32_t i = 0; i < l->n; i++) { TV pair[2] = {mk_int((int64_t)i + start), l->v[i]}; list_push(out, list_from(pair, 2)); }
    R.sp = base;
    return mk_obj(&out->h);
}
B(zip) {
    int base = R.sp;
    uint32_t n = argc ? UINT32_MAX : 0;
    for (int i = 0; i < argc; i++) { List *l = ITEMS(argv[i]); if (l->n < n) n = l->n; }
    List *out = list_new(n);
    push(mk_obj(&out->h));
    for (uint32_t k = 0; k < n; k++) {
        List *row = list_new((uint32_t)argc);
        for (int i = 0; i < argc; i++) row->v[i] = LIST(R.stack[base + i])->v[k];
        row->n = (uint32_t)argc;
        list_push(out, mk_obj(&row->h));
    }
    R.sp = base;
    return mk_obj(&out->h);
}
B(any) {
    arity("any", argc, 1, 1);
    List *l = ITEMS(argv[0]);
    bool r = false;
    for (uint32_t i = 0; i < l->n; i++) r |= truthy(l->v[i]);
    R.sp--;
    return mk_bool(r);
}
B(all) {
    arity("all", argc, 1, 1);
    List *l = ITEMS(argv[0]);
    bool r = true;
    for (uint32_t i = 0; i < l->n; i++) r &= truthy(l->v[i]);
    R.sp--;
    return mk_bool(r);
}
B(keys) { return builtin_method(argc ? argv[0] : NIL, "keys", 0, NULL); }
B(values) { return builtin_method(argc ? argv[0] : NIL, "values", 0, NULL); }
B(items) { return builtin_method(argc ? argv[0] : NIL, "items", 0, NULL); }
B(sleep) {
    arity("sleep", argc, 1, 1);
    double s;
    if (!seconds_of(argv[0], &s)) fail("sleep() expects seconds or a time quantity like 250[ms]");
    sleep_secs(s);
    return NIL;
}
B(sleep_ms) {
    arity("sleep_ms", argc, 1, 1);
    double s;
    if (argv[0].t == T_QTY) { if (!seconds_of(argv[0], &s)) fail("sleep_ms() expects milliseconds or a time quantity"); }
    else s = num("sleep_ms", argv[0]) / 1000.0;
    sleep_secs(s);
    return NIL;
}
B(quantity) {
    arity("quantity", argc, 2, 2);
    int u = unit_of(argv[1]);
    return mk_qty(num("quantity", argv[0]) * R.units[u].scale, R.units[u].dim, u);
}
B(signal) {
    arity("signal", argc, 0, 2);
    return signal_new(argc ? argv[0] : NIL, argc > 1 ? intarg("signal", argv[1]) : 256, NIL);
}
B(derive) {
    arity("derive", argc, 1, 1);
    if (!callable(argv[0])) fail("derive() expects a function, e.g. derive(|| a.get() * 2)");
    return signal_new(NIL, 1, argv[0]);
}
B(check) { (void)argc; (void)argv; fail("check() fuzzes contracts on the host (tulpa test); it is not available in firmware"); return NIL; }
B(millis) { (void)argc; (void)argv; return mk_int((int64_t)(tulpa_hal_micros() / 1000u)); }
B(micros) { (void)argc; (void)argv; return mk_int((int64_t)tulpa_hal_micros()); }
B(now) { (void)argc; (void)argv; return mk_float(now_secs()); }
B(randint) {
    arity("randint", argc, 2, 2);
    int64_t lo = intarg("randint", argv[0]), hi = intarg("randint", argv[1]);
    if (hi < lo) fail("randint(): empty range");
    return mk_int(lo + (int64_t)(rand_u64() % ((uint64_t)(hi - lo) + 1)));
}
B(random) { (void)argc; (void)argv; return mk_float((double)(rand_u64() >> 11) / 9007199254740992.0); }
B(choice) {
    arity("choice", argc, 1, 1);
    List *l = ITEMS(argv[0]);
    if (!l->n) fail("choice() from empty sequence");
    TV v = l->v[rand_u64() % l->n];
    R.sp--;
    return v;
}
B(read_file) { (void)argc; (void)argv; no_host("read_file"); return NIL; }
B(write_file) { (void)argc; (void)argv; no_host("write_file"); return NIL; }
B(append_file) { (void)argc; (void)argv; no_host("append_file"); return NIL; }
B(format) {
    arity("format", argc, 1, 2);
    SB b = {0};
    format_spec(argv[0], argc > 1 ? sarg("format", argv[1]) : "", &b);
    return sb_done(&b);
}
static TV prefixed(const char *name, TV v, int radix, const char *prefix) {
    int64_t n = intarg(name, v);
    SB b = {0};
    if (n < 0) { sb_putc(&b, '-'); n = -n; }
    radix_str(n, radix, false, prefix, &b);
    return sb_done(&b);
}
B(hex) { arity("hex", argc, 1, 1); return prefixed("hex", argv[0], 16, "0x"); }
B(bin) { arity("bin", argc, 1, 1); return prefixed("bin", argv[0], 2, "0b"); }
B(ord) {
    arity("ord", argc, 1, 1);
    const unsigned char *s = (const unsigned char *)sarg("ord", argv[0]);
    if (!*s) fail("ord() of empty string");
    if (s[0] < 0x80) return mk_int(s[0]);
    if (s[0] < 0xE0) return mk_int(((s[0] & 0x1F) << 6) | (s[1] & 0x3F));
    if (s[0] < 0xF0) return mk_int(((s[0] & 0x0F) << 12) | ((s[1] & 0x3F) << 6) | (s[2] & 0x3F));
    return mk_int(((s[0] & 0x07) << 18) | ((s[1] & 0x3F) << 12) | ((s[2] & 0x3F) << 6) | (s[3] & 0x3F));
}
B(chr) {
    arity("chr", argc, 1, 1);
    int64_t c = intarg("chr", argv[0]);
    char b[5] = {0};
    if (c < 0 || c > 0x10FFFF || (c >= 0xD800 && c <= 0xDFFF)) fail("chr() arg out of range");
    if (c < 0x80) b[0] = (char)c;
    else if (c < 0x800) { b[0] = (char)(0xC0 | (c >> 6)); b[1] = (char)(0x80 | (c & 0x3F)); }
    else if (c < 0x10000) { b[0] = (char)(0xE0 | (c >> 12)); b[1] = (char)(0x80 | ((c >> 6) & 0x3F)); b[2] = (char)(0x80 | (c & 0x3F)); }
    else { b[0] = (char)(0xF0 | (c >> 18)); b[1] = (char)(0x80 | ((c >> 12) & 0x3F)); b[2] = (char)(0x80 | ((c >> 6) & 0x3F)); b[3] = (char)(0x80 | (c & 0x3F)); }
    return str_n(b, c == 0 ? 1 : strlen(b));
}
B(bytes) {
    arity("bytes", argc, 1, 1);
    size_t n;
    uint8_t *b = to_bytes(argv[0], &n);
    TV r = from_bytes(b, n);
    free(b);
    return r;
}
B(decode) {
    arity("decode", argc, 1, 1);
    size_t n;
    uint8_t *b = to_bytes(argv[0], &n);
    TV r = str_n((const char *)b, n);
    free(b);
    return r;
}
B(int_from_bytes) {
    arity("int_from_bytes", argc, 1, 3);
    size_t n;
    uint8_t *b = to_bytes(argv[0], &n);
    if (!n || n > 8) { free(b); fail("int_from_bytes() needs 1..8 bytes"); }
    bool little = argc > 1 && argv[1].t == T_STR && !strcmp(STR(argv[1])->s, "little");
    bool is_signed = argc > 2 && truthy(argv[2]);
    uint64_t u = 0;
    for (size_t i = 0; i < n; i++) u = (u << 8) | b[little ? n - 1 - i : i];
    free(b);
    size_t bits = n * 8;
    if (is_signed && bits < 64 && ((u >> (bits - 1)) & 1)) return mk_int((int64_t)u - ((int64_t)1 << bits));
    return mk_int((int64_t)u);
}
B(int_to_bytes) {
    arity("int_to_bytes", argc, 2, 3);
    int64_t v = intarg("int_to_bytes", argv[0]), len = intarg("int_to_bytes", argv[1]);
    if (len < 1 || len > 8) fail("int_to_bytes() length must be 1..8");
    bool little = argc > 2 && argv[2].t == T_STR && !strcmp(STR(argv[2])->s, "little");
    uint8_t out[8];
    for (int64_t k = 0; k < len; k++) out[little ? k : len - 1 - k] = (uint8_t)((v >> (8 * k)) & 0xff);
    return from_bytes(out, (size_t)len);
}
B(error) {
    char msg[200] = "error";
    if (argc) show_buf(argv[0], msg, sizeof msg);
    fail("%s", msg);
    return NIL;
}
B(exit) {
    int64_t code = 0;
    if (argc) as_i64(argv[0], &code);
    exit((int)code);
    return NIL;
}
B(join_all) { (void)argc; (void)argv; drain_tasks(); return NIL; }
B(await_all) { arity("await_all", argc, 1, 1); return await_value(argv[0]); }

/* ---------------------------------------------------------------- kin:: */

#define KG 9.81
#define RAD(x) ((x) * (3.14159265358979323846 / 180.0))
#define DEG(x) ((x) * (180.0 / 3.14159265358979323846))
B(kin_velocity) { arity("kin::velocity", argc, 3, 3); return mk_float(num("velocity", argv[0]) + num("velocity", argv[1]) * num("velocity", argv[2])); }
B(kin_position) {
    arity("kin::position", argc, 4, 4);
    double x0 = num("position", argv[0]), v0 = num("position", argv[1]), a = num("position", argv[2]), t = num("position", argv[3]);
    return mk_float(x0 + v0 * t + 0.5 * a * t * t);
}
B(kin_acceleration) {
    arity("kin::acceleration", argc, 2, 2);
    double t = num("acceleration", argv[1]);
    if (t == 0) fail("kin::acceleration: time must be non-zero");
    return mk_float(num("acceleration", argv[0]) / t);
}
B(kin_time_of_flight) { arity("kin::time_of_flight", argc, 2, 2); return mk_float(2.0 * num("tof", argv[0]) * sin(RAD(num("tof", argv[1]))) / KG); }
B(kin_range) { arity("kin::range", argc, 2, 2); double v = num("range", argv[0]); return mk_float(v * v * sin(RAD(2.0 * num("range", argv[1]))) / KG); }
B(kin_max_height) { arity("kin::max_height", argc, 2, 2); double s = num("max_height", argv[0]) * sin(RAD(num("max_height", argv[1]))); return mk_float(s * s / (2.0 * KG)); }
B(kin_fk2) {
    arity("kin::fk2", argc, 4, 4);
    double l1 = num("fk2", argv[0]), l2 = num("fk2", argv[1]), t1 = RAD(num("fk2", argv[2])), t2 = RAD(num("fk2", argv[3]));
    TV xy[2] = {mk_float(l1 * cos(t1) + l2 * cos(t1 + t2)), mk_float(l1 * sin(t1) + l2 * sin(t1 + t2))};
    return list_from(xy, 2);
}
B(kin_ik2) {
    arity("kin::ik2", argc, 4, 5);
    double l1 = num("ik2", argv[0]), l2 = num("ik2", argv[1]), x = num("ik2", argv[2]), y = num("ik2", argv[3]);
    double c2 = (x * x + y * y - l1 * l1 - l2 * l2) / (2.0 * l1 * l2);
    if (!(c2 >= -1.0 && c2 <= 1.0)) {
        SB b = {0};
        sb_puts(&b, "kin::ik2: target (");
        fmt_shortest(x, &b); sb_puts(&b, ", "); fmt_shortest(y, &b);
        sb_puts(&b, ") is out of reach");
        char msg[120];
        snprintf(msg, sizeof msg, "%s", b.p);
        free(b.p);
        fail("%s", msg);
    }
    double t2 = acos(c2);
    if (argc > 4 && truthy(argv[4])) t2 = -t2;
    double t1 = atan2(y, x) - atan2(l2 * sin(t2), l1 + l2 * cos(t2));
    TV out[2] = {mk_float(DEG(t1)), mk_float(DEG(t2))};
    return list_from(out, 2);
}

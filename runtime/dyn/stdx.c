/* ---------------------------------------------------------------- pack:: (binary structs, CRC, COBS) */

typedef struct { uint8_t kind, size, is_signed; } PCode; /* kind: 0 pad, 1 int, 2 f32, 3 f64, 4 bool */

/* Parses a struct-style format ("<HBf", ">2H4x"). Returns the codes in a malloc'd array. */
static PCode *pack_parse(const char *fmt, int *count, bool *little) {
    const char *p = fmt;
    *little = true;
    if (*p == '<' || *p == '=' || *p == '@') p++;
    else if (*p == '>' || *p == '!') { *little = false; p++; }
    PCode *out = NULL;
    int n = 0;
    while (*p) {
        if (*p == ' ' || *p == '\t') { p++; continue; }
        int repeat = 1;
        if (*p >= '0' && *p <= '9') {
            repeat = 0;
            while (*p >= '0' && *p <= '9') repeat = repeat * 10 + (*p++ - '0');
            if (!*p) { free(out); fail("pack: format '%s' ends with a count", fmt); }
        }
        PCode c = {0, 1, 0};
        switch (*p++) {
        case 'x': break;
        case 'b': c = (PCode){1, 1, 1}; break;
        case 'B': c = (PCode){1, 1, 0}; break;
        case 'h': c = (PCode){1, 2, 1}; break;
        case 'H': c = (PCode){1, 2, 0}; break;
        case 'i': case 'l': c = (PCode){1, 4, 1}; break;
        case 'I': case 'L': c = (PCode){1, 4, 0}; break;
        case 'q': c = (PCode){1, 8, 1}; break;
        case 'Q': c = (PCode){1, 8, 0}; break;
        case 'f': c = (PCode){2, 4, 0}; break;
        case 'd': c = (PCode){3, 8, 0}; break;
        case '?': c = (PCode){4, 1, 0}; break;
        default:
            free(out);
            fail("pack: unknown format code '%c' in '%s' (use x b B h H i I q Q f d ?)", p[-1], fmt);
        }
        if (repeat > 4096) { free(out); fail("pack: repeat count too large in '%s'", fmt); }
        out = xrealloc(out, (size_t)(n + repeat + 1) * sizeof(PCode));
        for (int i = 0; i < repeat; i++) out[n++] = c;
    }
    *count = n;
    return out;
}
static bool pack_num(TV v, double *out) {
    if (v.t == T_QTY) { *out = QTY(v)->si; return true; }
    return as_f64(v, out);
}
static TV pack_values(const char *fmt, TV *vals, int nvals) {
    bool little;
    int n, need = 0, vi = 0;
    PCode *codes = pack_parse(fmt, &n, &little);
    for (int i = 0; i < n; i++) need += codes[i].kind != 0;
    if (need != nvals) { free(codes); fail("pack('%s'): format needs %d values, got %d", fmt, need, nvals); }
    SB out = {0};
    sb_puts(&out, "");
    for (int i = 0; i < n; i++) {
        PCode c = codes[i];
        uint8_t b[8] = {0};
        if (c.kind == 0) { sb_put(&out, (char *)b, 1); continue; }
        TV v = vals[vi++];
        double f;
        int64_t x;
        const char *err = NULL;
        char msg[96];
        if (c.kind == 4) b[0] = truthy(v);
        else if (c.kind == 2) {
            if (!pack_num(v, &f)) { snprintf(msg, sizeof msg, "pack: 'f' expects a number, got %s", type_name(v)); err = msg; }
            float f32 = (float)f;
            uint32_t u;
            memcpy(&u, &f32, 4);
            for (int k = 0; k < 4; k++) b[k] = (uint8_t)(u >> (8 * k));
        } else if (c.kind == 3) {
            if (!pack_num(v, &f)) { snprintf(msg, sizeof msg, "pack: 'd' expects a number, got %s", type_name(v)); err = msg; }
            uint64_t u;
            memcpy(&u, &f, 8);
            for (int k = 0; k < 8; k++) b[k] = (uint8_t)(u >> (8 * k));
        } else {
            if (!as_i64(v, &x)) { snprintf(msg, sizeof msg, "pack: integer code expects an int, got %s", type_name(v)); err = msg; }
            else {
                int bits = c.size * 8;
                bool ok = c.is_signed ? (bits == 64 || (x >= -((int64_t)1 << (bits - 1)) && x < ((int64_t)1 << (bits - 1))))
                                      : (x >= 0 && (bits == 64 || x < ((int64_t)1 << bits)));
                if (!ok) { snprintf(msg, sizeof msg, "pack: %lld does not fit in %s %d-bit", (long long)x, c.is_signed ? "signed" : "unsigned", bits); err = msg; }
                for (int k = 0; k < c.size; k++) b[k] = (uint8_t)((uint64_t)x >> (8 * k));
            }
        }
        if (err) { free(codes); free(out.p); fail("%s", err); }
        if (!little) for (int k = 0; k < c.size / 2; k++) { uint8_t t = b[k]; b[k] = b[c.size - 1 - k]; b[c.size - 1 - k] = t; }
        sb_put(&out, (char *)b, c.size);
    }
    free(codes);
    TV r = from_bytes((uint8_t *)out.p, out.n);
    free(out.p);
    return r;
}
static TV unpack_bytes(const char *fmt, const uint8_t *data, size_t len) {
    bool little;
    int n;
    size_t size = 0, off = 0;
    PCode *codes = pack_parse(fmt, &n, &little);
    for (int i = 0; i < n; i++) size += codes[i].size;
    if (size != len) { free(codes); fail("unpack('%s'): needs %u bytes, got %u", fmt, (unsigned)size, (unsigned)len); }
    List *out = list_new((uint32_t)n);
    for (int i = 0; i < n; i++) {
        PCode c = codes[i];
        uint8_t b[8] = {0};
        for (int k = 0; k < c.size; k++) b[k] = data[off + (little ? (size_t)k : (size_t)(c.size - 1 - k))];
        off += c.size;
        uint64_t u = 0;
        for (int k = 0; k < c.size; k++) u |= (uint64_t)b[k] << (8 * k);
        if (c.kind == 4) list_push(out, mk_bool(b[0] != 0));
        else if (c.kind == 2) { float f; uint32_t u32 = (uint32_t)u; memcpy(&f, &u32, 4); list_push(out, mk_float((double)f)); }
        else if (c.kind == 3) { double f; memcpy(&f, &u, 8); list_push(out, mk_float(f)); }
        else if (c.kind == 1) {
            if (c.is_signed && c.size < 8 && (b[c.size - 1] & 0x80)) u |= ~(uint64_t)0 << (8 * c.size);
            list_push(out, mk_int((int64_t)u));
        }
    }
    free(codes);
    return mk_obj(&out->h);
}
static const char *fmt_arg(int argc, TV *argv, const char *what) {
    if (argc < 1 || argv[0].t != T_STR) fail("%s: argument 1 must be a string", what);
    return STR(argv[0])->s;
}
static uint8_t *bytes_arg(int argc, TV *argv, int i, const char *what, size_t *n) {
    if (argc <= i) fail("%s: missing bytes argument", what);
    return to_bytes(argv[i], n);
}
B(pack_pack) {
    const char *fmt = fmt_arg(argc, argv, "pack::pack(fmt, values)");
    if (argc == 2 && argv[1].t == T_LIST) return pack_values(fmt, LIST(argv[1])->v, (int)LIST(argv[1])->n);
    return pack_values(fmt, argv + 1, argc - 1);
}
B(pack_unpack) {
    const char *fmt = fmt_arg(argc, argv, "pack::unpack(fmt, bytes)");
    size_t n;
    uint8_t *b = bytes_arg(argc, argv, 1, "pack::unpack", &n);
    push(mk_obj(&list_new(0)->h)); /* keeps a slot so the buffer is freed on the error path too */
    Try t = {.sp = R.sp, .depth = R.depth, .line = R.line, .reacting = R.reacting};
    if (R.ntry >= 32) { free(b); fail("try blocks nested too deeply"); }
    R.tries[R.ntry++] = &t;
    if (setjmp(t.jb)) {
        R.ntry--;
        free(b);
        char msg[240];
        snprintf(msg, sizeof msg, "%s", R.errmsg);
        fail("%s", msg);
    }
    TV r = unpack_bytes(fmt, b, n);
    R.ntry--;
    R.sp--;
    free(b);
    return r;
}
B(pack_calcsize) {
    bool little;
    int n;
    int64_t size = 0;
    PCode *codes = pack_parse(fmt_arg(argc, argv, "pack::calcsize(fmt)"), &n, &little);
    for (int i = 0; i < n; i++) size += codes[i].size;
    free(codes);
    return mk_int(size);
}
B(pack_f32s) {
    if (argc < 1 || argv[0].t != T_LIST) fail("pack::f32s(list)");
    char fmt[24];
    snprintf(fmt, sizeof fmt, "<%uf", (unsigned)LIST(argv[0])->n);
    return pack_values(fmt, LIST(argv[0])->v, (int)LIST(argv[0])->n);
}
B(pack_to_f32s) {
    size_t n;
    uint8_t *b = bytes_arg(argc, argv, 0, "pack::to_f32s", &n);
    if (n % 4) { free(b); fail("pack::to_f32s: %u bytes is not a multiple of 4", (unsigned)n); }
    char fmt[24];
    snprintf(fmt, sizeof fmt, "<%uf", (unsigned)(n / 4));
    TV r = unpack_bytes(fmt, b, n);
    free(b);
    return r;
}
B(pack_crc16) {
    size_t n;
    uint8_t *b = bytes_arg(argc, argv, 0, "pack::crc16", &n);
    uint16_t c = crc16(b, n);
    free(b);
    return mk_int(c);
}
B(pack_crc32) {
    size_t n;
    uint8_t *b = bytes_arg(argc, argv, 0, "pack::crc32", &n);
    uint32_t crc = 0xFFFFFFFFu;
    for (size_t i = 0; i < n; i++) {
        crc ^= b[i];
        for (int k = 0; k < 8; k++) crc = (crc & 1) ? (crc >> 1) ^ 0xEDB88320u : crc >> 1;
    }
    free(b);
    return mk_int((int64_t)(~crc & 0xFFFFFFFFu));
}
B(pack_cobs_encode) {
    size_t n;
    uint8_t *b = bytes_arg(argc, argv, 0, "pack::cobs_encode", &n), *out = xmalloc(n + n / 254 + 2);
    size_t o = 1, code_at = 0;
    uint8_t code = 1;
    for (size_t i = 0; i < n; i++) {
        if (b[i] == 0) { out[code_at] = code; code_at = o++; code = 1; }
        else {
            out[o++] = b[i];
            if (++code == 0xff) { out[code_at] = code; code_at = o++; code = 1; }
        }
    }
    out[code_at] = code;
    TV r = from_bytes(out, o);
    free(b);
    free(out);
    return r;
}
B(pack_cobs_decode) {
    size_t n, i = 0, o = 0;
    uint8_t *b = bytes_arg(argc, argv, 0, "pack::cobs_decode", &n), *out = xmalloc(n + 1);
    const char *err = NULL;
    while (i < n && !err) {
        uint8_t code = b[i++];
        if (!code) { err = "cobs_decode: unexpected zero byte"; break; }
        for (uint8_t k = 1; k < code; k++) {
            if (i >= n) { err = "cobs_decode: truncated frame"; break; }
            out[o++] = b[i++];
        }
        if (!err && code < 0xff && i < n) out[o++] = 0;
    }
    TV r = err ? NIL : from_bytes(out, o);
    free(b);
    free(out);
    if (err) fail("%s", err);
    return r;
}

/* ---------------------------------------------------------------- json:: */

static void json_str(const char *s, size_t n, SB *b) {
    sb_putc(b, '"');
    for (size_t i = 0; i < n; i++) {
        unsigned char c = (unsigned char)s[i];
        switch (c) {
        case '"': sb_puts(b, "\\\""); break;
        case '\\': sb_puts(b, "\\\\"); break;
        case '\n': sb_puts(b, "\\n"); break;
        case '\r': sb_puts(b, "\\r"); break;
        case '\t': sb_puts(b, "\\t"); break;
        case '\b': sb_puts(b, "\\b"); break;
        case '\f': sb_puts(b, "\\f"); break;
        default:
            if (c < 0x20) sb_printf(b, "\\u%04x", c);
            else sb_putc(b, (char)c);
        }
    }
    sb_putc(b, '"');
}
static void json_num(double f, SB *b) {
    if (!isfinite(f)) { sb_puts(b, "null"); return; }
    double a = fabs(f);
    if (f == floor(f) && a < 1e16) { sb_printf(b, "%.1f", f); return; }
    if (a >= 1e-5 && a < 1e16) { fmt_shortest(f, b); return; }
    /* shortest digits in exponent form, e.g. 1.5e-7 */
    char e[40];
    for (int prec = 0; prec < 17; prec++) {
        snprintf(e, sizeof e, "%.*e", prec, f);
        if (strtod(e, NULL) == f) break;
    }
    char *ex = strchr(e, 'e');
    int en = atoi(ex + 1);
    *ex = 0;
    sb_printf(b, "%se%d", e, en);
}
static void json_indent(SB *b, int pretty, int depth) {
    if (!pretty) return;
    sb_putc(b, '\n');
    for (int i = 0; i < depth * 2; i++) sb_putc(b, ' ');
}
static void json_encode(TV v, SB *b, int pretty, int depth) {
    if (depth > 64) { free(b->p); b->p = NULL; fail("json::encode: value nested too deeply"); }
    switch (v.t) {
    case T_NIL: sb_puts(b, "null"); break;
    case T_BOOL: sb_puts(b, v.u.i ? "true" : "false"); break;
    case T_INT: fmt_i64(v.u.i, b); break;
    case T_FLOAT: json_num(v.u.f, b); break;
    case T_QTY: json_num(QTY(v)->si, b); break;
    case T_STR: json_str(STR(v)->s, STR(v)->len, b); break;
    case T_SIGNAL: json_encode(((Signal *)v.u.o)->value, b, pretty, depth); break;
    case T_RANGE: case T_LIST: {
        TV items = v.t == T_RANGE ? iterate(v, 1) : v;
        push(items);
        List *l = LIST(items);
        sb_putc(b, '[');
        for (uint32_t i = 0; i < l->n; i++) {
            if (i) sb_putc(b, ',');
            json_indent(b, pretty, depth + 1);
            json_encode(l->v[i], b, pretty, depth + 1);
        }
        if (l->n) json_indent(b, pretty, depth);
        sb_putc(b, ']');
        R.sp--;
        break;
    }
    case T_DICT: case T_OBJ: {
        /* keys are written in sorted order, as the host does */
        uint32_t n = v.t == T_DICT ? DICT(v)->n : ((Inst *)v.u.o)->n;
        char **keys = xmalloc((n + 1) * sizeof(char *));
        uint32_t *order = xmalloc((n + 1) * sizeof(uint32_t));
        for (uint32_t i = 0; i < n; i++) {
            if (v.t == T_OBJ) keys[i] = dupstr(SYM(KID(KID(((Inst *)v.u.o)->cls->def, 0), i)->a));
            else {
                SB k = {0};
                to_sb(DICT(v)->k[i], &k, false);
                sb_puts(&k, "");
                keys[i] = k.p;
            }
            uint32_t at = i;
            while (at > 0 && strcmp(keys[order[at - 1]], keys[i]) > 0) { order[at] = order[at - 1]; at--; }
            order[at] = i;
        }
        sb_putc(b, '{');
        bool any = false;
        for (uint32_t q = 0; q < n; q++) {
            uint32_t i = order[q];
            /* of two keys that print alike, the later one wins */
            if (q + 1 < n && !strcmp(keys[order[q + 1]], keys[i])) continue;
            if (any) sb_putc(b, ',');
            any = true;
            json_indent(b, pretty, depth + 1);
            json_str(keys[i], strlen(keys[i]), b);
            sb_puts(b, pretty ? ": " : ":");
            json_encode(v.t == T_DICT ? DICT(v)->v[i] : ((Inst *)v.u.o)->f[i], b, pretty, depth + 1);
        }
        if (any) json_indent(b, pretty, depth);
        sb_putc(b, '}');
        for (uint32_t i = 0; i < n; i++) free(keys[i]);
        free(keys);
        free(order);
        break;
    }
    default: {
        SB s = {0};
        to_sb(v, &s, false);
        sb_puts(&s, "");
        json_str(s.p, s.n, b);
        free(s.p);
    }
    }
}
B(json_encode) {
    if (argc < 1) fail("json::encode(value, pretty=false)");
    SB b = {0};
    json_encode(argv[0], &b, argc > 1 && truthy(argv[1]), 0);
    return sb_done(&b);
}

typedef struct { const char *s, *p; bool bad; } JP;
static void jp_ws(JP *j) { while (*j->p == ' ' || *j->p == '\t' || *j->p == '\n' || *j->p == '\r') j->p++; }
static TV jp_value(JP *j, int depth);
static TV jp_string(JP *j) {
    SB b = {0};
    sb_puts(&b, "");
    j->p++;
    while (*j->p && *j->p != '"') {
        unsigned char c = (unsigned char)*j->p++;
        if (c < 0x20) { j->bad = true; break; }
        if (c != '\\') { sb_putc(&b, (char)c); continue; }
        char e = *j->p++;
        switch (e) {
        case 'n': sb_putc(&b, '\n'); break;
        case 't': sb_putc(&b, '\t'); break;
        case 'r': sb_putc(&b, '\r'); break;
        case 'b': sb_putc(&b, '\b'); break;
        case 'f': sb_putc(&b, '\f'); break;
        case '"': case '\\': case '/': sb_putc(&b, e); break;
        case 'u': {
            uint32_t cp = 0;
            for (int pass = 0; pass < 2; pass++) {
                uint32_t u = 0;
                for (int k = 0; k < 4; k++) {
                    char h = *j->p;
                    int d = h >= '0' && h <= '9' ? h - '0' : h >= 'a' && h <= 'f' ? h - 'a' + 10 : h >= 'A' && h <= 'F' ? h - 'A' + 10 : -1;
                    if (d < 0) { j->bad = true; break; }
                    u = u * 16 + (uint32_t)d;
                    j->p++;
                }
                if (pass == 0) {
                    cp = u;
                    if (u < 0xD800 || u > 0xDBFF || j->p[0] != '\\' || j->p[1] != 'u') break;
                    j->p += 2;
                } else cp = 0x10000 + ((cp - 0xD800) << 10) + (u - 0xDC00);
            }
            if (cp < 0x80) sb_putc(&b, (char)cp);
            else if (cp < 0x800) { sb_putc(&b, (char)(0xC0 | (cp >> 6))); sb_putc(&b, (char)(0x80 | (cp & 0x3F))); }
            else if (cp < 0x10000) { sb_putc(&b, (char)(0xE0 | (cp >> 12))); sb_putc(&b, (char)(0x80 | ((cp >> 6) & 0x3F))); sb_putc(&b, (char)(0x80 | (cp & 0x3F))); }
            else { sb_putc(&b, (char)(0xF0 | (cp >> 18))); sb_putc(&b, (char)(0x80 | ((cp >> 12) & 0x3F))); sb_putc(&b, (char)(0x80 | ((cp >> 6) & 0x3F))); sb_putc(&b, (char)(0x80 | (cp & 0x3F))); }
            break;
        }
        default: j->bad = true;
        }
        if (j->bad) break;
    }
    if (*j->p != '"') j->bad = true; else j->p++;
    return sb_done(&b);
}
static TV jp_value(JP *j, int depth) {
    jp_ws(j);
    if (depth > 64) { j->bad = true; return NIL; }
    char c = *j->p;
    if (c == '"') return jp_string(j);
    if (c == '{') {
        Dict *d = dict_new();
        push(mk_obj(&d->h));
        j->p++;
        jp_ws(j);
        if (*j->p == '}') j->p++;
        else
            for (;;) {
                jp_ws(j);
                if (*j->p != '"') { j->bad = true; break; }
                TV k = jp_string(j);
                push(k);
                jp_ws(j);
                if (j->bad || *j->p != ':') { j->bad = true; R.sp--; break; }
                j->p++;
                TV v = jp_value(j, depth + 1);
                R.sp--;
                if (j->bad) break;
                /* object keys come back sorted, as on the host */
                uint32_t before = d->n;
                dict_set(d, k, v);
                if (d->n > before) {
                    uint32_t at = d->n - 1;
                    while (at > 0 && compare(d->k[at - 1], k) > 0) { d->k[at] = d->k[at - 1]; d->v[at] = d->v[at - 1]; at--; }
                    d->k[at] = k;
                    d->v[at] = v;
                }
                jp_ws(j);
                if (*j->p == ',') { j->p++; continue; }
                if (*j->p == '}') { j->p++; break; }
                j->bad = true;
                break;
            }
        R.sp--;
        return mk_obj(&d->h);
    }
    if (c == '[') {
        List *l = list_new(0);
        push(mk_obj(&l->h));
        j->p++;
        jp_ws(j);
        if (*j->p == ']') j->p++;
        else
            for (;;) {
                TV v = jp_value(j, depth + 1);
                if (j->bad) break;
                list_push(l, v);
                jp_ws(j);
                if (*j->p == ',') { j->p++; continue; }
                if (*j->p == ']') { j->p++; break; }
                j->bad = true;
                break;
            }
        R.sp--;
        return mk_obj(&l->h);
    }
    if (!strncmp(j->p, "true", 4)) { j->p += 4; return mk_bool(true); }
    if (!strncmp(j->p, "false", 5)) { j->p += 5; return mk_bool(false); }
    if (!strncmp(j->p, "null", 4)) { j->p += 4; return NIL; }
    if (c == '-' || (c >= '0' && c <= '9')) {
        const char *start = j->p;
        bool is_float = false;
        if (*j->p == '-') j->p++;
        if (!(*j->p >= '0' && *j->p <= '9')) { j->bad = true; return NIL; }
        while (*j->p >= '0' && *j->p <= '9') j->p++;
        if (*j->p == '.') { is_float = true; j->p++; while (*j->p >= '0' && *j->p <= '9') j->p++; }
        if (*j->p == 'e' || *j->p == 'E') {
            is_float = true;
            j->p++;
            if (*j->p == '+' || *j->p == '-') j->p++;
            while (*j->p >= '0' && *j->p <= '9') j->p++;
        }
        if (!is_float) {
            /* an integer that fits in 64 bits stays an integer */
            size_t digits = (size_t)(j->p - start) - (*start == '-');
            if (digits <= 18) return mk_int(strtoll(start, NULL, 10));
            double f = strtod(start, NULL);
            if (f >= -9223372036854775808.0 && f < 9223372036854775808.0) {
                long long ll = strtoll(start, NULL, 10);
                char back[32];
                snprintf(back, sizeof back, "%lld", ll);
                if ((size_t)(j->p - start) == strlen(back) && !strncmp(back, start, strlen(back))) return mk_int(ll);
            }
            return mk_float(f);
        }
        return mk_float(strtod(start, NULL));
    }
    j->bad = true;
    return NIL;
}
B(json_decode) {
    if (argc < 1 || argv[0].t != T_STR) fail("json::decode(text): argument 1 must be a string");
    JP j = {STR(argv[0])->s, STR(argv[0])->s, false};
    int base = R.sp;
    TV v = jp_value(&j, 0);
    push(v);
    jp_ws(&j);
    R.sp = base;
    if (j.bad || *j.p) fail("json::decode: invalid JSON at character %d", (int)(j.p - j.s) + 1);
    return v;
}

/* ---------------------------------------------------------------- tlm:: (IDE telemetry; off in firmware) */

B(tlm_enabled) { (void)argc; (void)argv; return mk_bool(false); }
B(tlm_off) { (void)argc; (void)argv; return NIL; }

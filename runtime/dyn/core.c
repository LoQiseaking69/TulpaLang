/* ---------------------------------------------------------------- errors */

static void fail(const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    vsnprintf(R.errmsg, sizeof R.errmsg, fmt, ap);
    va_end(ap);
    if (R.ntry > 0) longjmp(R.tries[R.ntry - 1]->jb, 1);
    char buf[320];
    snprintf(buf, sizeof buf, "tulpa: runtime error at line %d: %s", R.line, R.errmsg);
    tulpa_hal_panic(buf);
    for (;;) {}
}

/* The device runtime (tulpa_rt.c) reports its failures here, so `try` can catch them too. */
void tulpa_dyn_violation(const char *msg) {
    if (strncmp(msg, "tulpa: ", 7) == 0) msg += 7;
    fail("%s", msg);
}

/* ---------------------------------------------------------------- heap and GC */

static void *xmalloc(size_t n) {
    void *p = malloc(n ? n : 1);
    if (!p) {
        R.ntry = 0;
        fail("out of memory (%u bytes requested)", (unsigned)n);
    }
    return p;
}
static void *xrealloc(void *p, size_t n) {
    p = realloc(p, n ? n : 1);
    if (!p) {
        R.ntry = 0;
        fail("out of memory (%u bytes requested)", (unsigned)n);
    }
    return p;
}

static Obj *alloc(uint8_t t, size_t size) {
    Obj *o = xmalloc(size);
    memset(o, 0, size);
    o->t = t;
    o->next = R.objs;
    R.objs = o;
    R.allocated += size + 16;
    return o;
}

static void mark(TV v);
static void mark_obj(Obj *o) {
    while (o && !o->mark) {
        o->mark = 1;
        switch (o->t) {
        case T_LIST: {
            List *l = (List *)o;
            for (uint32_t i = 0; i < l->n; i++) mark(l->v[i]);
            return;
        }
        case T_DICT: {
            Dict *d = (Dict *)o;
            for (uint32_t i = 0; i < d->n; i++) { mark(d->k[i]); mark(d->v[i]); }
            return;
        }
        case T_ENV: {
            Env *e = (Env *)o;
            for (int i = 0; i < e->n; i++) mark(e->vals[i]);
            o = (Obj *)e->up;
            continue;
        }
        case T_FN: mark(((Fn *)o)->self); o = (Obj *)((Fn *)o)->env; continue;
        case T_CLASS: case T_MTYPE: o = (Obj *)((Class *)o)->env; continue;
        case T_OBJ: {
            Inst *in = (Inst *)o;
            for (int i = 0; i < in->n; i++) mark(in->f[i]);
            o = (Obj *)in->cls;
            continue;
        }
        case T_MACH: {
            Mach *m = (Mach *)o;
            mark_obj((Obj *)m->ty); mark_obj((Obj *)m->history);
            o = (Obj *)m->env;
            continue;
        }
        case T_SIGNAL: {
            Signal *s = (Signal *)o;
            mark(s->value); mark(s->prev); mark(s->derive);
            mark_obj((Obj *)s->history);
            o = (Obj *)s->subs;
            continue;
        }
        case T_TASK: {
            Task *t = (Task *)o;
            mark(t->f); mark(t->result); mark_obj((Obj *)t->err);
            o = (Obj *)t->args;
            continue;
        }
        case T_THUNK: o = (Obj *)((Thunk *)o)->env; continue;
        case T_NATIVE: mark(((Native *)o)->extra); return;
        default: return;
        }
    }
}
static void mark(TV v) {
    if (v.t >= T_STR && v.t != T_BUILTIN && v.t != T_MODFN) mark_obj(v.u.o);
}

static void free_obj(Obj *o) {
    switch (o->t) {
    case T_LIST: free(((List *)o)->v); break;
    case T_DICT: free(((Dict *)o)->k); free(((Dict *)o)->v); break;
    case T_ENV: free(((Env *)o)->names); free(((Env *)o)->vals); free(((Env *)o)->konst); break;
    default: break;
    }
    free(o);
}

/* Heap bytes an object holds, for pacing the collector. */
static size_t obj_bytes(Obj *o) {
    switch (o->t) {
    case T_STR: return sizeof(Str) + ((Str *)o)->len + 1;
    case T_LIST: return sizeof(List) + ((List *)o)->cap * sizeof(TV);
    case T_DICT: return sizeof(Dict) + ((Dict *)o)->cap * 2 * sizeof(TV);
    case T_ENV: return sizeof(Env) + ((Env *)o)->cap * (sizeof(TV) + sizeof(int32_t) + 1);
    case T_OBJ: return sizeof(Inst) + ((Inst *)o)->n * sizeof(TV);
    default: return 64;
    }
}

static void gc(void) {
    for (int i = 0; i < R.sp; i++) mark(R.stack[i]);
    mark_obj((Obj *)R.globals);
    mark_obj((Obj *)R.tasks); mark_obj((Obj *)R.keep);
    mark(R.ret);
    for (int i = 0; i < R.nmodules; i++) mark(R.module_vals[i]);
    for (int i = 0; i < R.nsyms; i++) mark_obj(R.symstr[i]);
    for (int i = 0; i < R.nrules; i++) mark_obj((Obj *)R.rules[i].env);
    for (int i = 0; i < R.nmonitors; i++) mark_obj((Obj *)R.monitors[i].env);
    for (int i = 0; i < R.ndeadlines; i++) { mark_obj((Obj *)R.deadlines[i].env); mark_obj((Obj *)R.deadlines[i].shown); }
    Obj **p = &R.objs;
    size_t live = 0;
    while (*p) {
        Obj *o = *p;
        if (o->mark) { o->mark = 0; live += obj_bytes(o) + 16; p = &o->next; }
        else { *p = o->next; free_obj(o); }
    }
    R.live = live;
}

/* Called where every live value is rooted: at calls and loop iterations. The next collection
 * comes after allocating half as much again as is live, so a growing program is not collected
 * on every call and a small one never holds much garbage. */
static void safepoint(void) {
    if (R.allocated < R.threshold) return;
    gc();
    R.allocated = 0;
    size_t t = R.live / 2;
    R.threshold = t > TULPA_DYN_GC_BYTES ? t : TULPA_DYN_GC_BYTES;
}

/* ---------------------------------------------------------------- strings */

static void sb_put(SB *b, const char *s, size_t n) {
    if (b->n + n + 1 > b->cap) {
        b->cap = (b->n + n + 1) * 2 + 16;
        b->p = xrealloc(b->p, b->cap);
    }
    memcpy(b->p + b->n, s, n);
    b->n += n;
    b->p[b->n] = 0;
}
static void sb_puts(SB *b, const char *s) { sb_put(b, s, strlen(s)); }
static void sb_putc(SB *b, char c) { sb_put(b, &c, 1); }
static void sb_printf(SB *b, const char *fmt, ...) {
    char buf[96];
    va_list ap;
    va_start(ap, fmt);
    int n = vsnprintf(buf, sizeof buf, fmt, ap);
    va_end(ap);
    sb_put(b, buf, (size_t)(n < (int)sizeof buf ? n : (int)sizeof buf - 1));
}

static TV str_n(const char *s, size_t n) {
    Str *o = (Str *)alloc(T_STR, sizeof(Str) + n + 1);
    memcpy(o->s, s, n);
    o->s[n] = 0;
    o->len = (uint32_t)n;
    return mk_obj(&o->h);
}
static TV str_c(const char *s) { return str_n(s, strlen(s)); }
static TV sb_done(SB *b) {
    TV v = str_n(b->p ? b->p : "", b->n);
    free(b->p);
    b->p = NULL;
    return v;
}

/* ---------------------------------------------------------------- lists, dicts, envs */

static List *list_new(uint32_t cap) {
    List *l = (List *)alloc(T_LIST, sizeof(List));
    if (cap) { l->v = xmalloc(cap * sizeof(TV)); l->cap = cap; }
    return l;
}
static void list_push(List *l, TV v) {
    if (l->n == l->cap) {
        l->cap = l->cap ? l->cap * 2 : 4;
        l->v = xrealloc(l->v, l->cap * sizeof(TV));
    }
    l->v[l->n++] = v;
}
static TV list_from(const TV *v, uint32_t n) {
    List *l = list_new(n);
    if (n) memcpy(l->v, v, n * sizeof(TV));
    l->n = n;
    return mk_obj(&l->h);
}

static bool equals(TV a, TV b);
static Dict *dict_new(void) { return (Dict *)alloc(T_DICT, sizeof(Dict)); }
static bool key_eq(TV a, TV b) {
    if (a.t == T_STR && b.t == T_STR) return STR(a)->len == STR(b)->len && memcmp(STR(a)->s, STR(b)->s, STR(a)->len) == 0;
    if (a.t == T_STR || b.t == T_STR) return false;
    return equals(a, b);
}
static void check_key(TV k) {
    if (k.t != T_STR && k.t != T_INT && k.t != T_BOOL && k.t != T_NIL && k.t != T_FLOAT) fail("unhashable dict key");
}
static int dict_find(Dict *d, TV k) {
    for (uint32_t i = 0; i < d->n; i++) if (key_eq(d->k[i], k)) return (int)i;
    return -1;
}
static int dict_find_c(Dict *d, const char *k) {
    size_t n = strlen(k);
    for (uint32_t i = 0; i < d->n; i++)
        if (d->k[i].t == T_STR && STR(d->k[i])->len == n && memcmp(STR(d->k[i])->s, k, n) == 0) return (int)i;
    return -1;
}
static void dict_set(Dict *d, TV k, TV v) {
    check_key(k);
    int i = dict_find(d, k);
    if (i >= 0) { d->v[i] = v; return; }
    if (d->n == d->cap) {
        d->cap = d->cap ? d->cap * 2 : 4;
        d->k = xrealloc(d->k, d->cap * sizeof(TV));
        d->v = xrealloc(d->v, d->cap * sizeof(TV));
    }
    d->k[d->n] = k;
    d->v[d->n++] = v;
}
static void dict_set_c(Dict *d, const char *k, TV v) {
    int i = dict_find_c(d, k);
    if (i >= 0) { d->v[i] = v; return; }
    push(v);
    TV key = str_c(k);
    R.sp--;
    dict_set(d, key, v);
}
static bool dict_remove(Dict *d, TV k, TV *out) {
    int i = dict_find(d, k);
    if (i < 0) return false;
    if (out) *out = d->v[i];
    memmove(d->k + i, d->k + i + 1, (d->n - i - 1) * sizeof(TV));
    memmove(d->v + i, d->v + i + 1, (d->n - i - 1) * sizeof(TV));
    d->n--;
    return true;
}

static Env *env_new(Env *up) {
    Env *e = (Env *)alloc(T_ENV, sizeof(Env));
    e->up = up;
    return e;
}
static void define(Env *e, int32_t name, TV v, bool konst) {
    for (int i = 0; i < e->n; i++)
        if (e->names[i] == name) { e->vals[i] = v; if (e->konst) e->konst[i] = konst; return; }
    if (e->n == e->cap) {
        e->cap = e->cap ? e->cap * 2 : 4;
        e->names = xrealloc(e->names, e->cap * sizeof(int32_t));
        e->vals = xrealloc(e->vals, e->cap * sizeof(TV));
        e->konst = xrealloc(e->konst, e->cap);
    }
    e->names[e->n] = name;
    e->vals[e->n] = v;
    e->konst[e->n++] = konst;
}
static TV *lookup(Env *e, int32_t name) {
    for (; e; e = e->up)
        for (int i = 0; i < e->n; i++)
            if (e->names[i] == name) return &e->vals[i];
    return NULL;
}

static const char *sym_name(int32_t i) {
    return i < R.nsyms_image ? R.strs + R.sym_off[i] : R.extra_syms[i - R.nsyms_image];
}
static int find_sym(const char *s) {
    for (int i = 0; i < R.nsyms; i++) if (strcmp(sym_name(i), s) == 0) return i;
    return -1;
}
static int global_find(const char *name);
/* Names made at run time (a unit built from a string, a postcondition's x') join the table. */
static int32_t intern(const char *s) {
    int found = find_sym(s);
    if (found >= 0) return found;
    int extra = R.nsyms - R.nsyms_image;
    R.extra_syms = xrealloc(R.extra_syms, (size_t)(extra + 1) * sizeof(char *));
    R.sym_builtin = xrealloc(R.sym_builtin, (size_t)(R.nsyms + 1) * sizeof(int16_t));
    R.symstr = xrealloc(R.symstr, (size_t)(R.nsyms + 1) * sizeof(Obj *));
    char *copy = xmalloc(strlen(s) + 1);
    strcpy(copy, s);
    R.extra_syms[extra] = copy;
    R.symstr[R.nsyms] = NULL;
    R.sym_builtin[R.nsyms] = (int16_t)global_find(s);
    return R.nsyms++;
}

/* ---------------------------------------------------------------- units */

static const int8_t DIMLESS[8] = {0};
static bool dim_eq(const int8_t *a, const int8_t *b) { return memcmp(a, b, 8) == 0; }
static bool is_dimless(const int8_t *d) { return dim_eq(d, DIMLESS); }

static TV mk_qty(double si, const int8_t *dim, int32_t unit) {
    if (is_dimless(dim)) return mk_float(si);
    Qty *q = (Qty *)alloc(T_QTY, sizeof(Qty));
    q->si = si;
    memcpy(q->dim, dim, 8);
    q->unit = unit;
    return mk_obj(&q->h);
}

static void dim_name(const int8_t *d, SB *b) {
    static const struct { const char *n; int8_t d[8]; } NAMED[] = {
        {"N", {1, 1, -2}}, {"J", {2, 1, -2}}, {"W", {2, 1, -3}}, {"Pa", {-1, 1, -2}},
        {"V", {2, 1, -3, -1}}, {"ohm", {2, 1, -3, -2}}, {"C", {0, 0, 1, 1}}, {"Hz", {0, 0, -1}}};
    static const char *const BASE[8] = {"m", "kg", "s", "A", "K", "mol", "cd", "rad"};
    for (size_t i = 0; i < sizeof NAMED / sizeof NAMED[0]; i++)
        if (dim_eq(NAMED[i].d, d)) { sb_puts(b, NAMED[i].n); return; }
    bool any = false;
    for (int i = 0; i < 8; i++)
        if (d[i] > 0) {
            if (any) sb_putc(b, '*');
            sb_puts(b, BASE[i]);
            if (d[i] != 1) sb_printf(b, "^%d", d[i]);
            any = true;
        }
    if (!any) sb_putc(b, '1');
    for (int i = 0; i < 8; i++)
        if (d[i] < 0) {
            sb_putc(b, '/');
            sb_puts(b, BASE[i]);
            if (d[i] != -1) sb_printf(b, "^%d", -d[i]);
        }
}

/* "speed [m/s]" style description, for error messages. Returns a static buffer (two alternate). */
static const char *describe_dim(const int8_t *d) {
    static const struct { const char *n; int8_t d[8]; } KINDS[] = {
        {"length", {1}}, {"mass", {0, 1}}, {"time", {0, 0, 1}}, {"current", {0, 0, 0, 1}},
        {"temperature", {0, 0, 0, 0, 1}}, {"angle", {0, 0, 0, 0, 0, 0, 0, 1}}, {"frequency", {0, 0, -1}},
        {"force", {1, 1, -2}}, {"energy", {2, 1, -2}}, {"power", {2, 1, -3}}, {"pressure", {-1, 1, -2}},
        {"voltage", {2, 1, -3, -1}}, {"resistance", {2, 1, -3, -2}}, {"charge", {0, 0, 1, 1}},
        {"speed", {1, 0, -1}}, {"acceleration", {1, 0, -2}}, {"angular velocity", {0, 0, -1, 0, 0, 0, 0, 1}}};
    static char bufs[2][64];
    static int which;
    char *out = bufs[which ^= 1];
    if (is_dimless(d)) return "dimensionless";
    SB b = {0};
    dim_name(d, &b);
    const char *kind = NULL;
    for (size_t i = 0; i < sizeof KINDS / sizeof KINDS[0]; i++)
        if (dim_eq(KINDS[i].d, d)) kind = KINDS[i].n;
    if (kind) snprintf(out, 64, "%s [%s]", kind, b.p);
    else snprintf(out, 64, "[%s]", b.p);
    free(b.p);
    return out;
}

/* Unit atoms, as in src/units.rs. Units written as literals arrive resolved in the program
 * image; this table serves unit names built at run time (quantity(5, name), q.to(name)). */
static const struct { const char *sym; double scale; int8_t dim[8]; } ATOMS[] = {
    {"m", 1.0, {1}}, {"km", 1e3, {1}}, {"cm", 1e-2, {1}}, {"mm", 1e-3, {1}}, {"um", 1e-6, {1}}, {"nm", 1e-9, {1}},
    {"in", 0.0254, {1}}, {"ft", 0.3048, {1}}, {"yd", 0.9144, {1}}, {"mi", 1609.344, {1}},
    {"kg", 1.0, {0, 1}}, {"g", 1e-3, {0, 1}}, {"mg", 1e-6, {0, 1}}, {"t", 1e3, {0, 1}}, {"lb", 0.45359237, {0, 1}},
    {"oz", 0.028349523125, {0, 1}},
    {"s", 1.0, {0, 0, 1}}, {"ms", 1e-3, {0, 0, 1}}, {"us", 1e-6, {0, 0, 1}}, {"ns", 1e-9, {0, 0, 1}},
    {"min", 60.0, {0, 0, 1}}, {"h", 3600.0, {0, 0, 1}},
    {"A", 1.0, {0, 0, 0, 1}}, {"mA", 1e-3, {0, 0, 0, 1}}, {"uA", 1e-6, {0, 0, 0, 1}},
    {"V", 1.0, {2, 1, -3, -1}}, {"mV", 1e-3, {2, 1, -3, -1}}, {"kV", 1e3, {2, 1, -3, -1}},
    {"ohm", 1.0, {2, 1, -3, -2}}, {"kohm", 1e3, {2, 1, -3, -2}}, {"Mohm", 1e6, {2, 1, -3, -2}},
    {"C", 1.0, {0, 0, 1, 1}}, {"mAh", 3.6, {0, 0, 1, 1}}, {"Ah", 3600.0, {0, 0, 1, 1}},
    {"K", 1.0, {0, 0, 0, 0, 1}}, {"mol", 1.0, {0, 0, 0, 0, 0, 1}}, {"cd", 1.0, {0, 0, 0, 0, 0, 0, 1}},
    {"rad", 1.0, {0, 0, 0, 0, 0, 0, 0, 1}}, {"deg", 3.14159265358979323846 / 180.0, {0, 0, 0, 0, 0, 0, 0, 1}},
    {"rev", 6.28318530717958647692, {0, 0, 0, 0, 0, 0, 0, 1}},
    {"Hz", 1.0, {0, 0, -1}}, {"kHz", 1e3, {0, 0, -1}}, {"MHz", 1e6, {0, 0, -1}},
    {"rpm", 6.28318530717958647692 / 60.0, {0, 0, -1, 0, 0, 0, 0, 1}},
    {"N", 1.0, {1, 1, -2}}, {"kN", 1e3, {1, 1, -2}},
    {"J", 1.0, {2, 1, -2}}, {"kJ", 1e3, {2, 1, -2}}, {"Wh", 3600.0, {2, 1, -2}},
    {"W", 1.0, {2, 1, -3}}, {"mW", 1e-3, {2, 1, -3}}, {"kW", 1e3, {2, 1, -3}},
    {"Pa", 1.0, {-1, 1, -2}}, {"kPa", 1e3, {-1, 1, -2}}, {"bar", 1e5, {-1, 1, -2}}, {"psi", 6894.757293168, {-1, 1, -2}},
    {"kph", 1.0 / 3.6, {1, 0, -1}}, {"mph", 0.44704, {1, 0, -1}}, {"gee", 9.80665, {1, 0, -2}},
};

/* Parses a unit expression such as m/s^2, kg*m/s^2, N.m, 1/s. Returns its index in R.units. */
static int unit_parse(const char *src, size_t n) {
    char text[64], sym[24];
    size_t tn = 0;
    for (size_t i = 0; i < n && tn < sizeof text - 1; i++) if (src[i] != ' ' && src[i] != '\t') text[tn++] = src[i];
    text[tn] = 0;
    if (!tn) fail("empty unit");
    double scale = 1.0;
    int dim[8] = {0}, sign = 1;
    size_t i = 0;
    bool first = true;
    while (i < tn) {
        if (!first) {
            if ((unsigned char)text[i] == 0xC2 && (unsigned char)text[i + 1] == 0xB7) i++; /* the middle dot */
            else if (text[i] == '/') sign = -1;
            else if (text[i] != '*' && text[i] != '.') fail("unexpected '%c' in unit '%s'", text[i], text);
            i++;
        }
        first = false;
        size_t sn = 0;
        while (i < tn && (text[i] == '_' || (text[i] >= '0' && text[i] <= '9') || (text[i] >= 'a' && text[i] <= 'z') ||
                          (text[i] >= 'A' && text[i] <= 'Z'))) {
            if (sn < sizeof sym - 1) sym[sn++] = text[i];
            i++;
        }
        sym[sn] = 0;
        int ex = 1;
        if (i < tn && text[i] == '^') {
            char *end;
            ex = (int)strtol(text + i + 1, &end, 10);
            if (end == text + i + 1) fail("bad exponent in unit '%s'", text);
            i = (size_t)(end - text);
        }
        double k = 1.0;
        const int8_t *d = DIMLESS;
        if (strcmp(sym, "1")) {
            size_t at;
            for (at = 0; at < sizeof ATOMS / sizeof ATOMS[0]; at++) if (!strcmp(ATOMS[at].sym, sym)) break;
            if (at == sizeof ATOMS / sizeof ATOMS[0]) fail("unknown unit '%s'", sym);
            k = ATOMS[at].scale;
            d = ATOMS[at].dim;
        }
        ex *= sign;
        scale *= pow(k, ex);
        for (int q = 0; q < 8; q++) dim[q] += d[q] * ex;
    }
    R.units = xrealloc(R.units, (size_t)(R.nunits + 1) * sizeof(UnitDef));
    UnitDef *u = &R.units[R.nunits];
    u->written = u->canon = intern(text);
    u->scale = scale;
    for (int q = 0; q < 8; q++) u->dim[q] = (int8_t)dim[q];
    return R.nunits++;
}

static int unit_find(const char *s, size_t n) {
    if (n >= 2 && s[0] == '[' && s[n - 1] == ']') { s++; n -= 2; }
    for (int i = 0; i < R.nunits; i++) {
        const char *w = SYM(R.units[i].written);
        if (strlen(w) == n && memcmp(w, s, n) == 0) return i;
    }
    return -1;
}
static int unit_of(TV s) {
    if (s.t != T_STR) fail("expected a unit name string");
    int u = unit_find(STR(s)->s, STR(s)->len);
    return u >= 0 ? u : unit_parse(STR(s)->s, STR(s)->len);
}

/* ---------------------------------------------------------------- formatting */

/* Rust's `{}` for f64: the shortest digits that round-trip, never in exponent form. */
static void fmt_shortest(double v, SB *b) {
    if (v != v) { sb_puts(b, "NaN"); return; }
    if (isinf(v)) { sb_puts(b, v < 0 ? "-inf" : "inf"); return; }
    if (v == 0) { sb_puts(b, signbit(v) ? "-0" : "0"); return; }
    char e[40];
    int prec;
    for (prec = 0; prec < 17; prec++) {
        snprintf(e, sizeof e, "%.*e", prec, v);
        if (strtod(e, NULL) == v) break;
    }
    char digits[24];
    int nd = 0;
    const char *p = e;
    if (*p == '-') { sb_putc(b, '-'); p++; }
    for (; *p && *p != 'e'; p++) if (*p != '.') digits[nd++] = *p;
    int ex = atoi(p + 1);
    while (nd > 1 && digits[nd - 1] == '0') nd--;
    if (ex < 0) {
        sb_puts(b, "0.");
        for (int i = 0; i < -ex - 1; i++) sb_putc(b, '0');
        sb_put(b, digits, (size_t)nd);
    } else {
        for (int i = 0; i <= ex; i++) sb_putc(b, i < nd ? digits[i] : '0');
        if (nd > ex + 1) { sb_putc(b, '.'); sb_put(b, digits + ex + 1, (size_t)(nd - ex - 1)); }
    }
}
static void fmt_float(double f, SB *b) {
    if (isfinite(f) && f == floor(f) && fabs(f) < 1e15) sb_printf(b, "%.1f", f);
    else fmt_shortest(f, b);
}
static void fmt_i64(int64_t v, SB *b) {
    char t[24];
    int n = 0;
    uint64_t u = v < 0 ? 0 - (uint64_t)v : (uint64_t)v;
    do { t[n++] = (char)('0' + u % 10); u /= 10; } while (u);
    if (v < 0) sb_putc(b, '-');
    while (n) sb_putc(b, t[--n]);
}
static void fmt_qty(Qty *q, SB *b) {
    double v = q->unit >= 0 ? q->si / R.units[q->unit].scale : q->si;
    double r = v;
    if (v != 0 && isfinite(v)) {
        int digits = 12 - (int)ceil(log10(fabs(v)));
        if (digits < 0) digits = 0;
        if (digits > 15) digits = 15;
        double p = pow(10, digits);
        r = round(v * p) / p;
    }
    if (r == floor(r) && fabs(r) < 1e15) fmt_i64((int64_t)r, b);
    else fmt_shortest(r, b);
    sb_putc(b, ' ');
    if (q->unit >= 0) sb_puts(b, SYM(R.units[q->unit].canon));
    else dim_name(q->dim, b);
}

static const char *type_name(TV v) {
    static const char *const N[] = {"unset", "nil", "bool", "int", "float", "str", "list", "dict", "range", "fn", "fn",
                                    "quantity", "struct", "object", "machine", "machine instance", "signal", "task",
                                    "native", "thunk", "env", "fn"};
    if (v.t == T_NATIVE) {
        static const char *const D[] = {"GPIO", "I2C", "SPI", "UART", "PWM", "Servo", "PID", "LinkNode"};
        return D[((Native *)v.u.o)->kind];
    }
    return N[v.t];
}

static void to_sb(TV v, SB *b, bool repr);
static void repr_str(Str *s, SB *b) {
    sb_putc(b, '"');
    for (uint32_t i = 0; i < s->len; i++) {
        char c = s->s[i];
        if (c == '"' || c == '\\') { sb_putc(b, '\\'); sb_putc(b, c); }
        else if (c == '\n') sb_puts(b, "\\n");
        else if (c == '\t') sb_puts(b, "\\t");
        else if (c == '\r') sb_puts(b, "\\r");
        else sb_putc(b, c);
    }
    sb_putc(b, '"');
}
static void to_sb(TV v, SB *b, bool repr) {
    switch (v.t) {
    case T_NIL: sb_puts(b, "nil"); break;
    case T_BOOL: sb_puts(b, v.u.i ? "true" : "false"); break;
    case T_INT: fmt_i64(v.u.i, b); break;
    case T_FLOAT: fmt_float(v.u.f, b); break;
    case T_STR: if (repr) repr_str(STR(v), b); else sb_put(b, STR(v)->s, STR(v)->len); break;
    case T_LIST:
        sb_putc(b, '[');
        for (uint32_t i = 0; i < LIST(v)->n; i++) { if (i) sb_puts(b, ", "); to_sb(LIST(v)->v[i], b, true); }
        sb_putc(b, ']');
        break;
    case T_DICT:
        sb_putc(b, '{');
        for (uint32_t i = 0; i < DICT(v)->n; i++) {
            if (i) sb_puts(b, ", ");
            to_sb(DICT(v)->k[i], b, true);
            sb_puts(b, ": ");
            to_sb(DICT(v)->v[i], b, true);
        }
        sb_putc(b, '}');
        break;
    case T_RANGE: {
        Range *r = (Range *)v.u.o;
        fmt_i64(r->a, b); sb_puts(b, r->incl ? "..=" : ".."); fmt_i64(r->b, b);
        break;
    }
    case T_FN: sb_printf(b, "<fn %s>", SYM(((Fn *)v.u.o)->def->a)); break;
    case T_BUILTIN: case T_MODFN: sb_puts(b, "<builtin>"); break;
    case T_QTY: fmt_qty(QTY(v), b); break;
    case T_CLASS: sb_printf(b, "<struct %s>", SYM(((Class *)v.u.o)->def->a)); break;
    case T_MTYPE: sb_printf(b, "<machine %s>", SYM(((Class *)v.u.o)->def->a)); break;
    case T_OBJ: {
        Inst *in = (Inst *)v.u.o;
        Node *fields = KID(in->cls->def, 0);
        sb_puts(b, SYM(in->cls->def->a));
        sb_putc(b, '(');
        for (int i = 0; i < in->n; i++) {
            if (i) sb_puts(b, ", ");
            sb_puts(b, SYM(KID(fields, i)->a));
            sb_puts(b, ": ");
            to_sb(in->f[i], b, true);
        }
        sb_putc(b, ')');
        break;
    }
    case T_MACH: {
        Mach *m = (Mach *)v.u.o;
        sb_printf(b, "<%s in %s>", SYM(m->ty->def->a), SYM(KID(KID(m->ty->def, 1), m->state)->a));
        break;
    }
    case T_SIGNAL: sb_puts(b, "signal("); to_sb(((Signal *)v.u.o)->value, b, true); sb_putc(b, ')'); break;
    case T_TASK: {
        uint8_t s = ((Task *)v.u.o)->state;
        sb_puts(b, s == 2 ? "<task done>" : s == 3 ? "<task failed>" : "<task pending>");
        break;
    }
    case T_NATIVE: {
        Native *n = (Native *)v.u.o;
        switch (n->kind) {
        case 0: sb_printf(b, "<GPIO %d %s>", (int)n->d.gpio.pin, n->d.gpio.output ? "out" : "in"); break;
        case 1: sb_printf(b, "<I2C bus %d @ 0x%02x>", (int)n->d.i2c.bus, (unsigned)n->d.i2c.addr); break;
        case 2: sb_printf(b, "<SPI %d.%d>", (int)n->d.spi.bus, (int)n->d.spi.cs); break;
        case 3: sb_printf(b, "<UART %s @ %d>", n->d.uart.port, (int)n->d.uart.baud); break;
        case 4:
            sb_printf(b, "<PWM %d.%d ", (int)n->d.pwm.chip, (int)n->d.pwm.channel);
            fmt_shortest(1e9 / (double)n->d.pwm.period_ns, b);
            sb_printf(b, " Hz duty %.1f%%>", 100.0 * (double)n->d.pwm.duty_ns / (double)n->d.pwm.period_ns);
            break;
        case 5:
            sb_printf(b, "<Servo %d.%d ", (int)n->d.servo.pwm.chip, (int)n->d.servo.pwm.channel);
            if (n->d.servo.attached) { sb_puts(b, "at "); fmt_shortest(n->d.servo.angle_deg, b); sb_puts(b, " deg>"); }
            else sb_puts(b, "detached>");
            break;
        case 6:
            sb_puts(b, "<PID kp="); fmt_shortest(n->d.pid[0], b);
            sb_puts(b, " ki="); fmt_shortest(n->d.pid[1], b);
            sb_puts(b, " kd="); fmt_shortest(n->d.pid[2], b);
            sb_putc(b, '>');
            break;
        default: sb_puts(b, "<LinkNode>"); break;
        }
        break;
    }
    default: sb_printf(b, "<%s>", type_name(v)); break;
    }
}

/* Display form; objects with a to_str() method use it. */
static void stringify(TV v, SB *b);
static TV to_str(TV v) {
    SB b = {0};
    stringify(v, &b);
    return sb_done(&b);
}

/* ---------------------------------------------------------------- comparison */

static bool truthy(TV v) {
    switch (v.t) {
    case T_NIL: case T_UNSET: return false;
    case T_BOOL: case T_INT: return v.u.i != 0;
    case T_FLOAT: return v.u.f != 0;
    case T_STR: return STR(v)->len != 0;
    case T_LIST: return LIST(v)->n != 0;
    case T_DICT: return DICT(v)->n != 0;
    case T_RANGE: { Range *r = (Range *)v.u.o; return r->incl ? r->a <= r->b : r->a < r->b; }
    default: return true;
    }
}
static bool is_num(TV v) { return v.t == T_INT || v.t == T_FLOAT; }
static bool as_f64(TV v, double *out) {
    if (v.t == T_INT || v.t == T_BOOL) { *out = (double)v.u.i; return true; }
    if (v.t == T_FLOAT) { *out = v.u.f; return true; }
    return false;
}
static bool as_i64(TV v, int64_t *out) {
    if (v.t == T_INT || v.t == T_BOOL) { *out = v.u.i; return true; }
    if (v.t == T_FLOAT && v.u.f == floor(v.u.f)) { *out = (int64_t)v.u.f; return true; }
    return false;
}
static bool equals(TV a, TV b) {
    if (is_num(a) && is_num(b)) {
        if (a.t == T_INT && b.t == T_INT) return a.u.i == b.u.i;
        double x, y;
        as_f64(a, &x); as_f64(b, &y);
        return x == y;
    }
    if (a.t != b.t) return false;
    switch (a.t) {
    case T_NIL: return true;
    case T_BOOL: return a.u.i == b.u.i;
    case T_STR: return STR(a)->len == STR(b)->len && memcmp(STR(a)->s, STR(b)->s, STR(a)->len) == 0;
    case T_LIST: {
        if (a.u.o == b.u.o) return true;
        if (LIST(a)->n != LIST(b)->n) return false;
        for (uint32_t i = 0; i < LIST(a)->n; i++) if (!equals(LIST(a)->v[i], LIST(b)->v[i])) return false;
        return true;
    }
    case T_DICT: {
        if (a.u.o == b.u.o) return true;
        if (DICT(a)->n != DICT(b)->n) return false;
        for (uint32_t i = 0; i < DICT(a)->n; i++) {
            int j = dict_find(DICT(b), DICT(a)->k[i]);
            if (j < 0 || !equals(DICT(a)->v[i], DICT(b)->v[j])) return false;
        }
        return true;
    }
    case T_RANGE: {
        Range *x = (Range *)a.u.o, *y = (Range *)b.u.o;
        return x->a == y->a && x->b == y->b && x->incl == y->incl;
    }
    case T_QTY: {
        Qty *x = QTY(a), *y = QTY(b);
        if (!dim_eq(x->dim, y->dim)) return false;
        double m = fmax(fmax(fabs(x->si), fabs(y->si)), 1e-300);
        return fabs(x->si - y->si) <= 1e-12 * m;
    }
    case T_BUILTIN: case T_MODFN: return a.u.i == b.u.i;
    default: return a.u.o == b.u.o;
    }
}

static bool as_q(TV v, double *si, const int8_t **dim, int32_t *unit) {
    if (v.t == T_QTY) { *si = QTY(v)->si; *dim = QTY(v)->dim; *unit = QTY(v)->unit; return true; }
    if (is_num(v)) { as_f64(v, si); *dim = DIMLESS; *unit = -1; return true; }
    return false;
}

static int compare(TV a, TV b) {
    if (a.t == T_QTY || b.t == T_QTY) {
        double x, y; const int8_t *dx, *dy; int32_t ux, uy;
        if (!as_q(a, &x, &dx, &ux) || !as_q(b, &y, &dy, &uy)) fail("cannot compare %s with %s", type_name(a), type_name(b));
        bool zx = is_dimless(dx) && x == 0, zy = is_dimless(dy) && y == 0;
        if (!dim_eq(dx, dy) && !zx && !zy)
            fail("unit mismatch: cannot compare %s with %s (write the bound with a unit, e.g. 2[m/s])", describe_dim(dx), describe_dim(dy));
        if (x != x || y != y) fail("cannot compare NaN");
        return x < y ? -1 : x > y;
    }
    if (a.t == T_INT && b.t == T_INT) return a.u.i < b.u.i ? -1 : a.u.i > b.u.i;
    if (a.t == T_STR && b.t == T_STR) {
        uint32_t n = STR(a)->len < STR(b)->len ? STR(a)->len : STR(b)->len;
        int c = memcmp(STR(a)->s, STR(b)->s, n);
        if (c) return c < 0 ? -1 : 1;
        return STR(a)->len < STR(b)->len ? -1 : STR(a)->len > STR(b)->len;
    }
    if (a.t == T_LIST && b.t == T_LIST) {
        uint32_t n = LIST(a)->n < LIST(b)->n ? LIST(a)->n : LIST(b)->n;
        for (uint32_t i = 0; i < n; i++) {
            int c = compare(LIST(a)->v[i], LIST(b)->v[i]);
            if (c) return c;
        }
        return LIST(a)->n < LIST(b)->n ? -1 : LIST(a)->n > LIST(b)->n;
    }
    double x, y;
    if (as_f64(a, &x) && as_f64(b, &y)) {
        if (x != x || y != y) fail("cannot compare NaN");
        return x < y ? -1 : x > y;
    }
    fail("cannot compare %s with %s", type_name(a), type_name(b));
}

/* ---------------------------------------------------------------- operators */

static int64_t div_euclid(int64_t a, int64_t b) {
    int64_t q = a / b;
    if (a % b < 0) q = b > 0 ? q - 1 : q + 1;
    return q;
}
static int64_t rem_euclid(int64_t a, int64_t b) {
    int64_t r = a % b;
    if (r < 0) r = b < 0 ? r - b : r + b;
    return r;
}
static double frem_euclid(double a, double b) {
    double r = fmod(a, b);
    return r < 0 ? r + fabs(b) : r;
}

static TV concat(TV a, TV b) {
    SB s = {0};
    to_sb(a, &s, false);
    to_sb(b, &s, false);
    return sb_done(&s);
}

static bool contains(TV coll, TV x) {
    switch (coll.t) {
    case T_LIST:
        for (uint32_t i = 0; i < LIST(coll)->n; i++) if (equals(LIST(coll)->v[i], x)) return true;
        return false;
    case T_STR:
        if (x.t != T_STR) fail("'in <str>' requires a string on the left");
        return strstr(STR(coll)->s, STR(x)->s) != NULL;
    case T_DICT: return dict_find(DICT(coll), x) >= 0;
    case T_RANGE: {
        Range *r = (Range *)coll.u.o;
        int64_t i;
        if (!as_i64(x, &i)) return false;
        return i >= r->a && (r->incl ? i <= r->b : i < r->b);
    }
    default: fail("'in' not supported for %s", type_name(coll));
    }
}

static TV quantity_binary(TV a, int op, TV b) {
    double x, y; const int8_t *dx, *dy; int32_t ux, uy;
    if (!as_q(a, &x, &dx, &ux) || !as_q(b, &y, &dy, &uy)) {
        if (op == OP_EQ) return mk_bool(false);
        if (op == OP_NE) return mk_bool(true);
        if (op == OP_ADD && (a.t == T_STR || b.t == T_STR)) return concat(a, b);
        fail("unsupported operand types for %s: %s and %s", OP_TEXT[op], type_name(a), type_name(b));
    }
    int8_t d[8];
    switch (op) {
    case OP_ADD: case OP_SUB:
        if (!dim_eq(dx, dy)) {
            if (is_dimless(dy) && y == 0) return a;
            if (is_dimless(dx) && x == 0 && op == OP_ADD) return b;
            fail("unit mismatch: cannot apply '%s' to %s and %s", OP_TEXT[op], describe_dim(dx), describe_dim(dy));
        }
        return mk_qty(op == OP_ADD ? x + y : x - y, dx, is_dimless(dx) ? uy : (ux >= 0 ? ux : uy));
    case OP_MUL:
        for (int i = 0; i < 8; i++) d[i] = (int8_t)(dx[i] + dy[i]);
        return mk_qty(x * y, d, is_dimless(dy) ? ux : is_dimless(dx) ? uy : -1);
    case OP_DIV:
        if (y == 0) fail("division by zero");
        for (int i = 0; i < 8; i++) d[i] = (int8_t)(dx[i] - dy[i]);
        return mk_qty(x / y, d, is_dimless(dy) ? ux : -1);
    case OP_MOD:
        if (!dim_eq(dx, dy)) fail("unit mismatch: cannot apply '%%' to %s and %s", describe_dim(dx), describe_dim(dy));
        return mk_qty(frem_euclid(x, y), dx, ux >= 0 ? ux : uy);
    case OP_POW: {
        if (!is_dimless(dy) || y != floor(y)) fail("a quantity can only be raised to an integer power");
        for (int i = 0; i < 8; i++) d[i] = (int8_t)(dx[i] * (int)y);
        return mk_qty(pow(x, (int)y), d, -1);
    }
    case OP_EQ: return mk_bool(equals(a, b));
    case OP_NE: return mk_bool(!equals(a, b));
    case OP_LT: return mk_bool(compare(a, b) < 0);
    case OP_LE: return mk_bool(compare(a, b) <= 0);
    case OP_GT: return mk_bool(compare(a, b) > 0);
    case OP_GE: return mk_bool(compare(a, b) >= 0);
    default: fail("operator '%s' is not defined for quantities", OP_TEXT[op]);
    }
}

static void overflow(TV a, int op, TV b) {
    SB s = {0};
    sb_puts(&s, "integer overflow in ");
    to_sb(a, &s, false); sb_putc(&s, ' '); sb_puts(&s, OP_TEXT[op]); sb_putc(&s, ' '); to_sb(b, &s, false);
    char buf[160];
    snprintf(buf, sizeof buf, "%s", s.p);
    free(s.p);
    fail("%s", buf);
}

static TV binary(TV a, int op, TV b) {
    if (a.t == T_QTY || b.t == T_QTY) return quantity_binary(a, op, b);
    if (op == OP_EQ) return mk_bool(equals(a, b));
    if (op == OP_NE) return mk_bool(!equals(a, b));
    if (a.t == T_INT && b.t == T_INT) {
        int64_t x = a.u.i, y = b.u.i, r;
        switch (op) {
        case OP_ADD: if (__builtin_add_overflow(x, y, &r)) overflow(a, op, b); return mk_int(r);
        case OP_SUB: if (__builtin_sub_overflow(x, y, &r)) overflow(a, op, b); return mk_int(r);
        case OP_MUL: if (__builtin_mul_overflow(x, y, &r)) overflow(a, op, b); return mk_int(r);
        case OP_DIV: if (y == 0) fail("division by zero"); if (x == INT64_MIN && y == -1) overflow(a, op, b); return mk_int(div_euclid(x, y));
        case OP_MOD: if (y == 0) fail("division by zero"); if (y == -1) return mk_int(0); return mk_int(rem_euclid(x, y));
        case OP_POW:
            if (y >= 0) {
                r = 1;
                for (int64_t i = 0; i < y; i++) if (__builtin_mul_overflow(r, x, &r)) overflow(a, op, b);
                return mk_int(r);
            }
            break;
        case OP_AND: return mk_int(x & y);
        case OP_OR: return mk_int(x | y);
        case OP_XOR: return mk_int(x ^ y);
        case OP_SHL: if (y < 0 || y > 63) overflow(a, op, b); return mk_int((int64_t)((uint64_t)x << y));
        case OP_SHR: if (y < 0 || y > 63) overflow(a, op, b); return mk_int(x >> y);
        default: break;
        }
    }
    if (is_num(a) && is_num(b) && op >= OP_ADD && op <= OP_POW) {
        double x, y;
        as_f64(a, &x); as_f64(b, &y);
        switch (op) {
        case OP_ADD: return mk_float(x + y);
        case OP_SUB: return mk_float(x - y);
        case OP_MUL: return mk_float(x * y);
        case OP_DIV: if (y == 0) fail("division by zero"); return mk_float(x / y);
        case OP_MOD: return mk_float(frem_euclid(x, y));
        default: return mk_float(pow(x, y));
        }
    }
    if (op == OP_ADD && (a.t == T_STR || b.t == T_STR)) return concat(a, b);
    if (op == OP_ADD && a.t == T_LIST && b.t == T_LIST) {
        List *l = list_new(LIST(a)->n + LIST(b)->n);
        memcpy(l->v, LIST(a)->v, LIST(a)->n * sizeof(TV));
        memcpy(l->v + LIST(a)->n, LIST(b)->v, LIST(b)->n * sizeof(TV));
        l->n = LIST(a)->n + LIST(b)->n;
        return mk_obj(&l->h);
    }
    if (op == OP_MUL && ((a.t == T_STR && b.t == T_INT) || (a.t == T_INT && b.t == T_STR))) {
        Str *s = a.t == T_STR ? STR(a) : STR(b);
        int64_t n = a.t == T_INT ? a.u.i : b.u.i;
        SB sb = {0};
        for (int64_t i = 0; i < n; i++) sb_put(&sb, s->s, s->len);
        return sb_done(&sb);
    }
    if (op == OP_MUL && ((a.t == T_LIST && b.t == T_INT) || (a.t == T_INT && b.t == T_LIST))) {
        List *src = a.t == T_LIST ? LIST(a) : LIST(b);
        int64_t n = a.t == T_INT ? a.u.i : b.u.i;
        List *l = list_new(0);
        for (int64_t i = 0; i < n; i++) for (uint32_t j = 0; j < src->n; j++) list_push(l, src->v[j]);
        return mk_obj(&l->h);
    }
    switch (op) {
    case OP_LT: return mk_bool(compare(a, b) < 0);
    case OP_LE: return mk_bool(compare(a, b) <= 0);
    case OP_GT: return mk_bool(compare(a, b) > 0);
    case OP_GE: return mk_bool(compare(a, b) >= 0);
    case OP_IN: return mk_bool(contains(b, a));
    case OP_NOTIN: return mk_bool(!contains(b, a));
    default: break;
    }
    fail("unsupported operand types for %s: %s and %s", OP_TEXT[op], type_name(a), type_name(b));
}

static TV unary(int op, TV v) {
    if (op == OP_BANG || op == OP_NOT) return mk_bool(!truthy(v));
    if (op == OP_SUB) {
        if (v.t == T_INT) { if (v.u.i == INT64_MIN) fail("integer overflow"); return mk_int(-v.u.i); }
        if (v.t == T_FLOAT) return mk_float(-v.u.f);
        if (v.t == T_QTY) return mk_qty(-QTY(v)->si, QTY(v)->dim, QTY(v)->unit);
    }
    if (op == OP_TILDE && v.t == T_INT) return mk_int(~v.u.i);
    fail("bad operand type for unary %s: %s", OP_TEXT[op], type_name(v));
}

/* ---------------------------------------------------------------- indexing */

static uint32_t norm_index(TV i, uint32_t len) {
    int64_t x;
    if (!as_i64(i, &x)) fail("index must be an integer, got %s", type_name(i));
    int64_t idx = x < 0 ? (int64_t)len + x : x;
    if (idx < 0 || idx >= (int64_t)len) fail("index %lld out of range for length %u", (long long)x, (unsigned)len);
    return (uint32_t)idx;
}
static void slice_bounds(Range *r, uint32_t len, uint32_t *s, uint32_t *e) {
    int64_t a = r->a < 0 ? (int64_t)len + r->a : r->a;
    int64_t b = r->incl ? r->b + 1 : r->b;
    if (b < 0) b = (int64_t)len + b;
    if (a < 0) a = 0;
    if (a > (int64_t)len) a = len;
    if (b < 0) b = 0;
    if (b > (int64_t)len) b = len;
    if (b < a) b = a;
    *s = (uint32_t)a;
    *e = (uint32_t)b;
}
/* Strings are UTF-8; indexing, slicing and iteration count characters, as on the host. */
static size_t u8_count(const char *s, size_t n) {
    size_t c = 0;
    for (size_t i = 0; i < n; i++) c += ((unsigned char)s[i] & 0xC0) != 0x80;
    return c;
}
static size_t u8_offset(const char *s, size_t n, size_t chars) { /* byte offset of character `chars` */
    size_t i = 0;
    while (i < n && chars) {
        i++;
        while (i < n && ((unsigned char)s[i] & 0xC0) == 0x80) i++;
        chars--;
    }
    return i;
}
static TV index_value(TV o, TV i) {
    uint32_t s, e;
    switch (o.t) {
    case T_LIST:
        if (i.t == T_RANGE) { slice_bounds((Range *)i.u.o, LIST(o)->n, &s, &e); return list_from(LIST(o)->v + s, e - s); }
        return LIST(o)->v[norm_index(i, LIST(o)->n)];
    case T_STR:
    {
        const char *p = STR(o)->s;
        size_t bytes = STR(o)->len, chars = u8_count(p, bytes);
        if (i.t == T_RANGE) {
            slice_bounds((Range *)i.u.o, (uint32_t)chars, &s, &e);
            size_t a = u8_offset(p, bytes, s), b = u8_offset(p, bytes, e);
            return str_n(p + a, b - a);
        }
        size_t a = u8_offset(p, bytes, norm_index(i, (uint32_t)chars));
        return str_n(p + a, u8_offset(p + a, bytes - a, 1));
    }
    case T_DICT: {
        int j = dict_find(DICT(o), i);
        if (j < 0) {
            SB b = {0};
            to_sb(i, &b, true);
            char buf[120];
            snprintf(buf, sizeof buf, "%s", b.p);
            free(b.p);
            fail("key %s not found", buf);
        }
        return DICT(o)->v[j];
    }
    case T_RANGE: {
        Range *r = (Range *)o.u.o;
        int64_t len = (r->incl ? r->b - r->a + 1 : r->b - r->a);
        return mk_int(r->a + norm_index(i, len < 0 ? 0 : (uint32_t)len));
    }
    default: fail("%s is not indexable", type_name(o));
    }
}
static void set_index(TV o, TV i, TV v) {
    if (o.t == T_LIST) LIST(o)->v[norm_index(i, LIST(o)->n)] = v;
    else if (o.t == T_DICT) dict_set(DICT(o), i, v);
    else fail("%s does not support item assignment", type_name(o));
}

/* Items of an iterable, as a new list. For dicts: keys, or [key, value] pairs when arity > 1. */
static TV iterate(TV v, int arity) {
    switch (v.t) {
    case T_RANGE: {
        Range *r = (Range *)v.u.o;
        int64_t end = r->incl ? r->b + 1 : r->b;
        List *l = list_new(end > r->a ? (uint32_t)(end - r->a) : 0);
        for (int64_t i = r->a; i < end; i++) list_push(l, mk_int(i));
        return mk_obj(&l->h);
    }
    case T_LIST: return list_from(LIST(v)->v, LIST(v)->n);
    case T_STR: {
        List *l = list_new(STR(v)->len);
        push(mk_obj(&l->h));
        for (size_t i = 0; i < STR(v)->len;) {
            size_t k = u8_offset(STR(v)->s + i, STR(v)->len - i, 1);
            list_push(l, str_n(STR(v)->s + i, k));
            i += k;
        }
        R.sp--;
        return mk_obj(&l->h);
    }
    case T_DICT: {
        List *l = list_new(DICT(v)->n);
        push(mk_obj(&l->h));
        for (uint32_t i = 0; i < DICT(v)->n; i++) {
            if (arity > 1) { TV pair[2] = {DICT(v)->k[i], DICT(v)->v[i]}; list_push(l, list_from(pair, 2)); }
            else list_push(l, DICT(v)->k[i]);
        }
        R.sp--;
        return mk_obj(&l->h);
    }
    default: fail("cannot iterate over %s", type_name(v));
    }
}

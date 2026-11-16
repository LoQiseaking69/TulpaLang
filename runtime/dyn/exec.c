/* ---------------------------------------------------------------- time and reactive rules */

static double now_secs(void) { return (double)tulpa_hal_micros() / 1e6; }

static bool has_timed(void) {
    if (R.ndeadlines) return true;
    for (int i = 0; i < R.nrules; i++) if (R.rules[i].has_hold) return true;
    return false;
}

static void react_inner(void) {
    for (int round = 0; round < 100; round++) {
        bool fired = false;
        for (int i = 0; i < R.nrules; i++) {
            bool raw = truthy(eval(R.rules[i].cond, R.rules[i].env));
            Rule *r = &R.rules[i]; /* re-read: a rule body may register more rules */
            bool now = raw;
            if (r->has_hold) {
                if (raw) {
                    double t = now_secs();
                    if (!r->has_since) { r->has_since = true; r->since = t; }
                    now = t - r->since >= r->hold - 1e-9;
                } else {
                    r->has_since = false;
                    now = false;
                }
            }
            if (now != r->last) {
                r->last = now;
                Node *block = now ? r->body : r->els;
                if (block && block->k != K_NONE) {
                    exec_block(block, r->env);
                    R.ctl = C_NONE;
                    fired = true;
                }
            }
        }
        for (int i = 0; i < R.nmonitors; i++) {
            bool ok = truthy(eval(R.monitors[i].expr, R.monitors[i].env));
            Monitor *m = &R.monitors[i];
            if (ok) { m->tripped = false; continue; }
            if (m->tripped) continue;
            m->tripped = true;
            if (!m->handler || m->handler->k == K_NONE) fail("safety envelope violated: always %s", SYM(m->text));
            exec_block(m->handler, m->env);
            R.ctl = C_NONE;
            fired = true;
        }
        for (int i = 0; i < R.ndeadlines; i++) {
            bool trig = truthy(eval(R.deadlines[i].trigger, R.deadlines[i].env));
            Deadline *d = &R.deadlines[i];
            double now = now_secs();
            if (trig && !d->last) { d->armed = true; d->armed_at = now; }
            d->last = trig;
            if (!d->armed) continue;
            bool held = truthy(eval(d->resp, d->env));
            d = &R.deadlines[i];
            if (held) { d->armed = false; continue; }
            if (now - d->armed_at > d->within + 1e-9) {
                d->armed = false;
                if (!d->handler || d->handler->k == K_NONE)
                    fail("deadline missed: response '%s' did not hold within %s of the trigger", SYM(d->text), d->shown->s);
                exec_block(d->handler, d->env);
                R.ctl = C_NONE;
                fired = true;
            }
        }
        if (!fired) return;
    }
    fail("reactive rules did not settle after 100 rounds (a when-rule keeps re-triggering itself)");
}

static void react(void) {
    if (R.reacting || !has_reactive()) return;
    R.reacting = true;
    react_inner();
    R.reacting = false;
}

static void sleep_secs(double secs) {
    if (secs < 0) secs = 0;
    if (!has_timed()) {
        tulpa_sleep(secs);
        react();
        return;
    }
    /* timed rules (`when ... for`, deadlines) are checked every millisecond */
    uint64_t end = tulpa_hal_micros() + (uint64_t)(secs * 1e6 + 0.5);
    for (;;) {
        uint64_t now = tulpa_hal_micros();
        if (now >= end) break;
        tulpa_hal_delay_us(end - now < 1000 ? end - now : 1000);
        react();
    }
}

/* ---------------------------------------------------------------- tasks */

static TV force_task(Task *t) {
    switch (t->state) {
    case 0: {
        t->state = 1;
        Try tr = {.sp = R.sp, .depth = R.depth, .line = R.line, .reacting = R.reacting};
        if (R.ntry >= 32) fail("try blocks nested too deeply");
        R.tries[R.ntry++] = &tr;
        if (setjmp(tr.jb)) {
            R.ntry--;
            R.sp = tr.sp;
            R.depth = tr.depth;
            R.reacting = tr.reacting;
            R.ctl = C_NONE;
            char msg[240];
            snprintf(msg, sizeof msg, "%s", R.errmsg);
            t->err = (Str *)str_c(msg).u.o;
            t->state = 3;
            fail("%s", msg);
        }
        TV r = call(t->f, (int)t->args->n, t->args->v, 0, NULL, NULL);
        R.ntry--;
        t->result = r;
        t->state = 2;
        return r;
    }
    case 2: return t->result;
    case 3: fail("awaited task failed: %s", t->err->s);
    default: fail("task awaited itself (cycle)");
    }
}
static TV await_value(TV v) {
    if (v.t == T_TASK) return force_task((Task *)v.u.o);
    if (v.t == T_LIST) {
        List *out = list_new(LIST(v)->n);
        push(mk_obj(&out->h));
        for (uint32_t i = 0; i < LIST(v)->n; i++) list_push(out, await_value(LIST(v)->v[i]));
        R.sp--;
        return mk_obj(&out->h);
    }
    return v;
}
static void drain_tasks(void) {
    while (R.tasks->n) { /* tasks may spawn more tasks */
        List *batch = R.tasks;
        push(mk_obj(&batch->h));
        R.tasks = list_new(0);
        for (uint32_t i = 0; i < batch->n; i++) force_task((Task *)batch->v[i].u.o);
        R.sp--;
    }
}

/* ---------------------------------------------------------------- imports */

static TV import_module(int ix) {
    if (R.module_vals[ix].t != T_UNSET) return R.module_vals[ix];
    Node *m = MODULE(ix);
    const char *path = SYM(m->b);
    if (R.module_state[ix]) fail("circular import of \"%s\"", path);
    R.module_state[ix] = 1;
    Env *env = env_new(NULL);
    push(mk_obj(&env->h));
    Try t = {.sp = R.sp, .depth = R.depth, .line = R.line, .reacting = R.reacting};
    if (R.ntry >= 32) fail("try blocks nested too deeply");
    R.tries[R.ntry++] = &t;
    if (setjmp(t.jb)) {
        R.ntry--;
        R.sp = t.sp;
        R.depth = t.depth;
        R.module_state[ix] = 0;
        char msg[sizeof R.errmsg];
        snprintf(msg, sizeof msg, "%s", R.errmsg);
        fail("in module \"%s\": %s", path, msg);
    }
    run_block(m, env);
    R.ntry--;
    if (R.ctl == C_BREAK || R.ctl == C_CONT) { R.ctl = C_NONE; fail("break/continue at top level of module \"%s\"", path); }
    R.ctl = C_NONE;
    Dict *d = dict_new();
    R.module_vals[ix] = mk_obj(&d->h);
    for (int i = 0; i < env->n; i++)
        if (SYM(env->names[i])[0] != '_') dict_set(d, sym_str(env->names[i]), env->vals[i]);
    R.sp--;
    return R.module_vals[ix];
}

/* ---------------------------------------------------------------- patterns */

static bool matches(Node *p, TV v, Env *scope) {
    switch (p->k) {
    case K_PWILD: return true;
    case K_PBIND: define(scope, p->a, v, false); return true;
    case K_PLIT: return equals(eval(KID(p, 0), scope), v);
    case K_PRANGE: {
        double x, lo, hi;
        if (!as_f64(v, &x) || !as_f64(eval(KID(p, 0), scope), &lo) || !as_f64(eval(KID(p, 1), scope), &hi)) return false;
        return x >= lo && (p->a > 0 ? x <= hi : x < hi);
    }
    case K_PLIST: {
        if (v.t != T_LIST) return false;
        List *l = LIST(v);
        if ((p->b <= 0 && l->n != p->n) || l->n < p->n) return false;
        for (int i = 0; i < p->n; i++) if (!matches(KID(p, i), l->v[i], scope)) return false;
        if (p->b == 2) define(scope, p->a, list_from(l->v + p->n, l->n - p->n), false);
        return true;
    }
    default: return false;
    }
}

/* ---------------------------------------------------------------- macros */

static TV macro_arg(Node *a, Env *e) { return a->k == K_BLOCK ? exec_block(a, e) : eval(a, e); }

static TV rust_format(const char *fmt, int argc, TV *argv);

static __attribute__((noinline)) TV unwrap_or_default(Node *n, Env *e) {
    Try t = {.sp = R.sp, .depth = R.depth, .line = R.line, .reacting = R.reacting};
    if (R.ntry >= 32) fail("try blocks nested too deeply");
    R.tries[R.ntry++] = &t;
    if (!setjmp(t.jb)) {
        TV v = macro_arg(KID(n, 0), e);
        R.ntry--;
        if (R.ctl || v.t != T_NIL) return v;
    } else {
        R.ntry--;
        R.sp = t.sp;
        R.depth = t.depth;
        R.reacting = t.reacting;
        R.ctl = C_NONE;
    }
    return macro_arg(KID(n, 1), e);
}
static __attribute__((noinline)) void assert_eq_failed(TV x, TV y) {
    char sx[90], sy[90];
    repr_buf(x, sx, sizeof sx);
    repr_buf(y, sy, sizeof sy);
    fail("assertion failed: %s != %s", sx, sy);
}

static TV macro_call(Node *n, Env *e) {
    const char *name = SYM(n->a);
    int argc = n->n, base = R.sp;
#define ARGC(k) do { if (argc != (k)) fail("%s! expects %d argument(s), got %d", name, (k), argc); } while (0)
#define MA(dst, node) do { dst = macro_arg(node, e); if (R.ctl) { R.sp = base; return NIL; } } while (0)
    if (!strcmp(name, "simple_loop") || !strcmp(name, "repeat")) {
        ARGC(2);
        TV c;
        int64_t count;
        MA(c, KID(n, 0));
        if (!as_i64(c, &count)) fail("%s! count must be an integer", name);
        for (int64_t i = 0; i < count; i++) {
            safepoint();
            macro_arg(KID(n, 1), e);
            if (R.ctl == C_BREAK) { R.ctl = C_NONE; break; }
            if (R.ctl == C_CONT) R.ctl = C_NONE;
            if (R.ctl) return NIL;
        }
        return NIL;
    }
    if (!strcmp(name, "every")) {
        if (argc != 2 && argc != 3) fail("every! expects (period_ms, body) or (period_ms, count, body)");
        TV pv, cv;
        double period;
        int64_t count = -1;
        MA(pv, KID(n, 0));
        if (pv.t == T_QTY) {
            if (!seconds_of(pv, &period)) fail("every! period must be milliseconds or a time quantity like 20[ms]");
            period *= 1000;
        } else if (!as_f64(pv, &period)) fail("every! period must be milliseconds or a time quantity like 20[ms]");
        if (argc == 3) {
            MA(cv, KID(n, 1));
            if (!as_i64(cv, &count)) fail("every! count must be an integer");
        }
        double dt = (period < 0 ? 0 : period) / 1000.0, next = now_secs();
        int64_t i = 0;
        while (count < 0 || i < count) {
            safepoint();
            macro_arg(KID(n, argc - 1), e);
            if (R.ctl == C_BREAK) { R.ctl = C_NONE; break; }
            if (R.ctl == C_CONT) R.ctl = C_NONE;
            if (R.ctl) return NIL;
            i++;
            react();
            next += dt;
            double now = now_secs();
            if (next > now) sleep_secs(next - now);
            else next = now; /* overran the period; don't try to catch up */
        }
        return mk_int(i);
    }
    if (!strcmp(name, "time_execution")) {
        ARGC(1);
        uint64_t t0 = tulpa_hal_micros();
        TV v;
        MA(v, KID(n, 0));
        char buf[64];
        snprintf(buf, sizeof buf, "Execution time: %.3fms\n", (double)(tulpa_hal_micros() - t0) / 1000.0);
        tulpa_print_str(buf);
        return v;
    }
    if (!strcmp(name, "log_message")) {
        ARGC(1);
        TV m;
        MA(m, KID(n, 0));
        SB b = {0};
        uint64_t s = tulpa_hal_micros() / 1000000u;
        sb_printf(&b, "[%02u:%02u:%02u] ", (unsigned)(s / 3600 % 24), (unsigned)(s / 60 % 60), (unsigned)(s % 60));
        to_sb(m, &b, false);
        sb_putc(&b, '\n');
        tulpa_print_str(b.p);
        free(b.p);
        return NIL;
    }
    if (!strcmp(name, "println") || !strcmp(name, "print") || !strcmp(name, "format")) {
        for (int i = 0; i < argc; i++) { TV v; MA(v, KID(n, i)); push(v); }
        TV s;
        if (!argc) s = str_c("");
        else {
            if (R.stack[base].t != T_STR) fail("%s! first argument must be a format string, got %s", name, type_name(R.stack[base]));
            s = rust_format(STR(R.stack[base])->s, argc - 1, &R.stack[base + 1]);
        }
        R.sp = base;
        if (name[0] == 'f') return s;
        tulpa_print_str(STR(s)->s);
        if (name[5] == 'l') tulpa_print_nl();
        return NIL;
    }
    if (!strcmp(name, "vec")) {
        for (int i = 0; i < argc; i++) { TV v; MA(v, KID(n, i)); push(v); }
        TV l = list_from(&R.stack[base], (uint32_t)argc);
        R.sp = base;
        return l;
    }
    if (!strcmp(name, "assert_eq")) {
        if (argc < 2) fail("assert_eq! expects two values");
        TV x, y;
        MA(x, KID(n, 0));
        push(x);
        MA(y, KID(n, 1));
        R.sp = base;
        if (!equals(x, y)) assert_eq_failed(x, y);
        return NIL;
    }
    if (!strcmp(name, "assert")) {
        ARGC(1);
        TV v;
        MA(v, KID(n, 0));
        if (!truthy(v)) fail("assertion failed");
        return NIL;
    }
    if (!strcmp(name, "dbg")) {
        ARGC(1);
        TV v;
        MA(v, KID(n, 0));
        return v; /* the host prints dbg! to stderr; firmware has one console, so it stays quiet */
    }
    if (!strcmp(name, "unwrap_or_default")) {
        ARGC(2);
        return unwrap_or_default(n, e);
    }
    if (!strcmp(name, "define_test")) {
        ARGC(2); /* tests run on the host with `tulpa test`; firmware skips them */
        return NIL;
    }
    for (int i = 0; i < R.nmacros; i++)
        if (R.macros[i].name == n->a) {
            Node *def = R.macros[i].def, *params = KID(def, 0);
            if (params->n != argc) fail("%s! expects %d argument(s), got %d", name, params->n, argc);
            Env *scope = env_new(e);
            push(mk_obj(&scope->h));
            for (int k = 0; k < argc; k++) {
                Thunk *t = (Thunk *)alloc(T_THUNK, sizeof(Thunk));
                t->node = KID(n, k);
                t->env = e;
                define(scope, KID(params, k)->a, mk_obj(&t->h), false);
            }
            TV r = run_block(KID(def, 1), scope);
            R.sp = base;
            return r;
        }
    fail("unknown macro '%s!'", name);
#undef ARGC
#undef MA
}

/* ---------------------------------------------------------------- statements and expressions */

/* Binds the names in KID(n, first..) to the items of list `v`. */
static __attribute__((noinline)) void destructure(Env *e, Node *n, int first, TV v, bool konst) {
    int names = n->n - first;
    if (v.t != T_LIST) fail("cannot unpack %s into %d names", type_name(v), names);
    if ((int)LIST(v)->n != names) fail("cannot unpack %u values into %d names", (unsigned)LIST(v)->n, names);
    for (int i = 0; i < names; i++) define(e, KID(n, first + i)->a, LIST(v)->v[i], konst);
}

static bool assign_var(Env *e, int32_t name, TV v) {
    for (; e; e = e->up)
        for (int i = 0; i < e->n; i++)
            if (e->names[i] == name) {
                if (e->konst[i]) fail("cannot assign to constant '%s'", SYM(name));
                e->vals[i] = v;
                return true;
            }
    return false;
}

static TV global_value(int code) {
    switch (code) {
    case -2: return mk_float(3.14159265358979323846);
    case -3: return mk_float(6.28318530717958647692);
    case -4: return mk_float(2.71828182845904523536);
    case -5: return mk_int(1);
    default: return mk_int(0);
    }
}

/* Evaluates call arguments onto the stack: positionals first, then keyword values. */
static __attribute__((noinline)) void no_match(TV v) {
    char shown[100];
    repr_buf(v, shown, sizeof shown);
    fail("no match arm matched value %s", shown);
}
#define MAX_KW 12
static __attribute__((noinline)) int eval_args(Node *n, int first, Env *e, int *nkw, int32_t *kwn) {
    int total = n->n - first, base = R.sp, npos = 0;
    *nkw = 0;
    if (total > 32) fail("too many call arguments");
    int32_t tag[32];
    for (int i = 0; i < total; i++) {
        Node *arg = KID(n, first + i);
        tag[i] = arg->k == K_NAMED ? arg->a : -1;
        TV v = eval(arg->k == K_NAMED ? KID(arg, 0) : arg, e);
        if (R.ctl) return -1;
        push(v);
        if (tag[i] < 0) npos++;
        else {
            if (*nkw >= MAX_KW) fail("too many keyword arguments");
            kwn[(*nkw)++] = tag[i];
        }
    }
    if (*nkw) {
        TV tmp[32];
        int p = 0, k = npos;
        for (int i = 0; i < total; i++) tmp[tag[i] < 0 ? p++ : k++] = R.stack[base + i];
        memcpy(&R.stack[base], tmp, (size_t)total * sizeof(TV));
    }
    return npos;
}

static TV ev_call(Node *n, Env *e) __attribute__((noinline));
static TV ev_bind(Node *n, Env *e) __attribute__((noinline));
static TV ev_loop(Node *n, Env *e) __attribute__((noinline));
static TV ev_decl(Node *n, Env *e) __attribute__((noinline));
static TV ev_misc_k_fstr(Node *n, Env *e) __attribute__((noinline));
static TV ev_misc_k_path(Node *n, Env *e) __attribute__((noinline));
static TV ev_misc_k_list(Node *n, Env *e) __attribute__((noinline));
static TV ev_misc_k_repeat(Node *n, Env *e) __attribute__((noinline));
static TV ev_misc_k_dict(Node *n, Env *e) __attribute__((noinline));
static TV ev_misc_k_range(Node *n, Env *e) __attribute__((noinline));
static TV ev_misc_k_macro(Node *n, Env *e) __attribute__((noinline));
static TV ev_misc_k_match(Node *n, Env *e) __attribute__((noinline));
static TV ev_misc_k_spawn(Node *n, Env *e) __attribute__((noinline));
static TV ev_misc_k_await(Node *n, Env *e) __attribute__((noinline));
static TV ev_misc_k_named(Node *n, Env *e) __attribute__((noinline));
static TV ev_misc_k_assert(Node *n, Env *e) __attribute__((noinline));
static TV ev_misc_k_try(Node *n, Env *e) __attribute__((noinline));
static TV ev_call(Node *n, Env *e) {
    int base = R.sp;
    TV a, b;
    (void)base; (void)a; (void)b; (void)e;
    switch (n->k) {
    case K_CALL: {
        int nkw, npos;
        int32_t kwn[MAX_KW];
        EV(a, KID(n, 0));
        push(a);
        npos = eval_args(n, 1, e, &nkw, kwn);
        if (npos < 0) { R.sp = base; return NIL; }
        if (LINE(n)) R.line = LINE(n);
        a = call(a, npos, &R.stack[base + 1], nkw, kwn, &R.stack[base + 1 + npos]);
        R.sp = base;
        return a;
    }
    case K_MCALL: {
        int nkw, npos;
        int32_t kwn[MAX_KW];
        EV(a, KID(n, 0));
        push(a);
        npos = eval_args(n, 1, e, &nkw, kwn);
        if (npos < 0) { R.sp = base; return NIL; }
        if (LINE(n)) R.line = LINE(n);
        a = call_method(a, n->a, npos, &R.stack[base + 1], nkw, kwn, &R.stack[base + 1 + npos]);
        R.sp = base;
        return a;
    }
    default: return NIL;
    }
}

static TV ev_bind(Node *n, Env *e) {
    int base = R.sp;
    TV a, b;
    (void)base; (void)a; (void)b; (void)e;
    switch (n->k) {
    case K_LET: {
        if (LINE(n)) R.line = LINE(n);
        EV(a, KID(n, 0));
        int names = n->n - 1;
        if (names == 1) define(e, KID(n, 1)->a, a, n->a > 0);
        else destructure(e, n, 1, a, n->a > 0);
        return NIL;
    }
    case K_ASSIGN: {
        Node *t = KID(n, 0);
        if (LINE(n)) R.line = LINE(n);
        TV rhs;
        EV(rhs, KID(n, 1));
        push(rhs);
        if (t->k == K_ID) {
            if (n->a > 0) {
                TV *cur = lookup(e, t->a);
                if (!cur) fail("undefined variable '%s'", SYM(t->a));
                rhs = binary_op(*cur, n->a, rhs);
            }
            if (!assign_var(e, t->a, rhs))
                fail("assignment to undeclared variable '%s' (use 'let %s = ...')", SYM(t->a), SYM(t->a));
        } else if (t->k == K_INDEX) {
            EV(a, KID(t, 0));
            push(a);
            EV(b, KID(t, 1));
            push(b);
            if (n->a > 0) rhs = binary_op(index_value(a, b), n->a, rhs);
            set_index(a, b, rhs);
        } else {
            EV(a, KID(t, 0));
            push(a);
            if (n->a > 0) {
                b = get_field(a, t->a);
                push(b);
                rhs = binary_op(b, n->a, rhs);
                push(rhs);
            }
            set_field(a, t->a, rhs);
        }
        R.sp = base;
        react();
        return NIL;
    }
    default: return NIL;
    }
}

static TV ev_loop(Node *n, Env *e) {
    int base = R.sp;
    TV a, b;
    (void)base; (void)a; (void)b; (void)e;
    switch (n->k) {
    case K_WHILE: {
        Node *invs = KID(n, 1);
        for (;;) {
            safepoint();
            if (LINE(n)) R.line = LINE(n);
            for (int i = 0; i < invs->n; i++) {
                EV(a, KID(KID(invs, i), 0));
                if (!truthy(a)) fail("loop invariant violated: %s", SYM(KID(invs, i)->a));
            }
            EV(a, KID(n, 0));
            if (!truthy(a)) break;
            exec_block(KID(n, 2), e);
            if (R.ctl == C_BREAK) { R.ctl = C_NONE; break; }
            if (R.ctl == C_CONT) R.ctl = C_NONE;
            if (R.ctl) return NIL;
        }
        return NIL;
    }
    case K_LOOP:
        for (;;) {
            safepoint();
            exec_block(KID(n, 0), e);
            if (R.ctl == C_BREAK) { R.ctl = C_NONE; break; }
            if (R.ctl == C_CONT) R.ctl = C_NONE;
            if (R.ctl) return NIL;
        }
        return NIL;
    case K_FOR: {
        int names = n->n - 2;
        if (LINE(n)) R.line = LINE(n);
        EV(a, KID(n, 0));
        push(a);
        Node *body = KID(n, 1);
        /* a range is walked without building its item list */
        bool is_range = a.t == T_RANGE;
        int64_t lo = 0, hi = 0;
        List *items = NULL;
        if (is_range) {
            Range *r = (Range *)a.u.o;
            lo = r->a;
            hi = r->incl ? r->b + 1 : r->b;
        } else {
            b = iterate(a, names);
            push(b);
            items = LIST(b);
            hi = items->n;
        }
        int top = R.sp;
        for (int64_t i = lo; i < hi; i++) {
            safepoint();
            TV item = is_range ? mk_int(i) : items->v[i];
            Env *scope = env_new(e);
            push(mk_obj(&scope->h));
            if (names == 1) define(scope, KID(n, 2)->a, item, false);
            else {
                if (LINE(n)) R.line = LINE(n);
                destructure(scope, n, 2, item, false);
            }
            run_block(body, scope);
            R.sp = top;
            if (R.ctl == C_BREAK) { R.ctl = C_NONE; break; }
            if (R.ctl == C_CONT) R.ctl = C_NONE;
            if (R.ctl) { R.sp = base; return NIL; }
        }
        R.sp = base;
        return NIL;
    }
    default: return NIL;
    }
}

static TV ev_decl(Node *n, Env *e) {
    int base = R.sp;
    TV a, b;
    (void)base; (void)a; (void)b; (void)e;
    switch (n->k) {
    case K_STRUCT: {
        Node *impls = KID(n, 3);
        for (int i = 0; i < impls->n; i++) {
            int32_t iface = KID(impls, i)->a;
            if (!is_iface(iface))
                fail("struct %s implements unknown interface '%s' (declare it, or import the module that does)", SYM(n->a), SYM(iface));
            /* every method of the interface and of the interfaces it extends */
            for (int k = 0; k < R.nifaces; k++) {
                Node *def = R.ifaces[k];
                if (!iface_is(iface, def->a)) continue;
                Node *sigs = KID(def, 1);
                for (int s = 0; s < sigs->n; s++) {
                    Node *m = find_method(n, KID(sigs, s)->a);
                    if (!m) {
                        SB ps = {0};
                        sb_puts(&ps, "");
                        Node *names = KID(KID(sigs, s), 1);
                        for (int q = 0; q < names->n; q++) { if (q) sb_puts(&ps, ", "); sb_puts(&ps, SYM(KID(names, q)->a)); }
                        char shown[100];
                        snprintf(shown, sizeof shown, "%s", ps.p);
                        free(ps.p);
                        fail("struct %s implements %s but has no method %s(%s)", SYM(n->a), SYM(def->a), SYM(KID(sigs, s)->a), shown);
                    }
                    if (KID(m, 0)->n != KID(sigs, s)->b)
                        fail("%s.%s() takes %d parameter(s) but %s.%s declares %d", SYM(n->a), SYM(m->a), KID(m, 0)->n,
                             SYM(def->a), SYM(m->a), (int)KID(sigs, s)->b);
                }
            }
        }
        Class *c = (Class *)alloc(T_CLASS, sizeof(Class));
        c->def = n;
        c->env = e;
        define(e, n->a, mk_obj(&c->h), false);
        return NIL;
    }
    case K_IFACE: {
        Node *ext = KID(n, 0);
        if (LINE(n)) R.line = LINE(n);
        for (int i = 0; i < ext->n; i++)
            if (!is_iface(KID(ext, i)->a)) fail("interface %s extends unknown interface '%s'", SYM(n->a), SYM(KID(ext, i)->a));
        int j;
        for (j = 0; j < R.nifaces; j++) if (R.ifaces[j]->a == n->a) break;
        if (j == R.nifaces) R.ifaces = xrealloc(R.ifaces, ++R.nifaces * sizeof(Node *));
        R.ifaces[j] = n;
        SB sb = {0};
        sb_printf(&sb, "<interface %s>", SYM(n->a));
        define(e, n->a, sb_done(&sb), false);
        return NIL;
    }
    case K_MACHINE: {
        Class *c = (Class *)alloc(T_MTYPE, sizeof(Class));
        c->def = n;
        c->env = e;
        define(e, n->a, mk_obj(&c->h), false);
        return NIL;
    }
    case K_WHEN: {
        double hold = 0;
        if (LINE(n)) R.line = LINE(n);
        if (KID(n, 1)->k != K_NONE) {
            EV(a, KID(n, 1));
            if (!seconds_of(a, &hold)) fail("`when ... for` needs a duration, e.g. 50[ms] or 0.05");
        }
        R.rules = xrealloc(R.rules, (R.nrules + 1) * sizeof(Rule));
        R.rules[R.nrules++] = (Rule){.cond = KID(n, 0), .body = KID(n, 2), .els = KID(n, 3), .env = e,
                                     .has_hold = KID(n, 1)->k != K_NONE, .hold = hold};
        react();
        return NIL;
    }
    case K_ALWAYS:
        if (LINE(n)) R.line = LINE(n);
        R.monitors = xrealloc(R.monitors, (R.nmonitors + 1) * sizeof(Monitor));
        R.monitors[R.nmonitors++] = (Monitor){.expr = KID(n, 0), .handler = KID(n, 1), .env = e, .text = n->a};
        react();
        return NIL;
    case K_DEADLINE: {
        double secs;
        if (LINE(n)) R.line = LINE(n);
        EV(a, KID(n, 1));
        if (!seconds_of(a, &secs)) fail("`within` needs a duration, e.g. 200[ms] or 0.2");
        R.deadlines = xrealloc(R.deadlines, (R.ndeadlines + 1) * sizeof(Deadline));
        R.deadlines[R.ndeadlines] = (Deadline){.trigger = KID(n, 0), .resp = KID(n, 2), .handler = KID(n, 3), .env = e,
                                               .text = n->a, .within = secs};
        R.deadlines[R.ndeadlines].shown = (Str *)to_str(a).u.o;
        R.ndeadlines++;
        react();
        return NIL;
    }
    case K_IMPORT: {
        if (LINE(n)) R.line = LINE(n);
        a = import_module(n->a);
        Node *names = KID(n, 0);
        if (!names->n) {
            define(e, n->b >= 0 ? n->b : MODULE(n->a)->a, a, false);
            return NIL;
        }
        for (int i = 0; i < names->n; i++) {
            int32_t nm = KID(names, i)->a;
            int ix = dict_find_c(DICT(a), SYM(nm));
            if (ix < 0) fail("module \"%s\" has no export '%s'", SYM(MODULE(n->a)->b), SYM(nm));
            define(e, nm, DICT(a)->v[ix], false);
        }
        return NIL;
    }
    default: return NIL;
    }
}

static TV ev_misc_k_fstr(Node *n, Env *e) {
    int base = R.sp;
    TV a, b;
    (void)base; (void)a; (void)b; (void)e;
    switch (n->k) {
    case K_FSTR: {
        SB sb = {0};
        for (int i = 0; i < n->n; i++) {
            if (KID(n, i)->k == K_STR) { sb_puts(&sb, SYM(KID(n, i)->a)); continue; }
            a = eval(KID(n, i), e);
            if (R.ctl) { free(sb.p); return NIL; }
            push(a);
            stringify(a, &sb);
            R.sp = base;
        }
        return sb_done(&sb);
    }
    default: return NIL;
    }
}

static TV ev_misc_k_path(Node *n, Env *e) {
    int base = R.sp;
    TV a, b;
    (void)base; (void)a; (void)b; (void)e;
    switch (n->k) {
    case K_PATH:
        if (R.path_cache[n->u] >= 0) { TV f = {.t = T_BUILTIN}; f.u.i = R.path_cache[n->u]; return f; }
        if (!path_value(SYM(n->a), SYM(n->b), &a)) {
            if (LINE(n)) R.line = LINE(n);
            fail("unknown item '%s::%s'", SYM(n->a), SYM(n->b));
        }
        if (a.t == T_BUILTIN) R.path_cache[n->u] = (int16_t)a.u.i;
        return a;
    default: return NIL;
    }
}

static TV ev_misc_k_list(Node *n, Env *e) {
    int base = R.sp;
    TV a, b;
    (void)base; (void)a; (void)b; (void)e;
    switch (n->k) {
    case K_LIST:
        for (int i = 0; i < n->n; i++) { EV(a, KID(n, i)); push(a); }
        a = list_from(&R.stack[base], n->n);
        R.sp = base;
        return a;
    default: return NIL;
    }
}

static TV ev_misc_k_repeat(Node *n, Env *e) {
    int base = R.sp;
    TV a, b;
    (void)base; (void)a; (void)b; (void)e;
    switch (n->k) {
    case K_REPEAT: {
        int64_t count;
        EV(a, KID(n, 0));
        push(a);
        EV(b, KID(n, 1));
        if (!as_i64(b, &count)) fail("repeat count must be an integer");
        List *l = list_new(count > 0 ? (uint32_t)count : 0);
        for (int64_t i = 0; i < count; i++) list_push(l, a);
        R.sp = base;
        return mk_obj(&l->h);
    }
    default: return NIL;
    }
}

static TV ev_misc_k_dict(Node *n, Env *e) {
    int base = R.sp;
    TV a, b;
    (void)base; (void)a; (void)b; (void)e;
    switch (n->k) {
    case K_DICT: {
        Dict *d = dict_new();
        push(mk_obj(&d->h));
        for (int i = 0; i + 1 < n->n; i += 2) {
            EV(a, KID(n, i));
            push(a);
            EV(b, KID(n, i + 1));
            dict_set(d, a, b);
            R.sp = base + 1;
        }
        R.sp = base;
        return mk_obj(&d->h);
    }
    default: return NIL;
    }
}

static TV ev_misc_k_range(Node *n, Env *e) {
    int base = R.sp;
    TV a, b;
    (void)base; (void)a; (void)b; (void)e;
    switch (n->k) {
    case K_RANGE: {
        int64_t lo, hi;
        EV(a, KID(n, 0));
        EV(b, KID(n, 1));
        if (!as_i64(a, &lo) || !as_i64(b, &hi)) fail("range bounds must be integers");
        Range *r = (Range *)alloc(T_RANGE, sizeof(Range));
        r->a = lo;
        r->b = hi;
        r->incl = n->a > 0;
        return mk_obj(&r->h);
    }
    default: return NIL;
    }
}

static TV ev_misc_k_macro(Node *n, Env *e) {
    int base = R.sp;
    TV a, b;
    (void)base; (void)a; (void)b; (void)e;
    switch (n->k) {
    case K_MACRO:
        if (LINE(n)) R.line = LINE(n);
        return macro_call(n, e);
    default: return NIL;
    }
}

static TV ev_misc_k_match(Node *n, Env *e) {
    int base = R.sp;
    TV a, b;
    (void)base; (void)a; (void)b; (void)e;
    switch (n->k) {
    case K_MATCH: {
        EV(a, KID(n, 0));
        push(a);
        for (int i = 1; i < n->n; i++) {
            Node *arm = KID(n, i), *pats = KID(arm, 0);
            for (int p = 0; p < pats->n; p++) {
                Env *scope = env_new(e);
                push(mk_obj(&scope->h));
                if (matches(KID(pats, p), a, scope) && (KID(arm, 1)->k == K_NONE || truthy(eval(KID(arm, 1), scope)))) {
                    b = run_block(KID(arm, 2), scope);
                    R.sp = base;
                    return b;
                }
                R.sp = base + 1;
            }
        }
        if (LINE(n)) R.line = LINE(n);
        no_match(a);
        return NIL;
    }
    default: return NIL;
    }
}

static TV ev_misc_k_spawn(Node *n, Env *e) {
    int base = R.sp;
    TV a, b;
    (void)base; (void)a; (void)b; (void)e;
    switch (n->k) {
    case K_SPAWN: {
        Node *x = KID(n, 0);
        List *args = list_new(0);
        push(mk_obj(&args->h));
        if (x->k == K_CALL) {
            EV(a, KID(x, 0));
            push(a);
            for (int i = 1; i < x->n; i++) { EV(b, KID(x, i)); list_push(args, b); }
        } else {
            EV(a, x);
            push(a);
        }
        if (!callable(a)) {
            if (LINE(n)) R.line = LINE(n);
            fail("cannot spawn a %s", type_name(a));
        }
        Task *t = (Task *)alloc(T_TASK, sizeof(Task));
        t->f = a;
        t->args = args;
        t->result = NIL;
        list_push(R.tasks, mk_obj(&t->h));
        R.sp = base;
        return mk_obj(&t->h);
    }
    default: return NIL;
    }
}

static TV ev_misc_k_await(Node *n, Env *e) {
    int base = R.sp;
    TV a, b;
    (void)base; (void)a; (void)b; (void)e;
    switch (n->k) {
    case K_AWAIT:
        EV(a, KID(n, 0));
        push(a);
        if (LINE(n)) R.line = LINE(n);
        a = await_value(a);
        R.sp = base;
        return a;
    default: return NIL;
    }
}

static TV ev_misc_k_named(Node *n, Env *e) {
    int base = R.sp;
    TV a, b;
    (void)base; (void)a; (void)b; (void)e;
    switch (n->k) {
    case K_NAMED: fail("keyword argument '%s:' is only allowed inside a call", SYM(n->a));

    default: return NIL;
    }
}

static TV ev_misc_k_assert(Node *n, Env *e) {
    int base = R.sp;
    TV a, b;
    (void)base; (void)a; (void)b; (void)e;
    switch (n->k) {
    case K_ASSERT:
        if (LINE(n)) R.line = LINE(n);
        EV(a, KID(n, 0));
        if (!truthy(a)) {
            char extra[120] = "";
            if (KID(n, 1)->k != K_NONE) {
                EV(b, KID(n, 1));
                extra[0] = ':';
                extra[1] = ' ';
                show_buf(b, extra + 2, sizeof extra - 2);
            }
            if (LINE(n)) R.line = LINE(n);
            fail("assertion failed: %s%s", SYM(n->a), extra);
        }
        return NIL;
    default: return NIL;
    }
}

static TV ev_misc_k_try(Node *n, Env *e) {
    int base = R.sp;
    TV a, b;
    (void)base; (void)a; (void)b; (void)e;
    switch (n->k) {
    case K_TRY: {
        Try t = {.sp = R.sp, .depth = R.depth, .line = R.line, .reacting = R.reacting};
        if (R.ntry >= 32) fail("try blocks nested too deeply");
        R.tries[R.ntry++] = &t;
        if (!setjmp(t.jb)) {
            a = exec_block(KID(n, 0), e);
            R.ntry--;
            return a;
        }
        R.ntry--;
        R.sp = t.sp;
        R.depth = t.depth;
        R.reacting = t.reacting;
        R.ctl = C_NONE;
        Env *scope = env_new(e);
        push(mk_obj(&scope->h));
        if (n->a >= 0) define(scope, n->a, str_c(R.errmsg), false);
        a = exec_block(KID(n, 1), scope);
        R.sp = t.sp;
        return a;
    }
    default: return NIL;
    }
}

static TV eval(Node *n, Env *e) {
    int base = R.sp;
    TV a, b;
    switch (n->k) {
    case K_NONE: case K_NIL: return NIL;
    case K_INT: return mk_int(num_i(n));
    case K_FLOAT: return mk_float(num_f(n));
    case K_BOOL: return mk_bool(n->a > 0);
    case K_STR: return sym_str(n->a);
    case K_ID: {
        TV *v = lookup(e, n->a);
        if (v) {
            if (v->t != T_THUNK) return *v;
            Thunk *t = (Thunk *)v->u.o;
            return macro_arg(t->node, t->env);
        }
        int g = R.sym_builtin[n->a];
        if (g >= 0) { TV f = {.t = T_BUILTIN}; f.u.i = g; return f; }
        if (g < -1) return global_value(g);
        if (LINE(n)) R.line = LINE(n);
        fail("undefined variable '%s'", SYM(n->a));
    }
    case K_UNARY:
        EV(a, KID(n, 0));
        if (LINE(n)) R.line = LINE(n);
        return unary(n->a, a);
    case K_BIN:
        EV(a, KID(n, 0));
        if (a.t >= T_STR) {
            push(a);
            EV(b, KID(n, 1));
            push(b);
        } else EV(b, KID(n, 1));
        if (LINE(n)) R.line = LINE(n);
        a = binary_op(a, n->a, b);
        R.sp = base;
        return a;
    case K_LOGIC:
        EV(a, KID(n, 0));
        if (truthy(a) != (n->a > 0)) return a;
        return eval(KID(n, 1), e);
    case K_INDEX:
        EV(a, KID(n, 0));
        push(a);
        EV(b, KID(n, 1));
        if (LINE(n)) R.line = LINE(n);
        a = index_value(a, b);
        R.sp = base;
        return a;
    case K_FIELD:
        EV(a, KID(n, 0));
        if (LINE(n)) R.line = LINE(n);
        push(a);
        a = get_field(a, n->a);
        R.sp = base;
        return a;
    case K_LAMBDA: return fn_new(KID(n, 0), e);
    case K_FN:
        define(e, n->a, fn_new(n, e), false);
        return NIL;
    case K_IF:
        EV(a, KID(n, 0));
        if (truthy(a)) return exec_block(KID(n, 1), e);
        if (KID(n, 2)->k == K_NONE) return NIL;
        return KID(n, 2)->k == K_BLOCK ? exec_block(KID(n, 2), e) : eval(KID(n, 2), e);
    case K_BLOCK: return exec_block(n, e);
    case K_QTY: return mk_qty(num_f(n) * R.units[n->a].scale, R.units[n->a].dim, n->a);
    case K_RETURN:
        if (LINE(n)) R.line = LINE(n);
        a = NIL;
        if (n->n && KID(n, 0)->k != K_NONE) EV(a, KID(n, 0));
        R.ret = a;
        R.ctl = C_RET;
        return NIL;
    case K_BREAK: R.ctl = C_BREAK; return NIL;
    case K_CONTINUE: R.ctl = C_CONT; return NIL;
    case K_MACRODEF: return NIL; /* registered when its block was entered */
    case K_CALL: case K_MCALL: return ev_call(n, e);
    case K_LET: case K_ASSIGN: return ev_bind(n, e);
    case K_WHILE: case K_LOOP: case K_FOR: return ev_loop(n, e);
    case K_STRUCT: case K_IFACE: case K_MACHINE: case K_WHEN: case K_ALWAYS: case K_DEADLINE: case K_IMPORT: return ev_decl(n, e);
    case K_FSTR: return ev_misc_k_fstr(n, e);
    case K_PATH: return ev_misc_k_path(n, e);
    case K_LIST: return ev_misc_k_list(n, e);
    case K_REPEAT: return ev_misc_k_repeat(n, e);
    case K_DICT: return ev_misc_k_dict(n, e);
    case K_RANGE: return ev_misc_k_range(n, e);
    case K_MACRO: return ev_misc_k_macro(n, e);
    case K_MATCH: return ev_misc_k_match(n, e);
    case K_SPAWN: return ev_misc_k_spawn(n, e);
    case K_AWAIT: return ev_misc_k_await(n, e);
    case K_NAMED: return ev_misc_k_named(n, e);
    case K_ASSERT: return ev_misc_k_assert(n, e);
    case K_TRY: return ev_misc_k_try(n, e);
    default: fail("internal: cannot evaluate node kind %d", n->k);
    }
}

/* ---------------------------------------------------------------- entry point */

void tulpa_dyn_run(const uint8_t *program, int64_t len) {
    memset(&R, 0, sizeof R);
    char here;
    R.stack_base = &here;
    R.stack_limit = TULPA_DYN_C_STACK;
    R.threshold = TULPA_DYN_GC_BYTES;
    R.rng = (tulpa_hal_micros() * 2654435761u) | 1;
    load(program, (size_t)len);
    R.s_self = intern("self");
    R.s_result = intern("result'");
    R.s_init = intern("init");
    R.s_to_str = intern("to_str");
    R.globals = env_new(NULL);
    R.tasks = list_new(0);
    R.keep = list_new(0);
    R.ret = NIL;
    run_block(MODULE(0), R.globals);
    R.ctl = C_NONE;
    drain_tasks();
}

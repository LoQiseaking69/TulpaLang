/* ---------------------------------------------------------------- program loader */

static uint32_t rd32(const uint8_t *p) { return (uint32_t)p[0] | (uint32_t)p[1] << 8 | (uint32_t)p[2] << 16 | (uint32_t)p[3] << 24; }

/* The image is used where it lies: only the lookup caches and the unit table are copied. */
static void load(const uint8_t *p, size_t len) {
    if (len < 40 || memcmp(p, "TDYN\x02", 5) != 0) fail("not a Tulpa program image (built by a different tulpa version?)");
    if (((uintptr_t)p & 7) != 0) fail("the program image must be 8-byte aligned");
    uint32_t nsyms = rd32(p + 8), nunits = rd32(p + 12), nmodules = rd32(p + 16), nnodes = rd32(p + 20);
    uint32_t nkids = rd32(p + 24), nnums = rd32(p + 28), npaths = rd32(p + 32), nstr = rd32(p + 36);
    const uint8_t *q = p + 40;
    R.nums = q;
    q += (size_t)nnums * 8;
    R.units = xmalloc((nunits + 1) * sizeof(UnitDef));
    memcpy(R.units, q, (size_t)nunits * sizeof(UnitDef));
    R.nunits = (int)nunits;
    q += (size_t)nunits * 24;
    R.roots = (const uint32_t *)(const void *)q;
    q += (size_t)nmodules * 4;
    R.sym_off = (const uint32_t *)(const void *)q;
    q += (size_t)nsyms * 4;
    R.nodes = (Node *)(const void *)q;
    q += (size_t)nnodes * 16;
    R.kids = (const uint32_t *)(const void *)q;
    q += (size_t)nkids * 4;
    R.lines = (const uint16_t *)(const void *)q;
    q += (size_t)nnodes * 2;
    R.strs = (const char *)q;
    if ((size_t)(q - p) + nstr > len) fail("truncated program image");
    R.nsyms_image = R.nsyms = (int)nsyms;
    R.sym_builtin = xmalloc((nsyms + 1) * sizeof(int16_t));
    R.symstr = xmalloc((nsyms + 1) * sizeof(Obj *));
    for (uint32_t i = 0; i < nsyms; i++) {
        R.sym_builtin[i] = (int16_t)global_find(sym_name((int32_t)i));
        R.symstr[i] = NULL;
    }
    R.path_cache = xmalloc((npaths + 1) * sizeof(int16_t));
    for (uint32_t i = 0; i < npaths; i++) R.path_cache[i] = -2;
    R.nmodules = (int)nmodules;
    R.module_vals = xmalloc((nmodules + 1) * sizeof(TV));
    R.module_state = xmalloc(nmodules + 1);
    for (uint32_t i = 0; i < nmodules; i++) {
        R.module_vals[i] = UNSET;
        R.module_state[i] = 0;
    }
}
#define MODULE(i) (&R.nodes[R.roots[i]])
static int64_t num_i(Node *n) { int64_t v; memcpy(&v, R.nums + (size_t)n->u * 8, 8); return v; }
static double num_f(Node *n) { double v; memcpy(&v, R.nums + (size_t)n->u * 8, 8); return v; }

/* ---------------------------------------------------------------- small helpers */

#define KEEP(v) push(v)
#define EV(dst, node) do { dst = eval(node, e); if (R.ctl) { R.sp = base; return NIL; } } while (0)

static TV sym_str(int32_t s) {
    if (!R.symstr[s]) R.symstr[s] = str_c(SYM(s)).u.o;
    return mk_obj(R.symstr[s]);
}
static void repr_buf(TV v, char *out, size_t n) {
    SB b = {0};
    to_sb(v, &b, true);
    snprintf(out, n, "%s", b.p ? b.p : "");
    free(b.p);
}
static void show_buf(TV v, char *out, size_t n) {
    SB b = {0};
    to_sb(v, &b, false);
    snprintf(out, n, "%s", b.p ? b.p : "");
    free(b.p);
}
static bool callable(TV v) { return v.t == T_FN || v.t == T_BUILTIN; }

static TV fn_new(Node *def, Env *env) {
    Fn *f = (Fn *)alloc(T_FN, sizeof(Fn));
    f->def = def;
    f->env = env;
    return mk_obj(&f->h);
}

static bool seconds_of(TV v, double *out) {
    static const int8_t T[8] = {0, 0, 1};
    if (v.t == T_QTY) { if (!dim_eq(QTY(v)->dim, T)) return false; *out = QTY(v)->si; return true; }
    return as_f64(v, out);
}
static bool radians_of(TV v, double *out) {
    static const int8_t A[8] = {0, 0, 0, 0, 0, 0, 0, 1};
    if (v.t == T_QTY) { if (!dim_eq(QTY(v)->dim, A)) return false; *out = QTY(v)->si; return true; }
    return as_f64(v, out);
}

/* ---------------------------------------------------------------- types */

static bool satisfies(Node *impls, int32_t want) {
    int32_t todo[24];
    int n = 0, seen = 0;
    for (int i = 0; i < impls->n && n < 24; i++) todo[n++] = KID(impls, i)->a;
    while (seen < n) {
        int32_t name = todo[seen++];
        if (name == want) return true;
        for (int i = 0; i < R.nifaces; i++)
            if (R.ifaces[i]->a == name) {
                Node *ext = KID(R.ifaces[i], 0);
                for (int j = 0; j < ext->n && n < 24; j++) {
                    bool dup = false;
                    for (int k = 0; k < n; k++) dup |= todo[k] == KID(ext, j)->a;
                    if (!dup) todo[n++] = KID(ext, j)->a;
                }
            }
    }
    return false;
}
/* Whether interface `name` is, or extends, `want`. */
static bool iface_is(int32_t name, int32_t want) {
    if (name == want) return true;
    for (int i = 0; i < R.nifaces; i++)
        if (R.ifaces[i]->a == name) return satisfies(KID(R.ifaces[i], 0), want);
    return false;
}
static bool is_iface(int32_t name) {
    for (int i = 0; i < R.nifaces; i++) if (R.ifaces[i]->a == name) return true;
    return false;
}

static bool type_matches(Node *ty, TV v) {
    bool optional = ty->b > 0, ok = false;
    if (ty->k == K_TUNIT) ok = v.t == T_QTY && dim_eq(QTY(v)->dim, R.units[ty->a].dim);
    else {
        const char *n = SYM(ty->a);
        if (!strcmp(n, "any")) ok = true;
        else if (!strcmp(n, "int")) ok = v.t == T_INT;
        else if (!strcmp(n, "float")) ok = v.t == T_FLOAT;
        else if (!strcmp(n, "num") || !strcmp(n, "number")) ok = is_num(v);
        else if (!strcmp(n, "str")) ok = v.t == T_STR;
        else if (!strcmp(n, "bool")) ok = v.t == T_BOOL;
        else if (!strcmp(n, "list")) ok = v.t == T_LIST;
        else if (!strcmp(n, "dict")) ok = v.t == T_DICT;
        else if (!strcmp(n, "fn")) ok = callable(v) || v.t == T_CLASS;
        else if (!strcmp(n, "nil")) ok = v.t == T_NIL;
        else if (!strcmp(n, "task")) ok = v.t == T_TASK;
        else if (!strcmp(n, "range")) ok = v.t == T_RANGE;
        else if (!strcmp(n, "quantity")) ok = v.t == T_QTY;
        else if (!strcmp(n, "signal")) ok = v.t == T_SIGNAL;
        else if (v.t == T_OBJ) {
            Node *def = ((Inst *)v.u.o)->cls->def;
            ok = def->a == ty->a || (is_iface(ty->a) && satisfies(KID(def, 3), ty->a));
        } else if (v.t == T_MACH) ok = ((Mach *)v.u.o)->ty->def->a == ty->a;
        else if (v.t == T_NATIVE) ok = !strcmp(type_name(v), n);
    }
    return ok || (optional && v.t == T_NIL);
}
__attribute__((noinline)) static void check_type(Node *ty, TV v, const char *what) {
    if (type_matches(ty, v)) return;
    char want[80], got[120], hint[120] = "", shown[64];
    const char *opt = ty->b > 0 ? "?" : "";
    if (ty->k == K_TUNIT) {
        char kind[48];
        snprintf(kind, sizeof kind, "%s", describe_dim(R.units[ty->a].dim));
        char *br = strstr(kind, " [");
        if (br) *br = 0;
        snprintf(want, sizeof want, "[%s] (%s)%s", SYM(R.units[ty->a].canon), kind, opt);
    } else snprintf(want, sizeof want, "%s%s", SYM(ty->a), opt);
    if (v.t == T_QTY) {
        show_buf(v, shown, sizeof shown);
        snprintf(got, sizeof got, "%s = %s", describe_dim(QTY(v)->dim), shown);
    } else {
        repr_buf(v, shown, sizeof shown);
        snprintf(got, sizeof got, "%s %s", type_name(v), shown);
    }
    if (ty->k == K_TUNIT && is_num(v)) {
        show_buf(v, shown, sizeof shown);
        snprintf(hint, sizeof hint, " (did you mean %s[%s]?)", shown, SYM(R.units[ty->a].canon));
    }
    fail("type error: %s expects %s, got %s%s", what, want, got, hint);
}

/* ---------------------------------------------------------------- calls */

static void hoist(Node *b, Env *e) {
    for (int i = 0; i < b->n; i++) {
        Node *s = KID(b, i);
        if (s->k == K_FN) define(e, s->a, fn_new(s, e), false);
        else if (s->k == K_MACRODEF) {
            int j;
            for (j = 0; j < R.nmacros; j++) if (R.macros[j].name == s->a) break;
            if (j == R.nmacros) R.macros = xrealloc(R.macros, ++R.nmacros * sizeof *R.macros);
            R.macros[j].name = s->a;
            R.macros[j].def = s;
        }
    }
}

/* Set at build time: whether the block declares names of its own. */
static bool needs_scope(Node *b) { return b->numt != 2; }
static TV run_block(Node *b, Env *e) {
    hoist(b, e);
    TV last = NIL;
    for (int i = 0; i < b->n; i++) {
        last = eval(KID(b, i), e);
        if (R.ctl) return NIL;
    }
    return last;
}
static TV exec_block(Node *b, Env *parent) {
    if (!needs_scope(b)) return run_block(b, parent);
    Env *e = env_new(parent);
    push(mk_obj(&e->h));
    TV r = run_block(b, e);
    R.sp--;
    return r;
}

#define NOINLINE __attribute__((noinline))
/* Error paths live in their own functions so the interpreter's recursive frames stay small. */
static NOINLINE void arity_error(Node *def, int required, int argc) {
    char want[24];
    int np = KID(def, 0)->n;
    if (required == np) snprintf(want, sizeof want, "%d", required);
    else snprintf(want, sizeof want, "%d..%d", required, np);
    fail("%s() takes %s argument%s, got %d", SYM(def->a), want, np == 1 ? "" : "s", argc);
}
static NOINLINE void no_param_error(Node *def, int32_t kw) {
    Node *params = KID(def, 0);
    SB b = {0};
    sb_puts(&b, "");
    for (int j = 0; j < params->n; j++) { if (j) sb_puts(&b, ", "); sb_puts(&b, SYM(KID(params, j)->a)); }
    char names[120];
    snprintf(names, sizeof names, "%s", b.p);
    free(b.p);
    fail("%s() has no parameter '%s' (parameters: %s)", SYM(def->a), SYM(kw), names);
}
static NOINLINE void check_named(Node *ty, TV v, const char *fmt, const char *x, const char *y) {
    if (ty->k == K_NONE || type_matches(ty, v)) return;
    char what[96];
    snprintf(what, sizeof what, fmt, x, y);
    check_type(ty, v, what);
}
static NOINLINE void check_ensures(Node *def, Env *closure, Env *env, TV *initial, TV value) {
    Node *params = KID(def, 0), *ens = KID(def, 2);
    Env *post = env_new(closure);
    push(mk_obj(&post->h));
    char prime[72];
    for (int i = 0; i < params->n; i++) {
        int32_t pn = KID(params, i)->a;
        TV *fin = lookup(env, pn);
        snprintf(prime, sizeof prime, "%s'", SYM(pn));
        define(post, pn, initial[i], false);
        define(post, intern(prime), fin ? *fin : NIL, false);
    }
    define(post, R.s_result, value, false);
    for (int i = 0; i < ens->n; i++)
        if (!truthy(eval(KID(KID(ens, i), 0), post))) {
            char shown[80];
            repr_buf(value, shown, sizeof shown);
            fail("postcondition violated in %s(): ensure %s (result' = %s)", SYM(def->a), SYM(KID(ens, i)->a), shown);
        }
    R.sp--;
}

/* The C stack is the real limit on recursion depth, so it is checked on every call. */
static NOINLINE void stack_check(void) {
    char here;
    size_t used = R.stack_base > &here ? (size_t)(R.stack_base - &here) : (size_t)(&here - R.stack_base);
    if (used > R.stack_limit) fail("stack overflow: calls nested too deeply for this board's stack (%u bytes)", (unsigned)R.stack_limit);
}

static NOINLINE TV call_def(Node *def, Env *closure, int argc, TV *argv, int nkw, int32_t *kwn, TV *kwv) {
    Node *params = KID(def, 0);
    int np = params->n, required = 0, base = R.sp;
    for (int i = 0; i < np; i++) required += KID(KID(params, i), 0)->k == K_NONE;
    if ((!nkw && argc < required) || argc > np) arity_error(def, required, argc);
    for (int i = 0; i < np; i++) push(i < argc ? argv[i] : UNSET);
    TV *slot = &R.stack[base];
    for (int k = 0; k < nkw; k++) {
        int i;
        for (i = 0; i < np; i++) if (KID(params, i)->a == kwn[k]) break;
        if (i == np) no_param_error(def, kwn[k]);
        if (i < argc) fail("%s() got multiple values for '%s'", SYM(def->a), SYM(kwn[k]));
        slot[i] = kwv[k];
    }
    if (R.depth >= TULPA_DYN_DEPTH) fail("maximum recursion depth (%d) exceeded", TULPA_DYN_DEPTH);
    stack_check();
    Env *env = env_new(closure);
    push(mk_obj(&env->h));
    for (int i = 0; i < np; i++) {
        Node *p = KID(params, i);
        TV v = slot[i];
        if (v.t == T_UNSET) {
            if (KID(p, 0)->k == K_NONE) fail("%s() missing argument '%s'", SYM(def->a), SYM(p->a));
            v = eval(KID(p, 0), env);
        }
        if (KID(p, 1)->k != K_NONE) check_named(KID(p, 1), v, "%s() parameter '%s'", SYM(def->a), SYM(p->a));
        define(env, p->a, v, false);
        slot[i] = v; /* the initial value, for postconditions */
    }
    Node *req = KID(def, 1);
    for (int i = 0; i < req->n; i++)
        if (!truthy(eval(KID(KID(req, i), 0), env)))
            fail("precondition violated in %s(): require %s", SYM(def->a), SYM(KID(req, i)->a));
    R.depth++;
    safepoint();
    TV value = run_block(KID(def, 4), env);
    R.depth--;
    if (R.ctl == C_RET) { value = R.ret; R.ret = NIL; R.ctl = C_NONE; }
    else if (R.ctl) { R.ctl = C_NONE; fail("'break' or 'continue' outside of a loop"); }
    push(value);
    if (KID(def, 3)->k != K_NONE) check_named(KID(def, 3), value, "%s() return value%s", SYM(def->a), "");
    if (KID(def, 2)->n) check_ensures(def, closure, env, slot, value);
    R.sp = base;
    return value;
}

static TV construct(Class *c, int argc, TV *argv, int nkw, int32_t *kwn, TV *kwv);
static TV instantiate(Class *c, int argc, TV *argv, int nkw, int32_t *kwn, TV *kwv);

static TV call(TV f, int argc, TV *argv, int nkw, int32_t *kwn, TV *kwv) {
    switch (f.t) {
    case T_FN: return call_def(((Fn *)f.u.o)->def, ((Fn *)f.u.o)->env, argc, argv, nkw, kwn, kwv);
    case T_BUILTIN:
        if (nkw) fail("builtin %s() does not take keyword arguments", builtin_name((int)f.u.i));
        return builtin_call((int)f.u.i, argc, argv);
    case T_CLASS: return construct((Class *)f.u.o, argc, argv, nkw, kwn, kwv);
    case T_MTYPE: return instantiate((Class *)f.u.o, argc, argv, nkw, kwn, kwv);
    default: fail("%s is not callable", type_name(f));
    }
}
static TV call1(TV f, TV a) { return call(f, 1, &a, 0, NULL, NULL); }
static TV call2(TV f, TV a, TV b) {
    TV v[2] = {a, b};
    return call(f, 2, v, 0, NULL, NULL);
}

/* ---------------------------------------------------------------- structs */

static Node *find_method(Node *def, int32_t name) {
    Node *ms = KID(def, 1);
    for (int i = 0; i < ms->n; i++) if (KID(ms, i)->a == name) return KID(ms, i);
    return NULL;
}
static int field_ix(Node *def, int32_t name) {
    Node *fs = KID(def, 0);
    for (int i = 0; i < fs->n; i++) if (KID(fs, i)->a == name) return i;
    return -1;
}
/* Calls a method, passing `self` when the method declares it. */
static TV call_with_self(Node *m, Env *env, TV self, int argc, TV *argv, int nkw, int32_t *kwn, TV *kwv) {
    Node *ps = KID(m, 0);
    if (!ps->n || KID(ps, 0)->a != R.s_self) return call_def(m, env, argc, argv, nkw, kwn, kwv);
    int base = R.sp;
    push(self);
    for (int i = 0; i < argc; i++) push(argv[i]);
    TV r = call_def(m, env, argc + 1, &R.stack[base], nkw, kwn, kwv);
    R.sp = base;
    return r;
}

NOINLINE static void check_invariants(TV o, const char *prefix) {
    Inst *in = (Inst *)o.u.o;
    Node *invs = KID(in->cls->def, 2);
    if (!invs->n) return;
    Env *scope = env_new(in->cls->env);
    push(mk_obj(&scope->h));
    define(scope, R.s_self, o, false);
    for (int i = 0; i < invs->n; i++)
        if (!truthy(eval(KID(KID(invs, i), 0), scope))) {
            char now[120];
            show_buf(o, now, sizeof now);
            fail("%sstruct invariant violated for %s: %s (now %s)", prefix, SYM(in->cls->def->a), SYM(KID(invs, i)->a), now);
        }
    R.sp--;
}

NOINLINE static TV construct(Class *c, int argc, TV *argv, int nkw, int32_t *kwn, TV *kwv) {
    Node *def = c->def, *fs = KID(def, 0);
    const char *name = SYM(def->a);
    int base = R.sp;
    if (argc > fs->n) fail("%s() takes at most %d field values, got %d", name, fs->n, argc);
    for (int k = 0; k < nkw; k++)
        if (field_ix(def, kwn[k]) < 0) {
            SB b = {0};
            for (int j = 0; j < fs->n; j++) { if (j) sb_puts(&b, ", "); sb_puts(&b, SYM(KID(fs, j)->a)); }
            char names[120];
            snprintf(names, sizeof names, "%s", b.p ? b.p : "");
            free(b.p);
            fail("%s has no field '%s' (fields: %s)", name, SYM(kwn[k]), names);
        }
    Env *scope = env_new(c->env);
    push(mk_obj(&scope->h));
    Inst *in = (Inst *)alloc(T_OBJ, sizeof(Inst) + fs->n * sizeof(TV));
    in->cls = c;
    for (int i = 0; i < fs->n; i++) in->f[i] = NIL;
    in->n = (uint16_t)fs->n;
    TV obj = mk_obj(&in->h);
    push(obj);
    for (int i = 0; i < fs->n; i++) {
        Node *p = KID(fs, i);
        TV v = UNSET;
        if (i < argc) v = argv[i];
        for (int k = 0; k < nkw; k++)
            if (kwn[k] == p->a) {
                if (v.t != T_UNSET) fail("%s(): field '%s' given twice", name, SYM(p->a));
                v = kwv[k];
            }
        if (v.t == T_UNSET) {
            if (KID(p, 0)->k == K_NONE) fail("%s(): missing field '%s'", name, SYM(p->a));
            v = eval(KID(p, 0), scope);
        }
        check_named(KID(p, 1), v, "%s.%s", name, SYM(p->a));
        define(scope, p->a, v, false);
        in->f[i] = v;
    }
    char prefix[80];
    snprintf(prefix, sizeof prefix, "constructing %s: ", name);
    check_invariants(obj, prefix);
    Node *init = find_method(def, R.s_init);
    if (init) {
        call_def(init, c->env, 1, &obj, 0, NULL, NULL);
        check_invariants(obj, "");
    }
    R.sp = base;
    return obj;
}

/* ---------------------------------------------------------------- state machines */

static bool has_reactive(void) { return R.nrules || R.nmonitors || R.ndeadlines; }

NOINLINE static TV instantiate(Class *c, int argc, TV *argv, int nkw, int32_t *kwn, TV *kwv) {
    Node *def = c->def, *params = KID(def, 0);
    const char *name = SYM(def->a);
    int np = params->n, base = R.sp;
    if (argc > np) fail("%s() takes %d argument(s), got %d", name, np, argc);
    for (int i = 0; i < np; i++) push(i < argc ? argv[i] : UNSET);
    TV *slot = &R.stack[base];
    for (int k = 0; k < nkw; k++) {
        int i;
        for (i = 0; i < np; i++) if (KID(params, i)->a == kwn[k]) break;
        if (i == np) fail("%s() has no parameter '%s'", name, SYM(kwn[k]));
        if (i < argc) fail("%s() got multiple values for '%s'", name, SYM(kwn[k]));
        slot[i] = kwv[k];
    }
    Env *env = env_new(c->env);
    push(mk_obj(&env->h));
    for (int i = 0; i < np; i++) {
        Node *p = KID(params, i);
        TV v = slot[i];
        if (v.t == T_UNSET) {
            if (KID(p, 0)->k == K_NONE) fail("%s() missing argument '%s'", name, SYM(p->a));
            v = eval(KID(p, 0), env);
        }
        check_named(KID(p, 1), v, "%s() parameter '%s'", name, SYM(p->a));
        define(env, p->a, v, false);
    }
    Mach *m = (Mach *)alloc(T_MACH, sizeof(Mach));
    m->ty = c;
    m->env = env;
    TV mv = mk_obj(&m->h);
    push(mv);
    m->history = list_new(4);
    list_push(m->history, sym_str(KID(KID(def, 1), 0)->a));
    define(env, R.s_self, mv, false);
    Node *enter = KID(KID(KID(def, 1), 0), 0);
    if (enter->k != K_NONE) { exec_block(enter, env); R.ctl = C_NONE; }
    R.sp = base;
    return mv;
}

static bool is_event(Node *def, int32_t ev) {
    Node *states = KID(def, 1), *global = KID(def, 2);
    for (int s = 0; s < states->n; s++)
        for (int t = 2; t < KID(states, s)->n; t++) if (KID(KID(states, s), t)->a == ev) return true;
    for (int t = 0; t < global->n; t++) if (KID(global, t)->a == ev) return true;
    return false;
}

/* First enabled transition for `ev`; its scope (holding the event arguments) is left on the stack. */
static Node *pick_transition(Mach *m, int32_t ev, int argc, TV *argv, Env **scope_out) {
    Node *def = m->ty->def, *st = KID(KID(def, 1), m->state), *global = KID(def, 2);
    int total = (st->n - 2) + global->n;
    for (int k = 0; k < total; k++) {
        Node *t = k < st->n - 2 ? KID(st, k + 2) : KID(global, k - (st->n - 2));
        if (t->a != ev) continue;
        Node *ps = KID(t, 0);
        if (ps->n != argc) fail("event '%s' expects %d argument(s), got %d", SYM(ev), ps->n, argc);
        Env *scope = env_new(m->env);
        push(mk_obj(&scope->h));
        for (int i = 0; i < argc; i++) define(scope, KID(ps, i)->a, argv[i], false);
        if (KID(t, 1)->k == K_NONE || truthy(eval(KID(t, 1), scope))) { *scope_out = scope; return t; }
        R.sp--;
    }
    return NULL;
}

static bool machine_send(Mach *m, int32_t ev, int argc, TV *argv) {
    int base = R.sp;
    Env *scope;
    Node *t = ev < 0 ? NULL : pick_transition(m, ev, argc, argv, &scope);
    if (!t) return false;
    Node *def = m->ty->def, *states = KID(def, 1);
    Node *action = KID(t, 2);
    if (!num_i(KID(t, 3))) { /* no target: an internal transition */
        if (action->k != K_NONE) run_block(action, scope);
    } else {
        int to = 0;
        while (to < states->n && KID(states, to)->a != t->b) to++;
        if (to == states->n) fail("machine %s has no state '%s'", SYM(def->a), SYM(t->b));
        Node *exit_ = KID(KID(states, m->state), 1);
        if (exit_->k != K_NONE) exec_block(exit_, m->env);
        if (action->k != K_NONE) run_block(action, scope);
        m->state = to;
        list_push(m->history, sym_str(t->b));
        if (m->history->n > 1000) {
            memmove(m->history->v, m->history->v + 1, (m->history->n - 1) * sizeof(TV));
            m->history->n--;
        }
        Node *enter = KID(KID(states, to), 0);
        if (enter->k != K_NONE) exec_block(enter, m->env);
    }
    R.ctl = C_NONE;
    R.sp = base;
    react();
    return true;
}

static void mermaid_edge(SB *b, Node *from, int32_t to, Node *t) {
    sb_printf(b, "    %s --> %s : %s", SYM(from->a), SYM(to), SYM(t->a));
    if (KID(t, 0)->n) {
        sb_putc(b, '(');
        for (int i = 0; i < KID(t, 0)->n; i++) { if (i) sb_puts(b, ", "); sb_puts(b, SYM(KID(KID(t, 0), i)->a)); }
        sb_putc(b, ')');
    }
    if (KID(t, 1)->k != K_NONE) sb_puts(b, " [guarded]");
    sb_putc(b, '\n');
}
static void mermaid(Node *def, SB *b) {
    Node *states = KID(def, 1), *global = KID(def, 2);
    sb_puts(b, "stateDiagram-v2\n");
    sb_printf(b, "    [*] --> %s\n", SYM(KID(states, 0)->a));
    for (int s = 0; s < states->n; s++) {
        Node *st = KID(states, s);
        for (int k = 2; k < st->n; k++) mermaid_edge(b, st, num_i(KID(KID(st, k), 3)) ? KID(st, k)->b : st->a, KID(st, k));
    }
    for (int k = 0; k < global->n; k++) {
        Node *t = KID(global, k);
        if (!num_i(KID(t, 3))) continue;
        for (int s = 0; s < states->n; s++) if (KID(states, s)->a != t->b) mermaid_edge(b, KID(states, s), t->b, t);
    }
}

static int32_t event_sym(TV v) {
    char buf[64];
    show_buf(v, buf, sizeof buf);
    return find_sym(buf);
}

NOINLINE static TV machine_method(TV mv, int32_t name, int argc, TV *argv) {
    Mach *m = (Mach *)mv.u.o;
    Node *def = m->ty->def, *st = KID(KID(def, 1), m->state);
    const char *s = SYM(name);
    bool reserved = !strcmp(s, "send") || !strcmp(s, "fire") || !strcmp(s, "can") || !strcmp(s, "is") ||
                    !strcmp(s, "in_state") || !strcmp(s, "events");
    if (!reserved && is_event(def, name)) return mk_bool(machine_send(m, name, argc, argv));
    if (!strcmp(s, "send") || !strcmp(s, "fire")) {
        if (!argc) fail("send() needs an event name");
        return mk_bool(machine_send(m, event_sym(argv[0]), argc - 1, argv + 1));
    }
    if (!strcmp(s, "can")) {
        if (!argc) fail("can() needs an event name");
        int base = R.sp;
        Env *scope;
        int32_t ev = event_sym(argv[0]);
        bool ok = ev >= 0 && pick_transition(m, ev, argc - 1, argv + 1, &scope) != NULL;
        R.sp = base;
        return mk_bool(ok);
    }
    if (!strcmp(s, "is") || !strcmp(s, "in_state")) {
        char want[64] = "";
        if (argc) show_buf(argv[0], want, sizeof want);
        return mk_bool(!strcmp(SYM(st->a), want));
    }
    if (!strcmp(s, "events")) {
        List *l = list_new(4);
        Node *global = KID(def, 2);
        int32_t prev = -1;
        for (int k = 0; k < st->n - 2 + global->n; k++) {
            Node *t = k < st->n - 2 ? KID(st, k + 2) : KID(global, k - (st->n - 2));
            if (t->a != prev) list_push(l, sym_str(t->a));
            prev = t->a;
        }
        return mk_obj(&l->h);
    }
    if (!strcmp(s, "restart") || !strcmp(s, "reset")) {
        m->state = 0;
        m->history->n = 0;
        list_push(m->history, sym_str(KID(KID(def, 1), 0)->a));
        Node *enter = KID(KID(KID(def, 1), 0), 0);
        if (enter->k != K_NONE) { exec_block(enter, m->env); R.ctl = C_NONE; }
        return NIL;
    }
    if (!strcmp(s, "mermaid") || !strcmp(s, "diagram")) {
        SB b = {0};
        mermaid(def, &b);
        return sb_done(&b);
    }
    fail("machine %s has no event or method '%s'", SYM(def->a), s);
}

/* ---------------------------------------------------------------- signals */

static TV signal_new(TV value, int64_t cap, TV derive) {
    Signal *s = (Signal *)alloc(T_SIGNAL, sizeof(Signal));
    s->value = value;
    s->prev = NIL;
    s->derive = derive;
    s->capacity = cap < 1 ? 1 : (int32_t)cap;
    TV sv = mk_obj(&s->h);
    push(sv);
    s->history = list_new(4);
    s->subs = list_new(0);
    if (derive.t == T_NIL) list_push(s->history, value);
    R.sp--;
    return sv;
}
static TV signal_get(Signal *s) {
    if (s->derive.t != T_NIL) return call(s->derive, 0, NULL, 0, NULL, NULL);
    return s->value;
}
static void signal_set(Signal *s, TV v) {
    if (s->derive.t != T_NIL) fail("cannot set a derived signal");
    int base = R.sp;
    TV old = s->value;
    push(old);
    push(v);
    s->prev = old;
    s->value = v;
    s->prev_at = s->changed_at;
    s->changed_at = now_secs();
    list_push(s->history, v);
    while ((int32_t)s->history->n > s->capacity) {
        memmove(s->history->v, s->history->v + 1, (s->history->n - 1) * sizeof(TV));
        s->history->n--;
    }
    uint32_t n = s->subs->n;
    for (uint32_t i = 0; i < n && i < s->subs->n; i++) call2(s->subs->v[i], v, old);
    R.sp = base;
    react();
}
NOINLINE static TV signal_method(TV sv, const char *name, int argc, TV *argv) {
    Signal *s = (Signal *)sv.u.o;
    List *h = s->history;
    if (!strcmp(name, "get")) return signal_get(s);
    if (!strcmp(name, "set")) {
        if (!argc) fail("set() needs a value");
        signal_set(s, argv[0]);
        return NIL;
    }
    if (!strcmp(name, "update")) {
        if (!argc) fail("update() needs a function");
        TV cur = signal_get(s);
        push(cur);
        TV v = call1(argv[0], cur);
        push(v);
        signal_set(s, v);
        R.sp -= 2;
        return v;
    }
    if (!strcmp(name, "prev")) return s->prev;
    if (!strcmp(name, "history")) return list_from(h->v, h->n);
    if (!strcmp(name, "subscribe") || !strcmp(name, "on_change")) {
        if (!argc) fail("subscribe() needs a function");
        list_push(s->subs, argv[0]);
        return NIL;
    }
    if (!strcmp(name, "rate")) {
        static const int8_t T[8] = {0, 0, 1};
        double dt = s->changed_at - s->prev_at;
        if (s->prev.t == T_NIL || dt <= 0) return mk_float(0);
        TV dv = binary(s->value, OP_SUB, s->prev);
        push(dv);
        TV r = binary(dv, OP_DIV, mk_qty(dt, T, -1));
        R.sp--;
        return r;
    }
    if (!strcmp(name, "avg") || !strcmp(name, "mean")) {
        int64_t n = h->n, want;
        if (argc && as_i64(argv[0], &want)) n = want < 1 ? 1 : want;
        if (n > h->n) n = h->n;
        if (!n) return NIL;
        TV acc = mk_int(0);
        for (int64_t i = 0; i < n; i++) acc = binary(acc, OP_ADD, h->v[h->n - 1 - i]);
        push(acc);
        TV r = binary(acc, OP_DIV, mk_float((double)n));
        R.sp--;
        return r;
    }
    if (!strcmp(name, "min") || !strcmp(name, "max")) {
        if (!h->n) return NIL;
        TV best = h->v[0];
        for (uint32_t i = 1; i < h->n; i++) {
            int c = compare(h->v[i], best);
            if (name[1] == 'a' ? c > 0 : c < 0) best = h->v[i];
        }
        return best;
    }
    fail("signal has no method '%s'", name);
}

/* ---------------------------------------------------------------- quantities */

NOINLINE static TV quantity_method(TV qv, const char *name, int argc, TV *argv) {
    Qty *q = QTY(qv);
    double scale = q->unit >= 0 ? R.units[q->unit].scale : 1.0, shown = q->si / scale;
    if (!strcmp(name, "to") || !strcmp(name, "in") || !strcmp(name, "as")) {
        if (!argc) fail("%s() needs a unit string", name);
        int u = unit_of(argv[0]);
        if (!dim_eq(R.units[u].dim, q->dim)) {
            char have[64];
            snprintf(have, sizeof have, "%s", describe_dim(q->dim));
            fail("cannot convert %s to '%s' (%s)", have, STR(argv[0])->s, describe_dim(R.units[u].dim));
        }
        if (name[0] == 't') return mk_qty(q->si, q->dim, u);
        return mk_float(q->si / R.units[u].scale);
    }
    if (!strcmp(name, "value")) return mk_float(shown);
    if (!strcmp(name, "unit")) {
        SB b = {0};
        if (q->unit >= 0) sb_puts(&b, SYM(R.units[q->unit].canon)); else dim_name(q->dim, &b);
        return sb_done(&b);
    }
    if (!strcmp(name, "si")) return mk_float(q->si);
    if (!strcmp(name, "dim") || !strcmp(name, "kind")) return str_c(describe_dim(q->dim));
    if (!strcmp(name, "abs")) return mk_qty(fabs(q->si), q->dim, q->unit);
    if (!strcmp(name, "round")) {
        int64_t n = 0;
        if (argc) as_i64(argv[0], &n);
        double p = pow(10, (double)n);
        return mk_qty(round(shown * p) / p * scale, q->dim, q->unit);
    }
    fail("quantity has no method '%s'", name);
}

/* ---------------------------------------------------------------- fields and methods */

NOINLINE static TV get_field(TV o, int32_t name) {
    const char *f = SYM(name);
    switch (o.t) {
    case T_OBJ: {
        Inst *in = (Inst *)o.u.o;
        int i = field_ix(in->cls->def, name);
        if (i < 0) fail("%s has no field '%s'", SYM(in->cls->def->a), f);
        return in->f[i];
    }
    case T_MACH: {
        Mach *m = (Mach *)o.u.o;
        if (!strcmp(f, "state")) return sym_str(KID(KID(m->ty->def, 1), m->state)->a);
        if (!strcmp(f, "history")) return list_from(m->history->v, m->history->n);
        for (int i = 0; i < m->env->n; i++) if (m->env->names[i] == name) return m->env->vals[i];
        fail("machine %s has no variable '%s'", SYM(m->ty->def->a), f);
    }
    case T_SIGNAL:
        if (!strcmp(f, "value")) return signal_get((Signal *)o.u.o);
        if (!strcmp(f, "prev")) return ((Signal *)o.u.o)->prev;
        fail("signal has no field '%s'", f);
    case T_QTY:
        if (!strcmp(f, "value") || !strcmp(f, "unit") || !strcmp(f, "si")) return quantity_method(o, f, 0, NULL);
        fail("quantity has no field '%s'", f);
    case T_DICT: {
        int i = dict_find_c(DICT(o), f);
        if (i < 0) fail("dict has no field '%s'", f);
        return DICT(o)->v[i];
    }
    case T_LIST:
        if (*f && strspn(f, "0123456789") == strlen(f)) return index_value(o, mk_int(atoi(f)));
        /* fall through */
    default: fail("%s has no field '%s' (did you mean .%s()?)", type_name(o), f, f);
    }
}

/* Sets a field of a struct that has invariants; a violation restores the old value. */
static NOINLINE void set_checked(TV o, int i, TV v) {
    Inst *in = (Inst *)o.u.o;
    TV old = in->f[i];
    push(old);
    in->f[i] = v;
    Try t = {.sp = R.sp, .depth = R.depth, .line = R.line, .reacting = R.reacting};
    if (R.ntry >= 32) fail("try blocks nested too deeply");
    R.tries[R.ntry++] = &t;
    if (setjmp(t.jb)) { /* never leave the object in a state that violates its invariant */
        R.ntry--;
        R.sp = t.sp;
        R.depth = t.depth;
        ((Inst *)o.u.o)->f[i] = R.stack[t.sp - 1];
        char msg[240];
        snprintf(msg, sizeof msg, "%s", R.errmsg);
        fail("%s", msg);
    }
    check_invariants(o, "");
    R.ntry--;
    R.sp--;
}

static void set_field(TV o, int32_t name, TV v) {
    const char *f = SYM(name);
    switch (o.t) {
    case T_OBJ: {
        Inst *in = (Inst *)o.u.o;
        Node *def = in->cls->def;
        int i = field_ix(def, name);
        if (i < 0) fail("%s has no field '%s'", SYM(def->a), f);
        Node *ty = KID(KID(KID(def, 0), i), 1);
        check_named(ty, v, "%s.%s", SYM(def->a), f);
        if (!KID(def, 2)->n) { in->f[i] = v; return; }
        set_checked(o, i, v);
        return;
    }
    case T_MACH: {
        Env *env = ((Mach *)o.u.o)->env;
        for (int i = 0; i < env->n; i++) if (env->names[i] == name) { env->vals[i] = v; return; }
        fail("machine has no variable '%s'", f);
    }
    case T_SIGNAL:
        if (strcmp(f, "value")) fail("signal has no settable field '%s'", f);
        signal_set((Signal *)o.u.o, v);
        return;
    default: {
        push(v);
        TV key = sym_str(name);
        R.sp--;
        set_index(o, key, v);
    }
    }
}

static NOINLINE void check_after(TV o, TV r, const char *method) {
    char prefix[96];
    push(r);
    snprintf(prefix, sizeof prefix, "after %s.%s(): ", SYM(((Inst *)o.u.o)->cls->def->a), method);
    check_invariants(o, prefix);
    R.sp--;
}
NOINLINE static TV call_method(TV o, int32_t name, int argc, TV *argv, int nkw, int32_t *kwn, TV *kwv) {
    const char *s = SYM(name);
    switch (o.t) {
    case T_OBJ: {
        Inst *in = (Inst *)o.u.o;
        Node *def = in->cls->def, *m = find_method(def, name);
        if (m) {
            TV r = call_with_self(m, in->cls->env, o, argc, argv, nkw, kwn, kwv);
            if (KID(def, 2)->n) check_after(o, r, s);
            return r;
        }
        int i = field_ix(def, name);
        if (i >= 0 && callable(in->f[i])) return call(in->f[i], argc, argv, nkw, kwn, kwv);
        fail("%s has no method '%s'", SYM(def->a), s);
    }
    case T_CLASS: {
        Class *c = (Class *)o.u.o;
        Node *m = find_method(c->def, name);
        if (m) return call_def(m, c->env, argc, argv, nkw, kwn, kwv);
        if (!strcmp(s, "fields")) {
            List *l = list_new(KID(c->def, 0)->n);
            for (int i = 0; i < KID(c->def, 0)->n; i++) list_push(l, sym_str(KID(KID(c->def, 0), i)->a));
            return mk_obj(&l->h);
        }
        if (!strcmp(s, "name")) return sym_str(c->def->a);
        fail("struct %s has no static method '%s'", SYM(c->def->a), s);
    }
    case T_MACH: return machine_method(o, name, argc, argv);
    case T_MTYPE: {
        Node *def = ((Class *)o.u.o)->def;
        if (!strcmp(s, "mermaid") || !strcmp(s, "diagram")) {
            SB b = {0};
            mermaid(def, &b);
            return sb_done(&b);
        }
        if (!strcmp(s, "states")) {
            List *l = list_new(KID(def, 1)->n);
            for (int i = 0; i < KID(def, 1)->n; i++) list_push(l, sym_str(KID(KID(def, 1), i)->a));
            return mk_obj(&l->h);
        }
        fail("machine %s has no method '%s'", SYM(def->a), s);
    }
    case T_SIGNAL: return signal_method(o, s, argc, argv);
    case T_QTY: return quantity_method(o, s, argc, argv);
    case T_DICT: {
        int i = dict_find_c(DICT(o), s);
        if (i >= 0) {
            TV f = DICT(o)->v[i];
            if (callable(f) || f.t == T_CLASS || f.t == T_MTYPE) return call(f, argc, argv, nkw, kwn, kwv);
            if (nkw) return call(f, argc, argv, nkw, kwn, kwv);
        } else if (nkw) fail("module has no member '%s'", s);
        return builtin_method(o, s, argc, argv);
    }
    case T_NATIVE: {
        if (nkw) fail("%s.%s() does not take keyword arguments", type_name(o), s);
        TV r = native_method(o, s, argc, argv);
        if (has_reactive()) { push(r); react(); R.sp--; }
        return r;
    }
    default:
        if (nkw) fail("%s.%s() does not take keyword arguments", type_name(o), s);
        return builtin_method(o, s, argc, argv);
    }
}

static void stringify(TV v, SB *b) {
    if (v.t == T_OBJ) {
        Inst *in = (Inst *)v.u.o;
        Node *m = find_method(in->cls->def, R.s_to_str);
        if (m) {
            TV r = call_def(m, in->cls->env, 1, &v, 0, NULL, NULL);
            to_sb(r, b, false);
            return;
        }
    }
    to_sb(v, b, false);
}

/* Binary operator with struct operator overloading (op_add, op_mul, op_eq, op_cmp, ...). */
NOINLINE static TV binary_op(TV a, int op, TV b) {
    if (a.t != T_OBJ && b.t != T_OBJ) return binary(a, op, b);
    const char *name = "", *rname = "";
    switch (op) {
    case OP_ADD: name = "op_add"; rname = "op_radd"; break;
    case OP_SUB: name = "op_sub"; break;
    case OP_MUL: name = "op_mul"; rname = "op_rmul"; break;
    case OP_DIV: name = "op_div"; break;
    case OP_MOD: name = "op_mod"; break;
    case OP_EQ: case OP_NE: name = "op_eq"; break;
    case OP_LT: case OP_LE: case OP_GT: case OP_GE: name = "op_cmp"; break;
    default: break;
    }
    int sym = *name ? find_sym(name) : -1;
    Node *m = (a.t == T_OBJ && sym >= 0) ? find_method(((Inst *)a.u.o)->cls->def, sym) : NULL;
    if (m) {
        TV args[2] = {a, b};
        TV r = call_def(m, ((Inst *)a.u.o)->cls->env, 2, args, 0, NULL, NULL);
        if (op == OP_NE) return mk_bool(!truthy(r));
        if (op >= OP_LT && op <= OP_GE) {
            double n;
            if (!as_f64(r, &n)) fail("op_cmp must return a number");
            return mk_bool(op == OP_LT ? n < 0 : op == OP_LE ? n <= 0 : op == OP_GT ? n > 0 : n >= 0);
        }
        return r;
    }
    if (b.t == T_OBJ && (op == OP_ADD || op == OP_MUL)) {
        Node *def = ((Inst *)b.u.o)->cls->def;
        int rs = find_sym(rname);
        m = rs >= 0 ? find_method(def, rs) : NULL;
        if (!m && sym >= 0) m = find_method(def, sym);
        if (m) {
            TV args[2] = {b, a};
            return call_def(m, ((Inst *)b.u.o)->cls->env, 2, args, 0, NULL, NULL);
        }
    }
    return binary(a, op, b);
}

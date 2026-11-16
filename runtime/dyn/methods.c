/* ---------------------------------------------------------------- str / list / dict methods */

#define IS(s) (!strcmp(name, s))
#define FIRST() (argc ? argv[0] : (fail("%s() needs an argument", name), NIL))

static bool is_space(char c) { return c == ' ' || c == '\t' || c == '\n' || c == '\r' || c == '\f' || c == '\v'; }

static TV str_method(TV sv, const char *name, int argc, TV *argv) {
    Str *s = STR(sv);
    const char *p = s->s;
    size_t n = s->len;
#define ARG_S(k) (argc > (k) ? sarg(name, argv[k]) : (fail("%s() needs a string argument", name), ""))
    if (IS("len")) return mk_int((int64_t)u8_count(p, n));
    if (IS("upper") || IS("lower")) {
        TV r = str_n(p, n);
        for (size_t i = 0; i < n; i++) {
            char c = STR(r)->s[i];
            unsigned char prev = i ? (unsigned char)STR(r)->s[i - 1] : 0, u = (unsigned char)c;
            if (name[0] == 'u' && c >= 'a' && c <= 'z') STR(r)->s[i] = (char)(c - 32);
            if (name[0] == 'l' && c >= 'A' && c <= 'Z') STR(r)->s[i] = (char)(c + 32);
            /* Latin-1 letters (U+00C0..U+00FE) */
            if (prev == 0xC3 && name[0] == 'u' && u >= 0xA0 && u <= 0xBE && u != 0xB7) STR(r)->s[i] = (char)(u - 0x20);
            if (prev == 0xC3 && name[0] == 'l' && u >= 0x80 && u <= 0x9E && u != 0x97) STR(r)->s[i] = (char)(u + 0x20);
        }
        return r;
    }
    if (IS("strip") || IS("trim") || IS("lstrip") || IS("rstrip")) {
        size_t a = 0, b = n;
        if (name[0] != 'r') while (a < b && is_space(p[a])) a++;
        if (name[0] != 'l') while (b > a && is_space(p[b - 1])) b--;
        return str_n(p + a, b - a);
    }
    if (IS("split")) {
        List *l = list_new(0);
        push(mk_obj(&l->h));
        if (argc) {
            const char *sep = ARG_S(0);
            size_t sl = strlen(sep);
            if (!sl) {
                /* like Rust: "abc".split("") is ["", "a", "b", "c", ""] */
                list_push(l, str_c(""));
                for (size_t i = 0; i < n;) {
                    size_t j = i + 1;
                    while (j < n && ((unsigned char)p[j] & 0xC0) == 0x80) j++;
                    list_push(l, str_n(p + i, j - i));
                    i = j;
                }
                list_push(l, str_c(""));
            } else {
                const char *start = p, *hit;
                while ((hit = strstr(start, sep))) { list_push(l, str_n(start, (size_t)(hit - start))); start = hit + sl; }
                list_push(l, str_n(start, (size_t)(p + n - start)));
            }
        } else {
            size_t i = 0;
            while (i < n) {
                while (i < n && is_space(p[i])) i++;
                size_t j = i;
                while (j < n && !is_space(p[j])) j++;
                if (j > i) list_push(l, str_n(p + i, j - i));
                i = j;
            }
        }
        R.sp--;
        return mk_obj(&l->h);
    }
    if (IS("lines")) {
        List *l = list_new(0);
        push(mk_obj(&l->h));
        size_t i = 0;
        while (i < n) {
            size_t j = i;
            while (j < n && p[j] != '\n') j++;
            size_t end = j;
            if (end > i && p[end - 1] == '\r') end--;
            list_push(l, str_n(p + i, end - i));
            i = j + 1;
        }
        R.sp--;
        return mk_obj(&l->h);
    }
    if (IS("chars")) return iterate(sv, 1);
    if (IS("replace")) {
        const char *from = ARG_S(0), *to = ARG_S(1);
        size_t fl = strlen(from);
        SB b = {0};
        if (!fl) { sb_put(&b, p, n); return sb_done(&b); }
        const char *start = p, *hit;
        while ((hit = strstr(start, from))) { sb_put(&b, start, (size_t)(hit - start)); sb_puts(&b, to); start = hit + fl; }
        sb_puts(&b, start);
        return sb_done(&b);
    }
    if (IS("contains")) return mk_bool(strstr(p, ARG_S(0)) != NULL);
    if (IS("starts_with") || IS("startswith")) { const char *x = ARG_S(0); return mk_bool(strncmp(p, x, strlen(x)) == 0); }
    if (IS("ends_with") || IS("endswith")) {
        const char *x = ARG_S(0);
        size_t xl = strlen(x);
        return mk_bool(xl <= n && memcmp(p + n - xl, x, xl) == 0);
    }
    if (IS("find")) {
        const char *hit = strstr(p, ARG_S(0));
        return mk_int(hit ? (int64_t)u8_count(p, (size_t)(hit - p)) : -1);
    }
    if (IS("count")) {
        const char *x = ARG_S(0), *start = p, *hit;
        size_t xl = strlen(x);
        int64_t c = 0;
        if (!xl) return mk_int((int64_t)u8_count(p, n) + 1);
        while ((hit = strstr(start, x))) { c++; start = hit + xl; }
        return mk_int(c);
    }
    if (IS("join")) {
        if (!argc) fail("join() needs a list");
        List *l = ITEMS(argv[0]);
        SB b = {0};
        for (uint32_t i = 0; i < l->n; i++) { if (i) sb_put(&b, p, n); to_sb(l->v[i], &b, false); }
        R.sp--;
        return sb_done(&b);
    }
    if (IS("repeat")) {
        if (!argc) fail("repeat() needs a count");
        int64_t k = intarg("repeat", argv[0]);
        SB b = {0};
        for (int64_t i = 0; i < k; i++) sb_put(&b, p, n);
        return sb_done(&b);
    }
    if (IS("format")) return rust_format(p, argc, argv);
    if (IS("bytes") || IS("encode")) return from_bytes((const uint8_t *)p, n);
    if (IS("is_digit") || IS("isdigit")) {
        bool ok = n > 0;
        for (size_t i = 0; i < n; i++) ok &= p[i] >= '0' && p[i] <= '9';
        return mk_bool(ok);
    }
    if (IS("is_alpha") || IS("isalpha")) {
        bool ok = n > 0;
        for (size_t i = 0; i < n; i++) ok &= (p[i] >= 'a' && p[i] <= 'z') || (p[i] >= 'A' && p[i] <= 'Z') || ((unsigned char)p[i] & 0x80);
        return mk_bool(ok);
    }
    if (IS("to_int")) return b_int(1, &sv);
    if (IS("to_float")) return b_float(1, &sv);
    if (IS("pad_left") || IS("pad_right")) {
        if (!argc) fail("pad needs a width");
        int64_t w = intarg(name, argv[0]);
        size_t chars = u8_count(p, n);
        SB f = {0}, b = {0};
        if (argc > 1) to_sb(argv[1], &f, false); else sb_putc(&f, ' ');
        if (name[4] == 'r') sb_put(&b, p, n);
        for (int64_t i = (int64_t)chars; i < w; i++) sb_put(&b, f.p, f.n);
        if (name[4] == 'l') sb_put(&b, p, n);
        free(f.p);
        return sb_done(&b);
    }
    fail("str has no method '%s'", name);
#undef ARG_S
}

static __attribute__((noinline)) void not_in_list(TV x) {
    char shown[80];
    repr_buf(x, shown, sizeof shown);
    fail("remove(): %s not in list", shown);
}
static TV list_method(TV lv, const char *name, int argc, TV *argv) {
    List *l = LIST(lv);
    if (IS("len")) return mk_int(l->n);
    if (IS("push") || IS("append")) { list_push(l, FIRST()); return NIL; }
    if (IS("extend")) {
        List *more = ITEMS(FIRST());
        for (uint32_t i = 0; i < more->n; i++) list_push(l, more->v[i]);
        R.sp--;
        return NIL;
    }
    if (IS("pop")) {
        if (!argc) {
            if (!l->n) fail("pop() from empty list");
            return l->v[--l->n];
        }
        int64_t k = intarg("pop", argv[0]), n = l->n;
        if (k < 0) k += n;
        if (k < 0 || k >= n) fail("pop() index out of range");
        TV v = l->v[k];
        memmove(l->v + k, l->v + k + 1, (size_t)(n - k - 1) * sizeof(TV));
        l->n--;
        return v;
    }
    if (IS("insert")) {
        int64_t k = intarg("insert", FIRST()), n = l->n;
        if (argc < 2) fail("insert() needs (index, value)");
        if (k < 0) { k += n; if (k < 0) k = 0; } else if (k > n) k = n;
        list_push(l, NIL);
        memmove(l->v + k + 1, l->v + k, (size_t)(n - k) * sizeof(TV));
        l->v[k] = argv[1];
        return NIL;
    }
    if (IS("remove")) {
        TV x = FIRST();
        for (uint32_t i = 0; i < l->n; i++)
            if (equals(l->v[i], x)) {
                TV v = l->v[i];
                memmove(l->v + i, l->v + i + 1, (l->n - i - 1) * sizeof(TV));
                l->n--;
                return v;
            }
        not_in_list(x);
    }
    if (IS("clear")) { l->n = 0; return NIL; }
    if (IS("contains")) return mk_bool(contains(lv, argc ? argv[0] : NIL));
    if (IS("index")) {
        TV x = FIRST();
        for (uint32_t i = 0; i < l->n; i++) if (equals(l->v[i], x)) return mk_int(i);
        return mk_int(-1);
    }
    if (IS("count")) {
        TV x = FIRST();
        int64_t c = 0;
        for (uint32_t i = 0; i < l->n; i++) c += equals(l->v[i], x);
        return mk_int(c);
    }
    if (IS("first")) return l->n ? l->v[0] : NIL;
    if (IS("last")) return l->n ? l->v[l->n - 1] : NIL;
    if (IS("reverse")) {
        for (uint32_t i = 0, n = l->n; i < n / 2; i++) { TV t = l->v[i]; l->v[i] = l->v[n - 1 - i]; l->v[n - 1 - i] = t; }
        return NIL;
    }
    if (IS("sort") || IS("sorted")) {
        TV copy = list_from(l->v, l->n);
        push(copy);
        TV sorted = sort_values(copy, argc ? &argv[0] : NULL);
        R.sp--;
        if (name[4]) return sorted;
        l->n = 0;
        for (uint32_t i = 0; i < LIST(sorted)->n; i++) list_push(l, LIST(sorted)->v[i]);
        return NIL;
    }
    if (IS("copy")) return list_from(l->v, l->n);
    if (IS("join")) {
        SB sep = {0}, b = {0};
        if (argc) to_sb(argv[0], &sep, false);
        for (uint32_t i = 0; i < l->n; i++) { if (i && sep.n) sb_put(&b, sep.p, sep.n); to_sb(l->v[i], &b, false); }
        free(sep.p);
        return sb_done(&b);
    }
    if (IS("slice")) {
        Range r = {.a = argc ? intarg("slice", argv[0]) : 0, .b = argc > 1 ? intarg("slice", argv[1]) : (int64_t)l->n};
        uint32_t s, e;
        slice_bounds(&r, l->n, &s, &e);
        return list_from(l->v + s, e - s);
    }
    if (IS("map") || IS("filter")) {
        TV args[2] = {FIRST(), lv};
        return name[0] == 'm' ? b_map(2, args) : b_filter(2, args);
    }
    if (IS("reduce")) {
        TV args[3] = {FIRST(), lv, argc > 1 ? argv[1] : NIL};
        return b_reduce(argc > 1 ? 3 : 2, args);
    }
    if (IS("each") || IS("for_each") || IS("any") || IS("all")) {
        TV f = FIRST(), copy = list_from(l->v, l->n);
        push(copy);
        TV result = name[0] == 'a' ? mk_bool(name[1] == 'l') : NIL;
        for (uint32_t i = 0; i < LIST(copy)->n; i++) {
            bool t = truthy(call1(f, LIST(copy)->v[i]));
            if (IS("any") && t) { result = mk_bool(true); break; }
            if (IS("all") && !t) { result = mk_bool(false); break; }
        }
        R.sp--;
        return result;
    }
    if (IS("sum")) return b_sum(1, &lv);
    if (IS("min")) return b_min(1, &lv);
    if (IS("max")) return b_max(1, &lv);
    if (IS("enumerate")) return b_enumerate(1, &lv);
    if (IS("hex")) {
        SB b = {0};
        for (uint32_t i = 0; i < l->n; i++) {
            int64_t k;
            if (i) sb_putc(&b, ' ');
            if (as_i64(l->v[i], &k)) { if (k >= 0 && k < 16) sb_putc(&b, '0'); radix_str(k, 16, false, "", &b); }
            else sb_puts(&b, "??");
        }
        sb_puts(&b, "");
        return sb_done(&b);
    }
    fail("list has no method '%s'", name);
}

static TV dict_method(TV dv, const char *name, int argc, TV *argv) {
    Dict *d = DICT(dv);
    if (IS("len")) return mk_int(d->n);
    if (IS("get")) {
        TV k = FIRST();
        check_key(k);
        int i = dict_find(d, k);
        return i >= 0 ? d->v[i] : argc > 1 ? argv[1] : NIL;
    }
    if (IS("set") || IS("insert")) { dict_set(d, FIRST(), argc > 1 ? argv[1] : NIL); return NIL; }
    if (IS("has") || IS("contains") || IS("contains_key")) { TV k = FIRST(); check_key(k); return mk_bool(dict_find(d, k) >= 0); }
    if (IS("remove") || IS("pop")) {
        TV out, k = FIRST();
        check_key(k);
        if (dict_remove(d, k, &out)) return out;
        return argc > 1 ? argv[1] : NIL;
    }
    if (IS("keys")) return list_from(d->k, d->n);
    if (IS("values")) return list_from(d->v, d->n);
    if (IS("items")) return iterate(dv, 2);
    if (IS("copy")) {
        Dict *c = dict_new();
        for (uint32_t i = 0; i < d->n; i++) dict_set(c, d->k[i], d->v[i]);
        return mk_obj(&c->h);
    }
    if (IS("clear")) { d->n = 0; return NIL; }
    if (IS("update")) {
        TV o = FIRST();
        if (o.t == T_DICT) for (uint32_t i = 0; i < DICT(o)->n; i++) dict_set(d, DICT(o)->k[i], DICT(o)->v[i]);
        return NIL;
    }
    fail("dict has no method '%s'", name);
}

static TV builtin_method(TV o, const char *name, int argc, TV *argv) {
    switch (o.t) {
    case T_STR: return str_method(o, name, argc, argv);
    case T_LIST: return list_method(o, name, argc, argv);
    case T_DICT: return dict_method(o, name, argc, argv);
    case T_RANGE:
        if (IS("list") || IS("to_list")) return iterate(o, 1);
        if (IS("contains")) { if (!argc) fail("contains() needs an argument"); return mk_bool(contains(o, argv[0])); }
        if (IS("len")) return b_len(1, &o);
        {
            TV l = iterate(o, 1);
            push(l);
            TV r = list_method(l, name, argc, argv);
            R.sp--;
            return r;
        }
    case T_INT: case T_FLOAT: {
        TV args[4] = {o, argc > 0 ? argv[0] : NIL, argc > 1 ? argv[1] : NIL, NIL};
        if (IS("abs")) return b_abs(1, args);
        if (IS("round")) return b_round(argc + 1 > 2 ? 2 : argc + 1, args);
        if (IS("format")) return b_format(argc + 1 > 2 ? 2 : argc + 1, args);
        if (IS("clamp")) return b_clamp(argc + 1, args);
        if (IS("to_hex") || IS("hex")) { SB b = {0}; radix_str(intarg("hex", o), 16, false, "0x", &b); return sb_done(&b); }
        if (IS("bit")) {
            if (!argc) fail("bit() needs an index");
            return mk_int((intarg("bit", o) >> intarg("bit", argv[0])) & 1);
        }
        break;
    }
    case T_TASK:
        if (IS("done")) return mk_bool(((Task *)o.u.o)->state >= 2);
        if (IS("result") || IS("await")) return await_value(o);
        fail("task has no method '%s'", name);
    case T_FN: case T_BUILTIN:
        if (IS("call")) return call(o, argc, argv, 0, NULL, NULL);
        fail("fn has no method '%s'", name);
    default: break;
    }
    fail("%s has no method '%s'", type_name(o), name);
}

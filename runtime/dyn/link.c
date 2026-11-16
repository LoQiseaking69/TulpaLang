/* ---------------------------------------------------------------- Tulpa Link
 *
 * The Tulpa Link node for firmware: the same wire format ("TL" v1), discovery, typed topics,
 * reliable and latched delivery, and services as the host implementation (src/link.rs), so
 * `import "std/link"` runs unchanged on a microcontroller. Transports: the in-process sim
 * network, UDP (Linux, ESP-IDF/lwIP), and a UART with COBS framing that `tulpa link bridge`
 * joins to the UDP domain. */

#if defined(__linux__) || defined(__APPLE__) || defined(ESP_PLATFORM)
#define TULPA_LINK_UDP 1
#include <arpa/inet.h>
#include <fcntl.h>
#include <netinet/in.h>
#include <sys/socket.h>
#include <unistd.h>
#endif

enum { TL_ANNOUNCE = 1, TL_DATA, TL_NACK, TL_REQUEST, TL_REPLY, TL_BYE };
enum { TR_SIM, TR_UDP, TR_UART };
#define TL_PORT_BASE 7400
#define TL_PORTS 16
#define TL_MAX_PACKET 4096

typedef struct { uint8_t kind; uint64_t a; uint16_t port; } LAddr;
typedef struct Sch { uint8_t k; int8_t dim[8]; int unit, nf; struct Sch *elem, **fields; char **names; } Sch;
enum { S_ANY, S_INT, S_FLOAT, S_BOOL, S_STR, S_UNIT, S_LIST, S_RECORD };
typedef struct { uint8_t role, flags; char *topic; uint32_t hash; uint64_t last_seq; } Endpoint;
typedef struct { uint64_t id; char *name; LAddr addr; double last_seen; Endpoint *ends; int n; } Peer;
typedef struct { uint64_t seq; double stamp; uint8_t *data; size_t len; } Held;
typedef struct { char *topic; Sch *schema; uint64_t seq, sent; bool reliable, latched; int keep, nh; Held *history; } Pub;
typedef struct { uint64_t from, expected; Held *held; int n; } Stream;
typedef struct { char *topic; Sch *schema; bool reliable, has_deadline, has_rx, missed; double deadline, last_rx;
                 Stream *streams; int ns; uint64_t received, lost; } Sub;
typedef struct { uint64_t req; LAddr addr; uint8_t *pkt; size_t len; double last; int tries; } Pending;
typedef struct { uint64_t id, req; uint8_t *pkt; size_t len; } Answered;
typedef struct LNode {
    char *name;
    uint64_t id, next_req;
    uint16_t domain, base, port;
    uint8_t transport;
    uint32_t sim_addr;
    int sock;
    uint32_t extra[8];
    int nextra;
    tulpa_uart_t uart;
    uint8_t *rx;
    size_t rxn;
    Peer *peers; int npeers;
    Pub *pubs; int npubs;
    Sub *subs; int nsubs;
    char **services; int nservices;
    Pending *pending; int npending;
    struct { uint64_t id, req; } *in_progress; int nprogress;
    Answered answered[64]; int nanswered;
    struct { uint64_t id; char *topic; } *warned; int nwarned;
    double last_announce;
    bool dirty, closed;
    Native *owner; /* owner->extra is the queue of events for poll() */
} LNode;

static struct {
    struct { uint32_t addr; uint16_t domain; } *nodes; int nnodes;
    uint32_t next;
    struct SimPkt { uint32_t to, from; double at; uint64_t order; uint8_t *data; size_t len; } *queue; int nq;
    uint64_t order, rng, sent, dropped;
    double latency, jitter, loss;
    struct { uint32_t a, b; } *cut; int ncut;
} SIM;

static char *dupstr(const char *s) {
    char *c = xmalloc(strlen(s) + 1);
    strcpy(c, s);
    return c;
}
static uint8_t *dupbytes(const uint8_t *p, size_t n) {
    uint8_t *c = xmalloc(n);
    memcpy(c, p, n);
    return c;
}

/* ------------------------------------------------ schemas */

static Sch *sch_parse(TV v) {
    Sch *s = xmalloc(sizeof(Sch));
    memset(s, 0, sizeof *s);
    if (v.t == T_NIL) return s;
    if (v.t == T_STR) {
        const char *t = STR(v)->s;
        static const char *const NAMES[] = {"any", "int", "float", "bool", "str"};
        for (int i = 0; i < 5; i++) if (!strcmp(t, NAMES[i])) { s->k = (uint8_t)i; return s; }
        size_t n = STR(v)->len;
        if (n < 2 || t[0] != '[' || t[n - 1] != ']')
            fail("link: unknown schema '%s' (use any, int, float, bool, str, \"[unit]\", [schema] or {field: schema})", t);
        int u = unit_find(t, n);
        if (u < 0) u = unit_parse(t + 1, n - 2);
        s->k = S_UNIT;
        s->unit = u;
        memcpy(s->dim, R.units[u].dim, 8);
        return s;
    }
    if (v.t == T_LIST) {
        if (LIST(v)->n != 1) fail("link: a list schema has exactly one element schema, e.g. [\"[m]\"]");
        s->k = S_LIST;
        s->elem = sch_parse(LIST(v)->v[0]);
        return s;
    }
    if (v.t == T_DICT) {
        Dict *d = DICT(v);
        s->k = S_RECORD;
        s->nf = (int)d->n;
        s->names = xmalloc((d->n + 1) * sizeof(char *));
        s->fields = xmalloc((d->n + 1) * sizeof(Sch *));
        for (uint32_t i = 0; i < d->n; i++) {
            char key[64];
            show_buf(d->k[i], key, sizeof key);
            Sch *f = sch_parse(d->v[i]);
            int at = (int)i; /* keep fields sorted by name */
            while (at > 0 && strcmp(s->names[at - 1], key) > 0) { s->names[at] = s->names[at - 1]; s->fields[at] = s->fields[at - 1]; at--; }
            s->names[at] = dupstr(key);
            s->fields[at] = f;
        }
        return s;
    }
    fail("link: a schema cannot be a %s", type_name(v));
}
static void sch_canon(Sch *s, SB *b) {
    static const char *const NAMES[] = {"any", "int", "float", "bool", "str"};
    switch (s->k) {
    case S_UNIT: sb_putc(b, '['); dim_name(s->dim, b); sb_putc(b, ']'); break;
    case S_LIST: sb_putc(b, '['); sch_canon(s->elem, b); sb_putc(b, ']'); break;
    case S_RECORD:
        sb_putc(b, '{');
        for (int i = 0; i < s->nf; i++) { if (i) sb_putc(b, ';'); sb_puts(b, s->names[i]); sb_putc(b, ':'); sch_canon(s->fields[i], b); }
        sb_putc(b, '}');
        break;
    default: sb_puts(b, NAMES[s->k]); break;
    }
}
static uint32_t sch_hash(Sch *s) {
    if (s->k == S_ANY) return 0;
    SB b = {0};
    sch_canon(s, &b);
    uint32_t h = 0x811c9dc5u;
    for (size_t i = 0; i < b.n; i++) h = (h ^ (uint8_t)b.p[i]) * 0x01000193u;
    free(b.p);
    return h;
}
/* Returns false and fills `why` when `v` does not fit the schema. */
static bool sch_check(Sch *s, TV v, const char *at, char *why, size_t n) {
    const char *want = NULL;
    char unit[48];
    switch (s->k) {
    case S_ANY: return true;
    case S_INT: if (v.t != T_INT) want = "int"; break;
    case S_FLOAT: if (v.t != T_INT && v.t != T_FLOAT) want = "float"; break;
    case S_BOOL: if (v.t != T_BOOL) want = "bool"; break;
    case S_STR: if (v.t != T_STR) want = "str"; break;
    case S_UNIT:
        if (v.t != T_QTY || !dim_eq(QTY(v)->dim, s->dim)) {
            snprintf(unit, sizeof unit, "a quantity in [%s]", SYM(R.units[s->unit].canon));
            want = unit;
        }
        break;
    case S_LIST:
        if (v.t != T_LIST) { want = "a list"; break; }
        for (uint32_t i = 0; i < LIST(v)->n; i++) {
            char sub[96];
            snprintf(sub, sizeof sub, "%s[%u]", at, (unsigned)i);
            if (!sch_check(s->elem, LIST(v)->v[i], sub, why, n)) return false;
        }
        return true;
    case S_RECORD:
        if (v.t != T_DICT) { want = "a record"; break; }
        for (int i = 0; i < s->nf; i++) {
            int ix = dict_find_c(DICT(v), s->names[i]);
            if (ix < 0) { snprintf(why, n, "%s: missing field '%s'", at, s->names[i]); return false; }
            char sub[96];
            snprintf(sub, sizeof sub, "%s.%s", at, s->names[i]);
            if (!sch_check(s->fields[i], DICT(v)->v[ix], sub, why, n)) return false;
        }
        for (uint32_t i = 0; i < DICT(v)->n; i++) {
            char key[64];
            show_buf(DICT(v)->k[i], key, sizeof key);
            bool known = false;
            for (int f = 0; f < s->nf; f++) known |= !strcmp(s->names[f], key);
            if (!known) { snprintf(why, n, "%s: unexpected field '%s'", at, key); return false; }
        }
        return true;
    }
    if (!want) return true;
    if (v.t == T_QTY) {
        SB b = {0};
        dim_name(QTY(v)->dim, &b);
        snprintf(why, n, "%s: expected %s, got a quantity in [%s]", at, want, b.p);
        free(b.p);
    } else snprintf(why, n, "%s: expected %s, got %s", at, want, type_name(v));
    return false;
}
/* Gives a received value the subscriber's units and number types. */
static TV sch_shape(Sch *s, TV v) {
    if (s->k == S_UNIT && v.t == T_QTY) return mk_qty(QTY(v)->si, QTY(v)->dim, s->unit);
    if (s->k == S_FLOAT && v.t == T_INT) return mk_float((double)v.u.i);
    if (s->k == S_LIST && v.t == T_LIST) {
        List *out = list_new(LIST(v)->n);
        push(mk_obj(&out->h));
        for (uint32_t i = 0; i < LIST(v)->n; i++) list_push(out, sch_shape(s->elem, LIST(v)->v[i]));
        R.sp--;
        return mk_obj(&out->h);
    }
    if (s->k == S_RECORD && v.t == T_DICT) {
        Dict *out = dict_new();
        push(mk_obj(&out->h));
        for (uint32_t i = 0; i < DICT(v)->n; i++) {
            TV x = DICT(v)->v[i];
            if (DICT(v)->k[i].t == T_STR)
                for (int f = 0; f < s->nf; f++) if (!strcmp(s->names[f], STR(DICT(v)->k[i])->s)) x = sch_shape(s->fields[f], x);
            dict_set(out, DICT(v)->k[i], x);
        }
        R.sp--;
        return mk_obj(&out->h);
    }
    return v;
}

/* ------------------------------------------------ wire format */

static void w_u8(SB *w, uint8_t v) { sb_put(w, (const char *)&v, 1); }
static void w_le(SB *w, uint64_t v, int bytes) { for (int i = 0; i < bytes; i++) w_u8(w, (uint8_t)(v >> (8 * i))); }
static void w_f64(SB *w, double f) { uint64_t u; memcpy(&u, &f, 8); w_le(w, u, 8); }
static void w_str(SB *w, const char *s) {
    size_t n = strlen(s);
    if (n > 65535) n = 65535;
    w_le(w, n, 2);
    sb_put(w, s, n);
}
static bool w_value(SB *w, TV v) {
    switch (v.t) {
    case T_NIL: w_u8(w, 0); return true;
    case T_BOOL: w_u8(w, v.u.i ? 2 : 1); return true;
    case T_INT: w_u8(w, 3); w_le(w, (uint64_t)v.u.i, 8); return true;
    case T_FLOAT: w_u8(w, 4); w_f64(w, v.u.f); return true;
    case T_STR: w_u8(w, 5); w_le(w, STR(v)->len, 4); sb_put(w, STR(v)->s, STR(v)->len); return true;
    case T_LIST:
        w_u8(w, 6);
        w_le(w, LIST(v)->n, 4);
        for (uint32_t i = 0; i < LIST(v)->n; i++) if (!w_value(w, LIST(v)->v[i])) return false;
        return true;
    case T_DICT:
        w_u8(w, 7);
        w_le(w, DICT(v)->n, 4);
        for (uint32_t i = 0; i < DICT(v)->n; i++) if (!w_value(w, DICT(v)->k[i]) || !w_value(w, DICT(v)->v[i])) return false;
        return true;
    case T_QTY: w_u8(w, 8); w_f64(w, QTY(v)->si); sb_put(w, (const char *)QTY(v)->dim, 8); return true;
    default: return false;
    }
}
static void w_value_or_fail(SB *w, TV v) {
    if (w_value(w, v)) return;
    free(w->p);
    fail("link: a %s cannot be sent over a topic", type_name(v));
}

typedef struct { const uint8_t *p; size_t n; bool bad; } Rdr;
static const uint8_t *r_take(Rdr *r, size_t n) {
    static const uint8_t ZERO[8];
    if (r->n < n) { r->bad = true; r->n = 0; return ZERO; }
    const uint8_t *p = r->p;
    r->p += n;
    r->n -= n;
    return p;
}
static uint64_t r_le(Rdr *r, int bytes) {
    const uint8_t *p = r_take(r, (size_t)bytes);
    uint64_t v = 0;
    for (int i = 0; i < bytes; i++) v |= (uint64_t)p[i] << (8 * i);
    return v;
}
static double r_f64(Rdr *r) { uint64_t u = r_le(r, 8); double f; memcpy(&f, &u, 8); return f; }
static char *r_str(Rdr *r) { /* malloc'd */
    size_t n = (size_t)r_le(r, 2);
    if (r->n < n) { r->bad = true; n = 0; }
    char *s = xmalloc(n + 1);
    memcpy(s, r_take(r, n), n);
    s[n] = 0;
    return s;
}
static TV r_value(Rdr *r, int depth) {
    if (depth > 32) { r->bad = true; return NIL; }
    switch (r_le(r, 1)) {
    case 0: return NIL;
    case 1: return mk_bool(false);
    case 2: return mk_bool(true);
    case 3: return mk_int((int64_t)r_le(r, 8));
    case 4: return mk_float(r_f64(r));
    case 5: {
        size_t n = (size_t)r_le(r, 4);
        if (r->n < n) { r->bad = true; return NIL; }
        return str_n((const char *)r_take(r, n), n);
    }
    case 6: {
        uint32_t n = (uint32_t)r_le(r, 4);
        List *l = list_new(n < 256 ? n : 256);
        push(mk_obj(&l->h));
        for (uint32_t i = 0; i < n && !r->bad; i++) list_push(l, r_value(r, depth + 1));
        R.sp--;
        return mk_obj(&l->h);
    }
    case 7: {
        uint32_t n = (uint32_t)r_le(r, 4);
        Dict *d = dict_new();
        push(mk_obj(&d->h));
        for (uint32_t i = 0; i < n && !r->bad; i++) {
            TV k = r_value(r, depth + 1);
            push(k);
            TV v = r_value(r, depth + 1);
            if (k.t == T_STR || k.t == T_INT || k.t == T_BOOL || k.t == T_NIL || k.t == T_FLOAT) dict_set(d, k, v);
            else r->bad = true;
            R.sp--;
        }
        R.sp--;
        return mk_obj(&d->h);
    }
    case 8: {
        double si = r_f64(r);
        int8_t dim[8];
        memcpy(dim, r_take(r, 8), 8);
        if (is_dimless(dim)) return mk_float(si);
        return mk_qty(si, dim, -1);
    }
    default: r->bad = true; return NIL;
    }
}

/* ------------------------------------------------ transports */

static double sim_rand(void) {
    uint64_t x = SIM.rng ? SIM.rng : 0x9e3779b97f4a7c15ULL;
    x ^= x >> 12;
    x ^= x << 25;
    x ^= x >> 27;
    SIM.rng = x;
    return (double)((x * 0x2545f4914f6cdd1dULL) >> 11) / 9007199254740992.0;
}
static void sim_send_one(uint32_t from, uint32_t to, double now, const uint8_t *data, size_t len) {
    SIM.sent++;
    uint32_t lo = from < to ? from : to, hi = from < to ? to : from;
    bool cut = false;
    for (int i = 0; i < SIM.ncut; i++) cut |= SIM.cut[i].a == lo && SIM.cut[i].b == hi;
    if (cut || (SIM.loss > 0 && sim_rand() < SIM.loss)) { SIM.dropped++; return; }
    double delay = SIM.latency + (SIM.jitter > 0 ? sim_rand() * SIM.jitter : 0);
    SIM.queue = xrealloc(SIM.queue, (size_t)(SIM.nq + 1) * sizeof *SIM.queue);
    SIM.queue[SIM.nq++] = (struct SimPkt){.to = to, .from = from, .at = now + delay, .order = ++SIM.order,
                                          .data = dupbytes(data, len), .len = len};
}

static uint16_t crc16(const uint8_t *p, size_t n) {
    uint16_t c = 0xffff;
    for (size_t i = 0; i < n; i++) {
        c ^= (uint16_t)p[i] << 8;
        for (int b = 0; b < 8; b++) c = (uint16_t)((c & 0x8000) ? (c << 1) ^ 0x1021 : c << 1);
    }
    return c;
}
/* UART frame: COBS( dest node id (u64 LE, 0 = everyone) | packet | crc16 LE ) then a zero byte. */
static void uart_send(LNode *n, uint64_t dest, const uint8_t *data, size_t len) {
    size_t raw_n = len + 10;
    uint8_t *raw = xmalloc(raw_n), *out = xmalloc(raw_n + raw_n / 254 + 3);
    for (int i = 0; i < 8; i++) raw[i] = (uint8_t)(dest >> (8 * i));
    memcpy(raw + 8, data, len);
    uint16_t c = crc16(raw, len + 8);
    raw[len + 8] = (uint8_t)c;
    raw[len + 9] = (uint8_t)(c >> 8);
    size_t o = 1, code_at = 0;
    uint8_t code = 1;
    for (size_t i = 0; i < raw_n; i++) {
        if (raw[i] == 0) { out[code_at] = code; code_at = o++; code = 1; }
        else {
            out[o++] = raw[i];
            if (++code == 0xff) { out[code_at] = code; code_at = o++; code = 1; }
        }
    }
    out[code_at] = code;
    out[o++] = 0;
    tulpa_hal_uart_write(&n->uart, out, o);
    free(raw);
    free(out);
}

static void node_send(LNode *n, const LAddr *to, double now, const uint8_t *data, size_t len) {
    switch (n->transport) {
    case TR_SIM:
        if (to) sim_send_one(n->sim_addr, (uint32_t)to->a, now, data, len);
        else
            for (int i = 0; i < SIM.nnodes; i++)
                if (SIM.nodes[i].addr != n->sim_addr && SIM.nodes[i].domain == n->domain) sim_send_one(n->sim_addr, SIM.nodes[i].addr, now, data, len);
        break;
    case TR_UART: uart_send(n, to ? to->a : 0, data, len); break;
#ifdef TULPA_LINK_UDP
    case TR_UDP: {
        struct sockaddr_in sa;
        memset(&sa, 0, sizeof sa);
        sa.sin_family = AF_INET;
        if (to) {
            sa.sin_addr.s_addr = htonl((uint32_t)to->a);
            sa.sin_port = htons(to->port);
            sendto(n->sock, data, len, 0, (struct sockaddr *)&sa, sizeof sa);
            break;
        }
        for (int k = 0; k < TL_PORTS; k++) {
            uint16_t p = (uint16_t)(n->base + k);
            sa.sin_port = htons(p);
            if (p != n->port) { sa.sin_addr.s_addr = htonl(0x7f000001u); sendto(n->sock, data, len, 0, (struct sockaddr *)&sa, sizeof sa); }
            sa.sin_addr.s_addr = htonl(0xffffffffu);
            sendto(n->sock, data, len, 0, (struct sockaddr *)&sa, sizeof sa);
            for (int e = 0; e < n->nextra; e++) { sa.sin_addr.s_addr = htonl(n->extra[e]); sendto(n->sock, data, len, 0, (struct sockaddr *)&sa, sizeof sa); }
        }
        break;
    }
#endif
    default: break;
    }
}

/* ------------------------------------------------ node logic */

static void header(LNode *n, SB *w, uint8_t kind) {
    sb_put(w, "TL", 2);
    w_u8(w, 1);
    w_u8(w, kind);
    w_le(w, n->domain, 2);
    w_le(w, n->id, 8);
}
static TV ev_new(const char *kind) { /* left on the stack by the caller's push */
    Dict *d = dict_new();
    TV dv = mk_obj(&d->h);
    push(dv);
    dict_set_c(d, "kind", str_c(kind));
    R.sp--;
    return dv;
}
static void ev_str(TV ev, const char *k, const char *v) { push(ev); dict_set_c(DICT(ev), k, str_c(v)); R.sp--; }
static void ev_emit(LNode *n, TV ev) { list_push(LIST(n->owner->extra), ev); }

static Pub *find_pub(LNode *n, const char *topic) {
    for (int i = 0; i < n->npubs; i++) if (!strcmp(n->pubs[i].topic, topic)) return &n->pubs[i];
    return NULL;
}
static Sub *find_sub(LNode *n, const char *topic) {
    for (int i = 0; i < n->nsubs; i++) if (!strcmp(n->subs[i].topic, topic)) return &n->subs[i];
    return NULL;
}
static Peer *find_peer(LNode *n, uint64_t id) {
    for (int i = 0; i < n->npeers; i++) if (n->peers[i].id == id) return &n->peers[i];
    return NULL;
}
static bool has_service(LNode *n, const char *name) {
    for (int i = 0; i < n->nservices; i++) if (!strcmp(n->services[i], name)) return true;
    return false;
}
static void free_ends(Peer *p) {
    for (int i = 0; i < p->n; i++) free(p->ends[i].topic);
    free(p->ends);
}
static void drop_peer(LNode *n, Peer *p, const char *reason) {
    TV ev = ev_new("peer_lost");
    push(ev);
    ev_str(ev, "peer", p->name);
    ev_str(ev, "reason", reason);
    ev_emit(n, ev);
    R.sp--;
    free(p->name);
    free_ends(p);
    int at = (int)(p - n->peers);
    memmove(p, p + 1, (size_t)(n->npeers - at - 1) * sizeof(Peer));
    n->npeers--;
}

static void announce(LNode *n, double now) {
    SB w = {0};
    header(n, &w, TL_ANNOUNCE);
    w_str(&w, n->name);
    w_le(&w, (uint64_t)(n->npubs + n->nsubs + n->nservices), 2);
    for (int i = 0; i < n->npubs; i++) {
        Pub *p = &n->pubs[i];
        w_u8(&w, 0); w_str(&w, p->topic); w_le(&w, sch_hash(p->schema), 4);
        w_u8(&w, (uint8_t)(p->reliable | (p->latched << 1)));
        w_le(&w, p->seq, 8);
    }
    for (int i = 0; i < n->nsubs; i++) {
        Sub *s = &n->subs[i];
        w_u8(&w, 1); w_str(&w, s->topic); w_le(&w, sch_hash(s->schema), 4); w_u8(&w, s->reliable); w_le(&w, 0, 8);
    }
    for (int i = 0; i < n->nservices; i++) { w_u8(&w, 2); w_str(&w, n->services[i]); w_le(&w, 0, 4); w_u8(&w, 0); w_le(&w, 0, 8); }
    node_send(n, NULL, now, (uint8_t *)w.p, w.n);
    free(w.p);
    n->last_announce = now;
    n->dirty = false;
}
static void data_packet(LNode *n, SB *w, const char *topic, uint64_t seq, double stamp, const uint8_t *payload, size_t len) {
    header(n, w, TL_DATA);
    w_str(w, topic);
    w_le(w, seq, 8);
    w_f64(w, stamp);
    sb_put(w, (const char *)payload, len);
}

static void emit_msg(LNode *n, Sub *s, const char *topic, const char *from_name, uint64_t seq, double stamp, TV v, double now) {
    s->received++;
    s->has_rx = true;
    s->last_rx = now;
    s->missed = false;
    int base = R.sp;
    push(v);
    TV shaped = sch_shape(s->schema, v);
    push(shaped);
    TV ev = ev_new("msg");
    push(ev);
    ev_str(ev, "topic", topic);
    dict_set_c(DICT(ev), "value", shaped);
    ev_str(ev, "from", from_name);
    dict_set_c(DICT(ev), "seq", mk_int((int64_t)seq));
    dict_set_c(DICT(ev), "stamp", mk_float(stamp));
    dict_set_c(DICT(ev), "latency", mk_float(now - stamp));
    ev_emit(n, ev);
    R.sp = base;
}

static void deliver(LNode *n, const char *topic, uint64_t from, const char *from_name, uint64_t seq, double stamp, TV v,
                    const uint8_t *payload, size_t len, double now, bool in_order) {
    Sub *s = find_sub(n, topic);
    if (!s) return;
    char why[160];
    push(v);
    if (!sch_check(s->schema, v, topic, why, sizeof why)) {
        TV ev = ev_new("incompatible");
        push(ev);
        ev_str(ev, "topic", topic);
        ev_str(ev, "peer", from_name);
        ev_str(ev, "reason", why);
        ev_emit(n, ev);
        R.sp -= 2;
        return;
    }
    bool reliable = s->reliable && !in_order;
    Stream *st = NULL;
    for (int i = 0; i < s->ns; i++) if (s->streams[i].from == from) st = &s->streams[i];
    if (!st) {
        s->streams = xrealloc(s->streams, (size_t)(s->ns + 1) * sizeof(Stream));
        st = &s->streams[s->ns++];
        *st = (Stream){.from = from, .expected = seq};
    }
    if (seq < st->expected) { R.sp--; return; } /* duplicate or stale */
    if (!reliable) {
        if (seq > st->expected) s->lost += seq - st->expected;
        st->expected = seq + 1;
        emit_msg(n, s, topic, from_name, seq, stamp, v, now);
        R.sp--;
        return;
    }
    R.sp--;
    bool have = false;
    for (int i = 0; i < st->n; i++) have |= st->held[i].seq == seq;
    if (!have) {
        st->held = xrealloc(st->held, (size_t)(st->n + 1) * sizeof(Held));
        st->held[st->n++] = (Held){.seq = seq, .stamp = stamp, .data = dupbytes(payload, len), .len = len};
    }
    for (;;) {
        int at = -1;
        for (int i = 0; i < st->n; i++) if (st->held[i].seq == st->expected) at = i;
        if (at < 0) break;
        Held h = st->held[at];
        st->held[at] = st->held[--st->n];
        Rdr r = {h.data, h.len, false};
        TV hv = r_value(&r, 0);
        uint64_t q = st->expected++;
        emit_msg(n, s, topic, from_name, q, h.stamp, hv, now);
        free(h.data);
        /* emit_msg cannot move the stream arrays, so `st` stays valid */
    }
}

static void request_missing(LNode *n, const char *topic, uint64_t from, uint64_t last, double now) {
    Sub *s = find_sub(n, topic);
    Peer *p = find_peer(n, from);
    if (!s || !s->reliable || !p) return;
    Stream *st = NULL;
    for (int i = 0; i < s->ns; i++) if (s->streams[i].from == from) st = &s->streams[i];
    if (!st) return;
    uint64_t missing[64];
    int nm = 0;
    for (uint64_t q = st->expected; q <= last && nm < 64; q++) {
        bool held = false;
        for (int i = 0; i < st->n; i++) held |= st->held[i].seq == q;
        if (!held) missing[nm++] = q;
    }
    if (!nm) return;
    SB w = {0};
    header(n, &w, TL_NACK);
    w_str(&w, topic);
    w_le(&w, (uint64_t)nm, 2);
    for (int i = 0; i < nm; i++) w_le(&w, missing[i], 8);
    node_send(n, &p->addr, now, (uint8_t *)w.p, w.n);
    free(w.p);
}

static void handle(LNode *n, LAddr from_addr, const uint8_t *data, size_t len, double now) {
    Rdr r = {data, len, false};
    const uint8_t *magic = r_take(&r, 2);
    if (r.bad || magic[0] != 'T' || magic[1] != 'L' || r_le(&r, 1) != 1) return;
    uint8_t kind = (uint8_t)r_le(&r, 1);
    uint16_t domain = (uint16_t)r_le(&r, 2);
    uint64_t id = r_le(&r, 8);
    if (r.bad || domain != n->domain || id == n->id) return;
    if (n->transport == TR_UART) from_addr.a = id;
    int base = R.sp;
    switch (kind) {
    case TL_ANNOUNCE: {
        char *name = r_str(&r);
        int count = (int)r_le(&r, 2);
        Endpoint *ends = xmalloc((size_t)(count + 1) * sizeof(Endpoint));
        int ne = 0;
        for (int i = 0; i < count && !r.bad; i++) {
            Endpoint *e = &ends[ne++];
            e->role = (uint8_t)r_le(&r, 1);
            e->topic = r_str(&r);
            e->hash = (uint32_t)r_le(&r, 4);
            e->flags = (uint8_t)r_le(&r, 1);
            e->last_seq = r_le(&r, 8);
        }
        if (r.bad) {
            for (int i = 0; i < ne; i++) free(ends[i].topic);
            free(ends);
            free(name);
            return;
        }
        Peer *known = find_peer(n, id);
        if (!known) {
            TV ev = ev_new("peer_joined");
            push(ev);
            ev_str(ev, "peer", name);
            ev_emit(n, ev);
            R.sp = base;
            n->dirty = true; /* introduce ourselves promptly */
        }
        for (int i = 0; i < ne; i++) {
            Endpoint *e = &ends[i];
            bool is_new = true;
            if (known) for (int k = 0; k < known->n; k++) is_new &= !(known->ends[k].role == e->role && !strcmp(known->ends[k].topic, e->topic));
            Sub *s = e->role == 0 ? find_sub(n, e->topic) : NULL;
            Pub *p = e->role == 1 ? find_pub(n, e->topic) : NULL;
            Sch *mine = s ? s->schema : p ? p->schema : NULL;
            if (mine) {
                uint32_t h = sch_hash(mine);
                if (h != e->hash && h != 0 && e->hash != 0) {
                    bool warned = false;
                    for (int k = 0; k < n->nwarned; k++) warned |= n->warned[k].id == id && !strcmp(n->warned[k].topic, e->topic);
                    if (!warned) {
                        n->warned = xrealloc(n->warned, (size_t)(n->nwarned + 1) * sizeof *n->warned);
                        n->warned[n->nwarned].id = id;
                        n->warned[n->nwarned++].topic = dupstr(e->topic);
                        SB b = {0};
                        sb_puts(&b, "schema mismatch: this node uses ");
                        sch_canon(mine, &b);
                        TV ev = ev_new("incompatible");
                        push(ev);
                        ev_str(ev, "topic", e->topic);
                        ev_str(ev, "peer", name);
                        ev_str(ev, "reason", b.p);
                        free(b.p);
                        ev_emit(n, ev);
                        R.sp = base;
                    }
                    continue;
                }
            }
            if (e->role == 1 && is_new && p && p->latched && p->nh) {
                Held *h = &p->history[p->nh - 1];
                SB w = {0};
                data_packet(n, &w, e->topic, h->seq, now, h->data, h->len);
                node_send(n, &from_addr, now, (uint8_t *)w.p, w.n);
                free(w.p);
            }
            if (e->role == 0 && (e->flags & 1) && s && s->reliable) {
                bool have = false;
                for (int k = 0; k < s->ns; k++) have |= s->streams[k].from == id;
                if (!have) {
                    uint64_t expected = (e->flags & 2) ? (e->last_seq > 1 ? e->last_seq : 1) : e->last_seq + 1;
                    s->streams = xrealloc(s->streams, (size_t)(s->ns + 1) * sizeof(Stream));
                    s->streams[s->ns++] = (Stream){.from = id, .expected = expected};
                }
            }
        }
        if (known) {
            free(known->name);
            free_ends(known);
        } else {
            n->peers = xrealloc(n->peers, (size_t)(n->npeers + 1) * sizeof(Peer));
            int at = n->npeers++;
            while (at > 0 && n->peers[at - 1].id > id) { n->peers[at] = n->peers[at - 1]; at--; }
            known = &n->peers[at];
        }
        *known = (Peer){.id = id, .name = name, .addr = from_addr, .last_seen = now, .ends = ends, .n = ne};
        for (int i = 0; i < ne; i++)
            if (ends[i].role == 0 && (ends[i].flags & 1)) request_missing(n, ends[i].topic, id, ends[i].last_seq, now);
        break;
    }
    case TL_DATA: {
        char *topic = r_str(&r);
        uint64_t seq = r_le(&r, 8);
        double stamp = r_f64(&r);
        const uint8_t *payload = r.p;
        size_t plen = r.n;
        TV v = r_value(&r, 0);
        if (r.bad) { free(topic); return; }
        push(v);
        Peer *p = find_peer(n, id);
        char name[72];
        if (p) { snprintf(name, sizeof name, "%s", p->name); p->last_seen = now; }
        else snprintf(name, sizeof name, "%016llx", (unsigned long long)id);
        deliver(n, topic, id, name, seq, stamp, v, payload, plen - r.n, now, false);
        Sub *s = find_sub(n, topic);
        if (s)
            for (int i = 0; i < s->ns; i++)
                if (s->streams[i].from == id && s->streams[i].n) {
                    uint64_t last = 0;
                    for (int k = 0; k < s->streams[i].n; k++) if (s->streams[i].held[k].seq > last) last = s->streams[i].held[k].seq;
                    request_missing(n, topic, id, last, now);
                }
        free(topic);
        break;
    }
    case TL_NACK: {
        char *topic = r_str(&r);
        int count = (int)r_le(&r, 2);
        Pub *p = find_pub(n, topic);
        for (int i = 0; i < count && !r.bad; i++) {
            uint64_t q = r_le(&r, 8);
            if (!p) continue;
            for (int k = 0; k < p->nh; k++)
                if (p->history[k].seq == q) {
                    SB w = {0};
                    data_packet(n, &w, topic, q, now, p->history[k].data, p->history[k].len);
                    node_send(n, &from_addr, now, (uint8_t *)w.p, w.n);
                    free(w.p);
                }
        }
        free(topic);
        break;
    }
    case TL_REQUEST: {
        char *service = r_str(&r);
        uint64_t req = r_le(&r, 8);
        TV v = r_value(&r, 0);
        if (r.bad || !has_service(n, service)) { free(service); return; } /* someone else's service */
        push(v);
        for (int i = 0; i < n->nanswered; i++)
            if (n->answered[i].id == id && n->answered[i].req == req) {
                node_send(n, &from_addr, now, n->answered[i].pkt, n->answered[i].len);
                free(service);
                R.sp = base;
                return;
            }
        for (int i = 0; i < n->nprogress; i++)
            if (n->in_progress[i].id == id && n->in_progress[i].req == req) { free(service); R.sp = base; return; }
        n->in_progress = xrealloc(n->in_progress, (size_t)(n->nprogress + 1) * sizeof *n->in_progress);
        n->in_progress[n->nprogress].id = id;
        n->in_progress[n->nprogress++].req = req;
        Peer *p = find_peer(n, id);
        TV ev = ev_new("request");
        push(ev);
        ev_str(ev, "service", service);
        dict_set_c(DICT(ev), "id", mk_int((int64_t)req));
        dict_set_c(DICT(ev), "from", mk_int((int64_t)id));
        ev_str(ev, "peer", p ? p->name : "");
        dict_set_c(DICT(ev), "value", v);
        ev_emit(n, ev);
        free(service);
        break;
    }
    case TL_REPLY: {
        uint64_t req = r_le(&r, 8);
        bool ok = r_le(&r, 1) != 0;
        TV v = r_value(&r, 0);
        if (r.bad) return;
        push(v);
        for (int i = 0; i < n->npending; i++)
            if (n->pending[i].req == req) {
                free(n->pending[i].pkt);
                n->pending[i] = n->pending[--n->npending];
                TV ev = ev_new("reply");
                push(ev);
                dict_set_c(DICT(ev), "id", mk_int((int64_t)req));
                dict_set_c(DICT(ev), "ok", mk_bool(ok));
                dict_set_c(DICT(ev), "value", v);
                ev_emit(n, ev);
                break;
            }
        break;
    }
    case TL_BYE: {
        Peer *p = find_peer(n, id);
        if (p) drop_peer(n, p, "left");
        break;
    }
    default: break;
    }
    R.sp = base;
}

static void uart_pump(LNode *n, double now) {
    uint8_t chunk[64];
    for (int rounds = 0; rounds < 256; rounds++) {
        int avail = tulpa_hal_uart_available(&n->uart);
        if (avail <= 0) break;
        int got = tulpa_hal_uart_read(&n->uart, chunk, (size_t)(avail < 64 ? avail : 64));
        if (got <= 0) break;
        for (int i = 0; i < got; i++) {
            if (chunk[i] != 0) {
                if (n->rxn < TL_MAX_PACKET + 64) n->rx[n->rxn++] = chunk[i];
                continue;
            }
            /* end of frame: undo COBS in place */
            size_t in = 0, out = 0, fn = n->rxn;
            n->rxn = 0;
            bool ok = fn > 0;
            while (ok && in < fn) {
                uint8_t code = n->rx[in++];
                if (in + code - 1 > fn) { ok = false; break; }
                for (uint8_t k = 1; k < code; k++) n->rx[out++] = n->rx[in++];
                if (code != 0xff && in < fn) n->rx[out++] = 0;
            }
            if (!ok || out < 10) continue;
            uint16_t want = (uint16_t)(n->rx[out - 2] | (n->rx[out - 1] << 8));
            if (crc16(n->rx, out - 2) != want) continue;
            uint64_t dest = 0;
            for (int k = 0; k < 8; k++) dest |= (uint64_t)n->rx[k] << (8 * k);
            if (dest != 0 && dest != n->id) continue;
            LAddr from = {.kind = TR_UART};
            uint8_t *pkt = dupbytes(n->rx + 8, out - 10);
            handle(n, from, pkt, out - 10, now);
            free(pkt);
        }
    }
}

static void pump(LNode *n, double now) {
    if (n->closed) return;
    if (n->transport == TR_SIM) {
        for (;;) { /* due packets, oldest first */
            int best = -1;
            for (int i = 0; i < SIM.nq; i++) {
                struct SimPkt *p = &SIM.queue[i];
                if (p->to != n->sim_addr || p->at > now + 1e-12) continue;
                if (best < 0 || p->at < SIM.queue[best].at || (p->at == SIM.queue[best].at && p->order < SIM.queue[best].order)) best = i;
            }
            if (best < 0) break;
            struct SimPkt p = SIM.queue[best];
            memmove(&SIM.queue[best], &SIM.queue[best + 1], (size_t)(SIM.nq - best - 1) * sizeof p);
            SIM.nq--;
            LAddr from = {.kind = TR_SIM, .a = p.from};
            handle(n, from, p.data, p.len, now);
            free(p.data);
        }
    } else if (n->transport == TR_UART) uart_pump(n, now);
#ifdef TULPA_LINK_UDP
    else {
        static uint8_t buf[TL_MAX_PACKET];
        for (int k = 0; k < 256; k++) {
            struct sockaddr_in sa;
            socklen_t sl = sizeof sa;
            ssize_t got = recvfrom(n->sock, buf, sizeof buf, 0, (struct sockaddr *)&sa, &sl);
            if (got <= 0) break;
            LAddr from = {.kind = TR_UDP, .a = ntohl(sa.sin_addr.s_addr), .port = ntohs(sa.sin_port)};
            handle(n, from, buf, (size_t)got, now);
        }
    }
#endif
    if (n->dirty || now - n->last_announce >= 0.25) announce(n, now);
    for (int i = 0; i < n->npending; i++) {
        Pending *p = &n->pending[i];
        if (now - p->last >= 0.05 && p->tries < 40) {
            p->last = now;
            p->tries++;
            node_send(n, &p->addr, now, p->pkt, p->len);
        }
    }
    for (int i = n->npeers - 1; i >= 0; i--)
        if (now - n->peers[i].last_seen > 1.5) drop_peer(n, &n->peers[i], "timeout");
    for (int i = 0; i < n->nsubs; i++) {
        Sub *s = &n->subs[i];
        if (s->has_deadline && s->has_rx && !s->missed && now - s->last_rx > s->deadline) {
            s->missed = true;
            TV ev = ev_new("deadline");
            push(ev);
            ev_str(ev, "topic", s->topic);
            dict_set_c(DICT(ev), "age", mk_float(now - s->last_rx));
            ev_emit(n, ev);
            R.sp--;
        }
    }
}

/* ------------------------------------------------ tlink:: natives */

static LNode *node_arg(int argc, TV *argv) {
    if (argc < 1 || argv[0].t != T_NATIVE || ((Native *)argv[0].u.o)->kind != N_LINK) fail("link: expected a node handle");
    return (LNode *)((Native *)argv[0].u.o)->d.ptr;
}
static const char *text_arg(int argc, TV *argv, int i, const char *what) {
    if (argc <= i || argv[i].t != T_STR) fail("link: %s must be a string", what);
    return STR(argv[i])->s;
}
static bool opt(int argc, TV *argv, int i, const char *key, TV *out) {
    if (argc <= i || argv[i].t != T_DICT) return false;
    int ix = dict_find_c(DICT(argv[i]), key);
    if (ix < 0) return false;
    *out = DICT(argv[i])->v[ix];
    return true;
}
static bool opt_num(int argc, TV *argv, int i, const char *key, double *out) {
    TV v;
    if (!opt(argc, argv, i, key, &v)) return false;
    if (v.t == T_QTY) { *out = QTY(v)->si; return true; }
    return as_f64(v, out);
}
static uint64_t fnv64(const char *s) {
    uint64_t h = 0xcbf29ce484222325ULL;
    for (; *s; s++) h = (h ^ (uint8_t)*s) * 0x00000100000001b3ULL;
    return h;
}

B(tl_node) {
    const char *name = text_arg(argc, argv, 0, "the node name");
    double domain = 0;
    opt_num(argc, argv, 1, "domain", &domain);
    TV tv;
    char kind[16] = "";
    if (opt(argc, argv, 1, "transport", &tv) && tv.t == T_STR) snprintf(kind, sizeof kind, "%s", STR(tv)->s);
    if (!*kind) {
#if defined(__linux__) || defined(__APPLE__)
        const char *env = getenv("TULPA_LINK");
        if (env) snprintf(kind, sizeof kind, "%s", env);
#endif
        if (!*kind) {
#ifdef TULPA_LINK_UDP
            snprintf(kind, sizeof kind, "%s", tulpa_sim_gpio_level ? "sim" : "udp");
#else
            snprintf(kind, sizeof kind, "sim");
#endif
        }
    }
    LNode *n = xmalloc(sizeof(LNode));
    memset(n, 0, sizeof *n);
    n->name = dupstr(name);
    n->domain = (uint16_t)domain;
    n->last_announce = -1.0;
    n->dirty = true;
    n->sock = -1;
    if (!strcmp(kind, "sim")) {
        n->transport = TR_SIM;
        n->sim_addr = ++SIM.next;
        SIM.nodes = xrealloc(SIM.nodes, (size_t)(SIM.nnodes + 1) * sizeof *SIM.nodes);
        SIM.nodes[SIM.nnodes].addr = n->sim_addr;
        SIM.nodes[SIM.nnodes++].domain = n->domain;
        n->id = fnv64(name) ^ ((uint64_t)n->sim_addr << 48);
    } else if (!strcmp(kind, "uart") || !strcmp(kind, "serial")) {
        TV port;
        double baud = 115200;
        if (!opt(argc, argv, 1, "port", &port) || port.t != T_STR) fail("link: the uart transport needs a port, e.g. {transport: \"uart\", port: \"uart0\"}");
        opt_num(argc, argv, 1, "baud", &baud);
        n->transport = TR_UART;
        n->rx = xmalloc(TL_MAX_PACKET + 64);
        tulpa_uart_open(&n->uart, dupstr(STR(port)->s), (int32_t)baud);
        n->id = fnv64(name) ^ ((tulpa_hal_micros() | 1) * 0x9e3779b97f4a7c15ULL);
    } else if (!strcmp(kind, "udp")) {
#ifdef TULPA_LINK_UDP
        if (n->domain > 200) fail("link: UDP domains are 0..=200");
        n->transport = TR_UDP;
        n->base = (uint16_t)(TL_PORT_BASE + n->domain * TL_PORTS);
        n->sock = socket(AF_INET, SOCK_DGRAM, 0);
        if (n->sock < 0) fail("link: cannot open a UDP socket (is the network up?)");
        bool bound = false;
        for (int k = 0; k < TL_PORTS && !bound; k++) {
            struct sockaddr_in sa;
            memset(&sa, 0, sizeof sa);
            sa.sin_family = AF_INET;
            sa.sin_addr.s_addr = htonl(INADDR_ANY);
            sa.sin_port = htons((uint16_t)(n->base + k));
            if (bind(n->sock, (struct sockaddr *)&sa, sizeof sa) == 0) { bound = true; n->port = (uint16_t)(n->base + k); }
        }
        if (!bound) {
            close(n->sock);
            fail("link: no free UDP port in %d..%d (too many nodes on this host)", n->base, n->base + TL_PORTS);
        }
        fcntl(n->sock, F_SETFL, fcntl(n->sock, F_GETFL, 0) | O_NONBLOCK);
        int yes = 1;
        setsockopt(n->sock, SOL_SOCKET, SO_BROADCAST, &yes, sizeof yes);
        TV peers;
        if (opt(argc, argv, 1, "peers", &peers) && peers.t == T_LIST)
            for (uint32_t i = 0; i < LIST(peers)->n && n->nextra < 8; i++) {
                char host[48];
                show_buf(LIST(peers)->v[i], host, sizeof host);
                struct in_addr ia;
                if (inet_pton(AF_INET, host, &ia) != 1) fail("link: peer '%s' is not an IP address", host);
                n->extra[n->nextra++] = ntohl(ia.s_addr);
            }
        n->id = fnv64(name) ^ (((tulpa_hal_micros() | 1) ^ ((uint64_t)n->port << 32)) * 0x9e3779b97f4a7c15ULL);
#else
        fail("link: this board has no network stack; use {transport: \"uart\", port: ...} with `tulpa link bridge`");
#endif
    } else fail("link: unknown transport '%s' (use \"sim\", \"udp\" or \"uart\")", kind);
    Native *h = native_new(N_LINK);
    h->d.ptr = n;
    n->owner = h;
    TV hv = mk_obj(&h->h);
    push(hv);
    h->extra = mk_obj(&list_new(0)->h);
    R.sp--;
    announce(n, now_secs());
    return hv;
}
B(tl_advertise) {
    LNode *n = node_arg(argc, argv);
    const char *topic = text_arg(argc, argv, 1, "the topic name");
    Sch *schema = sch_parse(argc > 2 ? argv[2] : NIL);
    Pub *p = find_pub(n, topic);
    if (p) {
        if (sch_hash(p->schema) != sch_hash(schema)) {
            SB b = {0};
            sch_canon(p->schema, &b);
            char canon[120];
            snprintf(canon, sizeof canon, "%s", b.p);
            free(b.p);
            fail("link: '%s' is already advertised as %s", topic, canon);
        }
        return NIL;
    }
    TV f;
    double keep = 32;
    opt_num(argc, argv, 3, "keep", &keep);
    n->pubs = xrealloc(n->pubs, (size_t)(n->npubs + 1) * sizeof(Pub));
    int at = n->npubs++;
    while (at > 0 && strcmp(n->pubs[at - 1].topic, topic) > 0) { n->pubs[at] = n->pubs[at - 1]; at--; }
    n->pubs[at] = (Pub){.topic = dupstr(topic), .schema = schema, .keep = keep < 1 ? 1 : (int)keep,
                        .reliable = opt(argc, argv, 3, "reliable", &f) && truthy(f),
                        .latched = opt(argc, argv, 3, "latched", &f) && truthy(f)};
    n->dirty = true;
    return NIL;
}
B(tl_subscribe) {
    LNode *n = node_arg(argc, argv);
    const char *topic = text_arg(argc, argv, 1, "the topic name");
    Sch *schema = sch_parse(argc > 2 ? argv[2] : NIL);
    Sub *s = find_sub(n, topic);
    if (s) {
        if (sch_hash(s->schema) != sch_hash(schema)) {
            SB b = {0};
            sch_canon(s->schema, &b);
            char canon[120];
            snprintf(canon, sizeof canon, "%s", b.p);
            free(b.p);
            fail("link: '%s' is already subscribed as %s", topic, canon);
        }
        return NIL;
    }
    TV f;
    double deadline = 0;
    bool has_deadline = opt_num(argc, argv, 3, "deadline", &deadline);
    n->subs = xrealloc(n->subs, (size_t)(n->nsubs + 1) * sizeof(Sub));
    int at = n->nsubs++;
    while (at > 0 && strcmp(n->subs[at - 1].topic, topic) > 0) { n->subs[at] = n->subs[at - 1]; at--; }
    n->subs[at] = (Sub){.topic = dupstr(topic), .schema = schema, .reliable = opt(argc, argv, 3, "reliable", &f) && truthy(f),
                        .has_deadline = has_deadline, .deadline = deadline};
    n->dirty = true;
    return NIL;
}
B(tl_publish) {
    LNode *n = node_arg(argc, argv);
    const char *topic = text_arg(argc, argv, 1, "the topic name");
    TV v = argc > 2 ? argv[2] : NIL;
    double now = now_secs();
    Pub *p = find_pub(n, topic);
    if (!p) fail("link: '%s' is not advertised by node '%s'", topic, n->name);
    char why[160];
    if (!sch_check(p->schema, v, topic, why, sizeof why)) fail("%s", why);
    SB w = {0};
    w_value_or_fail(&w, v);
    p->seq++;
    p->sent++;
    if (p->nh == p->keep) {
        free(p->history[0].data);
        memmove(p->history, p->history + 1, (size_t)(p->nh - 1) * sizeof(Held));
        p->nh--;
    }
    p->history = xrealloc(p->history, (size_t)(p->nh + 1) * sizeof(Held));
    p->history[p->nh++] = (Held){.seq = p->seq, .data = dupbytes((uint8_t *)w.p, w.n), .len = w.n};
    uint32_t hash = sch_hash(p->schema);
    SB pkt = {0};
    data_packet(n, &pkt, topic, p->seq, now, (uint8_t *)w.p, w.n);
    for (int i = 0; i < n->npeers; i++) {
        bool listens = false;
        for (int k = 0; k < n->peers[i].n; k++) {
            Endpoint *e = &n->peers[i].ends[k];
            listens |= e->role == 1 && !strcmp(e->topic, topic) && (e->hash == hash || e->hash == 0 || hash == 0);
        }
        if (listens) node_send(n, &n->peers[i].addr, now, (uint8_t *)pkt.p, pkt.n);
    }
    free(pkt.p);
    uint64_t seq = p->seq;
    if (find_sub(n, topic)) deliver(n, topic, n->id, n->name, seq, now, v, (uint8_t *)w.p, w.n, now, true);
    free(w.p);
    return NIL;
}
B(tl_serve) {
    LNode *n = node_arg(argc, argv);
    const char *name = text_arg(argc, argv, 1, "the service name");
    if (!has_service(n, name)) {
        n->services = xrealloc(n->services, (size_t)(n->nservices + 1) * sizeof(char *));
        int at = n->nservices++;
        while (at > 0 && strcmp(n->services[at - 1], name) > 0) { n->services[at] = n->services[at - 1]; at--; }
        n->services[at] = dupstr(name);
    }
    n->dirty = true;
    return NIL;
}
B(tl_request) {
    LNode *n = node_arg(argc, argv);
    const char *service = text_arg(argc, argv, 1, "the service name");
    TV v = argc > 2 ? argv[2] : NIL;
    double now = now_secs();
    uint64_t req = ++n->next_req;
    if (has_service(n, service)) {
        TV ev = ev_new("request");
        push(ev);
        ev_str(ev, "service", service);
        dict_set_c(DICT(ev), "id", mk_int((int64_t)req));
        dict_set_c(DICT(ev), "from", mk_int((int64_t)n->id));
        ev_str(ev, "peer", n->name);
        dict_set_c(DICT(ev), "value", v);
        ev_emit(n, ev);
        R.sp--;
        return mk_int((int64_t)req);
    }
    Peer *target = NULL;
    for (int i = 0; i < n->npeers && !target; i++)
        for (int k = 0; k < n->peers[i].n; k++)
            if (n->peers[i].ends[k].role == 2 && !strcmp(n->peers[i].ends[k].topic, service)) target = &n->peers[i];
    if (!target) return NIL;
    SB w = {0};
    header(n, &w, TL_REQUEST);
    w_str(&w, service);
    w_le(&w, req, 8);
    w_value_or_fail(&w, v);
    n->pending = xrealloc(n->pending, (size_t)(n->npending + 1) * sizeof(Pending));
    n->pending[n->npending++] = (Pending){.req = req, .addr = target->addr, .pkt = dupbytes((uint8_t *)w.p, w.n), .len = w.n, .last = now, .tries = 1};
    node_send(n, &target->addr, now, (uint8_t *)w.p, w.n);
    free(w.p);
    return mk_int((int64_t)req);
}
B(tl_reply) {
    LNode *n = node_arg(argc, argv);
    if (argc < 3 || argv[1].t != T_INT || argv[2].t != T_INT) fail("link: reply(node, from, id, ok, value)");
    uint64_t from = (uint64_t)argv[1].u.i, req = (uint64_t)argv[2].u.i;
    bool ok = argc > 3 ? truthy(argv[3]) : true;
    TV v = argc > 4 ? argv[4] : NIL;
    double now = now_secs();
    if (from == n->id) {
        TV ev = ev_new("reply");
        push(ev);
        dict_set_c(DICT(ev), "id", mk_int((int64_t)req));
        dict_set_c(DICT(ev), "ok", mk_bool(ok));
        dict_set_c(DICT(ev), "value", v);
        ev_emit(n, ev);
        R.sp--;
        return NIL;
    }
    Peer *p = find_peer(n, from);
    if (!p) return NIL;
    SB w = {0};
    header(n, &w, TL_REPLY);
    w_le(&w, req, 8);
    w_u8(&w, ok);
    w_value_or_fail(&w, v);
    for (int i = 0; i < n->nprogress; i++)
        if (n->in_progress[i].id == from && n->in_progress[i].req == req) { n->in_progress[i] = n->in_progress[--n->nprogress]; break; }
    if (n->nanswered == 64) {
        free(n->answered[0].pkt);
        memmove(n->answered, n->answered + 1, 63 * sizeof(Answered));
        n->nanswered--;
    }
    n->answered[n->nanswered++] = (Answered){.id = from, .req = req, .pkt = dupbytes((uint8_t *)w.p, w.n), .len = w.n};
    node_send(n, &p->addr, now, (uint8_t *)w.p, w.n);
    free(w.p);
    return NIL;
}
B(tl_poll) {
    LNode *n = node_arg(argc, argv);
    pump(n, now_secs());
    TV events = n->owner->extra;
    push(events);
    n->owner->extra = mk_obj(&list_new(0)->h);
    R.sp--;
    return events;
}
static TV name_list(char **names, int n) {
    List *l = list_new((uint32_t)n);
    push(mk_obj(&l->h));
    for (int i = 0; i < n; i++) list_push(l, str_c(names[i]));
    R.sp--;
    return mk_obj(&l->h);
}
static TV ends_dict(const char *name, char **pubs, int np, char **subs, int ns, char **svcs, int nv) {
    Dict *e = dict_new();
    push(mk_obj(&e->h));
    dict_set_c(e, "publishes", name_list(pubs, np));
    dict_set_c(e, "subscribes", name_list(subs, ns));
    dict_set_c(e, "services", name_list(svcs, nv));
    dict_set_c(e, "name", str_c(name));
    R.sp--;
    return mk_obj(&e->h);
}
B(tl_info) {
    LNode *n = node_arg(argc, argv);
    int base = R.sp;
    Dict *d = dict_new();
    push(mk_obj(&d->h));
    dict_set_c(d, "name", str_c(n->name));
    dict_set_c(d, "domain", mk_int(n->domain));
    char tr[24];
    if (n->transport == TR_SIM) snprintf(tr, sizeof tr, "sim");
    else if (n->transport == TR_UDP) snprintf(tr, sizeof tr, "udp:%d", n->port);
    else snprintf(tr, sizeof tr, "uart:%s", n->uart.port);
    dict_set_c(d, "transport", str_c(tr));
    List *nodes = list_new(0);
    push(mk_obj(&nodes->h));
    int most = n->npubs + n->nsubs + 1;
    for (int i = 0; i < n->npeers; i++) if (n->peers[i].n + 1 > most) most = n->peers[i].n + 1;
    char **names = xmalloc((size_t)most * 3 * sizeof(char *));
    for (int i = 0; i < n->npubs; i++) names[i] = n->pubs[i].topic;
    for (int i = 0; i < n->nsubs; i++) names[most + i] = n->subs[i].topic;
    list_push(nodes, ends_dict(n->name, names, n->npubs, names + most, n->nsubs, n->services, n->nservices));
    for (int i = 0; i < n->npeers; i++) {
        Peer *p = &n->peers[i];
        int c[3] = {0, 0, 0};
        for (int k = 0; k < p->n; k++) if (p->ends[k].role < 3) names[p->ends[k].role * most + c[p->ends[k].role]++] = p->ends[k].topic;
        list_push(nodes, ends_dict(p->name, names, c[0], names + most, c[1], names + 2 * most, c[2]));
    }
    free(names);
    dict_set_c(d, "nodes", mk_obj(&nodes->h));
    List *topics = list_new(0);
    push(mk_obj(&topics->h));
    for (int i = 0; i < n->npubs + n->nsubs; i++) {
        bool is_pub = i < n->npubs;
        Pub *p = is_pub ? &n->pubs[i] : NULL;
        Sub *s = is_pub ? NULL : &n->subs[i - n->npubs];
        TV ev = ev_new(is_pub ? "publisher" : "subscriber");
        push(ev);
        ev_str(ev, "topic", is_pub ? p->topic : s->topic);
        SB b = {0};
        sch_canon(is_pub ? p->schema : s->schema, &b);
        ev_str(ev, "schema", b.p);
        free(b.p);
        if (is_pub) {
            dict_set_c(DICT(ev), "sent", mk_int((int64_t)p->sent));
            dict_set_c(DICT(ev), "reliable", mk_bool(p->reliable));
            dict_set_c(DICT(ev), "latched", mk_bool(p->latched));
        } else {
            dict_set_c(DICT(ev), "received", mk_int((int64_t)s->received));
            dict_set_c(DICT(ev), "lost", mk_int((int64_t)s->lost));
            dict_set_c(DICT(ev), "reliable", mk_bool(s->reliable));
        }
        list_push(topics, ev);
        R.sp--;
    }
    dict_set_c(d, "topics", mk_obj(&topics->h));
    R.sp = base;
    return mk_obj(&d->h);
}
B(tl_close) {
    LNode *n = node_arg(argc, argv);
    if (n->closed) return NIL;
    SB w = {0};
    header(n, &w, TL_BYE);
    node_send(n, NULL, now_secs(), (uint8_t *)w.p, w.n);
    free(w.p);
    n->closed = true;
    if (n->transport == TR_SIM)
        for (int i = 0; i < SIM.nnodes; i++)
            if (SIM.nodes[i].addr == n->sim_addr) { SIM.nodes[i] = SIM.nodes[--SIM.nnodes]; break; }
#ifdef TULPA_LINK_UDP
    if (n->transport == TR_UDP && n->sock >= 0) { close(n->sock); n->sock = -1; }
#endif
    return NIL;
}
B(tl_sim) {
    double v;
    if (opt_num(argc, argv, 0, "latency", &v)) SIM.latency = v < 0 ? 0 : v;
    if (opt_num(argc, argv, 0, "jitter", &v)) SIM.jitter = v < 0 ? 0 : v;
    if (opt_num(argc, argv, 0, "loss", &v)) SIM.loss = v < 0 ? 0 : v > 1 ? 1 : v;
    if (opt_num(argc, argv, 0, "seed", &v)) SIM.rng = (uint64_t)v ^ 0x9e3779b97f4a7c15ULL;
    Dict *d = dict_new();
    push(mk_obj(&d->h));
    dict_set_c(d, "sent", mk_int((int64_t)SIM.sent));
    dict_set_c(d, "dropped", mk_int((int64_t)SIM.dropped));
    dict_set_c(d, "in_flight", mk_int(SIM.nq));
    R.sp--;
    return mk_obj(&d->h);
}
B(tl_partition) {
    LNode *x = node_arg(argc, argv), *y = node_arg(argc - 1, argv + 1);
    bool cut = argc > 2 ? truthy(argv[2]) : true;
    if (x->transport != TR_SIM || y->transport != TR_SIM) fail("link: partition() only applies to sim nodes");
    uint32_t a = x->sim_addr < y->sim_addr ? x->sim_addr : y->sim_addr, b = x->sim_addr < y->sim_addr ? y->sim_addr : x->sim_addr;
    for (int i = 0; i < SIM.ncut; i++)
        if (SIM.cut[i].a == a && SIM.cut[i].b == b) {
            if (!cut) SIM.cut[i] = SIM.cut[--SIM.ncut];
            return NIL;
        }
    if (cut) {
        SIM.cut = xrealloc(SIM.cut, (size_t)(SIM.ncut + 1) * sizeof *SIM.cut);
        SIM.cut[SIM.ncut].a = a;
        SIM.cut[SIM.ncut++].b = b;
    }
    return NIL;
}
static TV link_method(TV o, const char *name, int argc, TV *argv) {
    (void)o; (void)argc; (void)argv;
    fail("LinkNode has no method '%s'", name);
}

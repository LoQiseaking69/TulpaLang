/* Tulpa dynamic tier: the Tulpa VM for firmware, in portable C.
 *
 * `tulpa build` uses the static native compiler when a program fits it. Otherwise the program
 * is serialised to T-code (src/dyncode.rs) and runs here, so the whole language is available
 * on microcontrollers: dicts, closures, try/catch, tasks, signals and reactive rules, structs,
 * state machines, units, contracts, the Tulpa-source standard library, and the devices.
 *
 * Part of Tulpa (Business Source License 1.1; see LICENSE). */
#include <math.h>
#include <setjmp.h>
#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "tulpa_hal.h"

#ifndef TULPA_DYN_STACK /* value-stack slots (16 bytes each) */
#if defined(__linux__) || defined(__APPLE__) || defined(_WIN32)
#define TULPA_DYN_STACK 65536
#else
#define TULPA_DYN_STACK 2048
#endif
#endif
#ifndef TULPA_DYN_DEPTH
#define TULPA_DYN_DEPTH 2500 /* nested Tulpa calls (the C stack usually runs out first) */
#endif
#ifndef TULPA_DYN_C_STACK /* bytes of C stack the VM may use before it reports a stack overflow */
#if defined(__linux__) || defined(__APPLE__) || defined(_WIN32)
#define TULPA_DYN_C_STACK (6u * 1024 * 1024)
#elif defined(ESP_PLATFORM)
#define TULPA_DYN_C_STACK (24u * 1024)
#else
#define TULPA_DYN_C_STACK (24u * 1024)
#endif
#endif
#ifndef TULPA_DYN_GC_BYTES /* garbage is collected after this much allocation, at least */
#if defined(__linux__) || defined(__APPLE__) || defined(_WIN32)
#define TULPA_DYN_GC_BYTES (256 * 1024)
#else
#define TULPA_DYN_GC_BYTES (12 * 1024)
#endif
#endif

enum {
    K_NONE, K_INT, K_FLOAT, K_STR, K_FSTR, K_BOOL, K_NIL, K_ID, K_PATH, K_LIST, K_REPEAT, K_DICT,
    K_UNARY, K_BIN, K_LOGIC, K_RANGE, K_CALL, K_MCALL, K_INDEX, K_FIELD, K_LAMBDA, K_MACRO, K_IF,
    K_MATCH, K_ARM, K_BLOCK, K_SPAWN, K_AWAIT, K_QTY, K_NAMED, K_PWILD, K_PBIND, K_PLIT,
    K_PRANGE, K_PLIST, K_FN, K_PARAM, K_CONTRACT, K_TNAME, K_TUNIT, K_LET, K_ASSIGN, K_RETURN,
    K_WHILE, K_LOOP, K_FOR, K_BREAK, K_CONTINUE, K_ASSERT, K_STRUCT, K_IFACE, K_MACHINE, K_STATE,
    K_TRANS, K_WHEN, K_DEADLINE, K_ALWAYS, K_IMPORT, K_TRY, K_MACRODEF, K_GROUP, K_SIG
};
enum {
    OP_NONE, OP_ADD, OP_SUB, OP_MUL, OP_DIV, OP_MOD, OP_POW, OP_EQ, OP_NE, OP_LT, OP_LE, OP_GT,
    OP_GE, OP_AND, OP_OR, OP_XOR, OP_SHL, OP_SHR, OP_IN, OP_NOTIN, OP_BANG, OP_TILDE, OP_NOT
};
static const char *const OP_TEXT[] = {"", "+", "-", "*", "/", "%", "**", "==", "!=", "<", "<=", ">",
                                      ">=", "&", "|", "^", "<<", ">>", "in", "not in", "!", "~", "not"};

enum {
    T_UNSET, T_NIL, T_BOOL, T_INT, T_FLOAT, T_STR, T_LIST, T_DICT, T_RANGE, T_FN, T_BUILTIN,
    T_QTY, T_CLASS, T_OBJ, T_MTYPE, T_MACH, T_SIGNAL, T_TASK, T_NATIVE, T_THUNK, T_ENV, T_MODFN
};

typedef struct Obj {
    struct Obj *next;
    uint8_t t, mark;
} Obj;

typedef struct {
    uint8_t t;
    union {
        int64_t i;
        double f;
        Obj *o;
    } u;
} TV;

/* A node of the program, read in place from the program image (flash on a microcontroller).
 * `u` is the first child's slot in the child table, a number's slot in the number table, or a
 * path's slot in the lookup cache. */
typedef const struct NodeS {
    uint8_t k, numt;
    uint16_t n;
    int32_t a, b;
    uint32_t u;
} Node;

typedef struct { Obj h; uint32_t len; char s[]; } Str;
typedef struct { Obj h; uint32_t n, cap; TV *v; } List;
typedef struct { Obj h; uint32_t n, cap; TV *k, *v; } Dict;
typedef struct { Obj h; int64_t a, b; uint8_t incl; } Range;
typedef struct Env { Obj h; struct Env *up; uint16_t n, cap; int32_t *names; TV *vals; uint8_t *konst; } Env;
typedef struct { Obj h; Node *def; Env *env; TV self; } Fn;
typedef struct { Obj h; double si; int8_t dim[8]; int32_t unit; } Qty;
typedef struct { Obj h; Node *def; Env *env; } Class; /* also machine types */
typedef struct { Obj h; Class *cls; uint16_t n; TV f[]; } Inst;
typedef struct { Obj h; Class *ty; Env *env; int32_t state; List *history; } Mach;
typedef struct { Obj h; TV value, prev, derive; double changed_at, prev_at; List *history, *subs; int32_t capacity; } Signal;
typedef struct { Obj h; uint8_t state; TV f, result; List *args; Str *err; } Task;
typedef struct { Obj h; Node *node; Env *env; } Thunk;
typedef struct { Obj h; uint8_t kind; TV extra; union { tulpa_gpio_t gpio; tulpa_i2c_t i2c; tulpa_spi_t spi; tulpa_uart_t uart; tulpa_pwm_t pwm; tulpa_servo_t servo; double pid[10]; void *ptr; } d; } Native;

typedef struct { double scale; int32_t written, canon; int8_t dim[8]; } UnitDef;
typedef struct { jmp_buf jb; int sp, depth, line; bool reacting; } Try;
typedef struct { Node *cond, *body, *els; Env *env; bool last, has_hold, has_since; double hold, since; } Rule;
typedef struct { Node *expr, *handler; Env *env; int32_t text; bool tripped; } Monitor;
typedef struct { Node *trigger, *resp, *handler; Env *env; int32_t text; Str *shown; double within, armed_at; bool last, armed; } Deadline;

typedef struct { char *p; size_t n, cap; } SB;
enum { C_NONE, C_BREAK, C_CONT, C_RET };

static struct {
    /* program image (read in place) and what the VM adds to it at run time */
    Node *nodes; const uint32_t *kids, *roots, *sym_off; const uint16_t *lines; const uint8_t *nums; const char *strs;
    int nsyms_image, nsyms; char **extra_syms; int16_t *sym_builtin; struct Obj **symstr; int16_t *path_cache;
    UnitDef *units; int nunits;
    int nmodules; TV *module_vals;
    /* heap */
    Obj *objs; size_t allocated, threshold, live;
    /* roots */
    TV stack[TULPA_DYN_STACK]; int sp;
    Env *globals;
    List *tasks, *keep;
    /* control */
    int ctl; TV ret;
    int depth, line;
    char *stack_base; size_t stack_limit;
    Try *tries[32]; int ntry;
    char errmsg[240];
    /* macros, interfaces, rules */
    struct { int32_t name; Node *def; } *macros; int nmacros;
    Node **ifaces; int nifaces;
    uint8_t *module_state;
    int32_t s_self, s_result, s_init, s_to_str;
    Rule *rules; int nrules;
    Monitor *monitors; int nmonitors;
    Deadline *deadlines; int ndeadlines;
    bool reacting;
    uint64_t rng;
} R;

static void fail(const char *fmt, ...) __attribute__((noreturn, format(printf, 1, 2)));
static TV eval(Node *n, Env *e);
static TV exec_block(Node *b, Env *parent);
static TV run_block(Node *b, Env *e);
static TV call(TV f, int argc, TV *argv, int nkw, int32_t *kwn, TV *kwv);
static TV call_method(TV o, int32_t name, int argc, TV *argv, int nkw, int32_t *kwn, TV *kwv);
static TV get_field(TV o, int32_t name);
static void set_field(TV o, int32_t name, TV v);
static void react(void);
static void sleep_secs(double s);
static double now_secs(void);
static int32_t intern(const char *s);
static TV builtin_call(int ix, int argc, TV *argv);
static int builtin_find(const char *name);
static const char *builtin_name(int ix);
static TV builtin_method(TV o, const char *name, int argc, TV *argv);
static TV native_method(TV o, const char *name, int argc, TV *argv);
static void stringify(TV v, SB *b);
static void drain_tasks(void);
static bool path_value(const char *m, const char *name, TV *out);
static TV await_value(TV v);

static const char *sym_name(int32_t i);
#define SYM(i) sym_name(i)
#define KID(n, i) (&R.nodes[R.kids[(n)->u + (uint32_t)(i)]])
#define LINE(n) (R.lines[(n) - R.nodes])
#define NIL ((TV){.t = T_NIL})
#define UNSET ((TV){.t = T_UNSET})
static inline TV mk_bool(bool b) { TV v = {.t = T_BOOL}; v.u.i = b; return v; }
static inline TV mk_int(int64_t i) { TV v = {.t = T_INT}; v.u.i = i; return v; }
static inline TV mk_float(double f) { TV v = {.t = T_FLOAT}; v.u.f = f; return v; }
static inline TV mk_obj(Obj *o) { TV v = {.t = o->t}; v.u.o = o; return v; }
#define STR(v) ((Str *)(v).u.o)
#define LIST(v) ((List *)(v).u.o)
#define DICT(v) ((Dict *)(v).u.o)
#define QTY(v) ((Qty *)(v).u.o)
static inline void push(TV v) {
    if (R.sp >= TULPA_DYN_STACK) fail("value stack overflow (expression or call nesting too deep)");
    R.stack[R.sp++] = v;
}

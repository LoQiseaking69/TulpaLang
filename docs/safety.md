# Proving safety

Tulpa has four kinds of safety boundary in the language, and three ways to check them.

| Boundary | Written as | Meaning |
|---|---|---|
| Dimensions | `v: [m/s]`, `1[m] + 2[s]` | A length is never added to a time or passed where a speed is expected. |
| Contracts | `require`, `ensure`, `assert` | What a function needs and what it promises. |
| Invariants | `invariant` in a struct or on a loop | What stays true of an object, or across iterations. |
| Envelopes | `always cond else { ... }` | What stays true of the running system, with a corrective action. |

| Check | When | Covers |
|---|---|---|
| At run time | every run: interpreter, bundles and native firmware | the inputs that occur |
| `tulpa verify` | on demand; random tests with shrinking | the inputs it generates |
| `tulpa prove` | on demand; static | **all** inputs the `require` clauses allow |

## `tulpa prove`

```
tulpa prove [--strict] [--json] [--trials=N] file.rav...
```

```tulpa
fn slew(target: float, current: float, step: float) -> float
    require step > 0.0
    ensure abs(result' - current) <= step
{
    current + clamp(target - current, -step, step)
}

fn brake_distance(v: [m/s], decel: [m/s^2]) -> [m]
    require v >= 0[m/s]
{
    v * v / (2.0 * decel)
}
```

```
  slew()
    proved    ensure abs(result' - current) <= step
  brake_distance()
    REFUTED   divisor is never zero  (line 11)
              counterexample: brake_distance(0 m/s, 0 m/s^2)
```

For every function and struct method, the prover collects obligations:

| Obligation | Source |
|---|---|
| `ensure ...` | each postcondition |
| `assert` | each assertion in the body |
| preserves invariant ... | each struct invariant, for every method |
| default fields satisfy invariant ... | each struct invariant, for a newly made object |
| loop invariant holds on entry / is preserved | each `while ... invariant` |
| `f(): require ...` | the preconditions of every function it calls |
| divisor is never zero | each `/` and `%` |

and for every state machine: that all states are reachable, that no transition is hidden by an
earlier unguarded one on the same event, and which states have no way out.

Each obligation gets one of four answers:

- **proved**: it holds for every input that satisfies the `require` clauses.
- **refuted**: testing found a counterexample, which is shown. (Machine findings are refuted
  directly.)
- **unproved**: the prover could not decide, and testing found no counterexample.
- **skipped**: the function uses something the prover does not model (`try`/`catch`, tasks,
  reactive rules inside a function).

The exit code is 1 if anything is refuted. With `--strict` it is also 1 if anything is
unproved or skipped, which makes `tulpa prove --strict` a CI gate. `--json` prints the report
for tools; HermiSim's Proofs dock uses it.

## What the prover can show

It runs each function symbolically, one path at a time. Numbers are linear expressions over the
parameters and fields. Each path carries bounds for every symbol and the linear facts from the
branches taken. An obligation is proved when its negation is impossible on every path.

It handles:

- linear arithmetic over ints and floats, and quantities (in SI);
- `if`/`elif`/`else`, `min`, `max`, `abs`, `clamp`, comparisons, `and`/`or`/`not`;
- integer division and remainder by constants;
- calls to other functions (inlined, or summarised by their `ensure` clauses when recursive);
- loops: unrolled when the trip count is known and small, otherwise by `invariant`;
- struct fields through `self`, with the struct's invariants assumed on entry.

Non-linear results (products of two variables, `sqrt`, `sin`) are kept as bounded intervals, so
obligations that depend on their exact value come back unproved.

**Assumptions.** Floats are treated as real numbers, so rounding is not modelled. A parameter
with no type annotation is treated as a number. Proofs are about the function's logic; they do
not cover timing or what hardware does.

## Making code provable

- Annotate parameters: `x: int`, `x: float`, `v: [m/s]`. Integer facts are exact.
- State the precondition the code relies on. `limit(x, lo, hi)` needs `require lo <= hi`.
- Give loops an `invariant` that implies what you need after the loop.
- Saturate with `clamp`, `min` and `max`; the prover follows each case.

```tulpa
struct Arm {
    angle: [deg] = 0[deg]
    limit: [deg] = 120[deg]
    invariant 0[deg] <= self.angle and self.angle <= self.limit
    fn move_to(self, a: [deg]) { self.angle = clamp(a, 0[deg], self.limit) }   # proved
    fn nudge(self, d: [deg]) { self.angle = self.angle + d }                    # not proved
}
```

## Guarantees that survive deployment

- Every `require`, `ensure`, invariant, `assert` and `always` is compiled into native firmware
  as a runtime check. A violation calls `tulpa_violation(msg)`, which you can override to run
  an emergency stop.
- Dimensions are checked when firmware is built.
- `tulpa deploy` runs the prover first and refuses to build if an obligation is refuted.
- For system-level properties across event orders, use `verify.explore`
  ([Simulation and verification](simulation.md)).

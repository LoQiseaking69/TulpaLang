# Autonomy: planning, decisions and missions

`std/plan` gives a Tulpa program the pieces of an autonomy stack. They are plain Tulpa, they are
deterministic (time comes from `millis()`, so runs under `--virtual-time` repeat exactly), and
you can use each one on its own.

| Layer | What it answers | API |
|---|---|---|
| Behavior trees | "How do I carry out this step robustly?" | `Sequence`, `Selector`, `Parallel`, `Action`, `Condition`, decorators |
| Goal planning (GOAP) | "Which actions reach this goal most cheaply?" | `Planner` |
| Task decomposition (HTN) | "How does this big task break into steps here?" | `Domain` |
| Utility decisions | "Which mode is best right now?" | `choose` |
| Orchestration | "Pursue goals by priority, execute, watch, replan." | `Mission` |

```tulpa
import "std/plan"
```

## Behavior trees

```tulpa
let grasp = plan.ReactiveSequence([
    plan.Condition("estop clear", |bb| not bb.estop),          # re-checked every tick
    plan.Retry(plan.Sequence([
        plan.Action("open", |bb| gripper.open()),
        plan.Action("close", |bb| gripper.close()),            # true/false/nil or a status
    ]), 5),
], "grasp")

every!(20[ms], { if grasp.tick(blackboard) != plan.RUNNING { break } })
print(grasp.render())                                           # the tree with each node's status
```

- An action returns `plan.SUCCESS`, `plan.FAILURE` or `plan.RUNNING`. `true` and `nil` count as
  success, `false` as failure.
- `Sequence` and `Selector` remember which child was running. `ReactiveSequence` and
  `ReactiveSelector` start from the first child on every tick, so guards stay live.
- `Parallel(children, need)` succeeds when `need` children have succeeded.
- Decorators: `Invert`, `Succeed`, `Retry(child, n)`, `Repeat(child, n)`,
  `Timeout(child, 2[s])`, `Cooldown(child, 500[ms])`.
- `plan.run_tree(tree, bb, period, max_ticks)` ticks until the tree finishes.
- `tree.snapshot()` returns `{kind, name, status, children}` for tools.

## Goal planning

World states are dicts. An action has preconditions, effects and a cost. A precondition is a
value (must be equal) or a predicate; an effect is a value or a function of the old value.

```tulpa
let p = plan.Planner()
p.action("undock", {docked: true}, {docked: false})
p.action("drive_to_shelf", {docked: false, battery: |b| b >= 20}, {at: "shelf", battery: |b| b - 20}, cost: 2)
p.action("pick", {at: "shelf", holding: false}, {holding: true})

p.plan({docked: true, at: "dock", holding: false, battery: 45}, {holding: true})
# ["undock", "drive_to_shelf", "pick"]
```

- `plan(state, goal)` returns the cheapest sequence of action names, `[]` if the goal already
  holds, or `nil` if there is none. The search is A\* with deterministic tie-breaking.
- `solve(state, goal)` also returns `{steps, cost, state, expanded}`.
- `max_nodes` (default 5000) bounds the search.

## Task decomposition

```tulpa
let d = plan.Domain()
d.primitive("unlock", {key: true}, {open: true})
d.primitive("break_in", {}, {open: true, alarm: true})
d.primitive("enter", {open: true}, {inside: true})
d.method("get_in", "politely", {}, ["unlock", "enter"])
d.method("get_in", "forcefully", {}, ["break_in", "enter"])

d.decompose({key: false}, ["get_in"])       # ["break_in", "enter"]: the first method fails, so it backtracks
```

Methods are tried in the order they were added. `explain()` also returns which method was
chosen for each compound task.

## Utility decisions

```tulpa
let modes = [
    {name: "explore",  score: |w| 0.5},
    {name: "recharge", score: |w| (100 - w.battery) / 100.0},
    {name: "deliver",  score: |w| 0.8, active: |w| w.orders > 0 and w.battery > 20},
]
mode = plan.choose(modes, world, mode, 0.1).name      # keeps the current mode unless beaten by > 0.1
```

## Missions

A `Mission` owns goals with priorities. On every tick it picks the most important unsatisfied
goal, plans for it, runs the current step's handler, and replans when a step fails, when a step
is no longer applicable, or when a more important goal becomes active.

```tulpa
let m = plan.Mission(p)
m.goal("delivered", {delivered: true}, priority: 1)
m.goal("charged", {battery: |b| b >= 90}, priority: 10, active: |w| w.battery < 15)
m.on("dock", |w| { ...; plan.SUCCESS })          # handlers update the world with what really happened
every!(100[ms], { m.tick(world) })
```

- `tick(world)` returns `"idle"`, `"running"`, `"done"` or `"stuck"`.
- `run(world, period, max_ticks)` ticks until nothing is left to do.
- `m.log` is a timestamped record of plans, steps, failures and achieved goals. `m.status()`
  returns the current goal, step and remaining plan.
- A goal is abandoned after `max_failures` (default 3) failed steps in a row.

`examples/autonomy` runs all five layers: a delivery robot whose battery sags mid-route, so the
charging goal preempts the delivery and the delivery is replanned afterwards.

## Where it runs

`std/plan` uses dicts and closures, so it runs on the interpreter, in bundles (`tulpa bundle`)
on desktops and Linux boards, and on microcontrollers on the
[Tulpa VM for firmware](embedded.md#the-whole-language-on-a-microcontroller): `tulpa build
--target rp2040` on a program that imports it produces firmware (`std/plan` adds 64 KB of
flash). The VM runs at interpreter speed. When the same board also needs a control loop at
kilohertz rates, put the planner on one board and the loop, compiled statically, on another,
and connect them with [Tulpa Link](link.md).

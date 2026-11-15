# Machine inspector

**⋮ → Machines** draws every `machine` in the open file as a state graph:

- states are nodes; the **initial** state has a double ring,
- `enter`/`exit` handlers are marked with ▸ and ◂,
- transitions are arrows labelled with their event and guard,
- global transitions (`on estop => …` outside any state) are listed separately,
- internal transitions (no target) are shown as loops.

Tap a state or a transition to jump to its line in the editor. **Copy Mermaid** copies the
diagram source for docs or a README.

```tulpa
machine Door(locked = false) {
    on estop => Closed { print("E-STOP") }
    state Closed {
        on open if not locked => Open
        on lock => Locked { locked = true }
    }
    state Open { on close => Closed }
    state Locked { on unlock(code) if code == 1234 => Closed { locked = false } }
}

let d = Door()
d.open(); d.close(); d.lock(); d.unlock(1234)
print(d.state, d.history)
```

The inspector reads the source, so it works even while the program can't run yet.

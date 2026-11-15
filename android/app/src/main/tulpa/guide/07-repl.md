# The REPL

The REPL keeps one Tulpa session alive: definitions persist, so you can build up a program a line
at a time. Multi-line input (an unfinished `{`) is collected until it balances — the prompt
changes to `…` while it waits.

- **↑ / ↓** recall previous entries (history is kept between launches).
- **Load file** runs the open file into the session, so its functions are available to poke at.
- **Interrupt** stops a long-running entry without losing the session; **Reset** starts afresh.
- Values are echoed with their units: try `3[ft] + 1[m]`.

`phone::` works in the REPL too, which makes it the fastest way to explore a sensor:

```text
phone::accel()
phone::orientation().yaw
```

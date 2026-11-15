# The editor

The editor is tuned for writing Tulpa on a touch screen.

## Symbol bar

Above the keyboard sits a row of the keys Tulpa needs most: `{ }` `( )` `[ ]` `=>` `:` `"`
`.` `|` and **unit keys** like `[m]`, `[s]`, `[deg]`. Long-press the unit key to choose from the
full unit palette. **Tab** indents by four spaces; **⇥** jumps to the end of the line.

Brackets and quotes are paired as you type, and **Enter** keeps the indentation of the line above
(and adds a level after `{`).

## Diagnostics

The file is checked as you type. Errors are underlined in red with a marker in the gutter; tap the
**⚠ count** in the top bar to list them and jump to each one.

## Completion

Suggestions appear in the strip above the symbol bar: keywords, your own functions and variables,
and module members after `::` — for example after `phone::` you get every sensor and actuator.
Tap one to insert it.

## Inspect and jump

Place the cursor on a name and tap **Inspect** (the ⓘ button) to see its documentation or
signature. **Go to definition** moves the cursor to where it is declared. **Outline** lists every
function, struct, machine and constant in the file.

## Files

A project is a folder. The file chip at the top of the editor switches between its files; add
modules with **New file** and import them with `import "lib/drive"`. Files are saved automatically
when you leave the editor or run (turn this off in Settings if you prefer manual saves).

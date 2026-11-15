# Verify: contracts & tests

Tulpa specifications are boolean expressions over the inputs and the result (`result'`). The
**Verify** screen (⋮ menu) fuzzes every function in the file that has an `ensure` or a return
type, exactly like `tulpa verify`:

- inputs are generated from parameter annotations (ints, floats, quantities in their unit…),
- only inputs that satisfy every `require` are used,
- failures are **shrunk** to a minimal counterexample, shown with a **Try in REPL** button.

```tulpa
fn clamp(x: float, lo: float, hi: float) -> float
    require lo <= hi
    ensure lo <= result' and result' <= hi
{
    if x < lo { lo } elif x > hi { hi } else { x }
}
print(check(clamp, 300))
```

The **Tests** tab runs every `define_test!` block in the file and lists what passed and why
anything failed.

```tulpa
fn mean(xs) { sum(xs) / len(xs) }
define_test!("mean of three", { assert_eq!(mean([1, 2, 3]), 2) })
```

Raise the number of trials in Settings when you want more confidence.

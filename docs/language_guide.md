# Tulpa Language Guide

## Overview

Tulpa is a multi-paradigm language inspired by Python, powered by Rust, and enhanced with Hehner's algebraic notation. This guide covers the syntax, features, and usage of the Tulpa language.

## Core Language Features

### 1. Macros
Macros in Tulpa allow for metaprogramming and code generation. Here's an example of a simple loop macro:

```rust
simple_loop!(5, {
    println!("This will print 5 times!");
});
```

### 2. Async Programming
Tulpa supports asynchronous programming for handling I/O-bound tasks. Here's how to use the async runtime:

```rust
use async::SimpleExecutor;

let mut executor = SimpleExecutor::new();

executor.spawn(async {
    async_sleep(2).await;
    println!("Slept for 2 seconds!");
});

executor.run();
```

### 3. Pattern Matching
Tulpa offers pattern matching similar to Rust or Python's structural pattern matching:

```rust
match value {
    1 => println!("Value is 1"),
    2 => println!("Value is 2"),
    _ => println!("Value is something else"),
}
```

## Device Integration

Tulpa provides modules for controlling devices directly. See the [Device Integration Guide](device_integration.md) for more details.

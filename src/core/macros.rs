/// Macro for creating a simple loop.
#[macro_export]
macro_rules! simple_loop {
    ($count:expr, $body:block) => {
        for _ in 0..$count {
            $body
        }
    };
}

/// Macro for defining a function with a name and body.
#[macro_export]
macro_rules! define_fn {
    ($name:ident, $body:block) => {
        fn $name() {
            $body
        }
    };
}

/// Macro for error handling, matching an expression and returning an error if it fails.
#[macro_export]
macro_rules! try_or_return {
    ($expr:expr) => {
        match $expr {
            Ok(val) => val,
            Err(e) => return Err(e),
        }
    };
}

/// Macro for logging messages with a timestamp.
#[macro_export]
macro_rules! log_message {
    ($msg:expr) => {
        let timestamp = chrono::Local::now();
        println!("[{}] {}", timestamp.format("%Y-%m-%d %H:%M:%S"), $msg);
    };
}

/// Macro for timing the execution of a block of code.
#[macro_export]
macro_rules! time_execution {
    ($block:block) => {{
        let start = std::time::Instant::now();
        let result = { $block };
        let duration = start.elapsed();
        println!("Execution time: {:?}", duration);
        result
    }};
}

/// Macro for creating a complex loop with break and continue conditions.
#[macro_export]
macro_rules! complex_loop {
    ($start:expr, $end:expr, $step:expr, $body:block, $break_cond:expr, $continue_cond:expr) => {
        let mut i = $start;
        while i < $end {
            if $break_cond {
                break;
            }
            if !$continue_cond {
                i += $step;
                continue;
            }
            $body
            i += $step;
        }
    };
}

/// Macro for defining a structure with getter and setter methods.
#[macro_export]
macro_rules! define_struct_with_getter_setter {
    ($name:ident, $field:ident, $type:ty) => {
        pub struct $name {
            $field: $type,
        }

        impl $name {
            pub fn get_$field(&self) -> &$type {
                &self.$field
            }

            pub fn set_$field(&mut self, value: $type) {
                self.$field = value;
            }
        }
    };
}

/// Macro for safely unwrapping an option or returning a default value.
#[macro_export]
macro_rules! unwrap_or_default {
    ($option:expr, $default:expr) => {
        match $option {
            Some(val) => val,
            None => $default,
        }
    };
}

/// Macro for creating an async task with logging.
#[macro_export]
macro_rules! async_task_with_logging {
    ($name:ident, $body:block) => {
        async fn $name() {
            log_message!(concat!(stringify!($name), " started"));
            $body
            log_message!(concat!(stringify!($name), " completed"));
        }
    };
}

/// Macro for defining an enum with a default variant.
#[macro_export]
macro_rules! define_enum_with_default {
    ($name:ident, $default_variant:ident, $($variant:ident),+) => {
        #[derive(Debug)]
        pub enum $name {
            $default_variant,
            $($variant),+
        }

        impl Default for $name {
            fn default() -> Self {
                $name::$default_variant
            }
        }
    };
}

/// Macro for creating a test function with setup and teardown phases.
#[macro_export]
macro_rules! define_test {
    ($name:ident, $setup:block, $body:block, $teardown:block) => {
        #[test]
        fn $name() {
            $setup
            $body
            $teardown
        }
    };
}

/// Macro for creating a HashMap with key-value pairs.
#[macro_export]
macro_rules! create_map {
    ($($key:expr => $value:expr),*) => {{
        let mut map = std::collections::HashMap::new();
        $(
            map.insert($key, $value);
        )*
        map
    }};
}

/// Macro for defining a module with public functions.
#[macro_export]
macro_rules! define_module {
    ($name:ident, $($fn_name:ident => $fn_body:block),*) => {
        pub mod $name {
            $(
                pub fn $fn_name() {
                    $fn_body
                }
            )*
        }
    };
}

// Example of using define_struct_with_getter_setter
define_struct_with_getter_setter!(Point, x, i32);
define_struct_with_getter_setter!(Point, y, i32);

// Example of using define_enum_with_default
define_enum_with_default!(Color, Red, Green, Blue);

// Example of creating a test function with setup and teardown
define_test!(
    example_test,
    {
        println!("Setting up...");
    },
    {
        assert_eq!(2 + 2, 4);
    },
    {
        println!("Tearing down...");
    }
);

// Example of creating a module with public functions
define_module!(math_utils,
    add => {
        println!("Result: {}", 2 + 2);
    },
    subtract => {
        println!("Result: {}", 4 - 2);
    }
);

fn main() {
    // Using the complex_loop macro
    complex_loop!(
        0,
        10,
        1,
        {
            println!("Value: {}", i);
        },
        i == 8,
        i % 2 == 0
    );

    // Using the time_execution macro
    time_execution!({
        let sum = (1..=100).sum::<i32>();
        println!("Sum: {}", sum);
    });

    // Using the async_task_with_logging macro
    let task = async_task_with_logging!(example_task, {
        async_sleep(1).await;
        println!("Task running...");
    });

    // Using the define_struct_with_getter_setter macro
    let mut p = Point { x: 0, y: 0 };
    p.set_x(10);
    p.set_y(20);
    println!("Point: ({}, {})", p.get_x(), p.get_y());

    // Running the example test
    example_test();

    // Using the module
    math_utils::add();
    math_utils::subtract();

    println!("All macros have been demonstrated.");
}


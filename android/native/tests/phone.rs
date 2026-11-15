//! `phone::` host module, exercised without a JVM.
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use tulpa::embed::{self, RunConfig};
use tulpa_android::phone::{self, kind};

fn run(src: &str) -> Result<String, String> {
    let src = src.to_string();
    std::thread::Builder::new()
        .stack_size(embed::RECOMMENDED_STACK)
        .spawn(move || {
            let out = Rc::new(RefCell::new(String::new()));
            let o = out.clone();
            let cfg = RunConfig {
                base_dir: ".".into(),
                args: vec![],
                virtual_time: false,
                output: Rc::new(move |s: &str| o.borrow_mut().push_str(s)),
                input: None,
                cancel: Arc::default(),
            };
            embed::run(&src, "t.rav", cfg).map(|_| out.borrow().clone())
        })
        .unwrap()
        .join()
        .unwrap()
}

#[test]
fn sensors_actions_and_units() {
    phone::register();
    phone::reset();
    // no data yet → helpful error
    let e = run("print(phone::accel())").unwrap_err();
    assert!(e.contains("no accel sensor"), "{e}");

    phone::push(kind::ACCEL, &[0.0, 0.0, 9.81]);
    phone::push(kind::PRESSURE, &[1013.25]);
    phone::push(kind::ORIENTATION, &[std::f32::consts::FRAC_PI_2, 0.0, 0.0]);
    phone::push(kind::BATTERY, &[0.8, 1.0]);
    let out = run(
        "let a = phone::accel()\nprint(a[2])\nprint(phone::tilt())\nprint(phone::pressure().to(\"Pa\"))\n\
         print(round(phone::altitude().in(\"m\")))\nprint(round(phone::heading().in(\"deg\")))\n\
         print(phone::battery(), phone::charging(), phone::has(\"gyro\"))\n\
         print(a[2] > 9[m/s^2])",
    )
    .unwrap();
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "9.81 m/s^2");
    assert!(
        lines[1].starts_with("0") && lines[1].ends_with("deg"),
        "{}",
        lines[1]
    );
    assert_eq!(lines[2], "101325 Pa");
    assert_eq!(lines[3], "0");
    assert_eq!(lines[4], "90");
    assert_eq!(lines[5], "0.8 true false");
    assert_eq!(lines[6], "true");

    // unit checking applies to sensor readings: acceleration + length is rejected
    assert!(run("print(phone::accel()[0] + 1[m])").is_err());

    // actuators reach the app's handler with JSON payloads
    let seen: Arc<Mutex<Vec<String>>> = Arc::default();
    let s2 = seen.clone();
    phone::set_action_handler(Box::new(move |k, p| {
        s2.lock().unwrap().push(format!("{k} {p}"));
        String::new()
    }));
    run("phone::vibrate(150[ms])\nphone::vibrate([100[ms], 50[ms], 100[ms]])\nphone::say(\"hello\")\nphone::torch(true)\nphone::gauge(\"speed\", 1.5[m/s], 0, 3[m/s])").unwrap();
    let seen = seen.lock().unwrap().clone();
    assert_eq!(seen[0], r#"vibrate {"amplitude":1.0,"ms":[150]}"#);
    assert_eq!(seen[1], r#"vibrate {"amplitude":1.0,"ms":[100,50,100]}"#);
    assert_eq!(seen[2], r#"say {"rate":1.0,"text":"hello"}"#);
    assert_eq!(seen[3], r#"torch {"on":true}"#);
    assert!(
        seen[4]
            .starts_with(r#"gauge {"max":3.0,"min":0.0,"name":"speed","unit":"m/s","value":1.5}"#),
        "{}",
        seen[4]
    );
}

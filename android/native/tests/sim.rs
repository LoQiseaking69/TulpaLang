//! Physics + co-simulation on the host: loads the bundled models and runs real Tulpa
//! `cosim::` programs against the server the app uses.

use std::path::PathBuf;
use std::sync::Mutex;
use tulpa::embed::{self, RunConfig};
use tulpa_android::sim::{Sim, GEOM_STRIDE};

static ENV: Mutex<()> = Mutex::new(());

fn app() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../app/src/main/tulpa")
}

#[test]
fn bundled_models_load_and_render() {
    for name in ["cartpole", "starter_model", "morty"] {
        let p = app().join(format!("models/{name}.xml"));
        let sim = Sim::load(p.to_str().unwrap()).unwrap_or_else(|e| panic!("{name}: {e}"));
        let (_, f) = sim.frame();
        assert!(
            !f.is_empty() && f.len() % GEOM_STRIDE == 0,
            "{name}: {} floats",
            f.len()
        );
        let m = sim.model.lock().unwrap();
        let info = m.info();
        assert!(info["nq"].as_u64().unwrap() > 0, "{name}");
        assert!(m.stat()[3] > 0.0);
        for i in 0..m.nmesh() {
            assert!(m.mesh(i).len().is_multiple_of(18));
        }
        drop(m);
        sim.shutdown();
    }
}

#[test]
fn free_run_and_push() {
    let sim = Sim::load(app().join("models/cartpole.xml").to_str().unwrap()).unwrap();
    sim.set_playing(true);
    std::thread::sleep(std::time::Duration::from_millis(300));
    let t = sim.frame().0;
    assert!(t > 0.1, "player advanced only to {t}");
    sim.push(None, [5.0, 0.0, 0.0], 0.1).unwrap();
    sim.set_playing(false);
    sim.reset();
    assert!(sim.frame().0 < 1e-9);
    sim.shutdown();
}

fn run_cosim(model: &str, dir: &str, args: &[&str]) -> String {
    let _g = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let sim = Sim::load(app().join("models").join(model).to_str().unwrap()).unwrap();
    sim.set_realtime(0.0);
    let addr = sim.serve("127.0.0.1").unwrap();
    std::env::set_var("TULPA_COSIM", &addr);
    let d = app().join("examples").join(dir);
    let src = std::fs::read_to_string(d.join("main.rav")).unwrap();
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    let (r, text) = std::thread::Builder::new()
        .stack_size(embed::RECOMMENDED_STACK)
        .spawn(move || {
            let out = std::rc::Rc::new(std::cell::RefCell::new(String::new()));
            let o = out.clone();
            let cfg = RunConfig {
                base_dir: d.clone(),
                args,
                virtual_time: false,
                output: std::rc::Rc::new(move |s: &str| o.borrow_mut().push_str(s)),
                input: None,
                cancel: Default::default(),
            };
            let prev = std::env::current_dir().unwrap();
            std::env::set_current_dir(&d).unwrap();
            let r = embed::run(&src, "main.rav", cfg);
            std::env::set_current_dir(prev).unwrap();
            let t = out.borrow().clone();
            (r, t)
        })
        .unwrap()
        .join()
        .unwrap();
    std::env::remove_var("TULPA_COSIM");
    sim.shutdown();
    if let Err(e) = r {
        panic!("{dir} failed: {e}\n{text}");
    }
    text
}

#[test]
fn cartpole_example_balances() {
    let out = run_cosim("cartpole.xml", "cosim_cartpole", &[]);
    let visible: Vec<&str> = out.lines().filter(|l| !l.starts_with("@@tlm")).collect();
    println!("{}", visible.join("\n"));
    assert!(out.contains("balanced ✓"), "{out}");
}

#[test]
fn morty_balance_demo_runs() {
    let out = run_cosim("morty.xml", "morty", &["3"]);
    let visible: Vec<&str> = out.lines().filter(|l| !l.starts_with("@@tlm")).collect();
    println!("{}", visible.join("\n"));
    assert!(out.contains("MORTY online"), "{out}");
}

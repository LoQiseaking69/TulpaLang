// Compiles the MuJoCo shim and links libmujoco. MUJOCO_DIR must point at a MuJoCo install
// (include/ and lib/) for the target being built; CI builds it per ABI.
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=csrc/hsim.c");
    println!("cargo:rerun-if-env-changed=MUJOCO_DIR");
    let target = std::env::var("TARGET").unwrap_or_default();
    let dir = std::env::var("MUJOCO_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let abi = if target.starts_with("aarch64-linux-android") {
                "arm64-v8a"
            } else if target.starts_with("armv7-linux-androideabi") {
                "armeabi-v7a"
            } else if target.starts_with("x86_64-linux-android") {
                "x86_64"
            } else {
                "host"
            };
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../mujoco")
                .join(abi)
        });
    let inc = dir.join("include");
    let lib = dir.join("lib");
    if !inc.join("mujoco/mujoco.h").exists() {
        panic!(
            "MuJoCo not found at {} (set MUJOCO_DIR; see android/README.md)",
            dir.display()
        );
    }
    cc::Build::new()
        .file("csrc/hsim.c")
        .include(&inc)
        .warnings(false)
        .compile("hsim");
    println!("cargo:rustc-link-search=native={}", lib.display());
    println!("cargo:rustc-link-lib=dylib=mujoco");
    if !target.contains("android") {
        println!("cargo:rustc-link-arg=-Wl,-rpath,{}", lib.display());
    }
}

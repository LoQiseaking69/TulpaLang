// Compiles the MuJoCo shim (shared with Android) into the staticlib. MUJOCO_DIR must point at a MuJoCo build (include/ and lib/libmujoco.a) for the
// target SDK; `ios/ci/build-mujoco.sh` produces ios/mujoco/<iphoneos|iphonesimulator>.
// Set TULPA_IOS_SKIP_NATIVE=1 to type-check the bridge without MuJoCo (e.g. on Linux).
use std::path::PathBuf;

fn main() {
    let shim = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../android/native/csrc/hsim.c");
    println!("cargo:rerun-if-changed={}", shim.display());
    println!("cargo:rerun-if-env-changed=MUJOCO_DIR");
    println!("cargo:rerun-if-env-changed=TULPA_IOS_SKIP_NATIVE");
    if std::env::var("TULPA_IOS_SKIP_NATIVE").is_ok() {
        return;
    }
    let target = std::env::var("TARGET").unwrap_or_default();
    let dir = std::env::var("MUJOCO_DIR").map(PathBuf::from).unwrap_or_else(|_| {
        let sdk = if target.ends_with("-sim") || target.starts_with("x86_64-apple-ios") {
            "iphonesimulator"
        } else if target.contains("apple-ios") {
            "iphoneos"
        } else {
            "host"
        };
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../mujoco").join(sdk)
    });
    let inc = dir.join("include");
    let lib = dir.join("lib");
    if !inc.join("mujoco/mujoco.h").exists() {
        panic!(
            "MuJoCo not found at {} (run ios/ci/build-mujoco.sh or set MUJOCO_DIR)",
            dir.display()
        );
    }
    cc::Build::new()
        .file(&shim)
        .include(&inc)
        .define("MJ_STATIC", None)
        .warnings(false)
        .compile("hsim");
    println!("cargo:rustc-link-search=native={}", lib.display());
    // Not bundled: the app links ios/mujoco/<sdk>/lib/libmujoco.a itself with -force_load, so
    // MuJoCo's self-registering mesh decoders (static constructors) are kept by the linker.
    println!("cargo:rustc-link-lib=static:-bundle=mujoco");
}

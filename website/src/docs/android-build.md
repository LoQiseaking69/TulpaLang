# Building Tulpa Field & releases

Tulpa Field lives in [`android/`](https://github.com/LoQiseaking69/TulpaLang/tree/main/android)
in the TulpaLang repository.

```text
android/
  native/        Rust cdylib: JNI bridge, phone:: module, MuJoCo shim + co-sim server, host tests
  app/           Kotlin + Jetpack Compose app (OpenGL ES 3 renderer for the 3D view)
  ci/            build-mujoco.sh (MuJoCo for host + each ABI), smoke.sh (emulator test)
  docs/          screenshots captured by the smoke test
  VERSION        bump on main to publish a signed release
```

The Gradle build copies the repository's `examples/`, `docs/` and `stdlib/` into the APK, plus
the bundled MuJoCo models, so the app always ships what the toolchain ships.

## What CI does

`.github/workflows/android.yml`, on every pull request and push to `main` that touches the app
or the language:

1. builds MuJoCo for the host and for `arm64-v8a`, `armeabi-v7a`, `x86_64` with the NDK (cached);
2. runs the bridge's host tests: `phone::`, physics and co-simulation (the cart-pole must
   balance and Morty must run), and every bundled example;
3. cross-compiles `libtulpa_android.so` with `cargo ndk` and packages `libmujoco.so` beside it;
4. assembles the release APK and verifies its signature;
5. boots an Android 14 emulator, launches the app, runs a guide snippet, an example, the REPL and
   both co-simulation examples, and captures screenshots;
6. publishes `android-v<version>` with the APK when `android/VERSION` changed on `main`.

## Building locally

You need the Android SDK and NDK, Rust with the Android targets, `cargo-ndk`, CMake, Ninja and
Gradle 8.11.

```bash
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android
cargo install cargo-ndk
export ANDROID_NDK_HOME=/path/to/ndk
android/ci/build-mujoco.sh                       # MuJoCo for host + ABIs → android/mujoco/
(cd android/native && cargo test && \
  cargo ndk -t arm64-v8a -t x86_64 --platform 26 -o ../app/src/main/jniLibs build --release)
for abi in arm64-v8a x86_64; do cp android/mujoco/$abi/lib/libmujoco.so android/app/src/main/jniLibs/$abi/; done
(cd android && gradle assembleDebug)             # or open android/ in Android Studio
```

## Signing

Releases are signed with the HermiTech release key from four repository secrets (or variables):
`KEYSTORE_BASE64`, `KEYSTORE_PASSWORD`, `KEY_ALIAS` and `KEY_PASSWORD`. The alias is read from
the keystore when `KEY_ALIAS` is missing. Without a keystore the APK is debug-signed, kept as a
workflow artifact, and never published.

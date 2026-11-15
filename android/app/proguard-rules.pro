# JNI entry points and callbacks are looked up by name from Rust.
-keep class dev.hermitech.tulpafield.core.TulpaNative { *; }
-keep class dev.hermitech.tulpafield.core.TulpaHost { *; }
-keep interface dev.hermitech.tulpafield.core.RunListener { *; }
-keep class * implements dev.hermitech.tulpafield.core.RunListener { *; }

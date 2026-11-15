import java.util.Properties

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
}

// Version comes from the tag (android-v1.2.3) or the TULPA_FIELD_VERSION env var in CI.
val appVersion: String = (System.getenv("TULPA_FIELD_VERSION") ?: "1.0.0").removePrefix("android-v").removePrefix("v")
val versionParts = appVersion.split(".").mapNotNull { it.toIntOrNull() } + listOf(0, 0, 0)
val appVersionCode = versionParts[0] * 10000 + versionParts[1] * 100 + versionParts[2]

// Release signing: keystore from CI secrets (see android/README.md). Falls back to the debug key.
val keystoreProps = Properties().apply {
    val f = rootProject.file("keystore.properties")
    if (f.exists()) f.inputStream().use { load(it) }
}

android {
    namespace = "dev.hermitech.tulpafield"
    compileSdk = 35

    defaultConfig {
        applicationId = "dev.hermitech.tulpafield"
        minSdk = 26
        targetSdk = 35
        versionCode = appVersionCode
        versionName = appVersion
        ndk { abiFilters += listOf("arm64-v8a", "armeabi-v7a", "x86_64") }
        vectorDrawables { useSupportLibrary = true }
    }

    signingConfigs {
        create("release") {
            if (keystoreProps.containsKey("storeFile")) {
                storeFile = rootProject.file(keystoreProps.getProperty("storeFile"))
                storePassword = keystoreProps.getProperty("storePassword")
                keyAlias = keystoreProps.getProperty("keyAlias")
                keyPassword = keystoreProps.getProperty("keyPassword")
            }
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            signingConfig = if (keystoreProps.containsKey("storeFile")) signingConfigs.getByName("release")
                            else signingConfigs.getByName("debug")
        }
        debug {
            applicationIdSuffix = ".debug"
            versionNameSuffix = "-debug"
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions { jvmTarget = "17" }
    buildFeatures {
        compose = true
        buildConfig = true
    }
    packaging {
        resources { excludes += "/META-INF/{AL2.0,LGPL2.1}" }
        jniLibs { useLegacyPackaging = false }
    }
    sourceSets["main"].assets.srcDir(layout.buildDirectory.dir("generated/tulpaAssets"))
}

// Bundle the language's examples, docs and std/ library straight from the repo, so the app
// always ships what the toolchain ships.
val syncTulpaAssets by tasks.registering(Sync::class) {
    val repo = rootProject.projectDir.parentFile
    from(File(repo, "examples")) { into("examples") }
    from(file("src/main/tulpa/examples")) { into("examples") }
    from(File(repo, "docs")) { into("guide/reference") }
    from(file("src/main/tulpa/guide")) { into("guide/field") }
    from(File(repo, "stdlib")) { into("stdlib") }
    into(layout.buildDirectory.dir("generated/tulpaAssets"))
}
tasks.named("preBuild") { dependsOn(syncTulpaAssets) }

dependencies {
    val composeBom = platform("androidx.compose:compose-bom:2024.12.01")
    implementation(composeBom)
    implementation("androidx.core:core-ktx:1.15.0")
    implementation("androidx.activity:activity-compose:1.9.3")
    implementation("androidx.lifecycle:lifecycle-runtime-compose:2.8.7")
    implementation("androidx.lifecycle:lifecycle-viewmodel-compose:2.8.7")
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.ui:ui-tooling-preview")
    implementation("androidx.compose.foundation:foundation")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.material:material-icons-extended")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.9.0")
    debugImplementation("androidx.compose.ui:ui-tooling")
}

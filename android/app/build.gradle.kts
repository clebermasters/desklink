plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

val ndkVersionUsed = "29.0.14206865"
val rustAbis = (findProperty("desklink.abis") as String? ?: "arm64-v8a,x86_64")
    .split(',').map { it.trim() }.filter { it.isNotEmpty() }
val repositoryRoot = rootProject.projectDir.parentFile
val rustOutput = layout.buildDirectory.dir("rustJniLibs")

android {
    namespace = "com.desklink.android"
    compileSdk = 35
    ndkVersion = ndkVersionUsed
    buildToolsVersion = "36.0.0"

    defaultConfig {
        applicationId = "com.desklink.android"
        minSdk = 26
        targetSdk = 35
        versionCode = 2
        versionName = "0.2.0"
        ndk { abiFilters += rustAbis }
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            // Sign with your own key for distribution; debug signing keeps local installs simple.
            signingConfig = signingConfigs.getByName("debug")
        }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions { jvmTarget = "17" }
    sourceSets["main"].jniLibs.srcDir(rustOutput)
    packaging { jniLibs { useLegacyPackaging = false } }
    lint { abortOnError = true; warningsAsErrors = false }
}

/** Builds the Rust receiver (`crates/android`) for each ABI with cargo-ndk. */
val cargoBuild by tasks.registering(Exec::class) {
    group = "build"
    description = "Builds libdesklink_android.so with cargo-ndk"
    workingDir = repositoryRoot
    val home = System.getProperty("user.home")
    val cargo = listOf("$home/.cargo/bin/cargo").firstOrNull { File(it).canExecute() } ?: "cargo"
    val args = mutableListOf(cargo, "ndk", "-P", "26", "-o", rustOutput.get().asFile.absolutePath)
    rustAbis.forEach { args += listOf("-t", it) }
    args += listOf("build", "-p", "desklink-android", "--release")
    commandLine(args)
    environment("ANDROID_NDK_HOME", android.sdkDirectory.resolve("ndk/$ndkVersionUsed").absolutePath)
    environment("PATH", "$home/.cargo/bin:" + System.getenv("PATH"))
    inputs.dir(repositoryRoot.resolve("crates"))
    inputs.file(repositoryRoot.resolve("Cargo.lock"))
    outputs.dir(rustOutput)
}

tasks.named("preBuild") { dependsOn(cargoBuild) }

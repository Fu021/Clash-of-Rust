plugins { id("com.android.application") }

val appVersion = providers.gradleProperty("appVersion").orElse("0.6.3-dev.1").get()
val appVersionCode = providers.gradleProperty("appVersionCode").orElse("603001").get().toInt()
val selectedAbi = providers.gradleProperty("appAbi").orNull

android {
    namespace = "org.clashofrust.android"
    compileSdk = 36
    buildToolsVersion = "36.0.0"
    testBuildType = "release"
    defaultConfig {
        applicationId = "org.clashofrust.android"
        minSdk = 26
        targetSdk = 36
        versionName = appVersion
        versionCode = appVersionCode
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        ndk { abiFilters += selectedAbi?.let { listOf(it) } ?: listOf("arm64-v8a", "armeabi-v7a", "x86_64") }
    }
    signingConfigs {
        val signingFile = System.getenv("ANDROID_KEYSTORE_PATH")
        if (signingFile != null) {
            create("distribution") {
                storeType = "PKCS12"
                storeFile = file(signingFile)
                storePassword = System.getenv("ANDROID_KEYSTORE_PASSWORD")
                keyAlias = System.getenv("ANDROID_KEY_ALIAS")
                keyPassword = System.getenv("ANDROID_KEY_PASSWORD")
            }
        }
    }
    buildTypes {
        debug { isJniDebuggable = false }
        release {
            isMinifyEnabled = false
            signingConfig = signingConfigs.findByName("distribution")
                ?: if (System.getenv("ANDROID_CI_TEST") == "1") signingConfigs.getByName("debug") else null
            vcsInfo { include = false }
        }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    packaging {
        // NDK r28 and the Rust linker produce 16 KiB aligned ELF libraries.
        jniLibs.useLegacyPackaging = true
        resources.excludes += setOf("META-INF/DEPENDENCIES", "META-INF/INDEX.LIST")
    }
    sourceSets["main"].jniLibs.srcDir(layout.projectDirectory.dir("native-libs"))
    sourceSets["main"].assets.srcDir(layout.projectDirectory.dir("offline-assets"))
    sourceSets["main"].res.srcDir(layout.projectDirectory.dir("generated-res"))
    lint { abortOnError = true }
}
dependencies {
    implementation(files("libs/mihomo.aar"))
    androidTestImplementation("androidx.test:runner:1.7.0")
    androidTestImplementation("androidx.test.ext:junit:1.3.0")
}

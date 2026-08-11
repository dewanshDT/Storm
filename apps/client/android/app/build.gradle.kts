plugins {
    id("com.android.application")
    // The Flutter Gradle Plugin must be applied after the Android and Kotlin Gradle plugins.
    id("dev.flutter.flutter-gradle-plugin")
}

android {
    namespace = "dev.storm.storm"
    compileSdk = flutter.compileSdkVersion
    ndkVersion = flutter.ndkVersion

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    defaultConfig {
        // TODO: Specify your own unique Application ID (https://developer.android.com/studio/publish/application-id.html).
        applicationId = "dev.storm.storm"
        // You can update the following values to match your application needs.
        // For more information, see: https://flutter.dev/to/review-gradle-config.
        minSdk = flutter.minSdkVersion
        targetSdk = flutter.targetSdkVersion
        versionCode = flutter.versionCode
        versionName = flutter.versionName
    }

    val uploadStoreFile = System.getenv("STORM_UPLOAD_STORE_FILE")
    signingConfigs {
        if (uploadStoreFile != null) {
            create("release") {
                storeFile = file(uploadStoreFile)
                storePassword = System.getenv("STORM_UPLOAD_STORE_PASSWORD")
                keyAlias = System.getenv("STORM_UPLOAD_KEY_ALIAS")
                keyPassword = System.getenv("STORM_UPLOAD_KEY_PASSWORD")
            }
        }
    }

    buildTypes {
        release {
            // CI sets STORM_UPLOAD_* secrets for a real keystore. Local
            // `flutter run --release` keeps using the debug key so it still works.
            signingConfig = if (uploadStoreFile != null) {
                signingConfigs.getByName("release")
            } else {
                signingConfigs.getByName("debug")
            }
        }
    }
}

kotlin {
    compilerOptions {
        jvmTarget = org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17
    }
}

flutter {
    source = "../.."
}

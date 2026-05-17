set dotenv-load := true
set dotenv-filename := ".env"

android-ndk-root := env("ANDROID_NDK_ROOT")
gstreamer-android-root := env("GSTREAMER_ROOT_ANDROID")

export PKG_CONFIG_PATH := gstreamer-android-root / "lib/pkgconfig" + ":" + gstreamer-android-root / "lib/gstreamer-1.0/pkgconfig"

run-desktop:
    cargo run

check-desktop:
    cargo check

build-android:
    cargo apk build --lib

run-android:
    cargo apk run --lib

check-android:
    cargo apk check --lib

set dotenv-load := true
set dotenv-filename := ".env"

android_pkg_config_path := "$GSTREAMER_ROOT_ANDROID/lib/pkgconfig:$GSTREAMER_ROOT_ANDROID/lib/gstreamer-1.0/pkgconfig"

run-desktop:
    cargo run

check-desktop:
    cargo check

build-android: _android
    PKG_CONFIG_PATH="{{android_pkg_config_path}}" cargo apk build --lib

run-android: _android
    PKG_CONFIG_PATH="{{android_pkg_config_path}}" cargo apk run --lib

check-android: _android
    PKG_CONFIG_PATH="{{android_pkg_config_path}}" cargo apk check --lib

_android:
    @test -n "${GSTREAMER_ROOT_ANDROID:-}" || (echo "GSTREAMER_ROOT_ANDROID is not set" >&2; exit 1)
    @test -n "${ANDROID_NDK_ROOT:-}" || (echo "ANDROID_NDK_ROOT is not set" >&2; exit 1)

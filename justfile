android-home := env_var_or_default("ANDROID_HOME", env_var_or_default("ANDROID_SDK_ROOT", `printf "%s/Library/Android/sdk" "$HOME"`))
android-ndk-root := env_var_or_default("ANDROID_NDK_ROOT", `sdk="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$HOME/Library/Android/sdk}}"; find "$sdk/ndk" -maxdepth 1 -mindepth 1 -type d 2>/dev/null | sort -V | tail -1`)
gstreamer-android-root := env_var_or_default("GSTREAMER_ROOT_ANDROID", "/Volumes/990PRO/deps/gstreamer/arm64")
gstreamer-pkg-config-path := gstreamer-android-root / "lib/pkgconfig" + ":" + gstreamer-android-root / "lib/gstreamer-1.0/pkgconfig"

run-desktop:
    cargo run

check-desktop:
    cargo check

build-android:
    env -u ANDROID_SDK_ROOT ANDROID_HOME="{{android-home}}" ANDROID_NDK_ROOT="{{android-ndk-root}}" GSTREAMER_ROOT_ANDROID="{{gstreamer-android-root}}" PKG_CONFIG_ALLOW_CROSS=1 PKG_CONFIG_PATH="{{gstreamer-pkg-config-path}}" SYSTEM_DEPS_LINK=static cargo apk build --lib

run-android:
    env -u ANDROID_SDK_ROOT ANDROID_HOME="{{android-home}}" ANDROID_NDK_ROOT="{{android-ndk-root}}" GSTREAMER_ROOT_ANDROID="{{gstreamer-android-root}}" PKG_CONFIG_ALLOW_CROSS=1 PKG_CONFIG_PATH="{{gstreamer-pkg-config-path}}" SYSTEM_DEPS_LINK=static cargo apk run --lib

check-android:
    env -u ANDROID_SDK_ROOT ANDROID_HOME="{{android-home}}" ANDROID_NDK_ROOT="{{android-ndk-root}}" GSTREAMER_ROOT_ANDROID="{{gstreamer-android-root}}" PKG_CONFIG_ALLOW_CROSS=1 PKG_CONFIG_PATH="{{gstreamer-pkg-config-path}}" SYSTEM_DEPS_LINK=static cargo apk check --lib

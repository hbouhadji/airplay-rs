set dotenv-load

android_target := env_var_or_default("ANDROID_TARGET", "aarch64-linux-android")
android_device := env_var_or_default("ANDROID_DEVICE", "")

run-desktop:
    cargo run

check-desktop:
    cargo check

build-android:
    #!/usr/bin/env sh
    set -eu
    export_android_pkg_config() {
        case "$1" in
            aarch64-linux-android) abi=arm64 ;;
            armv7-linux-androideabi) abi=armv7 ;;
            i686-linux-android) abi=x86 ;;
            x86_64-linux-android) abi=x86_64 ;;
            *) echo "Unsupported Android target: $1" >&2; exit 2 ;;
        esac
        export PKG_CONFIG_PATH="$GSTREAMER_ROOT_ANDROID/$abi/lib/pkgconfig"
        export PKG_CONFIG_LIBDIR="$GSTREAMER_ROOT_ANDROID/$abi/lib/pkgconfig"
        export PKG_CONFIG_SYSROOT_DIR="$GSTREAMER_ROOT_ANDROID/$abi"
    }
    export_android_pkg_config "{{android_target}}"
    cargo apk build --lib --target "{{android_target}}"

run-android:
    #!/usr/bin/env sh
    set -eu
    export_android_pkg_config() {
        case "$1" in
            aarch64-linux-android) abi=arm64 ;;
            armv7-linux-androideabi) abi=armv7 ;;
            i686-linux-android) abi=x86 ;;
            x86_64-linux-android) abi=x86_64 ;;
            *) echo "Unsupported Android target: $1" >&2; exit 2 ;;
        esac
        export PKG_CONFIG_PATH="$GSTREAMER_ROOT_ANDROID/$abi/lib/pkgconfig"
        export PKG_CONFIG_LIBDIR="$GSTREAMER_ROOT_ANDROID/$abi/lib/pkgconfig"
        export PKG_CONFIG_SYSROOT_DIR="$GSTREAMER_ROOT_ANDROID/$abi"
    }
    export_android_pkg_config "{{android_target}}"
    if [ -n "{{android_device}}" ]; then
        cargo apk run --lib --target "{{android_target}}" --device "{{android_device}}"
    else
        cargo apk run --lib --target "{{android_target}}"
    fi

check-android:
    #!/usr/bin/env sh
    set -eu
    export_android_pkg_config() {
        case "$1" in
            aarch64-linux-android) abi=arm64 ;;
            armv7-linux-androideabi) abi=armv7 ;;
            i686-linux-android) abi=x86 ;;
            x86_64-linux-android) abi=x86_64 ;;
            *) echo "Unsupported Android target: $1" >&2; exit 2 ;;
        esac
        export PKG_CONFIG_PATH="$GSTREAMER_ROOT_ANDROID/$abi/lib/pkgconfig"
        export PKG_CONFIG_LIBDIR="$GSTREAMER_ROOT_ANDROID/$abi/lib/pkgconfig"
        export PKG_CONFIG_SYSROOT_DIR="$GSTREAMER_ROOT_ANDROID/$abi"
    }
    export_android_pkg_config "{{android_target}}"
    cargo apk check --lib --target "{{android_target}}"

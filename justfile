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

use std::env;

fn main() {
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("android") {
        return;
    }

    let gstreamer_root =
        env::var("GSTREAMER_ROOT_ANDROID").expect("GSTREAMER_ROOT_ANDROID must be set");
    let target = env::var("TARGET").expect("TARGET must be set");
    let abi = gstreamer_abi(&target);

    println!("cargo:rustc-link-search=native={gstreamer_root}/{abi}/lib");
    println!("cargo:rustc-link-lib=static=ffi");
    println!("cargo:rustc-link-lib=static=gmodule-2.0");
    println!("cargo:rustc-link-lib=static=iconv");
    println!("cargo:rustc-link-lib=static=pcre2-8");
}

fn gstreamer_abi(target: &str) -> &'static str {
    match target {
        "aarch64-linux-android" => "arm64",
        "armv7-linux-androideabi" => "armv7",
        "i686-linux-android" => "x86",
        "x86_64-linux-android" => "x86_64",
        other => panic!("unsupported Android target for GStreamer SDK: {other}"),
    }
}

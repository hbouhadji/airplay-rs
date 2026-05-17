use std::{env, path::PathBuf, process::Command};

fn main() {
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("android") {
        return;
    }

    let gstreamer_root = env::var_os("GSTREAMER_ROOT_ANDROID")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/Volumes/990PRO/deps/gstreamer/arm64"));

    println!("cargo:rerun-if-env-changed=GSTREAMER_ROOT_ANDROID");
    println!("cargo:rerun-if-env-changed=ANDROID_NDK_ROOT");
    println!("cargo:rerun-if-env-changed=PKG_CONFIG_PATH");
    println!("cargo:rerun-if-env-changed=PKG_CONFIG_ALLOW_CROSS");

    link_android_compiler_builtins();

    let output = Command::new("pkg-config")
        .args(["--static", "--libs", "gstopengl"])
        .output()
        .expect("failed to invoke pkg-config for gstopengl");

    if !output.status.success() {
        panic!(
            "pkg-config --static --libs gstopengl failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let mut search_paths = vec![
        gstreamer_root.join("lib"),
        gstreamer_root.join("lib/gstreamer-1.0"),
    ];

    for arg in String::from_utf8_lossy(&output.stdout).split_whitespace() {
        if let Some(path) = arg.strip_prefix("-L") {
            let path = PathBuf::from(path);
            println!("cargo:rustc-link-search=native={}", path.display());
            search_paths.push(path);
        } else if let Some(lib) = arg.strip_prefix("-l") {
            if static_library_exists(&search_paths, lib) {
                println!("cargo:rustc-link-lib=static={lib}");
            } else {
                println!("cargo:rustc-link-lib={lib}");
            }
        }
    }
}

fn static_library_exists(search_paths: &[PathBuf], lib: &str) -> bool {
    search_paths
        .iter()
        .any(|path| path.join(format!("lib{lib}.a")).exists())
}

fn link_android_compiler_builtins() {
    let Some(ndk_root) = env::var_os("ANDROID_NDK_ROOT").map(PathBuf::from) else {
        return;
    };

    let Ok(target_arch) = env::var("CARGO_CFG_TARGET_ARCH") else {
        return;
    };

    let builtins_arch = match target_arch.as_str() {
        "aarch64" => "aarch64",
        "arm" => "arm",
        "x86" => "i686",
        "x86_64" => "x86_64",
        _ => return,
    };

    let Ok(entries) = ndk_root.join("toolchains/llvm/prebuilt").read_dir() else {
        return;
    };

    for entry in entries.flatten() {
        let builtins_dir = entry.path().join("lib/clang");
        let Ok(clang_versions) = builtins_dir.read_dir() else {
            continue;
        };

        for clang_version in clang_versions.flatten() {
            let lib_dir = clang_version.path().join("lib/linux");
            let lib_name = format!("clang_rt.builtins-{builtins_arch}-android");
            if lib_dir.join(format!("lib{lib_name}.a")).exists() {
                println!("cargo:rustc-link-search=native={}", lib_dir.display());
                println!("cargo:rustc-link-lib=static={lib_name}");
                return;
            }
        }
    }
}

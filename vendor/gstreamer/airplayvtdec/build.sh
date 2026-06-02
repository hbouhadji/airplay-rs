#!/usr/bin/env sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH= cd -- "$script_dir/../../.." && pwd)
build_root="$script_dir/build"
gst_src="$build_root/gstreamer-1.28.3"
plugin_src="$build_root/applemedia-airplayvtdec"
plugin_build="$build_root/plugin"
out_dir="$repo_root/vendor/gstreamer/plugins/macos-aarch64"
patch_file="$script_dir/patches/vtdec-low-latency.patch"

mkdir -p "$build_root"

if [ ! -d "$gst_src/.git" ]; then
    rm -rf "$gst_src"
    git clone --depth 1 --branch 1.28.3 \
        https://gitlab.freedesktop.org/gstreamer/gstreamer.git \
        "$gst_src"
fi

rm -rf "$plugin_src"
cp -R "$gst_src/subprojects/gst-plugins-bad/sys/applemedia" "$plugin_src"
cp "$script_dir/standalone/meson.build" "$plugin_src/meson.build"
cp "$script_dir/standalone/plugin.m" "$plugin_src/plugin.m"

(cd "$plugin_src" && patch -p1 < "$patch_file")

meson setup "$plugin_build" "$plugin_src" --wipe
ninja -C "$plugin_build"

mkdir -p "$out_dir"
cp "$plugin_build/libgstairplayvtdec.dylib" "$out_dir/"

echo "Built $out_dir/libgstairplayvtdec.dylib"

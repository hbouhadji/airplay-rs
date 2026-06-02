# airplayvtdec

This stores a local patch for GStreamer's AppleMedia `vtdec` element from
GStreamer 1.28.3. The build script fetches the upstream GStreamer source,
copies the standalone Meson/plugin entry files, applies the `vtdec.c` patch,
and builds a small standalone plugin that registers `airplayvtdec` and
`airplayvtdec_hw` instead of `vtdec` and `vtdec_hw`.

The HEVC DPB calculation is intentionally patched for AirPlay mirroring:
`compute_hevc_decode_picture_buffer_size()` returns `0`, so the output loop
does not hold the conservative 16-frame reorder queue used by upstream
`vtdec`. This matches the captured AirPlay HEVC stream, whose SPS advertises
`sps_max_num_reorder_pics = 0`.

The standalone build files are:

```text
standalone/meson.build
standalone/plugin.m
```

The `vtdec.c` patch file is:

```text
patches/vtdec-low-latency.patch
```

Build the plugin with:

```sh
just build-airplayvtdec
```

The compiled plugin is written to:

```text
vendor/gstreamer/plugins/macos-aarch64/libgstairplayvtdec.dylib
```

The fetched source tree, Meson build directory, and compiled plugin binary are
ignored by Git. Rebuild the plugin on each development machine against the
local GStreamer installation.

#!/usr/bin/env bash
set -euo pipefail

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
capture=${1:-/tmp/airplay.cap}
duration=${DURATION:-24}
decoders=${DECODERS:-"airplayvtdec_hw vtdec_hw avdec_h265"}
out_root=${OUT_DIR:-"$repo_root/target/gst-latency/$(date +%Y%m%d-%H%M%S)"}
features=${FEATURES:-macos-gstreamer}

if [[ ! -f "$capture" ]]; then
    echo "capture not found: $capture" >&2
    exit 1
fi

cd "$repo_root"

if [[ "$(uname -s)" == "Darwin" && "$(uname -m)" == "arm64" ]]; then
    if [[ ! -f vendor/gstreamer/plugins/macos-aarch64/libgstairplayvtdec.dylib ]]; then
        just build-airplayvtdec
    fi
fi

cargo build --features "$features"

mkdir -p "$out_root"
summary="$out_root/summary.tsv"
printf 'decoder\tstatus\tdpb\tqueue_max\tlatency_count\tlatency_min_ms\tlatency_p50_ms\tlatency_p95_ms\tlatency_max_ms\tlog\n' > "$summary"

for decoder in $decoders; do
    log="$out_root/$decoder.log"
    echo "measuring $decoder for ${duration}s..."

    status=$(
        AIRPLAY_GST_DECODER="$decoder" \
        GST_PLUGIN_PATH_1_0="$repo_root/vendor/gstreamer/plugins/macos-aarch64${GST_PLUGIN_PATH_1_0:+:$GST_PLUGIN_PATH_1_0}" \
        GST_TRACERS=latency \
        GST_DEBUG='GST_TRACER:7,vtdec:6' \
        python3 - "$duration" "$log" "$repo_root/target/debug/airplay-rs" "$capture" <<'PY'
import subprocess
import sys

duration = float(sys.argv[1])
log_path = sys.argv[2]
binary = sys.argv[3]
capture = sys.argv[4]

with open(log_path, "w") as log:
    process = subprocess.Popen(
        [binary, "--side-by-side-video", "--replay-video", capture],
        stdout=log,
        stderr=subprocess.STDOUT,
    )
    try:
        exit_code = process.wait(timeout=duration)
    except subprocess.TimeoutExpired:
        process.terminate()
        try:
            process.wait(timeout=1)
            print("stopped")
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
            print("killed")
    else:
        print("ok" if exit_code == 0 else f"exit-{exit_code}")
PY
    )

    python3 - "$decoder" "$status" "$log" "$summary" <<'PY'
import re
import sys

decoder, status, log_path, summary_path = sys.argv[1:]
ansi = re.compile(r"\x1b\[[0-9;]*m")
dpb = ""
queue_max = ""
latencies_ns = []

with open(log_path, "r", errors="replace") as log:
    for raw_line in log:
        line = ansi.sub("", raw_line)

        dpb_match = re.search(r"Calculated DPB size:\s*([0-9]+)", line)
        if dpb_match:
            dpb = dpb_match.group(1)

        queue_match = re.search(r"queue length\s+([0-9]+)", line)
        if queue_match:
            value = int(queue_match.group(1))
            queue_max = str(max(value, int(queue_max or 0)))

        if "latency, src-element-id=" not in line:
            continue
        if "sink-element=(string)video_sink" not in line:
            continue

        # GStreamer tracer lines vary slightly by version. These cover the
        # 1.28 format while keeping the parser tolerant of older names.
        for name in ("time", "ts", "duration"):
            match = re.search(rf"{name}=\(guint64\)([0-9]+)", line)
            if match:
                value = int(match.group(1))
                if value > 0:
                    latencies_ns.append(value)
                break

def percentile(values, pct):
    if not values:
        return ""
    ordered = sorted(values)
    index = round((len(ordered) - 1) * pct)
    return ordered[index] / 1_000_000

def fmt(value):
    if value == "":
        return ""
    return f"{value:.2f}"

if latencies_ns:
    min_ms = min(latencies_ns) / 1_000_000
    p50_ms = percentile(latencies_ns, 0.50)
    p95_ms = percentile(latencies_ns, 0.95)
    max_ms = max(latencies_ns) / 1_000_000
else:
    min_ms = p50_ms = p95_ms = max_ms = ""

def cell(value):
    return value if value != "" else "-"

row = [
    decoder,
    status,
    cell(dpb),
    cell(queue_max),
    str(len(latencies_ns)),
    cell(fmt(min_ms)),
    cell(fmt(p50_ms)),
    cell(fmt(p95_ms)),
    cell(fmt(max_ms)),
    log_path,
]

with open(summary_path, "a") as summary:
    summary.write("\t".join(row) + "\n")
PY
done

echo
echo "summary: $summary"
column -t -s $'\t' "$summary"

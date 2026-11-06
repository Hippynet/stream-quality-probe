#!/bin/sh
# End-to-end check: serve synthetic streams with known faults from a fake
# Icecast server and assert streamprobe reports each one. ~2 minutes.
# Needs ffmpeg (fixtures + decoding) and python3 (the fake server).
set -eu
cd "$(dirname "$0")/.."
BIN=${BIN:-target/release/streamprobe}
PORT=${PORT:-18765}
[ -f tests/fixtures/hls/seg000.ts ] || sh tests/make_fixtures.sh >/dev/null
out=$(mktemp -d)
fail=0

python3 tests/fake_icecast.py tests/fixtures "$PORT" >"$out/server.log" 2>&1 &
srv=$!
trap 'kill $srv 2>/dev/null; if [ "$fail" = 0 ]; then rm -rf "$out"; else echo "outputs kept in $out"; fi' EXIT
sleep 0.5

# The /faults stream is "live" from server start, so start it immediately.
pids=""
"$BIN" "http://127.0.0.1:$PORT/faults" -t 112 --json >"$out/faults.json" 2>/dev/null & pids="$pids $!"
for m in upscaled mono wrongtype listen.pls badmeta hls/live.m3u8 garbage; do
  "$BIN" "http://127.0.0.1:$PORT/$m" -t 25 --json >"$out/$(echo "$m" | tr / _).json" 2>/dev/null & pids="$pids $!"
done
"$BIN" "http://127.0.0.1:$PORT/missing" -t 5 --json >"$out/missing.json" 2>/dev/null & pids="$pids $!"
"$BIN" "http://127.0.0.1:$PORT/page" -t 5 --json >"$out/page.json" 2>/dev/null & pids="$pids $!"
echo "running (about 2 minutes)..."
for p in $pids; do wait "$p" || true; done  # non-zero exits are expected: these streams are faulty

expect() { # file code...: every code must appear as a finding or an event
  f=$1; shift
  for code in "$@"; do
    if python3 - "$out/$f.json" "$code" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))
codes = {x["code"] for x in d["findings"]} | {x["code"] for x in d["events"]}
sys.exit(0 if sys.argv[2] in codes else 1)
PY
    then echo "ok    $f: $code"; else echo "FAIL  $f: $code"; fail=1; fi
  done
}
absent() { # file code...: none of these codes may appear
  f=$1; shift
  for code in "$@"; do
    if python3 - "$out/$f.json" "$code" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))
codes = {x["code"] for x in d["findings"]} | {x["code"] for x in d["events"]}
sys.exit(1 if sys.argv[2] in codes else 0)
PY
    then echo "ok    $f: no $code"; else echo "FAIL  $f: unexpected $code"; fail=1; fi
  done
}
expect faults mono_content out_of_phase dead_air left_only clipping stall underrun disconnect
expect upscaled low_bandwidth
expect mono mono_stream
expect wrongtype content_type_mismatch
expect badmeta frame_sync
expect hls_live.m3u8 stereo
expect garbage no_audio
# clean streams must not trip the byte-level checks
absent faults frame_sync format_change
absent upscaled frame_sync decode_errors
absent mono frame_sync decode_errors
absent hls_live.m3u8 decode_errors
expect listen.pls playlist
expect missing http_404
expect page not_audio
exit $fail

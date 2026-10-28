#!/usr/bin/env bash
# Native acceptance without a desktop. Every native test process, including
# render-context destruction and worker join, is covered by GNU timeout.
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../.."
export LIBMPV="${LIBMPV:-/home/hewel/Codes/mpv/build/libmpv.so}"
export PL_LIBDIR="${PL_LIBDIR:-$HOME/.cache/mpv-preview-deps/libplacebo-7.387.0/install/lib}"
export LD_LIBRARY_PATH="$PL_LIBDIR${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
MEDIA_ROOT="${MEDIA_ROOT:-/home/hewel/Data/jellypilot/test-videos}"
export MPV_PREVIEW_TEST_MEDIA="${1:-$MEDIA_ROOT/bbb_h264_aac_1080p.mp4}"
REPORT_ROOT="${REPORT_ROOT:-/tmp/iced-preview-verification}"
mkdir -p -- "$REPORT_ROOT"

cargo test -p mpv_sdr_gpu --lib --no-run
cargo build -p mpv_sdr_gpu --example preview_smoke
timeout --kill-after=5s 45s cargo test -p mpv_sdr_gpu --lib preview::tests::native -- \
  --ignored --nocapture --test-threads=1
timeout --kill-after=5s 45s cargo run --quiet -p mpv_sdr_gpu --example preview_smoke -- \
  "$MPV_PREVIEW_TEST_MEDIA" --libmpv "$LIBMPV" --report "$REPORT_ROOT/sdr"

fixture="$(mktemp -d)"
producer=""
cleanup() {
  if [[ -n "$producer" ]]; then
    kill "$producer" 2>/dev/null || true
    wait "$producer" 2>/dev/null || true
  fi
  rm -rf -- "$fixture"
}
trap cleanup EXIT
mkfifo "$fixture/nonseekable.ts"
# A FIFO transport really is non-seekable. lavfi alone is not such a fixture.
timeout --kill-after=5s 45s ffmpeg -hide_banner -loglevel error -y \
  -f lavfi -i testsrc2=size=320x180:rate=10 -t 30 -c:v mpeg2video -f mpegts \
  "$fixture/nonseekable.ts" &
producer=$!
timeout --kill-after=5s 30s cargo run --quiet -p mpv_sdr_gpu --example preview_smoke -- \
  "$fixture/nonseekable.ts" --libmpv "$LIBMPV" --expect-nonseekable \
  --report "$REPORT_ROOT/nonseekable"
echo "PASS process-bounded native cancellation, missing-completion failure, unsupported source and shutdown"

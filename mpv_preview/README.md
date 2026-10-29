# iced_mpv_preview

Independent, lazy libmpv software thumbnails. This workspace package depends on
`iced_core` image handles, futures, synchronization and dynamic library loading;
it does not depend on winit, a renderer, Vulkan, or the GPU-host example.

## Ownership and source contract

```rust,no_run
use iced_mpv_preview::Preview;
use std::sync::Arc;

let (preview, notifications) = Preview::new("/path/to/libmpv.so".to_owned())?;
let owner = Arc::new(preview);
// Retain owner outside the application's UI runtime; UI state may clone it.
owner.set_media(Some("/path/to/video.mp4"));
let requested = owner.request(12.3);
// Drive notifications as an application subscription, then call owner.take().
owner.cancel();
owner.set_media(None);
# Ok::<(), std::io::Error>(())
```

- `new(String) -> io::Result<(Preview, futures::channel::mpsc::Receiver<()>)>`
  starts a worker, not a decoder. `Preview` is `Send + Sync`; `Arc<Preview>` is
  sufficient. Keep the notification subscription alive for the owner lifetime.
- `set_media(&self, Option<&str>)` borrows an already-resolved local path or
  authenticated HTTP(S) URL and privately copies it. Every call starts a new
  generation, even for the same URL. It immediately invalidates demand/results;
  the worker releases the prior decoder, cache and failure state. `None` retires
  the capability. The URL is passed verbatim, including its query. The library
  does not resolve sessions, add credentials/headers, rewrite URLs or log sources.
- `request(&self, f64) -> Option<f64>` replaces the latest demand and returns the
  quantized target (100 ms buckets). `None` means no active source or an invalid
  target. Requests use **mpv's normalized timeline**, preserving its default
  `rebase-start-time` behavior. Do not add raw MPEG-TS/HLS PTS or guess URL offsets.
- `take(&self) -> Option<Outcome>` consumes a result only if request and source
  generation still match. `Outcome { requested, result }` contains a `Frame` or a
  redacted error. `Frame { requested, actual, color, image }` distinguishes the
  target from the real frame timestamp and carries an `iced_core::image::Handle`.
- `cancel(&self)` invalidates demand and output immediately, but does not preempt
  native code or discard the reusable media cache. Clear any UI-held image too.
- Capability/load/seek failure releases the decoder/cache and fails later targets
  explicitly for that generation. There is no retry loop; `set_media` is the
  explicit recovery boundary. Failed or nonseekable streams should keep accurate
  time/chapter-only UI, not stale imagery.
- `Drop` signals shutdown and joins the worker. Keep the final owner outside the
  UI runtime. Ten-second native operation deadlines bound event waiting, not
  arbitrary blocking FFI; acceptance wraps entire processes, including teardown.

The decoder has one active operation, one latest target, one output slot and an
8-frame LRU (320x180 opaque RGBA). Starts are capped at ten uncached seeks per
second. Demux buffers and codec reference frames are additional memory. It never
seeks, pauses or shares a GPU queue with the main player. After SW render-context
creation it uses only asynchronous client commands/properties and continually
drains render updates. Probe frames never publish; the probe core is destroyed
before opening the configured conversion core.

## Color and runtime requirements

The libmpv-linked libavfilter must provide `zscale`, `tonemap` and `format`.
Linear-float conversion uses `npl=100`, BT.709 primaries, Hable for PQ/HLG, and
IEC sRGB encoding. HLG uses the reference 1000-nit/gamma-1.2 luminance OOTF.
Peak metadata falls back to 1000 nits; out-of-gamut colors clip rather than using
perceptual gamut mapping. Output is verified as sRGB/RGB/BT.709 before publishing.
Profile 8 with an HDR10-compatible PQ/BT.2020 base is explicitly labeled without
RPU/EL. Other Dolby Vision profiles/bases, unknown DV profile metadata and
unsupported transfers fail explicitly. This is not full Dolby Vision or a
physical-display calibration claim.

## Verification

Run from the workspace root. Set `LIBMPV` to the current shared library and
`PL_LIBDIR` to its dependency directory. The acceptance scripts default to
`$HOME/.cache/mpv-preview-deps/libplacebo-7.387.0/install/lib` and permit overrides.

```sh
cargo test -p iced_mpv_preview
cargo clippy -p iced_mpv_preview --all-targets --no-deps -- -D warnings
bash mpv_preview/tests/verify_preview.sh /path/to/moving-video.mp4
python3 mpv_preview/tests/verify_preview_color.py --report /tmp/preview-color
```

`cargo run -p iced_mpv_preview --example preview_smoke -- SOURCE --libmpv LIBRARY
--report DIRECTORY` is the reusable native entry. It checks latest-target cache
identity, cancellation, same-source reload, pixel differences and teardown. Its
internal watchdog does not cover Drop; wrap the process with GNU
`timeout --kill-after=5s 45s` for native acceptance. `--capture-at SECONDS` exports
one frame without claiming the motion/cache scenarios. Do not print authenticated
SOURCE arguments or include credentials in reports. The coordinated ignored
`tests::native` cases hold a real native acknowledgement before cancellation,
source replacement, injected missing completion, and shutdown.

The independent PQ/HLG oracle generates transfer-coded gray/color fixtures with
Python math and compares actual worker pixels; it does not use the conversion
filter as its expected-value generator. GPU composition, widget input, provider
selection and production-server authorization require the consuming application's
acceptance. The GPU-host example demonstrates integration, not an extra library
dependency or a replacement for product-level stream verification.

### Shared-library stream acceptance

The extracted package passed three ordinary tests and three coordinated native
tests on an owned localhost fixture. The shared timeline passed nine real
event/overlay tests; the migrated GPU-host example passed eleven ordinary tests.
Package-scoped strict Clippy and builds passed for this library and the example,
and formatting passed for all three packages. Strict `iced_widget` Clippy remains
blocked by the unrelated existing `default_trait_access` diagnostic at
`widget/src/backdrop.rs:138`; that file was not changed.

The fixture supplied authenticated range MP4 and HLS versions of an 18-second
moving BBB excerpt. Its first TS starts at PTS 122.8. Both sources returned
normalized targets 8.5, 0.5, 7.0 and 3.3 seconds with matching corresponding
RGBA hashes; first/last requests returned 0 and 17.966667 seconds. Whole-process
durations were 0.776 seconds for MP4 and 1.016 seconds for HLS. These are one-run
observations, not latency guarantees or production-Jellyfin measurements.

On HLS, real native-ack barriers verified in-flight cancellation/source
replacement, injected missing-completion failure and reload recovery, and
shutdown while a request was held in flight. The test process exited normally
in 1.988 seconds, under a 45-second process guard that also covers Drop/join.
An unauthenticated request failed explicitly and exited in 0.052 seconds.
Credential-screened reports are at `/tmp/iced-shared-stream-preview`.

To reuse an already-running private fixture, set `STREAM_CAPABILITY` to its
mode-0600 JSON file containing complete `mp4` and `hls` URLs. The owner must keep
the server alive. Do not echo the file, put URLs in shell history, or restart
another agent's fixture. This recipe deliberately withholds child output:

```sh
export STREAM_CAPABILITY=/path/to/private/capability.json
export LIBMPV=/home/hewel/Codes/mpv/build/libmpv.so
export PL_LIBDIR="${PL_LIBDIR:-$HOME/.cache/mpv-preview-deps/libplacebo-7.387.0/install/lib}"
export LD_LIBRARY_PATH="$PL_LIBDIR${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
cargo build -p iced_mpv_preview --example preview_smoke
cargo test -p iced_mpv_preview --lib --no-run
python3 - <<'PY'
import json, os, pathlib, subprocess
capability = pathlib.Path(os.environ["STREAM_CAPABILITY"])
assert capability.stat().st_mode & 0o777 == 0o600
sources = json.loads(capability.read_text())
report = pathlib.Path("/tmp/iced-stream-repeat")
for kind in ("mp4", "hls"):
    result = subprocess.run([
        "timeout", "--kill-after=5s", "45s",
        "target/debug/examples/preview_smoke", sources[kind],
        "--libmpv", os.environ["LIBMPV"],
        "--report", str(report / kind), "--duration", "18",
    ], capture_output=True)
    if result.returncode:
        raise SystemExit(f"{kind}: failed; private child output withheld")
    print(f"{kind}: passed, including process exit")
env = os.environ | {"MPV_PREVIEW_TEST_MEDIA": sources["hls"]}
result = subprocess.run([
    "timeout", "--kill-after=5s", "45s", "cargo", "test",
    "-p", "iced_mpv_preview", "--lib", "tests::native", "--",
    "--ignored", "--nocapture", "--test-threads=1",
], env=env, capture_output=True)
if result.returncode:
    raise SystemExit("HLS in-flight tests failed; private child output withheld")
print("HLS in-flight cancellation, failure and shutdown: passed")
PY
```

This is fixture evidence, not production-server authorization, Trickplay fallback
selection, main-playback isolation in a consuming app, or desktop composition.
Those integration scenarios remain with the JellyPilot application acceptance.

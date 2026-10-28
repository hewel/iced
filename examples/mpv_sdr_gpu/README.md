# Native iced / mpv GPU host

This Linux/Vulkan example plays through a shared device into a strict 10-bit
surface. `main.rs` is an ordinary iced daemon with normal controls. The example
library owns the mpv bridge, three-slot image pool, synchronization and teardown.
It uses existing iced extension points; it does not change stock format policy.

`Player::new(settings)` returns a player and a bounded notification stream. Run
that stream as an iced `Task`, call `Player::drain` on notifications, and place
`Player::view()` in the widget tree. Pass a clone to `Compositor::create` from
`run_with_compositor`. Clones share one session on the application thread; the
session survives zero-window compositor recreation. One player supplies one
output size; independent windows with different video sizes need separate players.

The shader samples a private wgpu texture, never a borrowed mpv image. It follows
ordinary widget clipping, overlay and screenshot paths. SDR code values remain
BT.709/gamma 2.2, full range, 203 nit white and 0.203 nit black. Human SDR/HDR color
acceptance is separate from these compile and pixel-presence checks.

## Run

Use a libmpv built from the matching fork and the maintained baseline file:

```sh
cargo run -p mpv_sdr_gpu -- /path/to/video.mp4 \
  --libmpv /path/to/libmpv.so --baseline /path/to/gpu-next-host-baseline.conf \
  --host-abi v3
```

The default `v1` keeps CPU-completed handoff. `v2` exercises the larger legacy
descriptor with the default SDR color contract. `v3` explicitly enables host-owned
timeline synchronization. An incompatible library fails registration; it does not
quietly select another ABI. `--pause`, `--duration`, `--ipc`, and `--no-overlay`
remain available. Target dimensions follow the actual physical window size.

## Timeline thumbnails

Hover or drag the timeline to preview without changing the main player's clock
or pause state. Release inside the timeline to seek the main player; Escape,
release outside, focus loss, and touch loss cancel. Leaving the timeline hides
the thumbnail. A cancellation or completed seek suppresses hover preview until
the pointer leaves and re-enters. The panel shows both the requested time and the
actual decoded frame time; sparse-frame video need not land on the exact time.

`preview::Preview` owns a lazy, independent libmpv software-render instance on a
background thread. It does not share the Vulkan host, GPU queue, or playback
commands. After creating the render context, it uses asynchronous client APIs
and continues draining render updates while waiting for seek/restart events.
Callbacks only unpark the worker. The UI neither decodes nor waits for native
work. The owner is retained until the window runtime exits, then joins teardown;
the render context is freed before its mpv core.

The first instance probes seekability, transfer, base-layer color parameters, and
the selected track's Dolby Vision profile. Its pixels are never published. The
worker destroys it and reopens with a fixed CPU filter graph; only one core is
alive at a time. This avoids mixing filter-reconfiguration restart events with
seek completion. Each load has its own deadline, so cold initialization includes
two bounded event-wait phases, not a single ten-second wall-clock guarantee.

The scheduler keeps one in-flight seek, one replaceable latest request, and one
result slot. Media generations and request revisions are checked at publication
and delivery. Canceling invalidates results immediately but does not preempt an
in-progress native decode. Reloading even the same path clears the media cache.
Requests use 100 ms timeline buckets and start at most ten uncached seeks per
second. The eight-entry LRU holds at most 1,843,200 bytes of RGBA payload, plus
the render buffer and any currently retained result/UI image. Decoder reference
frames and renderer uploads are additional memory; demux buffering is configured
for 8 MiB forward/zero backward cache and two decoder threads.

The worker queries `seekable` asynchronously before admitting the first seek.
It disables demuxer seekable-cache emulation: a live/FIFO source with only a
limited cached seek range is not treated as a random-access thumbnail source.
Unsupported capability fails explicitly. Capability is not a completion
guarantee: even an acknowledged seek may never emit SEEK or PLAYBACK_RESTART.
Loading/initial metadata and each seek/position snapshot have a ten-second
operation deadline, and the worker's park has that same wake deadline. A timeout
publishes an error, releases the independent decoder, and leaves this media
generation failed. Later requests fail explicitly rather than silently retrying;
reloading media, including the same path, clears the failure.

These deadlines bound event waiting, not arbitrary time spent inside a blocking
native call. Render-context destruction and `terminate_destroy` are library
boundaries; an in-process thread cannot forcibly interrupt them safely.
Acceptance therefore also uses a process-level timeout covering teardown and
worker join. Passing those scenarios is not a universal bounded-shutdown claim.

This implementation deliberately uses exact seeks only, rather than first
showing a possibly distant keyframe under a precise timestamp. Coalescing and a
small fixed output bound the work; long-GOP and remote media can still have high
latency. There is no background prefetch or persistent disk cache.

Thumbnails are 320x180 opaque, 8-bit **sRGB-encoded** RGB images, letterboxed by
mpv. The color transform runs inside the independent instance's libavfilter
chain, **before** the software render context packs RGB. It does not rely on
`libmpv_sw`/`mp_sws_scale` to tone-map HDR and does not create another GPU device
or shared Vulkan queue.

The conversion is:

1. Strip Dolby Vision RPU/enhancement-layer and HDR10+ dynamic metadata from the
   preview filter input, restoring base-layer color parameters.
2. `zscale` linearizes the declared transfer with `npl=100` and accurate gamma,
   producing single-precision `gbrpf32le` at 320x180. One linear unit is 100 nits.
   Source range and aspect ratio/SAR are retained.
3. Convert linear primaries to BT.709. For PQ/HLG, apply Hable tone mapping in
   linear float RGB with highlight desaturation disabled. The tone mapper uses
   MaxCLL, then mastering peak, and falls back to 1000 nits when these are absent.
   HLG uses zimg's reference 1000-nit display / gamma-1.2 luminance OOTF.
4. Encode **IEC 61966-2-1 sRGB**, full-range 8-bit RGB. The worker verifies the
   filtered output transfer, matrix and primaries before publishing; alpha is
   explicitly 255. These bytes match an sRGB image texture's interpretation.

This is static thumbnail rendering, not display calibration or a match to the
main player's GPU tone mapper. Out-of-BT.709 colors are clipped, not perceptually
gamut-mapped. Peaks above incorrectly tagged/fallback mastering limits can clip.
SDR is also converted to sRGB, without Hable compression. Untagged SDR uses mpv's
inferred source parameters, not a measured mastering intent. Subtitles, audio,
user configuration, and scripts remain disabled only in the preview instance.

**Dolby Vision boundary:** the accepted path is Profile 8 with restored PQ,
BT.2020 non-constant-luminance matrix and BT.2020 primaries. It is labeled
`DV8 HDR10 base → sRGB (no RPU/EL)`, not full Dolby Vision. The tested Andor file
is Profile 8, compatibility ID 1, HDR10-compatible BL; RPU/EL grading is not
applied. Profile 5, other profiles, Profile 8 SDR/HLG bases, and recognized Dolby
Vision without profile metadata are rejected. Profile 5 has policy regression
coverage but no real Profile 5 media acceptance. Unsupported/log transfers and
missing conversion support fail explicitly rather than exposing unconverted HDR.

Runtime requirements include FFmpeg's `zscale` (zimg), `tonemap`, and `format`
filters in the libmpv-linked libavfilter. No FFmpeg executable is spawned by the
application. See [FFmpeg zscale](https://ffmpeg.org/ffmpeg-filters.html#zscale),
[linear-float tonemap requirements](https://ffmpeg.org/ffmpeg-filters.html#tonemap-1),
and [mpv format metadata controls](https://mpv.io/manual/stable/#video-filters).

## Synchronization

Each slot retains one semaphore until mpv teardown and host GPU work finish.
For cycle `n`, mpv waits for `2*n` and signals `2*n+1`; the copy waits for that
signal and publishes `2*n+2`. The host only reuses a slot after its copy completes.
The extra copy lets ordinary iced frames and screenshots retain their own image
without extending the producer's lease or mutating wgpu's resource tracker.

`release_frame(0)` authorizes presentation at mpv's scheduled `flip_page`; it is
not CPU completion or scanout feedback. A negative release does not promise a
signal: the session stops and never queues a wait for it. Resize reserves only
idle slots, replaces their images, and preserves their semaphores and counters.

The [wgpu 30.0.1 Vulkan HAL](https://docs.rs/wgpu-hal/30.0.1/wgpu_hal/vulkan/struct.Queue.html#method.add_wait_semaphore)
attaches waits and signals to the next submission. Attachment and submit must be
inside the same external queue gate, including against iced image uploads.
Unconsumed attachments are removed before unwinding out of that gate.

mpv callbacks only update CPU state and send notifications. The completion worker
blocks on a channel while idle and uses `Device::poll(Wait)` for a particular
submission when work arrives. It never holds the queue or pool lock while waiting.
There is no periodic polling loop. Shutdown stops acquisition, terminates mpv,
drains/joins that worker, then waits for outstanding wgpu work before freeing the
native resources. No mpv call, poll, or join runs under the queue gate.

## Verification

Use three layers, in order. Do not treat a direct `Player` command test as a
timeline input test, or a successful decoder test as proof of the displayed UI.
Keep media, screenshots, and reports outside Git.

### Environment and media

```sh
export MEDIA_ROOT=/home/hewel/Data/jellypilot/test-videos
export LIBMPV=/home/hewel/Codes/mpv/build/libmpv.so
export MPV_BASELINE=/home/hewel/Codes/mpv/TOOLS/gpu-next-host-baseline.conf
export PL_LIBDIR="${PL_LIBDIR:-$HOME/.cache/mpv-preview-deps/libplacebo-7.387.0/install/lib}"
export LD_LIBRARY_PATH="$PL_LIBDIR${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export REPORT_ROOT=/tmp/iced-mpv-acceptance
```

Override these paths for your machine. Always pass `--libmpv` explicitly when
testing a new build; the example's default versioned library may be older.
Host ABI v3 requires the matching fork. The current upstream merge requires
libplacebo 7.387.0 or newer.

Use `bbb_h264_aac_1080p.mp4` (about 634 seconds) for repeated dragging, cancellation,
widely separated seek targets, and playback-isolation checks. Use
`bbb_hevc_1080p_5mb.mp4` and `bbb_av1_1080p_5mb.mp4` for codec spot checks.
`sintel_av1_1080p_5mb.mp4` is 1920x818 full-range video: check aspect ratio and
range separately. The short files are approximately ten seconds long, so avoid
accidentally reaching EOF during interaction checks.

Inspect actual stream metadata instead of inferring HDR from a filename:

```sh
ffprobe -v error -select_streams v:0 \
  -show_entries stream=codec_name,width,height,pix_fmt,color_range,color_space,color_transfer,color_primaries:stream_side_data \
  -of json "$MEDIA_ROOT/bbb_h264_aac_1080p.mp4"
```

The long H264 sample has no explicit color tags. The available PQ/BT.2020
samples require separate HDR acceptance. The inspected Andor sample has Dolby
Vision Profile 8 with an HDR10-compatible base layer, not Profile 5.
Neither correct SDR thumbnails nor successful decoding establishes HDR/DV
color correctness.

### Layer 1: timeline input and application state

```sh
cargo test -p mpv_sdr_gpu --bin mpv_sdr_gpu
cargo test -p mpv_sdr_gpu
cargo clippy -p mpv_sdr_gpu --all-targets --no-deps -- -D warnings
cargo fmt -p mpv_sdr_gpu -- --check
```

Timeline regressions replay `Widget::update` with a headless renderer, including
multiple events before a view rebuild, cancellation followed by release, and
accumulating keyboard targets. Additional input-state tests cover touch ownership
and focus. Controls tests reject stale seek completions and accept sparse-frame
and same-position results. These tests do not open a window or call libmpv.

### Layer 2: real libmpv request/result isolation

The independent worker can be exercised without a desktop:

```sh
cargo run -p mpv_sdr_gpu --example preview_smoke -- \
  "$MEDIA_ROOT/bbb_h264_aac_1080p.mp4" --libmpv "$LIBMPV" \
  --report "$REPORT_ROOT/preview-h264"
```

Repeat with the HEVC, AV1, and Sintel files above, using separate report
directories. The input must contain changing imagery at 0.5, 3.3, 7.0, and 8.5
seconds. The executable records requested/actual times, request-to-result latency,
RGBA hashes, and raw pixels. It requires distinct target images, opaque alpha,
latest-target delivery, cache reuse within one media generation, cache
invalidation on same-path reload, cancellation/unload invalidation, and teardown.
The internal 30-second watchdog only covers result waiting, **not** `Drop` or
worker join. Use the process-bounded entry below for lifecycle acceptance.
The instantaneous A/B/C batch checks latest-target behavior, not an in-flight
cancellation by itself.

Pass `--duration 634.6` for the long H264 sample to also check completion at
zero and at the media endpoint. For another source, use its probed duration,
not a filename-derived assumption. The final frame may precede the duration.

Use `--capture-at 300` to export a single frame from a long HDR sample, without
the moving-video/cache checks. Its `.txt` includes the actual frame timestamp and
color mode. A successful export alone does not prove correct color. For a known
unsupported input, `--expect-unavailable 'diagnostic substring'` requires explicit
failure. This executable has no main player and cannot establish playback
isolation or UI composition; those remain Layer 3.

#### Independent HDR color acceptance

```sh
python3 examples/mpv_sdr_gpu/tests/verify_preview_color.py \
  --report "$REPORT_ROOT/color-oracle"
```

The script directly encodes ST2084 and HLG OETF samples in Python into 16-bit RGB
planes; FFmpeg only losslessly encodes those planes with FFV1. No color filter
generates the fixtures or expected pixels. It then compares the actual worker's
RGBA with independent transfer equations, D65/BT.2020-to-BT.709 primary matrices,
Hable normalization and sRGB encoding. Each transfer has eight gray levels and
eight colored patches, including out-of-gamut colors. The fixture has no peak
metadata, so the expected tone-map peak is 1000 nits. The tolerance is three
eight-bit codes for conversion/quantization, not a timing threshold.

Observed: all 32 patch centers matched the independently computed RGB exactly
(maximum channel error 0). PQ gray inputs include 0, 0.1, 1, 10, 100, 203, 400,
and 1000 nits; HLG gray inputs cover scene values 0 through 1. Colored HLG patches
also exercise the luminance-dependent OOTF, not merely a per-channel gamma.
Every native invocation has a process-level timeout covering shutdown.

Compare the real 16:9 PQ and DV8 HDR10-base samples at 300 seconds:

```sh
python3 examples/mpv_sdr_gpu/tests/verify_preview_color.py \
  --report "$REPORT_ROOT/real-color-reference" \
  --real-pq "$MEDIA_ROOT/One Hundred Years of Solitude - S01E12 - The Innocent Train Had Arrived - [WEBDL-2160p][h265-10bit][EAC3 Atmos].mkv" \
  --real-dv8 "$MEDIA_ROOT/Star.Wars.Andor.S02E10.Make.It.Stop.2160p.DSNP.WEB-DL.DDP5.1.Atmos.DV.HDR.H.265-FLUX.mkv"
```

This mode compares worker RGB with direct FFmpeg decoding/base-layer conversion.
Both samples matched all 172,800 RGB bytes exactly (maximum and mean error 0),
with the correct PQ and DV8-base labels. This shared-filter reference establishes
the mpv bridge/base-layer integration; the independent synthetic oracle above,
not this reference alone, establishes the color math. No full-Dolby-Vision or
physical-display color acceptance is implied.

Run the repeatable, desktop-free lifecycle acceptance:

```sh
bash examples/mpv_sdr_gpu/tests/verify_preview.sh \
  "$MEDIA_ROOT/bbb_h264_aac_1080p.mp4"
```

The script builds first, then wraps every native test process in GNU
`timeout --kill-after=5s` (45 seconds for the coordinated tests and normal worker,
30 seconds for the FIFO fixture). Thus the deadline covers native destruction,
`Drop`, worker join, and process exit; a timeout is a failure, not a pass.
It uses the environment variables above, generates a temporary MPEG-TS FIFO
with ffmpeg, and removes only that fixture and its producer. The FIFO producer
may report Broken pipe when the rejected preview closes its reader.

The three opt-in tests under `preview::tests::native` first obtain a frame from
the configured color pipeline, not just the disposable metadata probe. They
then synchronize at a real libmpv seek command acknowledgement before the worker
may consume completion:

- Cancel A, request C, then release A; only C may be delivered. Repeat with a
  same-path media replacement while A is held.
- Suppress completion events after a real acknowledgement while still draining
  rendering. A test-only 200 ms deadline exercises the production failure path;
  later requests fail, and explicit media reload recovers.
- Start `Drop` while the acknowledged request is held; release it only after
  shutdown has invalidated the mailbox, then require context/core teardown and
  worker join to return.

These coordination/fault-injection hooks exist only under `cfg(test)`. The
missing-completion test models a lost/rejected native completion; it does not
claim the local H264 file naturally rejects its seek. The separate real FIFO
test requires `--expect-nonseekable`, checks explicit failure for a second target,
and proves process exit after rejection. `av://lavfi` is not a substitute for
this fixture: mpv may mark it seekable.

All three coordinated cases also passed on the real PQ source in 2.51 seconds,
after warming the configured HDR-to-sRGB pipeline. This includes cancellation,
same-path replacement, missing-completion failure/recovery, and shutdown with
an acknowledged request still in flight. Context/core destruction and worker join
returned within the process guard. Reproduce with:

```sh
cargo test -p mpv_sdr_gpu --lib --no-run
MPV_PREVIEW_TEST_MEDIA="$MEDIA_ROOT/One Hundred Years of Solitude - S01E12 - The Innocent Train Had Arrived - [WEBDL-2160p][h265-10bit][EAC3 Atmos].mkv" \
  timeout --kill-after=5s 45s cargo test -p mpv_sdr_gpu --lib \
  preview::tests::native -- --ignored --test-threads=1 --nocapture
```

On the restored persistent libplacebo installation, the long-H264 worker run and
FIFO rejection/teardown passed the process guards. Ordinary
`cargo test -p mpv_sdr_gpu` passed 19 tests and skipped the three opt-in native
tests. `cargo fmt -p mpv_sdr_gpu --check`, the application build, and package-scoped
`cargo clippy -p mpv_sdr_gpu --all-targets --no-deps -- -D warnings` passed.
Dependency-inclusive strict Clippy is currently blocked by unrelated diagnostics
in `wgpu/src/blur.rs` and `wgpu/src/image/progressive.rs`; those files were not
changed for this task.

### Layer 3: actual native UI and GPU integration

Run the ordinary application on a desktop with a Vulkan 10-bit surface:

```sh
cargo run -p mpv_sdr_gpu -- "$MEDIA_ROOT/bbb_h264_aac_1080p.mp4" \
  --libmpv "$LIBMPV" --baseline "$MPV_BASELINE" --host-abi v3 --pause
```

Tab/Shift+Tab navigate focus. On the focused timeline, arrows seek five seconds,
Shift+arrows one second, and Home/End select the bounds. Dragging changes only the
target; release inside commits, release outside cancels. Escape, window focus
loss, and loss of the owning touch cancel. A second touch must not steal the
gesture. Watch the observed playback clock separately from the pending target.

Absolute seeks have caller request IDs. The host keeps one submitted seek and
one replaceable latest target; it drains old events before submitting. Completion
requires command acknowledgement, a subsequent SEEK/restart, and an asynchronous
position snapshot tagged to that submission. The UI only acknowledges its latest
request. There is no fixed position tolerance: sparse video may legitimately land
well after the requested time, and same-position seeks need not emit time-pos
changes. External IPC seeks should not race the application's seek commands.

Capability success only marks a request ready: submission waits until
`MPV_EVENT_NONE`, after all already-queued End/Start/FileLoaded events have been
drained. Local LoadFile (including same-path reload) and Stop invalidate pending
capability/seek/queued work immediately, before native lifecycle notification.
Deterministic regressions cover capability-ready followed by a same-batch media
transition, and immediate local invalidation. External IPC load/seek commands
must still not race application commands; libmpv seek has no atomic media ID.

Before submitting an absolute seek, the host asynchronously queries its current
seek capability. A single host-owned wake timer enforces a ten-second deadline
across that query, command acknowledgement, restart, and position snapshot,
including when no further native event arrives. On timeout, both the active
request and the latest queued request receive their own correlated error.
The absolute-seek channel remains failed until a file transition: submitting
another seek immediately could misattribute an old, untagged restart to it.
No main-player reload, pause change, or success is synthesized. The original
native seek may still finish later; a timeout is not proof that it was canceled.
Host timeout/latest-request and timer-shutdown regressions run without libmpv.
The unlocked desktop run below also passed normal native Host surface, seek,
resize/reopen and reload integration. Successful native seeks are not a
fault-injection proof of the Host timeout path; that remains covered separately
by deterministic state/timer regressions.

Run these five core scenarios:

| Scenario | Pass criterion | Coverage status |
| --- | --- | --- |
| Drag, Escape, then release | No seek; paused playback position is unchanged | State regression and actual desktop input checked |
| Drag through several targets, then release | Only the final target is committed; stale notifications do not pull the pending target back | Widget/state regression and actual desktop input checked |
| Request A, B, C while earlier work is in flight | Only C may replace the displayed thumbnail, even if A finishes late | Deterministic late-publication and coordinated native regressions passed; actual rapid A/B/C hover displayed the final target |
| Leave, cancel, change media, or close | No stale thumbnail becomes visible; shutdown completes normally | Worker invalidation/reload/teardown and actual leave, cancel, focus loss, media changes and normal UI exit checked |
| Preview while playing and while paused | Preview does not seek, pause, or resume the main player; playing clock continues normally | Fresh-draw combined native smoke and ordinary application input both passed with IPC clock/pause observations |

For the first two scenarios also check release outside, keyboard repeat,
Shift+arrows, Home/End, Tab focus, and a second finger. Keep screenshots of the
actual controls and thumbnail surface; record observed positions and pause state.
An IPC observer may help, but it must not replace input through the widget.
Keep an IPC connection open while observing to avoid broken-pipe log noise.

The existing native GPU acceptance remains useful:

```sh
cargo run -p mpv_sdr_gpu --example native_smoke -- \
  "$MEDIA_ROOT/bbb_h264_aac_1080p.mp4" \
  --libmpv "$LIBMPV" --baseline "$MPV_BASELINE" \
  --host-abi v3 --report "$REPORT_ROOT/native-v3"
```

It needs a moving video at least ten seconds long and a window manager that
honors fullscreen requests. It waits for a fresh presentation before capturing,
and separately requires colored video pixels in the lower half and image-overlay
pixels at the top. The overlay alone must not satisfy the video check. It covers
five RGBA screenshots, pause, resize, paused seek, window close/reopen,
resume, stop/reload, and teardown. Each `.txt` records the corresponding `.rgba`
dimensions and pixel counts; a 30-second watchdog rejects a stuck run.
By default it calls `Player` directly without timeline input or thumbnails.
Repeat with `v1` and `v2` when checking legacy ABI compatibility.

Add `--preview` to run the independent software worker in the same process as the
GPU main player. It requests four distinct targets while playing and again while
paused, rejects unexpected main-player seek/restart or pause transitions, requires
the playing clock to advance, and requires the paused position to remain exactly
unchanged. It also composes the decoded image in the native window. Use the
focused mode to avoid making this isolation check depend on fullscreen support:

```sh
cargo run -p mpv_sdr_gpu --example native_smoke -- \
  "$MEDIA_ROOT/bbb_h264_aac_1080p.mp4" \
  --libmpv "$LIBMPV" --baseline "$MPV_BASELINE" --host-abi v3 \
  --preview --seek-only --report "$REPORT_ROOT/native-preview"
```

This is a combined native integration regression, **not** timeline input replay.
The actual application's hover, drag, cancellation, and image placement still
need the ordinary-window checks above.

For sparse-frame and same-position seek completion without a fullscreen
prerequisite, use the same executable's focused mode:

```sh
mkdir -p "$REPORT_ROOT"
ffmpeg -n -f lavfi -i testsrc2=size=320x180:rate=1 -t 12 \
  -c:v libx264 -g 1 "$REPORT_ROOT/low-fps.mkv"
cargo run -p mpv_sdr_gpu --example native_smoke -- \
  "$REPORT_ROOT/low-fps.mkv" --libmpv "$LIBMPV" \
  --baseline "$MPV_BASELINE" --host-abi v3 --seek-only \
  --report "$REPORT_ROOT/low-fps-seek"
```

This mode checks a paused seek to 5.25 seconds followed by a seek to the actual
returned position, two screenshots, and shutdown. A 1 fps file can return 6.0
seconds for both requests; both must complete without a time-pos change or a
fixed proximity threshold. It does not claim resize/reopen coverage. An earlier
sparse-file smoke timed out in Resize, before seeking. That sparse resize case
was not repeated in final acceptance; the successful full H264 run below does
not establish sparse-file resize behavior.

When measuring preview performance, record target-change-to-valid-result
latency, media/codec, cache state, and the configured resource bounds. Do not
invent a pass threshold or add product telemetry just for acceptance.

Current worker acceptance passed on long H264, short HEVC and AV1, full-range
Sintel AV1, and the synthetic 1 fps source. Sintel's decoded content occupies
136 rows within the 180-row output, consistent with its 1920x818 aspect ratio;
this is not a calibrated full-range color comparison. The long H264 endpoint
request 634.6 returned 634.566667; the 1 fps target 3.3 returned 4.0.
Native color acceptance now includes the independent PQ/HLG fixture oracle and
the real PQ/DV8 HDR10-base comparisons above. HLG has synthetic native acceptance,
not a camera/source-media corpus. No real Profile 5 acceptance or full Dolby
Vision RPU/enhancement-layer rendering is claimed.

One uncached SDR H264 acceptance run with conversion recorded 175.6, 87.3, 276.7,
and 78.0 ms for the four targets. Initial real PQ and DV8-base captures at 300
seconds took 363.1 and 365.5 ms, including color probing/reopen. These are
observations, not latency guarantees or a benchmark; CPU decode remains the
dominant variable and the first request includes native initialization.

#### Unlocked desktop acceptance

Final acceptance used dedicated windows on niri, a 1.75-scale output, the
persistent libplacebo installation, and Host ABI v3. Existing user players and
lock-screen settings were not operated. No backend matrix was rerun.

- `native_smoke --preview --seek-only` passed two fresh screenshots, separate
  video/overlay pixel checks, fractional and same-position seek completion, and
  independent thumbnail composition. Across its four playing previews, observed
  main time advanced from 0.366667 to 0.800000 seconds. All four paused previews
  kept main time exactly 5.266667 and `pause=true`.
- Full `native_smoke` passed all five screenshots, paused resize/seek,
  zero-window close/reopen, resume, stop/reload and shutdown.
- The ordinary application was driven through compositor pointer/keyboard input,
  not `Controls` messages or direct seek calls. A persistent IPC connection
  observed the main player. Rapid hover A/B/C ended at C; holding a drag,
  Escape followed by release, and release outside kept paused main time at
  0.966667 with no main seek events. Focus loss to a second owned test window,
  then release after returning, kept 399.166667 with no seek.
- Internal release produced one seek/restart pair and landed at 385.166667.
  Tab/Shift+Tab returned focus to the timeline; three Right presses followed by
  Shift+Left accumulated exactly +14 seconds to 399.166667. Home/End completed
  at 0.066667/634.533333, with no lingering Seeking state.
- Actual playing hover advanced main time from 399.266667 to 400.533333 with
  `pause=false` and no main seek/restart events. Paused hover preserved position.
  The fixed preview panel stayed inside the window at both timeline ends;
  leaving cleared it. Resizing the owned window from 1000x700 to 700x600 logical
  pixels preserved input, thumbnail composition and paused state.
- Solitude PQ and Andor DV8 displayed actual `300.00s` thumbnails while main
  time stayed at 300.0 with `pause=true`. The DV8 caption explicitly states
  HDR10 base, no RPU/EL. Their screenshots were compared with the same-timestamp
  SDR references already checked against direct FFmpeg, not with the main GPU
  video as a color reference. After BOX-reducing the 560x315 physical-pixel
  thumbnail crops to 320x180, mean absolute eight-bit RGB differences were
  1.77 (PQ) and 0.45 (DV8). Fractional-scale sampling/crop alignment contributes
  to these differences; these are screenshot observations, not colorimetric
  pass thresholds or physical-monitor calibration.
- Leaving the DV8 timeline cleared the image. The ordinary application then
  closed normally with exit code 0; the temporary input client disconnected.

Evidence from this run is outside Git:

| Directory | Evidence |
| --- | --- |
| `/tmp/iced-final-native-preview` | Two RGBA captures and dimension/video/overlay counts; PNG of paused combined composition |
| `/tmp/iced-final-native-full` | Five RGBA captures and counts; PNG of the final reload |
| `/tmp/iced-final-ui` | Input/hover/cancel/resize screenshots, `observations.json`, PQ/DV8 300-second captures, extracted thumbnails, SDR references and side-by-side comparisons |

The prior locked-desktop screenshot passes were invalid because the old gate
could accept stale overlay-only pixels. They remain excluded from visual
evidence; the unlocked runs above use the tightened fresh-video gate.
Physical touch, other compositor/scale combinations, real Profile 5, full Dolby
Vision RPU/EL rendering and instrumented display calibration were not tested.

The application-level joint regressions and manual color comparison remain in
[FORK_MAINTENANCE.md](../../FORK_MAINTENANCE.md). Passing this example does not
prove JellyPilot packaging, tray behavior, or HDR display acceptance.

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

Headless checks:

```sh
cargo test -p mpv_sdr_gpu
cargo clippy -p mpv_sdr_gpu --all-targets --no-deps -- -D warnings
cargo fmt -p mpv_sdr_gpu -- --check
```

Opt-in real desktop acceptance needs a moving video at least ten seconds long,
a Vulkan 10-bit surface and a window manager that honors fullscreen requests:

```sh
cargo run -p mpv_sdr_gpu --example native_smoke -- /path/to/video.mp4 \
  --libmpv /path/to/libmpv.so --baseline /path/to/gpu-next-host-baseline.conf \
  --host-abi v3 --report /tmp/iced-mpv-v3
```

Repeat with `v1` and `v2` for legacy compatibility. This executable verifies video
and an image overlay in five RGBA screenshots, pauses, changes window size, seeks
while paused, closes all windows, reopens the same player, resumes, stops/reloads,
and closes during active playback. The process must exit successfully after host
and completion-worker teardown. Each `.txt` records its corresponding `.rgba`
dimensions and pixel counts. A one-shot watchdog fails a stuck run after 30 s.
Ordinary `cargo test` does not launch windows.

The application-level joint regressions and manual color comparison remain in
[FORK_MAINTENANCE.md](../../FORK_MAINTENANCE.md); passing this example does not
prove JellyPilot packaging, tray behavior or HDR display acceptance.

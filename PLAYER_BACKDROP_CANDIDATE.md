# Player live backdrop candidate

This is a targeted backport on **25607290dbb60a1365850f7e2afb5ec1b3201063**,
the inspected JellyPilot pin. It does not merge the current iced main/upstream.
It ports local scene/profile rendering from `2584b95bb316e4909b036688b31fbaae83ee19c5`
and the dense Gaussian/GL texture fixes from the preserved main working tree.
The isolated checkout is `/tmp/iced-player-backdrop-25607290-sqOqub`.

## API and dependency scope

| API | Samples normal live `Primitive::draw` video? | Candidate |
| --- | --- | --- |
| `image(handle).blur(radius)` | No: filters that image's source | Old scalar API unchanged |
| Main's `image(handle).blur(Blur::vertical_gradient(...))` | No: still an image-source effect | Not backported |
| Existing `Renderer::blur_backdrop(f32)` | Yes: earlier scene, uniform radius | Old API and scalar marker preserved |
| New `backdrop(Blur::vertical_gradient(...), content)` | Yes: earlier scene inside the widget's bounds | Wgpu live effect; software/custom renderers keep sharp scrim/content |

The additions are `core::Blur`, `renderer::Backdrop`, a default no-op
`Renderer::draw_backdrop`, a transparent widget wrapper and wgpu scene passes.
Widget/Element/Layout, scalar image blur, Engine constructors, queue synchronization
and consumer color/HDR contracts retain their pinned interfaces. Public
`wgpu::Layer.backdrop_blur` remains `Option<f32>`; local descriptors are private.
The software renderer stays identical to the pin, including its public Layer.
All iced crates must still come from the same candidate checkout.

No dependency versions change. `half` is already locked; the only Cargo change
adds it as a root dev-dependency for raw FP16 readback. Glass optics, progressive
images, adaptive quality and upstream Widget migrations are outside this slice.

## Caller recipe

The compiled recipe is [tests/support/player_backdrop.rs](tests/support/player_backdrop.rs).
Pass the existing live video Element, existing bottom Minimal/Full content (including
its original scrim), and the independent popover. Keep upper player chrome as a
separate sharp layer; this helper does not discover bottom controls inside a full-screen
overlay. Use the application's existing state:

```rust,ignore
player_layers(
    video,                         // Existing embedded video, not a poster Image.
    Some(current_controls),        // Existing Minimal or Full content and scrim.
    independent_popover,
    full_visible,                  // False draws Minimal sharply without a marker.
    320.0,                         // Paper candidate height in logical pixels.
)
```

The recipe wraps only Full in a bottom-aligned panel, using
`Blur::vertical_gradient(0.0, 16.0).range(0.15, 1.0)`. The **320 px region and 16 px
sigma are static design candidates**, not visual acceptance or fixed product
requirements. Keep the existing scrim within the foreground content: unsupported
renderers then draw the same scrim and controls through the default no-op effect.
Do not replace the existing scrim with the blur or change product tint styling.

Full -> Minimal removes the backdrop entirely. `None` also removes controls;
`.enabled(false)` is available when the child itself must remain. No effect marker
means no extra scene capture, blur convolution or blur-cache sampling. Previously
allocated buffers may remain resident for reuse; hidden does not promise a memory
release. Popovers are appended after the controls and remain sharp.

The wrapper has no animation clock, subscription, timer or redraw request. Continue
using the existing video producer's frames and the caller's reduced-motion policy.
For reduced motion, keep the profile/panel size static and any existing visibility
transition immediate; reduced motion does not freeze the playing video.

### Subtitles and color

JellyPilot's `video_surface` contains mpv's final composed image. Subtitles already
burned into those pixels **cannot be identified, excluded or restored sharply by
this API**. Subtitles in the bottom gradient region are filtered along with video.
Only subsequent iced content (buttons, tooltips, independent popovers) stays sharp.
Choose the bottom falloff with that limitation; no mpv ABI or subtitle layer is added.

Scene capture, immutable snapshot, tile intermediates and compositing use the
Engine's format, including `Rgba16Float`. Blur operates on premultiplied scene RGB
and alpha; it adds no HDR transfer, tone mapping or display conversion. The consumer
continues to own the final presentation conversion.

## Evidence and limits

The focused test commands below passed: **17 executions / 15 distinct tests**
(the two FP16 tests also run on GL). All commands run from this candidate:

```sh
# 6 GPU tests: 2 raw-FP16 player tests + 4 scene/clip/legacy regressions
env ICED_TEST_BACKEND=wgpu cargo test -p iced --locked --offline --features advanced,image,svg,canvas --test player_backdrop --test player_backdrop_fallback -- --nocapture
# 2 raw-FP16 tests on OpenGL
env ICED_TEST_BACKEND=wgpu WGPU_BACKEND=gl cargo test -p iced --locked --offline --features advanced,image,svg,canvas --test player_backdrop -- --nocapture
# 5 unchanged old-pin tests
env ICED_TEST_BACKEND=wgpu cargo test -p iced --locked --offline --features advanced,image,svg,canvas --test native_blur -- scene_barrier scene_marker widget_glass blur_samples resuming_parent
# 1 actual-widget software scrim fallback test
cargo test -p iced --locked --offline --features advanced,image,svg,canvas --test player_backdrop_scrim -- --nocapture
# 3 transparent-widget lifecycle/input/overlay tests
cargo test -p iced_widget backdrop::tests --lib --offline
```

`tests/player_backdrop.rs` uses normal
`Primitive::draw`, `Engine::new_with_queue_synchronization`, a raw RGBA16F target,
and half-float readback. It changes video pixels without changing primitive identity,
checks unclipped profile coordinates, premultiplied alpha, sharp upper picture,
foreground/popover order and Full -> sharp Minimal / disabled effects.

At the tested lower gradient point, RGB/alpha changed from
`[2.8964844, 1.8417969, 1.2871094, 0.42114258]` to
`[5.7929688, 3.6835938, 2.5742188, 0.42114258]` when video RGB doubled.
These are raw float values, not RGBA8 screenshots. Fresh hidden controls allocated
zero blur bytes; hiding after Full left the scene hit/miss counters unchanged.

The targeted legacy image/scene checks are reused from this pin; the previous main
57-test run is historical evidence and is not claimed as a run of this candidate.
The added backend-neutral GPU tests inspect numeric bytes only. The tiny-skia test
checks the no-op/scrim fallback, not software progressive filtering.

Compilation passed with both default features plus `advanced,image,svg,canvas`,
and `--no-default-features --features wgpu,advanced,image,svg,x11,wayland,thread-pool`
(`cargo check -p iced --locked --offline`). The independent source review checked
public API compatibility, queue/format preservation, dynamic primitive caching and
the separate full-scene semantics of the old scalar marker. Both the public scalar
field and those semantics were preserved during backport review; a new regression
compares full outputs for scalar markers recorded in small/offscreen clip layers.

`git diff --check` passes. `cargo fmt --all --check` reports pre-existing formatting
in pinned program/daemon/queue/compositor code, including unchanged queue-guard
lines in `wgpu/src/lib.rs` and Engine constructor calls. Those unrelated lines are
preserved; new files and the other changed Rust files pass scoped rustfmt checks.

This is source/headless evidence, **not JellyPilot execution, physical HDR,
subtitle visual acceptance, or playback/game frame-rate measurement**. Local
progressive filtering keeps two viewport-sized scene targets plus bounded tiles;
video changes require fresh filtering. A smaller affected region does not eliminate
the full-scene copy. GL's two-layer texture workaround increases retained bytes.
At 320 logical px, cost also depends on DPI, video/window size and sigma. Do not
infer real-time 4K performance from small offscreen fixtures.

## Publication and consumption plan (not executed)

This candidate is a local commit on the old pin; the original iced main/dirty tree
and all JellyPilot files/pins are preserved. The isolated clone's `origin` points to
the local source repository, so do **not** use an unqualified `git push origin`.

After the owner approves the exact revision/patch:

1. Verify remote branch state, then publish this exact commit to an agreed new
   topic branch at `git@github.com:hewel/iced.git`, with a normal non-force push.
   Do not move the fork's current main backwards to this old-pin branch.
2. Read the remote ref back and compare its full SHA with the candidate. A local
   commit or a patch file is not evidence of remote publication.
3. The JellyPilot owner updates `tools/embedded-mpv/iced-source.json` to that full
   published SHA and prepares one consistent application candidate combination.
   Follow its README published-pin contract: in a fresh application checkout,
   `bun install --frozen-lockfile`, then `bun run task iced prepare` **without
   `--source`**; verify exact fetched HEAD, tracked cleanliness and all iced crates
   under `target/vendor/iced`. No checked-in patch/fallback revision is introduced.
4. Apply the maintained consumer validation and separately authorized native/manual
   gates before product acceptance. Local `--source` preparation is useful for
   development but does not satisfy remote cold-prepare acceptance.

No publication, pin change, mpv rebuild, GUI launch or native visual acceptance is
performed by this preparation task.

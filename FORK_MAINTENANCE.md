# Fork maintenance

This fork's existing `main` is the long-lived product branch. Git commits, not an
application's exported patch, are the source of local modifications. This document
describes maintenance policy; it does not authorize a merge, publication, or Git
configuration change.

## Upstream baseline and provenance

The official baseline for the 2026-10-04 synchronization is
[`82acd61ea207d6f85b99da5e95b66f9a968de3f2`](https://github.com/iced-rs/iced/commit/82acd61ea207d6f85b99da5e95b66f9a968de3f2),
verified against the official repository's `refs/heads/master` before merging.
The previous upstream baseline was `c756d97e98c67b0be78ffdf6f71b577a44fe394c`;
the pre-sync product tip was `25607290dbb60a1365850f7e2afb5ec1b3201063`.
The synchronization retains both histories with a real merge. These are recorded
revisions, not a promise that upstream's moving branch still points there.

Recheck evidence rather than inferring ancestry from a release number or subject:

```sh
git remote -v
git log --merges -n 1 --format='%H %P' main
git merge-base main 82acd61ea207d6f85b99da5e95b66f9a968de3f2
git log --oneline 82acd61ea207d6f85b99da5e95b66f9a968de3f2..main
git ls-remote https://github.com/iced-rs/iced.git refs/heads/master
```

The inspected checkout has only `origin = git@github.com:hewel/iced.git`, no
`upstream`. If history or official remote evidence is unavailable, record the
baseline as unverified; do not substitute an assumed tag or latest release.

This baseline changes custom widget contracts (`Meta`, generic widget content,
layout stored in `widget::Tree`, and overlay invalidation) and includes upstream
wgpu 30 support. The fork now uses the official `iced-rs/cryoglyph` revision
`e13618df29a5593968040c12ab086b7237249ded`; the temporary `hewel/cryoglyph` wgpu-30
pin is retired. Custom compositor and shared queue contracts below still apply.

The existing fork hashes under `examples/styling/snapshots` and
`examples/todos/snapshots` remain reference artifacts from before this sync.
They were not regenerated to accept the new layout/text output. Visual snapshot
acceptance and the application's joint native/color acceptance remain separate
from compilation and the focused behavioral regression tests.

## Local changes to preserve

The following are integration changes, not an exhaustive inventory of this fork.
The baseline-to-main history also contains native corner smoothing and image/scene
backdrop blur (`bc93c1f1`, `f812ae50`); preserve them during synchronization.
Use `git show <commit>` for rationale and the exact changed-file set.

| Commit / concern | Source and interface | Why it exists |
| --- | --- | --- |
| [`2f114f845d9ba0a8489364edf752be6234ba8f73`](https://github.com/hewel/iced/commit/2f114f845d9ba0a8489364edf752be6234ba8f73): generic compositor factory | [src/daemon.rs](src/daemon.rs), [program/src/lib.rs](program/src/lib.rs), [winit/src/lib.rs](winit/src/lib.rs): `Daemon::run_with_compositor<C, F, Fut>` and `iced_winit::run_with_compositor` | Let an application retain an external device/host and construct its compositor without replacing its existing renderer or ordinary widget tree. |
| Same commit: shared queue synchronization | [wgpu/src/queue.rs](wgpu/src/queue.rs), [engine.rs](wgpu/src/engine.rs), [lib.rs](wgpu/src/lib.rs), [image/mod.rs](wgpu/src/image/mod.rs), [image/cache.rs](wgpu/src/image/cache.rs), [window/compositor.rs](wgpu/src/window/compositor.rs): `QueueSynchronization`, `QueueGuard`, `Engine::new_with_queue_synchronization` | Exclude concurrent native queue use across iced submissions, background uploads, surfaces, and an external renderer. |
| [`3e32da72f7faf6e9b16f0b259756aba3c3ed0b96`](https://github.com/hewel/iced/commit/3e32da72f7faf6e9b16f0b259756aba3c3ed0b96): committed isolation example | [examples/mpv_sdr_gpu](examples/mpv_sdr_gpu): `Cargo.toml`, `src/{main,controls,gpu,interop,video}.rs`, `src/video.wgsl`; root `Cargo.lock` | Exercise shared-device Vulkan SDR playback, bounded GPU copies, a required 10-bit surface, and ordinary iced controls before application integration. It predates the generic hook and is not evidence that a consumer has opted into it. |

### Contracts at the synchronization boundary

The isolation example optionally enables supported `VK_KHR_external_memory_fd`,
`VK_EXT_external_memory_dma_buf`, and `VK_EXT_image_drm_format_modifier` extensions
on its actual Vulkan device. The same enabled set is used for HAL import and
reported to mpv; missing optional extensions do not reject device creation.
This permits testing direct VAAPI import with a matching mpv host, but does not
establish hardware decoding or support for a particular plane format/modifier.
The application's separate device creation path requires its own integration.

- The factory is application-scoped, not called once per window. It receives owned
  backend settings, display handle, `Arc<Window>`, and graphics `Shell`; its owned
  `'static` future returns `Result<C, backend::Error>`. Neither factory nor future
  requires `Send`. `C::Renderer = P::Renderer` is preserved through daemon builders
  and program decorators, including the default fallback renderer type.
- Creation is lazy on first window, repeats after the last window closes, and is
  reused for native backend reconfiguration. Capture shared host/device ownership
  in the factory if it must survive a zero-window daemon. Surfaces/renderers are
  recreated; ordinary exit drops window resources before the compositor. This
  does not make `Compositor::create_surface` or `configure_surface` fallible.
- `unsafe trait QueueSynchronization: Send + Sync` has safe `lock` and unsafe
  `unlock`. Implementers guarantee real exclusion, current-thread ownership, and
  acquire/release visibility. `lock` must not panic after acquiring; `unlock` must
  not panic. The gate is non-reentrant.
- Safe `QueueGuard::acquire` constructs the allocation-free guard only after lock
  succeeds. Private fields and `PhantomData<Rc<()>>` make it **neither Send nor
  Sync**; Drop releases on the acquiring thread. Do not retain it across an iced
  operation that locks the same gate.
- Install the same `Arc<dyn QueueSynchronization>` before creating renderers and
  image workers. Guard actual renderer/screenshot/image submissions; the worker's
  submit and completion registration form one guarded transaction. Queue writes
  stage data for submission. Do not put an outer guard around draw, present, or
  primitive preparation: that would nest internal acquisition.
- Surface configure, acquire, present, and abandoned/suboptimal frame discard
  each need exclusion. The stock helper guards its own drop path, but wgpu may
  suppress native texture discard during panic unwinding; this is not a native
  cleanup guarantee. Custom compositor surface operations and independent/raw
  primitive submissions must acquire the same gate themselves.
- Submission callbacks can run while the guard is held: CPU-only wake/send, no
  native queue calls or gate reentry. Device polling, mpv API calls, termination,
  and thread joins stay **outside** the guard. The application owns the enabled
  device feature chain and must retain native device/queue/image resources until
  mpv shutdown and outstanding GPU work are complete. A guard is not a GPU fence.
- Ordinary `run` still constructs the default compositor; `Engine::new` installs
  no hook. There is no global gate registry. The stock `Rgb10a2Unorm` /
  `Rgb10a2Uint` format blacklist is unchanged. A custom compositor, not a global
  blacklist edit, owns strict 10-bit capability selection and failure reporting.

### Business boundary and upstream proposals

iced owns generic rendering/lifecycle extension points. mpv owns playback timing,
scheduling and its host ABI; JellyPilot owns host lifetime, IPC, playback policy,
settings, tray, locale, packaging and capability errors. Do not import those
business rules into iced. See [mpv fork maintenance](https://github.com/hewel/mpv/blob/iced-player/DOCS/fork-maintenance.md)
and the [application maintenance contract][app-maintenance].

The generic factory and optional queue hook are **candidates for discussion with
upstream**, not accepted upstream APIs or promises of acceptance. Follow
[CONTRIBUTING.md](CONTRIBUTING.md); propose independently reviewable generic needs,
not an mpv-specific renderer or a stock format-policy change.

## Synchronizing one dependency group

Start from a clean, preserved product `main`; do not discard someone else's
changes. Use a `sync/*` branch and a **real merge of an exact official commit**.
Never squash an upstream sync or rewrite published history. Keep unrelated iced,
mpv, and other dependency upgrades separate so failures have an attributable
change set. If an ABI/type-compatible dependency group must move together, explain
that coupling in its PR and validate it as one unit; do not mix iced crate sources.

The following are maintainer instructions, not commands run by this document:

1. Inspect `git remote -v`. Only if `upstream` is absent and configuration is
   intentionally approved, add it explicitly:
   `git remote add upstream https://github.com/iced-rs/iced.git`.
   If it exists, verify its URL; do not silently replace it.
2. Fetch with `git fetch upstream master`. Record the full target SHA from
   `git rev-parse upstream/master`, verify it against the official source/history,
   and record `git merge-base main upstream/master`. A moving branch is not a pin.
3. Set `UPSTREAM_SHA` to the reviewed full SHA. Create
   `git switch -c sync/iced-SHORT_SHA main` (replace `SHORT_SHA`), then run
   `git merge --no-ff --no-commit "$UPSTREAM_SHA"`. Resolve conflicts against the
   contracts above and the full local commit history, not only the example.
4. Run the gates below, inspect the resolution, and commit the merge preserving
   both parents. Submit a PR into existing `main` with the report below. Retain
   that merge ancestry when integrating the PR: no squash/rebase integration.
   Publication and PR merge require their own authorization.

`rerere` is optional: a maintainer may deliberately enable repository-local
`git config --local rerere.enabled true` and
`git config --local rerere.autoupdate false`. Never enable it automatically.
Review reused resolutions and rerun acceptance; a remembered resolution is not proof.

## Synchronization acceptance

Run the existing four compile groups from this repository. The no-default-features
case explicitly retains `thread-pool` so it tests an actual native executor:

```sh
cargo check -p iced --features advanced,image,svg
cargo check -p iced --features debug,tester,hot,advanced,image,svg
cargo check -p iced --no-default-features --features wgpu,advanced,image,svg,x11,wayland,thread-pool
cargo build -p mpv_sdr_gpu
```

Then use the application's maintained [joint acceptance entry point][app-maintenance]
and [opt-in native regressions][app-regressions] as the **single source of commands,
media requirements, report paths and pass criteria**. Do not copy its command
catalog or dependency/version inventory here. Compile/startup smoke alone does
not cover joint GPU playback, concurrent images/screenshots, resize and reopen,
external playback, or real desktop tray/locale behavior. Run the actual native
regressions, not a headless substitute for tray interaction.

Perform the separate [manual three-way color comparison][app-color] against the
accepted native/iced/application surfaces. Record its evidence separately; never
silently regenerate the reference or retune color to make a sync pass. Report
unavailable GPU/desktop/media prerequisites as unverified, not passing.

### Sync PR report template

- **Scope:** dependency group, reason for any coupled updates, excluded changes.
- **Provenance:** old product SHA; old/new official full SHAs; remote URL;
  merge-base; resulting merge SHA/parents; local commits retained or intentionally
  changed, with rationale and conflict-resolution notes.
- **Boundary:** factory/renderer identity, queue ownership and lock/callback
  changes, device/surface teardown, format behavior and host ABI compatibility.
- **Evidence:** four compile commands/results; application revision and maintained
  acceptance command/report locations; GPU/desktop/media environment; manual
  color evidence; real tray/locale result; failures and checks not performed.
- **Consumption:** published fork revision, application pin/lockfile change,
  vendor provenance and complete artifact manifest/hash references.
- **Rollback:** known-good application revision and matching complete artifact
  set; exact restoration path and any remaining risks. Approval/publication state
  is explicit, never inferred from green checks.

## Application pinning and rollback

JellyPilot's cutover target is published commit
`2f114f845d9ba0a8489364edf752be6234ba8f73`, replacing its former
`3e32da72` base plus exported extension patch. This records the cutover, not a
second maintained dependency inventory. The application's
[iced source pin][app-pin], lockfile, vendor preparation and
[maintenance instructions][app-maintenance] remain authoritative. Keep the vendor
workflow and route all iced crates to that one verified source. Do not edit
`target/vendor/iced` or revive the old patch as an independent source of changes.
Future iced fixes are commits, then a published full-SHA application pin update.

Rollback the **whole known-good application pin/artifact set**: application
revision, source pins and lockfile, corresponding prepared vendor source, and
matching executable/libmpv/baseline/manifest artifacts. Use the application's
maintained preparation/build/recovery commands; never combine a new executable
with an old native library or stale vendor tree. Record artifact provenance and
hashes from the existing manifest, not a new version system. Reverting a published
application cutover is a new commit, not a reset or history rewrite; an emergency
previous release uses its complete matching artifacts. An explicit external-mode
recovery is a user choice, not evidence that embedded acceptance passed.

[app-maintenance]: https://github.com/hewel/jellypilot/blob/main/README.md#fork-maintenance-and-joint-acceptance
[app-regressions]: https://github.com/hewel/jellypilot/blob/main/docs/agents/validation.md#opt-in-native-regressions
[app-color]: https://github.com/hewel/jellypilot/blob/main/README.md#manual-three-way-color-comparison
[app-pin]: https://github.com/hewel/jellypilot/blob/main/tools/embedded-mpv/iced-source.json

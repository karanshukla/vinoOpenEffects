# Architecture

How video frames move from the physical webcam to applications like Zoom, and
the reasoning behind each design choice. Inline code comments stay brief and
link here whenever longer explanation is needed.

## Process model

```
 vinoopeneffects (GUI)      vinoopeneffects-ctl (CLI)      scripts, Waybar, ...
          │                          │                            │
          └──────────── D-Bus session bus ────────────────────────┘
                                     │
                          vinoopeneffects-daemon
              ┌──────────────────────┼───────────────────────┐
         D-Bus server          GStreamer worker         PipeWire provide node
         (tokio)               (its own thread)         (its own pw_main_loop thread)
```

The project ships three binaries. Only the daemon accesses the camera, ML
models, or PipeWire; the GUI and CLI are stateless D-Bus clients. CI enforces
this separation: the `core` job fails if `vinoopeneffects-core` links GStreamer,
GTK, libadwaita, or PipeWire, or if either client depends on `core` or `daemon`
rather than `shared` alone (preventing ONNX Runtime from being dragged into the
GUI).

| Crate | Responsibilities |
|---|---|
| `shared` | D-Bus names and value helpers, `state.toml` config, legacy-path migration, `doctor` probes, and the shared SHA-256 verification helper used by `core` and `doctor` |
| `core` | Pure effect and ML logic: inference (`ort`), gesture classification, and all effects; has no media-stack dependencies so it tests without GStreamer |
| `daemon` | D-Bus server, GStreamer capture pipeline, `oe_effects` element, and PipeWire provide node |
| `cli`, `gui` | Stateless D-Bus clients; `gui/build.rs` generates proxies from `data/dbus/*.xml` via `zbus_xmlgen`, while `cli` splits into `args` (clap), `daemon`/`effects` (commands), `bus` (proxies), and `values` (D-Bus parsing/printing) |
| `tests/integration` | `videotestsrc` pipeline tests and `#[ignore]`d D-Bus round-trip tests that require a running daemon |

The daemon starts on demand: both the D-Bus activation file and the systemd user
unit (`Type=dbus`) invoke `vinoopeneffects-daemon --start` on the first client
call. Before loading configuration, the daemon migrates any pre-fork
`openeffects` state (see
[install.md](install.md#upgrading-from-openeffects)).

## D-Bus API

The daemon owns `io.github.karanshukla.vinoopeneffects.Daemon` at object path
`/io/github/karanshukla/vinoopeneffects/Daemon` on the session bus only. The
introspection XML files in [`data/dbus/`](../data/dbus) define the contract:

| Interface | Members |
|---|---|
| `Daemon1` | Methods: `Start`, `Stop`, `Quit`, `SetInferenceProvider`; Properties: `Status`, `Capabilities`; Signals: `StatusChanged`, `InferenceChanged` |
| `Effects1` | Methods: `ListEffects`, `SetEnabled`, `SetParam`, `GetParams`, `GetAllState`, `SetBypass`, `GetBypass`, `TriggerReaction`; Signals: `EffectChanged`, `BypassChanged` |
| `Devices1` | Methods: `ListCameras`, `SelectCamera`; Properties: `VirtualCameraInfo`, `ActiveCamera` |

Effect IDs are `center_stage`, `portrait_blur`, `bg_replace`, `studio_light`,
and `reactions`. The daemon maintains one canonical `AppState`, persists it to
`~/.config/vinoopeneffects/state.toml`, and sends the full state snapshot to the
worker on every update instead of re-parsing individual keys.

## The camera opens only when someone is watching

```
                         consumer connects                consumer leaves
 provide node  PAUSED ──────────────────────▶ STREAMING ─────────────────▶ PAUSED
 (LED off)                                        │                          │
                                                  ▼                          ▼
 worker                                 build + start capture        300 ms grace, then
                                        pipeline (LED on)            tear down (LED off)
```

At startup, the daemon advertises a native `libpipewire` `Video/Source` node
while keeping the physical webcam closed. Because PipeWire graph laziness alone
does not gate hardware capture, the provide node's `state_changed` callback
drives capture lifecycle: entering `STREAMING` opens the camera, and moving to
`PAUSED`, `UNCONNECTED`, or an error state closes it.

Key lifecycle behaviors:

- **Release grace period:** Closing waits for `CAPTURE_RELEASE_GRACE` (300 ms).
  Browsers accessing cameras via xdg-desktop-portal probe the node with brief
  connect/disconnect blips and retry if they initially see warmup placeholder
  frames. Because opening a webcam takes ~250 ms, tearing it down on every blip
  would cause thrashing and prevent a stable stream.
- **Fault recovery and watchdog:** If the camera fails while streaming (unplug
  or USB stall), the worker drops the capture pipeline, clears `Bridge` (so
  consumers receive black frames instead of a frozen frame), and sets status to
  `error`. Failures are detected by both a bus sync handler (capturing the first
  error or EOS while `PLAYING`) and a stall watchdog (`CAPTURE_STALL` = 5 s) for
  silent stalls. While a consumer remains connected, the worker silently retries
  opening the camera every `CAPTURE_RETRY` (2 s) and returns to `running` on
  success (initial open failures retry the same way). It does not retry when no
  camera exists at all, avoiding a fallback to `videotestsrc` color bars.
- **Zero idle wakeups:** The worker blocks on a single inbox shared by daemon
  commands, provide-node state changes, and capture errors, setting a timeout
  only while a grace window, retry timer, or stall watchdog is active. The frame
  tick thread parks whenever no consumer is streaming, and closing the camera
  calls `malloc_trim` to return frame buffers to the OS.

### The provide node

Because `libpipewire` objects are not `Send`, the provide node runs on its own
thread with a dedicated `pw_main_loop`, communicating with the daemon via
channels and a shared latest-frame `Bridge`.

- **`media.class = Video/Source`:** WirePlumber only exposes nodes with this
  class to apps as cameras.
- **`DRIVER`:** A virtual camera has no hardware clock, yet consumers expect the
  source to drive the PipeWire graph. Without `DRIVER`, the graph never cycles,
  `process()` never runs, and consumers show a black "no active stream" screen.
  A dedicated tick thread using absolute deadlines (avoiding rate drift after
  late wakeups) calls `trigger_process()` while driving.
- **`RT_PROCESS`:** Without this flag, processing runs on the main loop and
  PipeWire 1.2+ schedules the node asynchronously (`node.async = true`). An
  async driver forces connecting consumers into async mode too, triggering a
  connect/crash/retry loop in Chromium's video-capture service.
- **`SPA_META_Header`:** WebRTC consumers (Chromium, Firefox) request header
  metadata and dereference it without a null check (`video_capture_pipewire.cc`,
  `h->flags`). Because PipeWire only allocates the region when both ports
  advertise it, the node replies during format negotiation with a `Meta(Header)`
  param alongside buffer requirements and stamps `seq` and `pts` on every
  buffer.
- **Format:** Always I420 at the selected camera's native resolution and frame
  rate. Before the first processed frame arrives, the node outputs limited-range
  black so consumers immediately see a valid signal.

Switching to a camera with a different native mode re-advertises the PipeWire
node, because consumers only renegotiate formats on reconnect.

## Capture pipeline

```
 v4l2src / pipewiresrc → capsfilter → decodebin → videoconvert → videoscale → capsfilter
   (real camera)        (native mode,                                          (I420)
                         raw or MJPEG)                                            │
          ┌───────────────────────────────────────────────────────────────────────┘
          ▼
 queue (leaky, 2) → videoconvert → RGBA → oe_effects → videoconvert → videoscale
                                                                          │
                                                   appsink (max-buffers=1, drop)
                                                                          │
                                                       Bridge → provide node → consumers
```

- **Camera selection:** Uses the configured camera if present (honoring an
  explicit `/dev/videoN` even if `DeviceMonitor` missed it), otherwise
  autodetects. When a device appears under both V4L2 and PipeWire providers,
  V4L2 wins because `v4l2src` on `/dev/videoN` requires no PipeWire camera
  permission grant (whereas `pipewiresrc` fails with `target not found` without
  one). `pipewiresrc` is reserved for PipeWire nodes without a V4L2 device path.
- **Native mode pinning:** The source capsfilter pins the camera to the virtual
  camera's advertised native mode in raw or MJPEG. Many UVC webcams only reach
  720p via MJPEG and otherwise negotiate raw 640×480, which `videoscale` would
  blur by upscaling. `decodebin` decodes MJPEG and passes raw through; its src
  pad appears dynamically and is linked in `pad-added`.
- **Fixed frame rate:** `v4l2src` sets `exposure_dynamic_framerate=0` on open.
  Other apps can leave it enabled, allowing auto-exposure to drop the frame rate
  in dim light (Chrome resets this control for the same reason).
- **Startup timeout:** Pipeline startup waits up to 5 s for `PLAYING` so caps
  negotiation errors surface immediately instead of failing asynchronously.
- **Latest-frame semantics:** `appsink` sets `max-buffers=1` and drops older
  frames, matching `Bridge`'s latest-frame design for live video.

## The `oe_effects` element

A single in-place CPU `GstBaseTransform` hosts all effects. It maps GObject
properties onto `core::effects::Settings`, packs each RGBA frame into a tight
reusable buffer, and delegates to `core::effects::Effects::process`.

When no effect is active and no reaction burst is running, the element switches
to passthrough so GStreamer skips invoking it. Triggering a reaction disables
passthrough even when all persistent effects are off, and the next idle frame
re-enables passthrough. Because idle frames never reach the element, a timer
thread evicts unused inference sessions after 60 s and reloads them on demand.

## Effects

Effects run per frame in a fixed order:

1. **Gestures** on the unmodified frame, before Auto light or Center Stage crop
   rewrites pixels in place. Confirmed gestures queue a reaction.
2. **Denoise** whenever Auto light, Studio Light, or Center Stage is enabled.
3. **Auto light** (opt-in, NPU only).
4. **Center Stage** (if enabled): computes framing on the full frame, runs
   masked effects only on the cropped ROI, and upscales once to output size. If
   Center Stage is off, masked effects run on the full frame.
5. **Reactions** last, so emoji overlays are never cropped or blurred.

### Denoise

Gain, contrast, and zoom amplify webcam sensor noise, so temporal denoising runs
first whenever Auto light, Studio Light, or Center Stage is enabled. Each pixel
is recursively averaged with the previous output, retaining up to 75% history
for static pixels (`MAX_HISTORY`).

Motion detection operates per 8×8 block (`BLOCK`) using the block's mean signed
luma difference against the history:

- Sensor noise cancels across an 8×8 block while real motion does not, allowing
  low thresholds (`STILL`, `MOVING`) without mistaking noise for movement (a
  per-pixel threshold high enough to ignore noise would smear slowly drifting
  faces).
- Each block adopts the highest motion value of its neighbors so static blocks
  adjacent to a moving edge do not trail into it.
- Any individual pixel changing by 24 luma levels (`PIXEL_MOVING`) bypasses
  history directly to preserve thin edges that a block average could dilute.
- History resets on resolution changes, skipped frames, or passthrough periods
  (when frames stop arriving and the frame counter stalls).

The history buffer stores 8 fractional bits. Auto light reads those sub-integer
values directly rather than the rounded frame so a 6× gain multiplies the
averaged signal and rounds to `u8` once, avoiding visible banding.

### The person mask

Studio Light, Portrait blur, and Background replace share one selfie
segmentation mask computed at most once per frame, so enabling multiple masked
effects adds no extra ONNX inference.

- **Cadence:** Segmentation runs every 2nd frame (`SEG_INTERVAL`, ~15 Hz at
  30 fps), cutting ONNX work in half. The held frame is masked by temporal EMA
  and per-frame guided filtering.
- **Temporal EMA:** Each fresh 256² mask blends via `e = α·new + (1-α)·e` with
  `SEG_EMA_ALPHA = 0.5`, suppressing boundary flicker without blurring the
  downstream image-aligned edge (lower values are steadier but laggier).
  Switching between full-frame and ROI coordinates clears EMA history since the
  grids differ.
- **Hands:** Tracked hands are painted into the mask prior to EMA so hand edges
  smooth consistently with the body.
- **Guided filter:** Linear coefficients are computed at 1/`GF_SUBSAMPLE`²
  (1/16) of pixels and applied to full-resolution luma every frame so mask edges
  snap to real luma boundaries. `GF_EPS = 1e-4` balances edge sharpness against
  noise, and the window radius is `min(width, height) / GF_RADIUS_DIVISOR` (96)
  to keep fits local and avoid wide halos.
- **Edge shaping:** Filtered alpha passes through a smoothstep across
  `0.5 ± GF_EDGE_WIDTH` (0.06), producing a crisp anti-aliased boundary instead
  of a soft halo.

### Portrait blur and Background replace

Both composite via `out = fg·mask + bg·(1-mask)`.

For Background replace, `bg` is the selected image or solid color. For Portrait
blur, `bg` is a background-weighted blur of the frame so foreground colors do
not bleed outward into a halo: the frame is downsampled with premultiplied color
and background weight in alpha, blurred with two separable box passes
(approximating a triangle kernel in radius-independent time), and
un-premultiplied during compositing.

### Studio Light

Applies a masked brightness and contrast adjustment to the foreground subject
along with an independent background brightness control. Contrast pivots around
the subject's mean luma so dim faces are not darkened; below ~1% mask coverage,
the mean is too noisy and the previous pivot is held.

**Auto light** uses an IAT low-light model on the NPU only (it takes ~850 ms per
frame on CPU, so it never runs on CPU). Rather than using the model's RGB output
directly—which over-brightens well-lit scenes and shifts colors—vinoOpenEffects
extracts only the luminance ratio relative to the input as a low-resolution gain
map, EMA-smooths it, applies it to the full-resolution frame, and fades it out
with a mean-luma gate in well-lit rooms. Without an NPU or the model file, Auto
light is a no-op.

### Center Stage

YuNet face detection drives an EMA-smoothed crop and zoom (`CS_ALPHA = 0.18`,
~12 frames to reach 90% of target). The crop stays square in normalized
coordinates to preserve aspect ratio, and never shrinks below `CS_MIN_CROP`
(4× max zoom) to avoid excessive pixelation on distant faces. YuNet runs every
4th frame (`FACE_INTERVAL`), with the comfort zone absorbing the update cadence.
If no face is detected, the current target holds steady rather than drifting on
a brief look-away.

Framing targets the eye midpoint shifted down by one-quarter of face height for
balanced headroom; `group` mode frames the union of all detected head boxes.
When combined with a masked effect, the ROI is extracted at whole-pixel bounds
(with a 1-pixel margin) and the exact fractional crop is applied once during the
final bilinear upscale so the ROI path stays as sharp as plain zoom and panning
remains sub-pixel smooth.

To keep framing locked until meaningful movement (similar to macOS Center
Stage):

- **Comfort zone:** Framing stays locked while the ideal center remains within
  `CS_COMFORT_POS` (20%) of the committed crop's span and the ideal/committed
  zoom ratio stays within `0.78`–`1.30`, ignoring head turns, tilts, and small
  leans. Scaling thresholds to the committed span makes tighter crops
  proportionally more responsive.
- **Breach:** Movement beyond 45% of the span, or a zoom ratio outside
  `0.55`–`1.85`, marks a major change (standing up, walking across frame, or
  people entering/leaving).
- **Confirmation:** Reframing requires `CS_CONFIRM` (30 frames, ~1 s) of
  consecutive out-of-comfort detections for moderate drift, or `CS_FAST_CONFIRM`
  (9 frames, ~0.3 s) for a breach. Returning to the comfort zone before
  confirmation cancels the reframe.

### Reactions

Renders rising emoji particle bursts triggered manually or via hand gestures
when Reactions is enabled. Gesture recognition uses geometric rules over the 21
MediaPipe hand landmarks with no separate classifier model:

- **Thumbs up / thumbs down:** Fingers curled with thumb extended vertically.
- **Two-hand heart:** Thumb and index tips meeting with index above thumb and at
  most one stray lower finger per hand.
- **Wave:** Open palm wave.

A hold-to-confirm debounce ensures a held gesture fires only one burst.

## Inference

Inference runs through ONNX Runtime via the `ort` crate. Models are discovered
by `<dir>/<id>/manifest.toml` in search order:
`~/.local/share/vinoopeneffects/models`, `share/vinoopeneffects/models` under
the binary's prefix, `/usr/local/share`, and `/usr/share`. Each model is
verified against its manifest SHA-256 before loading.

| Model | Input | Used by |
|---|---|---|
| MediaPipe selfie segmentation | 256² | Person mask |
| YuNet | 640² | Center Stage |
| MediaPipe palm detection | 192² letterbox | Reactions, hand masking |
| MediaPipe hand landmarks | 224² crop | Reactions, hand masking |
| IAT low-light (experimental, not fetched) | 256² | Auto light |

### Lazy loading and warm-up

Models load lazily on a background loader thread rather than blocking the
streaming thread. When a frame needs an unloaded model, the worker queues it and
skips that effect until it arrives. The loader thread executes one inference on
a blank frame because OpenVINO compiles for the NPU on the first run rather than
at session load (taking 365 ms with a cold cache). Stale load results are
discarded if a provider swap occurred in between.

At daemon startup, enabled effects' models are also compiled once and dropped so
OpenVINO's compile cache (`~/.cache/vinoopeneffects/openvino`) is already warm
before a consumer connects. A model that fails to load is cached as failed and
logged once instead of retrying every frame.

While waiting for the first person mask, Portrait blur and Background replace
treat the entire frame as background (outputting a fully blurred frame or the
replacement background) rather than exposing the raw room behind you.

### Runtime library loading

With the default `npu` feature, `libonnxruntime.so` is loaded dynamically at
startup from `ORT_DYLIB_PATH`, or the first match in:

1. `~/.local/share/vinoopeneffects/ort`
2. `~/.local/lib/vinoopeneffects/ort`
3. `<prefix>/lib[64]/vinoopeneffects/ort`
4. `/usr/{lib,lib64,local/lib}/vinoopeneffects/ort`

This applies to CPU inference as well: an `npu` build without `libonnxruntime.so`
runs no models. Building with `--no-default-features` links a CPU-only runtime
statically instead.

### Execution provider selection and runtime swaps

- **Precedence:** `VINOOPENEFFECTS_EP` overrides the saved `[inference] provider`
  in `state.toml`.
- **Fallback:** Both `auto` and explicit `npu` fall back to CPU if no NPU is
  available, the runtime lacks the OpenVINO provider, or NPU compilation fails
  (explicit `npu` is a saved speed preference so a driver update never disables
  effects). Fallbacks are always surfaced via `inference_detail` and
  `inference_degraded` whenever an NPU is present but not running all models.
  Only explicit `gpu` is strict and errors rather than falling back to CPU.
- **Compile failure memory:** An NPU compile failure logs once at error level
  and writes `$XDG_STATE_HOME/vinoopeneffects/npu-failure.toml` tagged with the
  current NPU driver and runtime versions. `auto` skips the NPU while that
  record matches; explicit `npu` retries and clears the record on success, and
  `RetryInference` (`vinoopeneffects-ctl inference --retry`) clears it and
  reloads.
- **Live provider swaps:** Changing providers builds a new engine off the frame
  path and reloads every active model on the target provider while frames keep
  flowing on the existing sessions, then swaps engines atomically. On error, the
  existing engine remains active. Swaps clear the accelerator-failed memo first
  so a previous failure does not keep `auto` pinned to CPU.

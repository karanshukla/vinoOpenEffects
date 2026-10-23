# Architecture

How a frame gets from the webcam to Zoom, and why each piece is shaped the way
it is. Code comments stay at one line and point here when the reasoning is
longer than that.

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

Three binaries. Only the daemon touches the camera, the models or PipeWire; the
GUI and CLI are stateless D-Bus clients. That split is enforced by CI: the
`core` job fails if `vinoopeneffects-core` links GStreamer, GTK, libadwaita or
PipeWire, or if either client depends on `core` or the daemon instead of
`shared` alone. A client that linked `core` would drag ONNX Runtime into the GUI.

| Crate | What it holds |
|---|---|
| `shared` | D-Bus names and value helpers, `state.toml` config, legacy-path migration, the `doctor` probes |
| `core` | Pure logic: inference (`ort`), gesture classification, every effect. No media stack, so it tests without GStreamer |
| `daemon` | The D-Bus server, the GStreamer capture pipeline, the `oe_effects` element and the PipeWire provide node |
| `cli`, `gui` | The two clients. `gui/build.rs` generates its D-Bus proxies from `data/dbus/*.xml` with `zbus_xmlgen` |
| `tests/integration` | A `videotestsrc` pipeline test, and D-Bus round trips that need a running daemon (`#[ignore]`d) |

The daemon starts on demand. The D-Bus activation file and the systemd user unit
(`Type=dbus`) both run `vinoopeneffects-daemon --start`, so the first client
call brings it up. Before reading any config it migrates pre-fork `openeffects`
state (see [install.md](install.md#upgrading-from-openeffects)).

## D-Bus API

Bus name `io.github.karanshukla.vinoopeneffects.Daemon`, object
`/io/github/karanshukla/vinoopeneffects/Daemon`, session bus only. The
introspection XML in [`data/dbus/`](../data/dbus) is the contract.

| Interface | Members |
|---|---|
| `Daemon1` | `Start`, `Stop`, `Quit`, `SetInferenceProvider`; properties `Status`, `Capabilities`; signals `StatusChanged`, `InferenceChanged` |
| `Effects1` | `ListEffects`, `SetEnabled`, `SetParam`, `GetParams`, `GetAllState`, `SetBypass`, `GetBypass`, `TriggerReaction`; signals `EffectChanged`, `BypassChanged` |
| `Devices1` | `ListCameras`, `SelectCamera`; properties `VirtualCameraInfo`, `ActiveCamera` |

Effect ids are `center_stage`, `portrait_blur`, `bg_replace`, `studio_light` and
`reactions`. The daemon holds one canonical `AppState` and persists it to
`~/.config/vinoopeneffects/state.toml`; the worker receives the whole state on
every change rather than re-parsing per-key values.

## The camera opens only when someone is watching

```
                         consumer connects                consumer leaves
 provide node  PAUSED ──────────────────────▶ STREAMING ─────────────────▶ PAUSED
 (LED off)                                        │                          │
                                                  ▼                          ▼
 worker                                 build + start capture        300 ms grace, then
                                        pipeline (LED on)            tear down (LED off)
```

At start the daemon advertises a native libpipewire `Video/Source` node and
leaves the real camera closed. Graph laziness alone does not gate a hardware
capture stream, so the provide node's `state_changed` callback is the gate: a
move to `STREAMING` tells the worker to open the camera, and `PAUSED`,
`UNCONNECTED` or an error tells it to close.

The close waits `CAPTURE_RELEASE_GRACE` (300 ms). Browsers go through
xdg-desktop-portal and probe the node with quick connect/disconnect blips,
retrying if the first frames are the warmup placeholder. Opening the camera
takes about 250 ms, so tearing down on every blip would thrash it and never
deliver a stable stream.

If the camera fails while a consumer is streaming (unplugged, or a USB stall),
the worker drops the capture pipeline and clears the `Bridge`, so the consumer
gets black frames instead of the last one frozen, and the status goes to
`error`. Two things catch it: a bus sync handler forwards the pipeline's first
error or EOS (until then nothing read the bus once it was `PLAYING`), and a
watchdog fails a capture that delivered no frame for `CAPTURE_STALL` (5 s),
for stalls that post no error. While the consumer stays, the worker tries the
camera again every `CAPTURE_RETRY` (2 s), quietly, and the status returns to
`running` once it opens. It doesn't retry with no camera at all, since that
would build the `videotestsrc` fallback and swap black for colour bars. A
camera that fails to open when the consumer connects is retried the same way.

An idle daemon takes no wakeups. The worker blocks on one inbox (daemon commands,
provide-node state and capture failures share it) and only uses a timeout while
a grace window, a retry or a streaming capture's watchdog is pending. The frame tick thread parks until a consumer is streaming. Releasing the
camera also calls `malloc_trim` to hand the frame buffers back to the OS.

### The provide node

libpipewire objects are not `Send`, so the node runs on its own thread with its
own `pw_main_loop`. It talks to the rest of the daemon through channels and the
shared `Bridge`, a latest-frame slot.

- **`media.class = Video/Source`.** WirePlumber only offers nodes of that class
  to consumers as cameras.
- **`DRIVER`.** A virtual camera has no hardware clock, and camera consumers
  expect the source to drive the graph. Without it the graph never cycles,
  `process()` never runs, and consumers see a black "no active stream". The node
  clocks itself: a tick thread with absolute deadlines (so a late wakeup does
  not drift the rate) calls `trigger_process()` while the node is driving.
- **`RT_PROCESS`.** Without it the stream processes on the main loop, and
  PipeWire 1.2 and later schedule the node asynchronously (`node.async = true`).
  An async driver flips connecting consumers to async scheduling too, which
  crashes Chromium's video-capture service in a connect/crash/retry loop.
- **`SPA_META_Header`.** WebRTC consumers (Chromium, Firefox) request the header
  meta and dereference it without a null check (`video_capture_pipewire.cc`,
  `h->flags`). The region is only allocated when both ports announce it, so the
  node answers format negotiation with a `Meta(Header)` param alongside its
  buffer requirements, and stamps `seq` and `pts` on every buffer.
- **Format.** One format: I420 at the selected camera's native mode. Before the
  first processed frame the node serves limited-range black, so a new consumer
  sees a valid signal immediately.

Switching to a camera with a different native mode re-advertises the node, since
consumers only renegotiate on reconnect.

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

- **Camera choice.** The configured camera if it exists (an explicit
  `/dev/videoN` is honoured even when `DeviceMonitor` missed it), otherwise
  autodetect. When a device shows up through both the v4l2 and PipeWire device
  providers, the v4l2 entry wins: `v4l2src` on `/dev/videoN` needs no PipeWire
  camera-access grant, while `pipewiresrc` fails with `target not found` without
  one. `pipewiresrc` is only used for PipeWire nodes with no v4l2 path.
- **Native mode.** The source capsfilter pins the camera to the mode the virtual
  camera advertises, raw or MJPEG. Many UVC webcams only do 720p in MJPEG, and
  left to negotiate they pick something like raw 640x480, which `videoscale`
  would then stretch. `decodebin` decodes MJPEG and passes raw through; its src
  pad only appears after negotiation, so it is linked from `pad-added`.
- **Fixed frame rate.** `v4l2src` sets `exposure_dynamic_framerate=0` on open.
  Another app can leave it on, which lets auto-exposure stretch frames in dim
  light, so the feed starts at 30 fps and sags. Chrome resets it for the same
  reason.
- **Start-up errors.** Starting waits up to 5 s for `PLAYING`, so a camera caps
  failure surfaces as an error instead of an async transition that fails later.
- **Latest frame only.** The appsink keeps one buffer and drops the rest,
  matching the `Bridge`'s latest-frame semantics for a live camera.

## The `oe_effects` element

One in-place CPU `GstBaseTransform` hosts every effect. It maps GObject
properties onto `core::effects::Settings`, packs each RGBA frame into a tight
reusable buffer, and hands it to `core::effects::Effects::process`.

When no effect is on and no reaction burst is in flight, it switches itself to
passthrough so GStreamer stops handing it frames. Firing a reaction turns
passthrough off even with every effect off, and the next idle frame turns it
back on. Because idle frames never reach the element, unused models are freed
by a timer thread, not per frame: a session unused for 60 s is dropped and
reloaded on demand.

## Effects

Per frame, in this order:

1. **Gestures**, on the original frame. They must see the whole frame before
   auto light and the center-stage crop rewrite it in place. A confirmed
   gesture queues a reaction.
2. **Denoise**, when Auto light, Studio Light or Center Stage is on. See
   below.
3. **Auto light** (opt-in, NPU only). See below.
4. **Center Stage**, if on. Framing is decided on the full frame, then the
   masked effects run on just the crop ROI, the only pixels the zoom keeps,
   before it is upscaled to the output. Otherwise the masked effects run on the
   full frame.
5. **Reactions**, last, so emoji are never cropped or blurred.

### Denoise

Gain, contrast and zoom all amplify sensor noise, so a temporal denoise runs
first whenever one of them is on. Each pixel is a recursive average with the
previous output. A still pixel keeps 75% of the history (`MAX_HISTORY`).

Whether a pixel is still is judged per 8×8 block (`BLOCK`), from the block's
mean signed luma change against the history. Noise averages out across a block
and real motion doesn't, so the threshold can sit at a couple of levels
(`STILL`, `MOVING`) without treating noise as motion. A per-pixel test would
need a threshold above the noise, and a face drifting a few levels a frame
falls under it and smears. Each block takes the most-moving of its neighbours,
so a still block beside a moving edge doesn't trail into it, and a pixel whose
own change reaches 24 levels (`PIXEL_MOVING`) passes through regardless, for
thin edges a block average would miss. The history resets on a resolution
change, a skipped frame, or the filter going passthrough (no effect on, so
frames stop reaching it and the frame count stalls).

The history keeps 8 fractional bits. Auto light reads that instead of the
rounded frame, so a 6× gain multiplies the averaged value and rounds to u8
once, rather than stretching 8-bit steps into visible bands.

### The person mask

Studio Light, Portrait blur and Background replace share one selfie
segmentation mask, segmented once per frame at most, so enabling all three
costs no extra ONNX work.

- **Cadence.** Segmentation runs every 2nd frame (`SEG_INTERVAL`, about 15 Hz
  at 30 fps), which takes the per-frame ONNX cost off every other frame. The
  one-frame hold is hidden by the temporal EMA and by the guided-filter re-snap
  below, which realigns the edge to the current frame every frame.
- **Temporal EMA.** Each fresh 256² model output is blended
  `e = α·new + (1-α)·e` with `SEG_EMA_ALPHA = 0.5`, which kills boundary flicker
  without smearing the image-aligned edge (that lives downstream). Lower is
  steadier but laggier. A switch between full-frame and ROI coordinates drops
  the EMA history, since the two grids are not comparable.
- **Hands.** Tracked hands are painted into the mask before the EMA, so their
  edges smooth with the body's.
- **Guided filter.** The coefficients are computed at 1/`GF_SUBSAMPLE`² (1/16)
  of the pixels, then applied against full-resolution luma, which carries the
  sharp edge. `GF_EPS = 1e-4` keeps the matte hugging real luma boundaries
  (lower is snappier and noisier). The window radius is the shorter side over
  `GF_RADIUS_DIVISOR` (96): a small radius keeps the linear fit local and the
  matte crisp, where a wide box smears the edge into a halo.
- **Edge.** The filtered alpha is steepened with a smoothstep across
  `0.5 ± GF_EDGE_WIDTH` (0.06), so the boundary is a thin anti-aliased line
  rather than a soft halo. Smaller is harder, larger is softer.

### Portrait blur and Background replace

`out = fg·mask + bg·(1-mask)`. For replace, `bg` is the user's image or colour.
For blur, `bg` is a box blur of the frame weighted by the background mask, so
the subject's own pixels are excluded from it; otherwise the person's colours
bleed outward under the blur and leave a halo. The weighted frame is
downsampled with premultiplied colour and the background weight in alpha, blurred
with two separable box passes (about a triangle kernel, cost independent of
radius), and un-premultiplied when composited back.

### Studio Light

A brightness and contrast lift blended by the mask, so only the subject is lit,
with a separate brightness for the background. Contrast pivots on the subject's
own mean luma, so it never darkens a dim face; below about 1% mask coverage the
mean is noise and the pivot is held.

**Auto light** is an IAT low-light model that runs only on the NPU (about
850 ms per frame on CPU, so it never runs there). Its output is not used
directly, since it over-brightens well-lit frames and shifts colour. Only the
luminance ratio to the input is kept as a low-res gain map, EMA-smoothed and
applied to the full-res frame, and a gate on mean luma fades it out as the
scene gets well lit. Without an NPU or the model it does nothing.

### Center Stage

A YuNet face box drives an EMA-smoothed crop and zoom (`CS_ALPHA = 0.18`, about
12 frames to 90%). The crop is square in normalized space, so the output keeps
the frame's aspect ratio, and never tighter than `CS_MIN_CROP` (a 4× zoom), so a
distant face does not blow up into an unusably pixelated frame. YuNet runs every
4th frame (`FACE_INTERVAL`); the comfort zone absorbs the staleness. With no
face the target is held, so a brief look-away does not drift the crop.

Framing follows the eye midpoint, nudged down a quarter of the face height
toward the face centre for balanced headroom. In `group` mode it frames the
union of every face's head box.

With a masked effect on, the crop is copied out at whole-pixel bounds (plus a
pixel of margin) and the exact fractional crop is applied once, on the bilinear
upscale back to the output. The ROI path is therefore no softer than the plain
zoom, and the EMA pan stays sub-pixel smooth.

The crop moves like macOS Center Stage, locked until the subject really moves:

- **Comfort zone.** While the ideal crop centre stays within `CS_COMFORT_POS`
  (20%) of the committed crop's span, and the ideal/committed zoom ratio stays
  within 0.78–1.30, the framing is locked. Turning, tilting, grinning and
  leaning never move it. Errors are measured against the committed crop's span,
  so a zoomed-in crop is proportionally more sensitive.
- **Breach.** Beyond 45% of the span, or a zoom ratio outside 0.55–1.85, the
  move is drastic: the subject stood up, walked across, or the face count
  changed.
- **Confirmation.** A reframe needs `CS_CONFIRM` (30, about 1 s) consecutive
  out-of-comfort frames for a moderate drift, or `CS_FAST_CONFIRM` (9, about
  0.3 s) for a breach. A move that returns to the comfort zone before confirming
  never reframes.

### Reactions

A burst of reverse-rain emoji particles, triggered manually or, with reactions
on, by a gesture. Gestures are rule-based geometry on the 21 MediaPipe hand
landmarks, with no model of their own: thumbs up and down (fingers curled, thumb
extended and vertical), a two-hand heart (thumb and index tips meeting, index
above thumb, at most one stray lower finger per hand) and an open-palm wave. A
hold-to-confirm debounce turns a held gesture into one reaction.

## Inference

ONNX Runtime through the `ort` crate. Models are found by
`<dir>/<id>/manifest.toml` in `~/.local/share/vinoopeneffects/models`, then
`share/vinoopeneffects/models` under the binary's prefix, `/usr/local/share`
and `/usr/share`, and verified against the manifest's sha256.

| Model | Input | Used by |
|---|---|---|
| MediaPipe selfie segmentation | 256² | the person mask |
| YuNet | 640² | Center Stage |
| MediaPipe palm detection | 192² letterbox | Reactions, hand masking |
| MediaPipe hand landmarks | 224² crop | Reactions, hand masking |
| IAT low-light (experimental, not fetched) | 256² | Auto light |

Models load lazily, but never on the streaming thread: in the daemon a frame
that needs an unloaded model queues it, a loader thread loads it and runs it
once on a blank frame, and the effect skips frames until the model lands. The
blank run matters: OpenVINO compiles for the NPU on a session's first run, not
at load, and that first run took 365 ms with a cold cache. A swap in between discards the
stale result. At start the daemon also compiles the enabled effects' models
once and drops them, so OpenVINO's compile cache
(`~/.cache/vinoopeneffects/openvino`) is warm before a consumer connects. A model
that fails to load is remembered as failed and logged once rather than retried
every frame.

Blur and background replace never pass those frames through untouched: until
the first mask exists they treat the whole frame as background, so it goes out
fully blurred or as only the replacement.

**The runtime is loaded, not linked.** The default `npu` feature loads
`libonnxruntime.so` at start, from `ORT_DYLIB_PATH`, else the first of
`~/.local/share/vinoopeneffects/ort`, `~/.local/lib/vinoopeneffects/ort`,
`<prefix>/lib[64]/vinoopeneffects/ort` and `/usr/{lib,lib64,local/lib}/vinoopeneffects/ort`.
That holds for CPU inference too: an `npu` build without the library runs no
models. `--no-default-features` links a CPU-only runtime statically instead.

**Execution provider.** `VINOOPENEFFECTS_EP`, when set, beats the saved
`[inference] provider`. `auto` uses OpenVINO on the NPU when there is one and
falls back to CPU if there is no NPU, the runtime has no OpenVINO provider, or a
model fails to compile for the NPU. Explicit `npu` falls back the same way: it is
a speed preference saved in `state.toml`, and a driver update shouldn't strip
the effects weeks later. A fallback is never quiet. `inference_detail` says why
and how to fix it, and `inference_degraded` is set whenever an NPU exists but
the models aren't all on it. A compile failure is logged at error level once and
written to `$XDG_STATE_HOME/vinoopeneffects/npu-failure.toml` with the NPU
driver and runtime versions. `auto` skips the NPU while that record matches the
installed versions; an explicit `npu` retries and clears the record on success,
and `RetryInference` (`vinoopeneffects-ctl inference --retry`) clears it and
reloads. Only an explicit `gpu` is strict and fails rather than running on CPU.

**Swapping at runtime.** A provider change builds a whole new engine off the
frame path, with every loaded model reloaded on the new provider, while frames
keep flowing on the old sessions; then it swaps them in at once. On error the
old engine stays. A swap resets the accelerator-failed memo first, so an old
provider's failure does not pin `auto` to CPU.

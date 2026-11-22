# Hardware

Execution provider behavior, webcam quirks, and inference measurements. Every
measurement includes its date and test machine and reflects that setup rather
than a guarantee across all hardware.

## NPU, GPU and CPU

| Provider | How it is selected | Behavior |
|---|---|---|
| **NPU** (Intel, OpenVINO EP) | `auto` when a `/dev/accel/accel*` node bound to `intel_vpu` exists (AMD `amdxdna` nodes are ignored) and the loaded ONNX Runtime includes the OpenVINO provider; or explicitly via `npu` | Runs all models, including Auto light |
| **GPU** (OpenVINO EP) | `VINOOPENEFFECTS_EP=gpu` only (`auto` never picks it, and the saved setting only accepts `auto`, `cpu`, or `npu`) | Same models as the NPU path; not yet benchmarked |
| **CPU** | `auto` when no usable NPU is available, or explicitly via `cpu` | Runs all models except Auto light, which stays disabled |

Provider selection and fallback rules:

- **Configuration:** Change the provider with
  `vinoopeneffects-ctl inference auto|cpu|npu` (saved to `state.toml` and
  swappable while streaming) or override it via `VINOOPENEFFECTS_EP` in the
  daemon's environment. See [architecture.md](architecture.md#inference) for
  internal details.
- **Fallback and degraded state:** Both `auto` and `npu` fall back to CPU
  without interrupting video if the runtime lacks the OpenVINO provider, the
  user cannot open `/dev/accel`, or a model fails to compile for the NPU. When
  an NPU is present but not used, the state is reported as *degraded* (with
  reason and fix) in the daemon log, `vinoopeneffects-ctl status`,
  `vinoopeneffects-ctl inference`, `doctor`, and a GUI banner. Machines without
  an NPU run on CPU normally without being marked degraded.
- **Compile failure cache:** An NPU compile failure is recorded under
  `$XDG_STATE_HOME/vinoopeneffects/`, keyed by the NPU driver and runtime
  versions, so `auto` skips the NPU on later starts until
  `vinoopeneffects-ctl inference --retry` or a driver/runtime upgrade. Explicit
  `npu` always retries.
- **Strict GPU mode:** Only an explicit `gpu` setting returns an error instead
  of falling back if it cannot be satisfied.

**NPU device permissions by distro:** Fedora's udev rules make
`/dev/accel/accel*` world read-write. Debian, Ubuntu, and their derivatives set
them to `root:render 0660` with no seat ACL, so a user outside the `render`
group can see the device node but cannot open it. In that case the daemon
reports a degraded NPU; add your user with `sudo usermod -aG render $USER` and
log out and back in. Other distros follow whichever default their systemd build
uses (`ls -l /dev/accel`).

**ONNX Runtime:** `scripts/install.sh` always installs ONNX Runtime via
[ovfetch](https://github.com/karanshukla/ovfetch), picking the OpenVINO build
matched to your NPU and driver (or the newest build on CPU-only machines). That
build includes the CPU provider as well, so there is no separate CPU-only
runtime. To update an existing install, run
`ovfetch install --prefix ~/.local/lib/vinoopeneffects/ort` (add `--build` for an
OpenVINO newer than Intel's prebuilt wheels). Run `vinoopeneffects-ctl doctor`
to verify which runtime loaded and whether the OpenVINO provider is present.

## Measured inference latency

Measured 2026-10-11 on a Dell XPS 13 (Wildcat Lake NPU, NPU driver 1.35.0,
OpenVINO 2025.4.1). Values are mean milliseconds per call over 100 iterations
after warmup on a synthetic 1280×720 frame with no face or hand present. Timing
includes RGBA resampling and output decoding, not just `session.run`, and
excludes camera capture, GStreamer, and PipeWire.

| Model | CPU (ms) | NPU (ms) | NPU load, first run |
|---|---|---|---|
| Selfie segmentation | 3.4 | 1.4 | 35 ms |
| YuNet | 8.1 | 4.9 | 4 ms |
| Palm detection | 4.9 | 2.1 | 533 ms |
| Hand landmarks | 2.6 | 1.2 | 426 ms |

NPU execution was confirmed on hardware via `npu_busy_time_us`.

### NPU load time and the compile cache

OpenVINO caches compiled models in `~/.cache/vinoopeneffects/openvino` (~6 MB
for the four bundled models). Load times (ms per model) measured 2026-10-15 on
the same XPS 13 (NPU driver 1.35.0, ONNX Runtime 1.24.1 with OpenVINO 2026.4.1)
using `--bench-inference` on the NPU:

| Model | No cache (ms) | Cached (ms) |
|---|---|---|
| Selfie segmentation | 39–88 | 34–71 |
| YuNet | 3–9 | 3–7 |
| Palm detection | 301–354 | 33–40 |
| Hand landmarks | 240–288 | 34–56 |

OpenVINO finishes compiling on a session's first inference call rather than at
load time. Selfie segmentation's first call took 365 ms without cache and 30 ms
with cache, then ~1.3 ms thereafter. The daemon runs that first inference on a
background loader thread and pre-compiles enabled effects' models at startup so
the streaming thread never stalls; until a model finishes loading, its effect
skips frames and video passes through unchanged.

Other effect timings on the same XPS 13:

- **Auto light (IAT model, 2026-10-12):** ~10 ms per 256² frame on the NPU
  versus ~850 ms on CPU, which is why Auto light is NPU-only.
- **Denoise (2026-10-15):** ~3.8 ms median per 1280×720 frame on CPU in a
  release build.
- **Center Stage super-resolution evaluation (2026-10-15):** Intel's
  `single-image-super-resolution-1032` and `-1033` were benchmarked with
  `benchmark_app -hint latency` (OpenVINO 2026.4.1), median ms:

| Model | Shape (in → out) | NPU (ms) | GPU (ms) | CPU (ms) |
|---|---|---|---|---|
| `-1032` (4×) | 480×270 → 1920×1080 (shipped) | 42 | 24 | 101 |
| `-1032` (4×) | 320×180 → 1280×720 | 13.5 | 10 | — |
| `-1033` (3×) | 640×360 → 1920×1080 (shipped) | 65 | — | — |
| `-1033` (3×) | 426×240 → 1278×720 | 29 | — | — |

Both models reshape on the NPU, but each shape requires its own compile with a
fixed scale factor. At 720p output only `-1032` fits alongside the other models,
and its 320×180 input matches Center Stage's maximum 4× zoom, so any lighter zoom
would have to downscale the crop before upscaling. Center Stage therefore uses
bilinear upscaling for now.

### Reproducing

```bash
VINOOPENEFFECTS_EP=cpu vinoopeneffects-daemon --bench-inference [ITERS] [FRAME.png]
VINOOPENEFFECTS_EP=npu vinoopeneffects-daemon --bench-inference [ITERS] [FRAME.png]
scripts/bench-video.sh [CLIP] [SECONDS]   # whole pipeline on a real clip: latency, CPU time, power
```

`bench-video.sh` requires the `npu` build, an OpenVINO ONNX Runtime, and
installed models. Run it on battery to capture power readings; reading package
energy (RAPL) requires root.

## Cameras

- **Native mode matching:** The virtual camera matches the physical camera's
  preferred native mode in either raw or MJPEG, so a UVC webcam that only
  reaches 720p in MJPEG is still treated as native 720p.
- **USB bandwidth limits on raw modes:** Observed 2026-10-13 on the Dell XPS 13
  integrated webcam: raw YUYV at 1080p caps at 5 fps over USB. An app opening
  the physical camera in that mode will lag; select **vinoOpenEffects Virtual
  Camera** instead.
- **Dynamic frame rate:** Some UVC webcams allow auto-exposure to drop the frame
  rate in dim light when `exposure_dynamic_framerate` is enabled, and another
  app can leave it turned on. The daemon resets it to `0` whenever it opens the
  camera. For apps opening the webcam directly, run
  `v4l2-ctl -d /dev/video0 -c exposure_dynamic_framerate=0` (does not persist
  across replug or reboot).
- **IR sensors:** Laptops with Windows Hello cameras expose an IR device node
  alongside the RGB webcam. `vinoopeneffects-ctl doctor` tags each node (`RGB`,
  `IR`, `meta`) and warns if the selected camera is an IR sensor.

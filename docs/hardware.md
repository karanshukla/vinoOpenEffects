# Hardware

What runs where, and what the numbers were on the one machine they were
measured on. Every number here carries its date and machine; treat it as a
snapshot, not a promise.

## NPU, GPU and CPU

| Provider | How it is selected | What happens |
|---|---|---|
| **NPU** (Intel, OpenVINO EP) | `auto` when `/dev/accel/accel*` exists and the loaded onnxruntime has the OpenVINO provider; or `npu` explicitly | Every model runs on it, and Auto light only runs here |
| **GPU** (OpenVINO EP) | `VINOOPENEFFECTS_EP=gpu` only; `auto` never picks it, and the saved setting takes `auto`, `cpu` or `npu` | Same models as the NPU path. Not measured yet |
| **CPU** | `auto` without a usable NPU, or `cpu` explicitly | Every model except Auto light, which stays off |

`auto` and `npu` fall back to CPU when the runtime has no OpenVINO provider or a
model fails to compile for the NPU, and never stop the video over it. When an
NPU exists but isn't used the install is *degraded*: the reason and its fix show
in the daemon log, `vinoopeneffects-ctl status` and `inference`, `doctor`, and a
banner in the GUI. A machine without an NPU just runs on CPU and isn't degraded.
A compile failure is recorded under `$XDG_STATE_HOME/vinoopeneffects/`, keyed
on the NPU driver and runtime versions, so `auto` skips the NPU on later starts
until `vinoopeneffects-ctl inference --retry` or a driver or runtime upgrade.
An explicit `npu` always tries again. Only an explicit `gpu` that can't be
honoured is an error. Set the provider with
`vinoopeneffects-ctl inference auto|cpu|npu` (saved to `state.toml`, swappable
while streaming) or `VINOOPENEFFECTS_EP` in the daemon's environment, which wins.
[architecture.md](architecture.md#inference) has the mechanics.

onnxruntime always comes from [ovfetch](https://github.com/karanshukla/ovfetch),
which `scripts/install.sh` runs: the OpenVINO build matched to the NPU and its
driver, or the newest one without an NPU. It carries the CPU provider too, so
there is no separate CPU runtime. To change an existing install, run
`ovfetch install --prefix ~/.local/lib/vinoopeneffects/ort` (`--build` for an
OpenVINO newer than Intel's prebuilt wheels).
`vinoopeneffects-ctl doctor` reports which runtime loaded and whether it has the
OpenVINO provider.

## Measured inference latency

Measured 2026-10-11 on a Dell XPS 13 (Wildcat Lake NPU, NPU driver 1.35.0,
OpenVINO 2025.4.1). Mean ms per call over 100 iterations after warmup, on a
synthetic 1280×720 frame with no face or hand in it. Includes the RGBA resample
and output decode, not just `session.run`. Inference only: the camera,
GStreamer and PipeWire are not in it.

| Model | CPU | NPU | NPU load (first run) |
|---|---|---|---|
| Selfie segmentation | 3.4 | 1.4 | 35 ms |
| YuNet | 8.1 | 4.9 | 4 ms |
| Palm detection | 4.9 | 2.1 | 533 ms |
| Hand landmarks | 2.6 | 1.2 | 426 ms |

The NPU run was confirmed on hardware through `npu_busy_time_us`.

### NPU load time and the compile cache

OpenVINO's compiled models are cached in `~/.cache/vinoopeneffects/openvino`
(about 6 MB for the four models). Load ms per model, measured 2026-10-15 on the
same XPS 13 (NPU driver 1.35.0, onnxruntime 1.24.1 with OpenVINO 2026.4.1),
`--bench-inference` on the NPU:

| Model | No cache | Cached |
|---|---|---|
| Selfie segmentation | 39–88 | 34–71 |
| YuNet | 3–9 | 3–7 |
| Palm detection | 301–354 | 33–40 |
| Hand landmarks | 240–288 | 34–56 |

Load is only half of it: OpenVINO compiles on a session's first run. Selfie
segmentation's first inference took 365 ms with no cache and 30 ms cached, then
about 1.3 ms. The daemon does that first run on a loader thread, and compiles
the enabled effects' models at start too, so the streaming thread never
compiles: until a model is ready its effect skips frames and the video passes
through unchanged.

Auto light's IAT model, measured 2026-10-12 on the same machine: about 10 ms per
256² frame on the NPU and about 850 ms on CPU, which is why it never runs on CPU.

Denoise, measured 2026-10-15 on the same machine: about 2.1 ms median per
1280×720 frame on CPU, release build.

Intel's `single-image-super-resolution-1032` and `-1033`, tried as an NPU
upscale for Center Stage, measured 2026-10-15 on the same machine with
`benchmark_app -d NPU -hint latency` (OpenVINO 2026.4.1): 42 ms and 65 ms
median at their fixed 640×360 input. Each is over the 33 ms frame budget on its
own, so Center Stage upscales bilinearly.

### Reproducing

```bash
VINOOPENEFFECTS_EP=cpu vinoopeneffects-daemon --bench-inference [ITERS] [FRAME.png]
VINOOPENEFFECTS_EP=npu vinoopeneffects-daemon --bench-inference [ITERS] [FRAME.png]
scripts/bench-video.sh [CLIP] [SECONDS]   # whole pipeline on a real clip: latency, CPU time, power
```

`bench-video.sh` needs the `npu` build, an OpenVINO onnxruntime and installed
models; it reads battery power, so run it unplugged, and package energy (RAPL)
needs root.

## Cameras

- **The virtual camera matches the real camera's native mode.** The capture
  pipeline pins the camera to its preferred mode in raw or MJPEG, so a UVC
  webcam that only does 720p in MJPEG still counts as native 720p.
- **USB bandwidth caps raw modes.** Observed 2026-10-13 on the Dell XPS 13
  integrated webcam: raw YUYV at 1080p is capped at 5 fps over USB. An app that
  opens the real camera in that mode lags; pick the vinoOpenEffects Virtual
  Camera instead.
- **Dynamic frame rate.** Some UVC cameras let auto-exposure lower the frame
  rate in dim light when `exposure_dynamic_framerate` is on, and another app can
  leave it on. The daemon turns it off whenever it opens the camera. For other
  apps that open the camera directly:
  `v4l2-ctl -d /dev/video0 -c exposure_dynamic_framerate=0` (not persistent
  across replug or reboot).
- **IR sensors.** Laptops with Windows Hello cameras expose an IR node next to
  the RGB one. `doctor` classifies each node (`RGB`, `IR`, `meta`) and warns if
  the selected camera is IR.

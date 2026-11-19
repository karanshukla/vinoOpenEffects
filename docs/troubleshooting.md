# Troubleshooting

Start by running `doctor`. It inspects the daemon, inference runtime, models,
cameras, PipeWire, GUI dependencies, and configuration, and prints suggested
fixes alongside any failures:

```bash
vinoopeneffects-ctl doctor          # human-readable; exits 1 if any check fails
vinoopeneffects-ctl doctor --json   # machine-readable output for bug reports or scripts
```

`doctor` works even when the daemon is stopped. Each check prints as a
`group: name` line:

```
[ok  ] daemon: dbus                 reachable (daemon reports idle)
[ok  ] inference: onnxruntime       ~/.local/lib/vinoopeneffects/ort/libonnxruntime.so (OpenVINO EP available)
[ok  ] camera: selected             /dev/video0 (Integrated_Webcam_FHD: Integrat) is RGB
...
0 failed, 0 warning(s)
```

Next, check the daemon's journal log:

```bash
journalctl --user -u vinoopeneffects-daemon -e
```

For verbose logs, set `RUST_LOG` (defaults to `info`). For example, stop the
service with `systemctl --user stop vinoopeneffects-daemon` and run
`RUST_LOG=debug vinoopeneffects-daemon --start` in a terminal.

## What each failing check means

| Check | Likely cause | Fix |
|---|---|---|
| `daemon: dbus` | The daemon is not running and cannot be activated (missing activation file, broken `Exec=` path, failed unit, or stale D-Bus config) | Follow the printed fix, or run `systemctl --user start vinoopeneffects-daemon` |
| `daemon: activation` | Missing D-Bus activation file, so clients cannot start the daemon on demand | Reinstall or re-run `scripts/install.sh` |
| `daemon: systemd` | The systemd user unit is missing or failed | Check `journalctl --user -u vinoopeneffects-daemon -e` |
| `inference: build` | Built with `--no-default-features` (CPU-only) | Rebuild with default features for NPU support |
| `inference: onnxruntime` | `libonnxruntime.so` is missing or lacks the OpenVINO provider (no models run without it, including on CPU) | Run `ovfetch install --prefix ~/.local/lib/vinoopeneffects/ort`, or set `ORT_DYLIB_PATH` |
| `inference: npu` | No `/dev/accel/accel*` bound to `intel_vpu` (no Intel NPU—other vendors' NPUs such as AMD's are not used—or driver not loaded). A warning means the node exists without open permission (Debian/Ubuntu restrict it to the `render` group) | Normal on systems without an Intel NPU (`auto` uses CPU). For the permission warning: `sudo usermod -aG render $USER`, then log out and back in |
| `inference: npu-failure` | A model failed to compile for the NPU on this driver/runtime pair, so `auto` stays on CPU | Check `ovfetch detect`, then run `vinoopeneffects-ctl inference --retry` |
| `models: <id>` | Model file is missing or failed SHA-256 verification against its manifest | Run `scripts/fetch-models.sh` |
| `camera: devices` | No V4L2 or PipeWire camera found | Connect a webcam and check `ls /dev/video*` |
| `camera: selected` | Configured camera is missing or is an IR sensor | Run `vinoopeneffects-ctl camera list`, then `vinoopeneffects-ctl camera select /dev/videoN` |
| `pipeline: pipewire` | PipeWire is not running | Run `systemctl --user start pipewire.service` |
| `pipeline: virtual camera` | Informational: the PipeWire node only exists while the daemon is armed | Run `vinoopeneffects-ctl start` |
| `gui: sink` | `gtk4paintablesink` is missing, so the GUI preview cannot display video | Install the package `doctor` suggests for your distro |
| `gui: pipewiresrc` | PipeWire's GStreamer plugin is missing, so the GUI preview cannot read the virtual camera (packaged separately from PipeWire on Arch and openSUSE) | Install the package `doctor` suggests for your distro |
| `gui: libraries` | GTK 4 or libadwaita is too old (the GUI requires libadwaita 1.7) | Upgrade your packages, or use the CLI |
| `config: state.toml` | Syntax error or unrecognized keys (unparseable files are moved to `state.toml.bad-<time>` and replaced with defaults) | Fix or delete `state.toml`. To recover old settings, fix the `.bad-` file, stop the daemon, and rename it back to `state.toml` |

`models: iat_lowlight` is optional and not downloaded by `fetch-models.sh`; see
[hardware.md](hardware.md).

## Common symptoms

### An app does not list "vinoOpenEffects Virtual Camera"

The virtual camera node is only advertised once the daemon is armed via
`vinoopeneffects-ctl start` (any `ctl` command targeting the daemon starts it).
Verify the node exists with `pw-cli ls Node | grep vinoopeneffects`. If you
upgraded from `openeffects`, re-select the camera in each app because its name
changed.

Because the virtual camera is a PipeWire node rather than a `/dev/video` device,
only apps that support PipeWire cameras can see it:

- **Chromium-based browsers (Chrome, Edge, Brave):** Enable
  **PipeWire Camera support** in `chrome://flags` and restart the browser.
- **V4L2-only apps:** Apps that only open `/dev/video*` devices directly cannot
  see PipeWire virtual cameras.

### The camera LED stays off and apps show black

The physical webcam only opens while a consumer is actively streaming from the
virtual camera. If it never opens when a consumer connects, check `doctor`'s
`camera: selected` line and the daemon journal to see which device failed and
why.

### The GUI preview says "PipeWire can't find the virtual camera"

WirePlumber lost track of the virtual camera node. Click **Restart Camera
Services** in the preview, or run
`systemctl --user restart wireplumber vinoopeneffects-daemon`.

### The video feed lags or frame rate drops in dim light

See [hardware.md](hardware.md#cameras): an app may be opening the physical
webcam in raw 1080p (USB bandwidth capped), or another app may have left
`exposure_dynamic_framerate` enabled.

### Effects are enabled but nothing changes

No ML model is running. Check the `inference: onnxruntime` and `models:` lines
in `doctor` output. Missing models are logged once in the daemon journal rather
than on every frame.

### The NPU is not being used

Run `vinoopeneffects-ctl inference` to view the requested and active providers,
whether the install is degraded, and why. After a failed NPU compile, `auto`
stays on CPU across restarts until the driver or runtime changes. Inspect the
driver and compiler with `ovfetch detect`, then retry with
`vinoopeneffects-ctl inference --retry` (or **Retry NPU** on the GUI banner).

### Clients cannot reach a freshly installed daemon

The D-Bus session bus only loads new activation files after a reload. Run:

```bash
busctl --user call org.freedesktop.DBus /org/freedesktop/DBus org.freedesktop.DBus ReloadConfig
```

Or log out and back in.

## Reporting a bug

Open a GitHub issue containing `vinoopeneffects-ctl doctor --json`, your distro,
the video app consuming the camera, and relevant lines from
`journalctl --user -u vinoopeneffects-daemon`. For security issues, follow
[SECURITY.md](../SECURITY.md) instead.

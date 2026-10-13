# Troubleshooting

Start with `doctor`. It checks the daemon, the inference runtime, the models,
the cameras, PipeWire, the GUI's dependencies and the config in one go, and
prints a fix next to anything that fails:

```bash
vinoopeneffects-ctl doctor          # human-readable; exits 1 if any check fails
vinoopeneffects-ctl doctor --json   # the same, for a bug report or a script
```

`doctor` does not need the daemon running. Every check is a `group: name` line:

```
[ok  ] daemon: dbus                 reachable (daemon reports idle)
[ok  ] inference: onnxruntime       ~/.local/lib/vinoopeneffects/ort/libonnxruntime.so (OpenVINO EP available)
[ok  ] camera: selected             /dev/video0 (Integrated_Webcam_FHD: Integrat) is RGB
...
0 failed, 0 warning(s)
```

Then the daemon's log:

```bash
journalctl --user -u vinoopeneffects-daemon -e
```

For more detail, set `RUST_LOG` in the daemon's environment (it defaults to
`info`), for example `RUST_LOG=debug vinoopeneffects-daemon --start` after
`systemctl --user stop vinoopeneffects-daemon`.

## What each failing check means

| Check | Usually means | Fix |
|---|---|---|
| `daemon: dbus` | The daemon isn't running and can't be activated. The line says why: no activation file, a broken `Exec=` path, a failed unit, or a bus that hasn't reloaded | Follow the printed fix; `systemctl --user start vinoopeneffects-daemon` |
| `daemon: activation` | No D-Bus activation file, so clients can't start the daemon on demand | Reinstall, or run `scripts/install.sh` again |
| `daemon: systemd` | The user unit is missing or failed | `journalctl --user -u vinoopeneffects-daemon -e` |
| `inference: build` | A `--no-default-features` (CPU-only) build | Rebuild with default features for the NPU |
| `inference: onnxruntime` | No `libonnxruntime.so` found, or it has no OpenVINO provider. Without it no model runs, CPU included | `ovfetch install --prefix ~/.local/lib/vinoopeneffects/ort`, or set `ORT_DYLIB_PATH` |
| `inference: npu` | No `/dev/accel/accel*`: no NPU, or the `intel_vpu` driver isn't loaded | Expected on machines without an NPU; `auto` uses CPU |
| `models: <id>` | A model is missing or its sha256 doesn't match the manifest | `scripts/fetch-models.sh` |
| `camera: devices` | No camera found | Connect one; check `ls /dev/video*` |
| `camera: selected` | The configured camera is gone, or it's an IR sensor | `vinoopeneffects-ctl camera list`, then `vinoopeneffects-ctl camera /dev/videoN` |
| `pipeline: pipewire` | PipeWire isn't running | `systemctl --user start pipewire.service` |
| `pipeline: virtual camera` | Informational: the node only exists while the daemon is armed | `vinoopeneffects-ctl start` |
| `gui: sink` | `gtk4paintablesink` is missing, so the GUI preview can't show video | Install the package `doctor` names for your distro |
| `gui: libraries` | GTK 4 or libadwaita too old (the GUI needs libadwaita 1.7) | Upgrade, or use the CLI |
| `config: state.toml` | A syntax error or unknown keys | Fix the file, or delete it to start from defaults |

`models: iat_lowlight` is optional and not fetched by `fetch-models.sh`; see
[hardware.md](hardware.md).

## Symptoms

**An app doesn't list "vinoOpenEffects Virtual Camera".** The node is
advertised once the daemon is armed: `vinoopeneffects-ctl start` (any `ctl`
command that talks to the daemon starts it). Check
`pw-cli ls Node | grep vinoopeneffects`. After upgrading from `openeffects` the
device name changed, so re-select it in each app.

**The camera LED stays off and apps show black.** The real camera opens only when
a consumer starts streaming. If it never does, `doctor`'s `camera: selected`
line and the journal say which device failed and why.

**The GUI preview says "PipeWire can't find the virtual camera".** WirePlumber
lost track of the node. The preview's **Restart camera services** button runs
`systemctl --user restart wireplumber vinoopeneffects-daemon`.

**The feed is laggy or the frame rate sags in dim light.** See
[hardware.md](hardware.md#cameras): an app opening the real camera in raw
1080p, or `exposure_dynamic_framerate` left on by another app.

**Effects are on but nothing changes.** No model ran. Check `inference:
onnxruntime` and the `models:` lines; a missing model is logged once in the
journal, not every frame.

**The NPU isn't used.** `vinoopeneffects-ctl inference` shows the requested and
active provider and why they differ. After a failed NPU compile `auto` stays on
CPU until the provider is switched; `vinoopeneffects-ctl inference npu` reports
the error instead of falling back.

**Clients can't reach a daemon that was just installed.** The session bus reads
new activation files only on reload:
`busctl --user call org.freedesktop.DBus /org/freedesktop/DBus org.freedesktop.DBus ReloadConfig`,
or log out and back in.

## Reporting a bug

Open an issue with `vinoopeneffects-ctl doctor --json`, your distro, the app
consuming the camera, and the relevant part of
`journalctl --user -u vinoopeneffects-daemon`. For a security problem, see
[SECURITY.md](../SECURITY.md) instead.

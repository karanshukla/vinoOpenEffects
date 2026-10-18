# Install

vinoOpenEffects installs from source only for now. There are no `.deb`, `.rpm`
or Arch packages: shipping the right onnxruntime (CPU or OpenVINO) per machine
is easier from a checkout, and packaging will come back later.

## Requirements

- Wayland, PipeWire and WirePlumber
- Rust 1.92 or newer, and the GStreamer, GTK 4, libadwaita and PipeWire
  development packages ([development.md](development.md#environments) lists
  them for Fedora and Debian/Ubuntu)
- For the GUI: libadwaita 1.7 or newer, and `gtk4paintablesink` for the live
  preview (`gstreamer1-plugin-gtk4` on Fedora, `gstreamer1.0-gtk4` on
  Debian/Ubuntu, `gst-plugin-gtk4` on Arch). Use `--no-gui` without them.
- [ovfetch](https://github.com/karanshukla/ovfetch) 0.5.0 or newer
  (`cargo install ovfetch --locked`), on every machine: it is the only source of
  onnxruntime
- x86_64 has been tested; nothing else has

## Installing

```bash
scripts/install.sh                          # into ~/.local, no root
scripts/install.sh --no-gui                 # daemon and CLI only
scripts/install.sh --no-models              # don't touch models at all
scripts/install.sh --build-runtime          # let ovfetch compile a newer OpenVINO build
sudo scripts/install.sh --prefix /usr/local # system-wide
```

It builds a release and installs:

| What | Where (under the prefix) |
|---|---|
| `vinoopeneffects-daemon`, `vinoopeneffects-ctl`, `vinoopeneffects` | `bin/` |
| D-Bus interfaces and activation file | `share/dbus-1/interfaces/`, `share/dbus-1/services/` |
| systemd user unit | `share/systemd/user/vinoopeneffects-daemon.service` |
| Desktop entry and icons (GUI) | `share/applications/`, `share/icons/hicolor/` |
| onnxruntime | `lib/vinoopeneffects/ort/`, installed by `ovfetch` (see below) |
| Models | `~/.local/share/vinoopeneffects/models/`, sha256-verified against the manifests; ones already present and verified are skipped |

**ovfetch decides the runtime.** `install.sh` stops before building if ovfetch
is missing or older than 0.5.0, then runs `ovfetch install` before installing
anything else. ovfetch picks the OpenVINO build of onnxruntime that fits the
machine's NPU and driver, or the newest one on a machine without an NPU; that
build also carries the CPU provider, so the same runtime serves CPU inference.

If ovfetch doesn't install (for example, the prefix holds a newer build it won't
downgrade, such as one compiled with `ovfetch install --build`, or the download
fails) but the prefix passes `ovfetch verify`, the install continues with that
runtime and a warning. With nothing verified to keep, it stops with ovfetch's
reason. `--build-runtime` passes `--build`, so ovfetch resolves the compiled
build itself (an unchanged one is not recompiled).

Then it asks the session bus to reload, so the daemon is D-Bus-activatable
straight away, and warns if `bin/` isn't on `PATH`. It does not restart a daemon
that is already running: `systemctl --user restart vinoopeneffects-daemon`.

The daemon starts on demand, the first time a client talks to it. To start it
with the desktop session instead, see `vinoopeneffects-ctl autostart` in
[cli.md](cli.md#autostart).

## Files at runtime

- `~/.config/vinoopeneffects/state.toml`: effect settings, the selected camera
  and the inference provider, written by the daemon
- `~/.config/vinoopeneffects/gui.toml`: GUI-only preferences
- `~/.local/share/vinoopeneffects/backgrounds/`: the preset background images,
  extracted from the GUI binary on first launch
- `~/.local/share/vinoopeneffects/models/`: the models (the daemon also looks in
  `/usr/share/vinoopeneffects/models`)

## Uninstalling

```bash
scripts/uninstall.sh                        # from ~/.local
sudo scripts/uninstall.sh --prefix /usr/local
scripts/uninstall.sh --purge                # also models, config and backgrounds
```

It disables and stops the user unit first, then removes everything
`install.sh` put under the prefix, including the onnxruntime.

## Upgrading from openeffects

vinoOpenEffects is a hard fork of `openeffects`, and every installed name
changed:

| Old | New |
|---|---|
| `openeffectsd`, `openeffectsctl`, `openeffects` | `vinoopeneffects-daemon`, `vinoopeneffects-ctl`, `vinoopeneffects` (no `openeffectsctl` alias) |
| `openeffectsd.service` | `vinoopeneffects-daemon.service` |
| `in.co.funinkina.Daemon.service` (D-Bus) | `io.github.karanshukla.vinoopeneffects.Daemon.service` |
| `OE_EP`, `OE_MODELS_DEST` | `VINOOPENEFFECTS_EP`, `VINOOPENEFFECTS_MODELS_DEST` |
| camera "OpenEffects Virtual Camera" | "vinoOpenEffects Virtual Camera" |

On first start the daemon and the GUI move your per-user state to the new names,
logging each move:

- `~/.config/openeffects/` to `~/.config/vinoopeneffects/`
- `~/.local/share/openeffects/` (models, backgrounds) to
  `~/.local/share/vinoopeneffects/`, rewriting the background paths stored in
  `state.toml`
- `~/.config/systemd/user/openeffectsd.service.d/` to
  `~/.config/systemd/user/vinoopeneffects-daemon.service.d/`, with `OE_EP` and
  `OE_MODELS_DEST` in its `*.conf` files renamed
- enablement links `*.wants/openeffectsd.service`, re-created for the new unit

An item whose destination already exists is skipped with a warning; old and new
are never merged. A user-installed `~/.config/systemd/user/openeffectsd.service`
is left alone with a warning. Run `systemctl --user daemon-reload` afterwards.

The old binaries, unit and D-Bus files are not removed: uninstall `openeffects`
the way you installed it. Then update scripts, keybinds and Waybar modules that
call `openeffectsctl`, and re-select the camera in each app, since its name
changed.

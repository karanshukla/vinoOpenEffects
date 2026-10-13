<p align="center">
  <img src="https://raw.githubusercontent.com/karanshukla/vinoOpenEffects/main/docs/assets/vinoopeneffects.svg" width="112" alt="The vinoOpenEffects mark: a dark viewfinder with a green silhouette over blurred bars and crop brackets">
</p>

# vinoOpenEffects

[![CI](https://github.com/karanshukla/vinoOpenEffects/actions/workflows/ci.yml/badge.svg)](https://github.com/karanshukla/vinoOpenEffects/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-GPL--3.0-blue)](https://github.com/karanshukla/vinoOpenEffects/blob/main/LICENSE)
[![Language](https://img.shields.io/badge/language-Rust-orange?logo=rust&logoColor=white)](https://www.rust-lang.org/)

Webcam effects for the Linux desktop, run locally on your NPU or CPU. vinoOpenEffects
publishes a **vinoOpenEffects Virtual Camera** in PipeWire that any app can pick
(browsers, Zoom, OBS), with:

- **Portrait blur** and **Background replace** (a colour or your own image)
- **Studio Light**: lifts you, not the room behind you
- **Center Stage**: crops and zooms to keep your face, or a group, in frame
- **Reactions**: emoji bursts from a button, the CLI, or a thumbs up, thumbs
  down, heart or wave

Nothing leaves the machine. The real camera only opens while an app is watching
the virtual one. Wayland only.

## Quick start

From source, into `~/.local` with no root (requirements are in
[docs/install.md](docs/install.md)):

```bash
scripts/install.sh            # add --no-gui for the daemon and CLI only
```

It fetches the sha256-verified models and an onnxruntime, using the OpenVINO
build when it finds an NPU. Then open **vinoOpenEffects** from your app menu, or:

The GUI needs GTK 4 and libadwaita 1.7 or newer.

### Upgrading from openeffects

This project is a hard fork of `openeffects` and everything was renamed. Installing the new packages removes the old ones and their leftover files (old unit, D-Bus service, desktop entry, icon, binaries).

| Before | Now |
|---|---|
| packages `openeffectsd`, `openeffectsctl`, `openeffects` | `vinoopeneffects-daemon`, `vinoopeneffects-ctl`, `vinoopeneffects` |
| `systemctl --user ... openeffectsd` | `systemctl --user ... vinoopeneffects-daemon` |
| `OE_EP` | `VINOOPENEFFECTS_EP` |
| camera picker entry "OpenEffects Virtual Camera" | "vinoOpenEffects Virtual Camera" |
| `openeffectsctl` | `vinoopeneffects-ctl` (there is no `openeffectsctl` alias) |

Your config and downloaded models are migrated automatically on first start of the new daemon/GUI. If you set `OE_EP` in a systemd drop-in, the migration rewrites it to `VINOOPENEFFECTS_EP`. Update any scripts, keybinds or Waybar modules that call `openeffectsctl`, and re-select the camera in apps (Zoom, OBS, browsers) since the device name changed.

## Usage

Start the daemon (usually handled automatically by the systemd user service):
```bash
systemctl --user start vinoopeneffects-daemon
```

### CLI Examples

Control effects via `vinoopeneffects-ctl` (see the full [CLI reference](docs/cli.md)):
```bash
vinoopeneffects-ctl start                          # advertise the virtual camera
vinoopeneffects-ctl portrait-blur --on --strength 70
vinoopeneffects-ctl doctor                         # check the whole install
```

and pick "vinoOpenEffects Virtual Camera" in your video app.

There are no distro packages for now; source is the supported install.
[docs/install.md](docs/install.md) covers options, uninstalling and upgrading
from `openeffects`.

## Documentation

| | |
|---|---|
| [docs/install.md](docs/install.md) | Requirements, install options, files, uninstalling, upgrading from `openeffects` |
| [docs/cli.md](docs/cli.md) | Every `vinoopeneffects-ctl` command, Waybar and keybind examples |
| [docs/troubleshooting.md](docs/troubleshooting.md) | Starting from `doctor`'s output |
| [docs/hardware.md](docs/hardware.md) | NPU, GPU and CPU behaviour, measured latency, camera quirks |
| [docs/architecture.md](docs/architecture.md) | The pipeline, the D-Bus API, how each effect works |
| [docs/development.md](docs/development.md) | Building, tests, CI, releases |
| [CONTRIBUTING.md](CONTRIBUTING.md) | What helps most, and the one command before you push |
| [SECURITY.md](SECURITY.md) | Threat model, and how to report a vulnerability |
| [CHANGELOG.md](CHANGELOG.md) | What changed |

## Credits & License

vinoOpenEffects is a hard fork of [funinkina/openeffects](https://github.com/funinkina/openeffects) by Aryan, and keeps its GPL-3.0-or-later license. See [NOTICE](NOTICE) for attribution and [LICENSE](LICENSE) for the license text.

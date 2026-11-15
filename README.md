<p align="center">
  <img src="https://raw.githubusercontent.com/karanshukla/vinoOpenEffects/main/docs/assets/vinoopeneffects.svg" width="112" alt="The vinoOpenEffects mark: a dark viewfinder with a green silhouette over blurred bars and crop brackets">
</p>

# vinoOpenEffects

[![CI](https://github.com/karanshukla/vinoOpenEffects/actions/workflows/ci.yml/badge.svg)](https://github.com/karanshukla/vinoOpenEffects/actions/workflows/ci.yml)
[![Coverage](https://coveralls.io/repos/github/karanshukla/vinoOpenEffects/badge.svg?branch=main)](https://coveralls.io/github/karanshukla/vinoOpenEffects?branch=main)
[![License](https://img.shields.io/badge/license-GPL--3.0-blue)](https://github.com/karanshukla/vinoOpenEffects/blob/main/LICENSE)
[![Language](https://img.shields.io/badge/language-Rust-orange?logo=rust&logoColor=white)](https://www.rust-lang.org/)

Webcam effects for the Linux desktop, run locally on your NPU or CPU. It publishes
a **vinoOpenEffects Virtual Camera** in PipeWire that any app can pick (browsers,
Zoom, OBS).

- **Portrait blur** and **Background replace** (a colour or your own image)
- **Studio Light**: lifts you, not the room behind you
- **Center Stage**: crops and zooms to keep your face, or a group, in frame
- **Reactions**: emoji bursts from a button, the CLI, or a thumbs up, thumbs
  down, heart or wave

Nothing leaves the machine, and the real camera only opens while an app is
watching the virtual one. Forked from OpenEffects to add NPU support, for better
performance, lower battery drain and improved effects.

## Requirements

- Wayland, PipeWire and WirePlumber (x86_64 is the only architecture tested)
- libadwaita 1.7 or newer for the window (`--no-gui` skips it)
- Python 3.11 or newer and `curl`, to fetch the models

The full list, per distro, is in [docs/install.md](docs/install.md).

## Quick start

The latest release's prebuilt binaries, into `~/.local` with no root:

```bash
curl -fsSL https://github.com/karanshukla/vinoOpenEffects/releases/latest/download/install.sh | bash
```

The installer fetches the sha256-verified models and has
[ovfetch](https://github.com/karanshukla/ovfetch) install the onnxruntime that
fits your NPU (or CPU). Add `-s -- --no-gui` after `bash` for the daemon and CLI
only. From a checkout, `scripts/install.sh` builds from source instead. There
are no distro packages yet.

To upgrade, use **Check for Updates** in the window's menu or run
`vinoopeneffects-upgrade`.

## Usage

Open **vinoOpenEffects** from your app menu, or use the CLI:

```bash
vinoopeneffects-ctl start                          # advertise the virtual camera
vinoopeneffects-ctl portrait-blur --on --strength 70
vinoopeneffects-ctl doctor                         # check the whole install
```

Then pick "vinoOpenEffects Virtual Camera" in your video app.
[docs/cli.md](docs/cli.md) has every command.

## Documentation

| | |
|---|---|
| [docs/install.md](docs/install.md) | Requirements, distros, install options, files, uninstalling, upgrading from `openeffects` |
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

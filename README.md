<p align="center">
  <img src="https://raw.githubusercontent.com/karanshukla/vinoOpenEffects/main/docs/assets/vinoopeneffects.svg" width="112" alt="The vinoOpenEffects mark: a dark viewfinder with a green silhouette over blurred bars and crop brackets">
</p>

# vinoOpenEffects

[![CI](https://github.com/karanshukla/vinoOpenEffects/actions/workflows/ci.yml/badge.svg)](https://github.com/karanshukla/vinoOpenEffects/actions/workflows/ci.yml)
[![Coverage](https://coveralls.io/repos/github/karanshukla/vinoOpenEffects/badge.svg?branch=main)](https://coveralls.io/github/karanshukla/vinoOpenEffects?branch=main)
[![License](https://img.shields.io/badge/license-GPL--3.0-blue)](https://github.com/karanshukla/vinoOpenEffects/blob/main/LICENSE)
[![Language](https://img.shields.io/badge/language-Rust-orange?logo=rust&logoColor=white)](https://www.rust-lang.org/)

Webcam effects for the Linux desktop running locally on your Intel NPU or CPU.
It exposes a **vinoOpenEffects Virtual Camera** in PipeWire that any video app
can use (browsers, Zoom, OBS).

- **Portrait blur** and **Background replace** (solid color or custom image)
- **Studio Light:** brightens and adds contrast to the foreground subject
  independently of the background
- **Center Stage:** automatically crops and zooms to keep one face or a group in
  frame
- **Reactions:** emoji bursts triggered from the GUI, CLI, or hand gestures
  (thumbs up, thumbs down, heart, or wave)

All processing runs locally, and the physical webcam only opens while an app is
actively consuming the virtual camera. Forked from OpenEffects to add Intel NPU
support for lower CPU usage, reduced battery drain, and improved effects.

## Requirements

- Wayland, PipeWire, and WirePlumber (`x86_64` is the only tested architecture)
- libadwaita 1.7 or newer for the GUI (`--no-gui` installs the daemon and CLI
  only)
- Python 3.11 or newer and `curl` to download the models

See [docs/install.md](docs/install.md) for full distro-by-distro requirements.

## Quick start

Install the latest prebuilt release binaries into `~/.local` without root:

```bash
curl -fsSL https://github.com/karanshukla/vinoOpenEffects/releases/latest/download/install.sh | bash
```

The installer downloads the SHA-256-verified models and uses
[ovfetch](https://github.com/karanshukla/ovfetch) to install the ONNX Runtime
build suited to your NPU (or CPU). Pass `-s -- --no-gui` after `bash` to install
only the daemon and CLI. From a source checkout, run `scripts/install.sh` to
build from source instead. Distro packages are not yet available.

To upgrade later, choose **Check for Updates** in the window menu or run
`vinoopeneffects-upgrade`.

## Usage

Launch **vinoOpenEffects** from your application menu, or control it from the
CLI:

```bash
vinoopeneffects-ctl start                          # advertise the virtual camera
vinoopeneffects-ctl portrait-blur --on --strength 70
vinoopeneffects-ctl doctor                         # verify the installation
```

Then select **vinoOpenEffects Virtual Camera** in your video app. See
[docs/cli.md](docs/cli.md) for the complete command reference.

## Documentation

| Document | Contents |
|---|---|
| [docs/install.md](docs/install.md) | Requirements, distro support, install options, runtime files, uninstalling, and upgrading from `openeffects` |
| [docs/cli.md](docs/cli.md) | Full `vinoopeneffects-ctl` reference, Waybar integration, and keybind examples |
| [docs/troubleshooting.md](docs/troubleshooting.md) | Diagnosing issues with `doctor` and common fixes |
| [docs/hardware.md](docs/hardware.md) | NPU, GPU, and CPU behavior, latency measurements, and webcam quirks |
| [docs/architecture.md](docs/architecture.md) | Process model, D-Bus API, capture pipeline, and effect internals |
| [docs/development.md](docs/development.md) | Build dependencies, tests, CI jobs, and release workflow |
| [CONTRIBUTING.md](CONTRIBUTING.md) | Hardware testing priorities and pre-push checks |
| [SECURITY.md](SECURITY.md) | Privacy guarantees, threat model, and vulnerability reporting |
| [CHANGELOG.md](CHANGELOG.md) | Release history |

## Credits & License

vinoOpenEffects is a hard fork of [funinkina/openeffects](https://github.com/funinkina/openeffects) by Aryan, and keeps its GPL-3.0-or-later license. See [NOTICE](NOTICE) for attribution and [LICENSE](LICENSE) for the license text.

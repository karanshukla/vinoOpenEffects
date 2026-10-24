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
- Python 3.11 or newer and `curl`, for fetching the models (or `--no-models`)
- x86_64 has been tested; nothing else has

## Distros

Developed on Fedora 45 KDE. Others are expected to work from the package
lists in [development.md](development.md#environments); "not tried" means
nobody has reported back yet, not that something is known to break.

| Distro | Status | Notes |
|---|---|---|
| Fedora 42+ (Workstation, KDE) | Primary target | |
| Fedora Atomic (Silverblue, Kinoite, Bazzite) | Not tried | Build in a toolbox, see below |
| Ubuntu 26.04, Debian 13 | CI builds and tests on Ubuntu 26.04 | NPU users need the `render` group ([hardware.md](hardware.md)) |
| Ubuntu 24.04, Debian 12 | Daemon and CLI only | libadwaita 1.5/1.2 is too old for the GUI: `--no-gui`. Their packaged Rust is older than 1.92, so use rustup |
| Arch | Not tried | `gst-plugin-pipewire` isn't pulled in by `pipewire` |
| openSUSE Tumbleweed | Not tried | The GUI preview's sink is in `gstreamer-plugins-rs` |
| openSUSE Leap 15 | Unsupported | GTK, libadwaita and Python are too old |
| openSUSE Aeon / MicroOS | Not tried | Build in a distrobox, as for Fedora Atomic |

`vinoopeneffects-ctl doctor` names the missing package for your distro.

### Fedora Atomic and other image-based distros

`/usr` is read-only, so the default `~/.local` install is the one to use
(`/usr/local` also works: on Fedora Atomic it is `/var/usrlocal`). Nothing here
needs root or a kernel module. The build needs the `-devel` packages and Rust,
which belong in a toolbox (or distrobox) rather than layered on the host:

```bash
toolbox create --release "$(rpm -E %fedora)" vinoopeneffects
toolbox enter vinoopeneffects
sudo dnf install rust cargo  # plus the Fedora list in development.md
cargo install ovfetch --locked
scripts/install.sh
```

The toolbox shares your home directory and session bus, so the install lands in
`~/.local` and the daemon runs on the host. Create the toolbox with the host's
release so the binaries link against the same library versions the host has.
The host then needs the runtime side: GStreamer with `-plugins-good`,
`pipewire-gstreamer`, and for the GUI `gtk4`, `libadwaita` and
`gstreamer1-plugin-gtk4`. Silverblue ships most of these; Kinoite and Bazzite
may not ship the GTK ones. Run `vinoopeneffects-ctl doctor` on the host: it
names what to `rpm-ostree install`. If `vinoopeneffects-ctl status` on the host
says the daemon isn't activatable, run `systemctl --user daemon-reload` there or
log in again.

On openSUSE Aeon and MicroOS the same applies with a Tumbleweed distrobox and
`transactional-update pkg install` for anything the host lacks.

## Installing

```bash
scripts/install.sh                          # into ~/.local, no root
scripts/install.sh --no-gui                 # daemon and CLI only
scripts/install.sh --no-models              # don't touch models at all
scripts/install.sh --build-runtime          # let ovfetch compile a newer OpenVINO build
```

System-wide, build as yourself first, then install as root. As root the script
never builds, since that would leave a root-owned `target/` in the checkout:

```bash
cargo build --release --locked -p vinoopeneffects-daemon -p vinoopeneffects-ctl -p vinoopeneffects
sudo scripts/install.sh --prefix /usr/local
```

Under sudo it finds ovfetch in your `~/.cargo/bin` even though sudo's
`secure_path` drops it (elsewhere: `sudo env PATH="$PATH" scripts/install.sh ...`),
warns if a source file is newer than the build, puts the models under the
prefix instead of root's home, and reloads your user session's D-Bus and
systemd rather than root's. A `CARGO_TARGET_DIR` has to be passed through:
`sudo CARGO_TARGET_DIR=... scripts/install.sh ...`.

It builds a release and installs:

| What | Where (under the prefix) |
|---|---|
| `vinoopeneffects-daemon`, `vinoopeneffects-ctl`, `vinoopeneffects` | `bin/` |
| D-Bus interfaces and activation file | `share/dbus-1/interfaces/`, `share/dbus-1/services/` |
| systemd user unit | `share/systemd/user/vinoopeneffects-daemon.service` |
| Desktop entry and icons (GUI) | `share/applications/`, `share/icons/hicolor/` |
| onnxruntime | `lib/vinoopeneffects/ort/`, installed by `ovfetch` (see below) |
| Models | `~/.local/share/vinoopeneffects/models/`, or `share/vinoopeneffects/models/` under the prefix when run as root; sha256-verified against the manifests, ones already present and verified are skipped |

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
  `share/vinoopeneffects/models` under its own prefix, `/usr/local/share` and
  `/usr/share`)

## Uninstalling

```bash
scripts/uninstall.sh                        # from ~/.local
sudo scripts/uninstall.sh --prefix /usr/local
scripts/uninstall.sh --purge                # also models, config and backgrounds
```

It disables and stops the user unit first, then removes everything
`install.sh` put under the prefix, including the onnxruntime (and, as root, the
models under the prefix).

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

# Install

vinoOpenEffects can be installed either from a prebuilt `x86_64` release (no
Rust toolchain required) or from a source checkout. Native `.deb`, `.rpm`, and
Arch packages are not yet provided because selecting the matching ONNX Runtime
build (OpenVINO/CPU) per machine is currently handled by the installer script.

## Requirements

- **Display and media:** Wayland, PipeWire, and WirePlumber.
- **Source builds only:** Rust 1.92 or newer, plus GStreamer, GTK 4, libadwaita,
  and PipeWire development packages (see
  [development.md](development.md#environments) for distro package lists).
- **GUI:** libadwaita 1.7 or newer and `gtk4paintablesink` for live preview
  (`gstreamer1-plugin-gtk4` on Fedora, `gstreamer1.0-gtk4` on Debian/Ubuntu,
  `gst-plugin-gtk4` on Arch). Pass `--no-gui` if you do not have these.
- **ONNX Runtime installer:**
  [ovfetch](https://github.com/karanshukla/ovfetch) 0.5.0 or newer
  (`cargo install ovfetch --locked`), which is the only source of ONNX Runtime.
  The release installer downloads a pinned copy automatically if `ovfetch` is
  not on `PATH`.
- **Model downloads:** Python 3.11 or newer and `curl` (unless you pass
  `--no-models`).
- **Architecture:** Only `x86_64` has been tested.

## Distros

Development targets Fedora 45 KDE. Other distros should work with the package
lists in [development.md](development.md#environments); "Not tried" means no one
has reported results yet rather than a known failure.

| Distro | Status | Notes |
|---|---|---|
| Fedora 42+ (Workstation, KDE) | Primary target | — |
| Fedora Atomic (Silverblue, Kinoite, Bazzite) | Not tried | Install from a release into `~/.local`; the GUI may need layered packages (see below) |
| Ubuntu 26.04, Debian 13 | CI builds and tests on Ubuntu 26.04 | NPU users need the `render` group ([hardware.md](hardware.md)) |
| Ubuntu 24.04, Debian 12 | Daemon and CLI only | libadwaita 1.5/1.2 is too old for the GUI (use `--no-gui`); packaged Rust is older than 1.92, so install Rust via `rustup` |
| Arch | Not tried | `gst-plugin-pipewire` is not pulled in automatically by `pipewire` |
| openSUSE Tumbleweed | Not tried | The GUI preview sink is packaged in `gstreamer-plugins-rs` |
| openSUSE Leap 15 | Unsupported | GTK, libadwaita, and Python are too old |
| openSUSE Aeon / MicroOS | Not tried | Same approach as Fedora Atomic, using a Tumbleweed distrobox for builds |

Run `vinoopeneffects-ctl doctor` after installing to see the exact package names
for any missing runtime dependency on your distro.

### Fedora Atomic and other image-based distros

Because `/usr` is read-only, use the default `~/.local` install (`/usr/local`
also works—on Fedora Atomic it maps to `/var/usrlocal`). Nothing requires root
or a kernel module, and the [release install](#from-a-release) needs no build
tools, so run it directly on the host:

```bash
curl -fsSL https://github.com/karanshukla/vinoOpenEffects/releases/latest/download/install.sh | bash
```

Runtime package notes:

- The daemon and CLI require GStreamer with `-plugins-good` and
  `pipewire-gstreamer`.
- The GUI also requires `gtk4`, `libadwaita`, and `gstreamer1-plugin-gtk4`.
  Silverblue ships most of these, while Kinoite and Bazzite may omit the GTK
  packages.
- Run `vinoopeneffects-ctl doctor` to see what to pass to `rpm-ostree install`
  (followed by a reboot), or skip the GUI with `bash -s -- --no-gui`.
- If `vinoopeneffects-ctl status` reports that the daemon is not activatable,
  run `systemctl --user daemon-reload` or log out and back in.

Because the prebuilt binaries require libraries at least as new as Ubuntu
26.04's, build inside a toolbox or distrobox if your host libraries are older or
when installing from a source checkout (keeping Rust and `-devel` packages out
of the host image):

```bash
toolbox create --release "$(rpm -E %fedora)" vinoopeneffects
toolbox enter vinoopeneffects
sudo dnf install rust cargo  # plus the Fedora packages in development.md
cargo install ovfetch --locked
scripts/install.sh
```

The toolbox shares your home directory and session bus, so the installation
lands in `~/.local` and the daemon runs on the host. Match the toolbox release
to your host OS so linked library versions align. The host still needs the
runtime packages listed above; run `vinoopeneffects-ctl doctor` on the host
rather than inside the toolbox.

On openSUSE Aeon and MicroOS, follow the same workflow with a Tumbleweed
distrobox for builds and `transactional-update pkg install` for any missing host
runtime packages.

## Installing

### From a release

```bash
curl -fsSL https://github.com/karanshukla/vinoOpenEffects/releases/latest/download/install.sh | bash
```

How the release installer works:

- **Verification:** Downloads `SHA256SUMS` and `vinoopeneffects-x86_64-linux.tar.gz`,
  verifies the tarball checksum, and runs the bundled `scripts/install.sh` from
  a temporary directory. Because prebuilt binaries sit in `bin/` beside
  `scripts/`, no Rust toolchain is needed.
- **Options:** Pass flags after `bash -s --`, such as `--no-gui`, `--no-models`,
  a specific tag like `v5`, or `--prefix /usr/local` (with `sudo`).
- **Library compatibility:** Prebuilt binaries are compiled on Ubuntu 26.04 (the
  CI image) and require glibc, GTK, libadwaita, GStreamer, and PipeWire at least
  that new. On older distributions (such as SteamOS or Endless OS 6), build from
  source or inside a toolbox. On NixOS, enable `programs.nix-ld` for the
  standard dynamic linker. Releases prior to `v4` do not include prebuilt
  binaries.
- **ovfetch bootstrap:** If `ovfetch` 0.5.0 or newer is not on `PATH`, the
  installer fetches a SHA-256-pinned `ovfetch` release binary into
  `lib/vinoopeneffects/bin/` to install ONNX Runtime.
- **Upgrading and uninstalling:** Every release install saves the installer as
  `vinoopeneffects-upgrade`. Clicking **Check for Updates** in the window menu
  checks GitHub on demand and offers to run it, or you can run
  `vinoopeneffects-upgrade` (or `vinoopeneffects-upgrade v5`) in a terminal.
  Upgrading restarts a running daemon, briefly pausing the camera. Run
  `vinoopeneffects-upgrade --uninstall` to remove the installation (see
  [Uninstalling](#uninstalling)).

### From a checkout

```bash
scripts/install.sh                          # install into ~/.local without root
scripts/install.sh --no-gui                 # daemon and CLI only
scripts/install.sh --no-models              # skip model downloads
scripts/install.sh --build-runtime          # let ovfetch compile a newer OpenVINO build
```

For a system-wide install, compile as your regular user first and then run the
installer with `sudo` (when run as root, `install.sh` refuses to compile so it
never leaves a root-owned `target/` directory in your checkout):

```bash
cargo build --release --locked -p vinoopeneffects-daemon -p vinoopeneffects-ctl -p vinoopeneffects
sudo scripts/install.sh --prefix /usr/local
```

When invoked under `sudo`, `install.sh` locates `ovfetch` in your `~/.cargo/bin`
even if `sudo`'s `secure_path` strips it (if `ovfetch` lives elsewhere, run
`sudo env PATH="$PATH" scripts/install.sh ...`), warns if source files are newer
than the build, installs models under the prefix instead of root's home, and
reloads your user session's D-Bus and systemd instances rather than root's. Pass
`CARGO_TARGET_DIR` explicitly if you use one:
`sudo CARGO_TARGET_DIR=... scripts/install.sh ...`.

Files installed under the prefix:

| Component | Path under prefix |
|---|---|
| `vinoopeneffects-daemon`, `vinoopeneffects-ctl`, `vinoopeneffects` | `bin/` |
| D-Bus interfaces and activation file | `share/dbus-1/interfaces/`, `share/dbus-1/services/` |
| systemd user unit | `share/systemd/user/vinoopeneffects-daemon.service` |
| Desktop entry and icons (GUI) | `share/applications/`, `share/icons/hicolor/` |
| ONNX Runtime | `lib/vinoopeneffects/ort/` (managed by `ovfetch`) |
| Models | `~/.local/share/vinoopeneffects/models/` (or `share/vinoopeneffects/models/` under the prefix when run as root); SHA-256-verified against manifests, skipping already verified files, and removed on uninstall |

**ONNX Runtime selection (`ovfetch`):** `install.sh` verifies that `ovfetch`
0.5.0 or newer is available before building, then runs `ovfetch install`.
`ovfetch` selects the OpenVINO build of ONNX Runtime matching your NPU and
driver, or the newest build on machines without an NPU (it also bundles the CPU
provider, so one runtime covers both NPU and CPU).

If `ovfetch install` fails or refuses to downgrade a newer local build (for
example, one built with `ovfetch install --build`), `install.sh` falls back to
the existing runtime with a warning as long as `ovfetch verify` succeeds;
otherwise it aborts with `ovfetch`'s error. Passing `--build-runtime` forwards
`--build` to `ovfetch` (unchanged builds are not recompiled).

At the end of the install, the script reloads the D-Bus session bus so the
daemon is immediately activatable, and warns if `bin/` is not on `PATH`. It does
not restart an already-running daemon—run
`systemctl --user restart vinoopeneffects-daemon` to pick up the new binary.

By default the daemon starts on demand on the first client request. To start it
at login instead, see `vinoopeneffects-ctl autostart` in
[cli.md](cli.md#autostart).

## Desktops

vinoOpenEffects requires Wayland. Nothing in the daemon or GUI is tied to a
specific desktop environment: the virtual camera is a standard PipeWire node and
the GUI uses GTK 4/libadwaita. The only desktop differences are login autostart
and portal permissions for sandboxed apps.

| Desktop | `autostart enable` | Notes |
|---|---|---|
| KDE Plasma | Works | Primary target |
| GNOME | Works | — |
| sway, Hyprland | Only under systemd integration (`sway-systemd`, `uwsm`) | Otherwise start the daemon from compositor config (below) |
| COSMIC, others | Check with `vinoopeneffects-ctl autostart status` | Reports whether `graphical-session.target` is active |

Because autostart attaches to `graphical-session.target`, compositors that do
not start that target should launch the daemon from their config file:

```
# sway (~/.config/sway/config)
exec systemctl --user start vinoopeneffects-daemon

# Hyprland (~/.config/hypr/hyprland.conf)
exec-once = systemctl --user start vinoopeneffects-daemon
```

Even without autostart, the daemon starts on demand whenever the GUI or CLI
connects; autostart is only needed if you want video apps to see the virtual
camera before launching vinoOpenEffects.

Flatpak apps and browsers using PipeWire cameras access the virtual camera via
xdg-desktop-portal's Camera portal, which delegates permission prompts to the
desktop's portal backend. The GNOME and KDE backends implement this;
`xdg-desktop-portal-wlr` and `xdg-desktop-portal-hyprland` do not, so on sway or
Hyprland also install `xdg-desktop-portal-gtk`.

## Files at runtime

- `~/.config/vinoopeneffects/state.toml` — effect settings, selected camera, and
  inference provider, maintained by the daemon. If parsing fails, the file is
  moved to `state.toml.bad-<time>` instead of being overwritten.
- `~/.config/vinoopeneffects/gui.toml` — GUI-only preferences.
- `~/.local/share/vinoopeneffects/backgrounds/` — preset background images
  extracted from the GUI binary on first launch.
- `~/.local/share/vinoopeneffects/models/` — downloaded models (the daemon also
  searches `share/vinoopeneffects/models` under its own prefix,
  `/usr/local/share`, and `/usr/share`).
- `~/.cache/vinoopeneffects/openvino/` — NPU compiled-model cache so subsequent
  starts skip model compilation.

In the GUI, **Downloaded Models** under Preferences shows the combined size of
your user models and compile cache and can delete both. It stops the daemon
first (D-Bus restarts it when the window reconnects, briefly pausing the camera)
and preserves backgrounds and settings. It never modifies system-wide models
under `/usr/local` or `/usr`—if those exist, effects continue using them. To
re-download models later, run `vinoopeneffects-upgrade` (release install) or
`scripts/fetch-models.sh` (source checkout).

## Uninstalling

For a release install:

```bash
vinoopeneffects-upgrade --uninstall                 # remove from ~/.local
sudo vinoopeneffects-upgrade --uninstall --prefix /usr/local
vinoopeneffects-upgrade --uninstall --purge         # also remove config and backgrounds
```

Because a release install leaves only `vinoopeneffects-upgrade` on disk for
lifecycle management, `--uninstall` downloads and verifies the release tarball
and runs its `scripts/uninstall.sh`. From a checkout, run the script directly:

```bash
scripts/uninstall.sh                        # remove from ~/.local
sudo scripts/uninstall.sh --prefix /usr/local
scripts/uninstall.sh --purge                # also remove config and backgrounds
```

Uninstalling disables and stops the user service, removes everything
`install.sh` placed under the prefix (including ONNX Runtime and, when run as
root, prefix-installed models), and deletes user models
(`~/.local/share/vinoopeneffects/models`) and the compile cache
(`~/.cache/vinoopeneffects`, targeting the invoking user under `sudo`).
Configuration (`~/.config/vinoopeneffects`) and backgrounds are kept unless
`--purge` is passed.

## Upgrading from openeffects

vinoOpenEffects is a hard fork of `openeffects`, and all installed names
changed:

| Old | New |
|---|---|
| `openeffectsd`, `openeffectsctl`, `openeffects` | `vinoopeneffects-daemon`, `vinoopeneffects-ctl`, `vinoopeneffects` (no `openeffectsctl` alias) |
| `openeffectsd.service` | `vinoopeneffects-daemon.service` |
| `in.co.funinkina.Daemon.service` (D-Bus) | `io.github.karanshukla.vinoopeneffects.Daemon.service` |
| `OE_EP`, `OE_MODELS_DEST` | `VINOOPENEFFECTS_EP`, `VINOOPENEFFECTS_MODELS_DEST` |
| Camera `"OpenEffects Virtual Camera"` | `"vinoOpenEffects Virtual Camera"` |

On first launch, the daemon and GUI automatically migrate per-user state and log
each move:

- `~/.config/openeffects/` → `~/.config/vinoopeneffects/`
- `~/.local/share/openeffects/` (models and backgrounds) →
  `~/.local/share/vinoopeneffects/`, updating background paths in `state.toml`
- `~/.config/systemd/user/openeffectsd.service.d/` →
  `~/.config/systemd/user/vinoopeneffects-daemon.service.d/`, renaming `OE_EP`
  and `OE_MODELS_DEST` inside `*.conf` files
- Enablement symlinks (`*.wants/openeffectsd.service`) recreated for the new
  unit

If a destination already exists, that item is skipped with a warning rather than
merged. A user-installed `~/.config/systemd/user/openeffectsd.service` is left
untouched with a warning. Run `systemctl --user daemon-reload` afterward.

The old binaries, service unit, and D-Bus files are not removed automatically;
uninstall `openeffects` using whichever method you originally used, update any
scripts, keybinds, or Waybar modules calling `openeffectsctl`, and re-select
**vinoOpenEffects Virtual Camera** in your video apps.

# Development

Build setup, testing, CI jobs, and release process. See
[CONTRIBUTING.md](../CONTRIBUTING.md) for the short version.

## Environments

Requires Rust 1.92 or newer (`rust-version` in `Cargo.toml`, verified by the
`msrv` CI job) plus GStreamer, GTK 4, libadwaita, and PipeWire development
packages:

```bash
# Fedora (primary dev target)
sudo dnf install gcc pkgconf-pkg-config clang-devel \
  gstreamer1-devel gstreamer1-plugins-base-devel gstreamer1-plugins-bad-free-devel \
  gtk4-devel libadwaita-devel pipewire-devel \
  gstreamer1-plugins-base gstreamer1-plugins-good

# Debian / Ubuntu (used in CI)
sudo apt install build-essential pkg-config libclang-dev \
  libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev \
  libgtk-4-dev libadwaita-1-dev \
  libpipewire-0.3-dev libspa-0.2-dev gstreamer1.0-plugins-base gstreamer1.0-plugins-good

# Arch
sudo pacman -S --needed base-devel clang gstreamer gst-plugins-base gst-plugins-good \
  gtk4 libadwaita pipewire

# openSUSE Tumbleweed
sudo zypper install gcc pkgconf clang-devel gstreamer-devel gstreamer-plugins-base-devel \
  gtk4-devel libadwaita-devel pipewire-devel gstreamer-plugins-good
```

Notes:

- `clang` is needed for `bindgen` in `pipewire-sys`/`libspa-sys` (GitHub's
  Ubuntu runners preinstall it, whereas a fresh Debian or Ubuntu machine does
  not).
- The GUI requires libadwaita 1.7, so on Ubuntu 24.04 and Debian 12 build only
  the daemon and CLI
  (`cargo build -p vinoopeneffects-daemon -p vinoopeneffects-ctl`).
- On Fedora Atomic or openSUSE Aeon, install these packages inside a toolbox or
  distrobox ([install.md](install.md#fedora-atomic-and-other-image-based-distros)).

For the GUI live preview, install the GStreamer PipeWire source and GTK 4 sink:

| Distro | PipeWire source | GTK 4 sink |
|---|---|---|
| Fedora | `pipewire-gstreamer` | `gstreamer1-plugin-gtk4` |
| Debian / Ubuntu | `gstreamer1.0-pipewire` | `gstreamer1.0-gtk4` |
| Arch | `gst-plugin-pipewire` | `gst-plugin-gtk4` |
| openSUSE | `gstreamer-plugin-pipewire` | `gstreamer-plugins-rs` |

Those packages are sufficient to compile the workspace and run
`cargo test --workspace`. To run ML inference (and enable the model-backed tests
that otherwise skip), you also need:

- **ONNX Runtime:** The default `npu` build dynamically loads
  `libonnxruntime.so` for both CPU and NPU inference. Install it via `ovfetch`:
  `cargo install ovfetch --locked`, then
  `ovfetch install --prefix ~/.local/lib/vinoopeneffects/ort` (or point
  `ORT_DYLIB_PATH` at an existing `ovfetch` prefix). Building with
  `cargo build --no-default-features` links a CPU-only runtime statically
  instead.
- **Models:** Run `scripts/fetch-models.sh` (idempotent) to download and
  SHA-256-verify the four bundled models into
  `~/.local/share/vinoopeneffects/models`.

The `core` crate builds and tests without any of the system packages above, as
verified by the `core` CI job on every PR.

## Running your own build

`scripts/install.sh` builds a release binary and installs everything into
`~/.local` (binaries, D-Bus activation file and interfaces, systemd user unit,
desktop entry, icon, models, and ONNX Runtime). It does not restart a running
daemon:

```bash
scripts/install.sh                       # rebuild and reinstall; verified models and runtime are kept
systemctl --user restart vinoopeneffects-daemon
```

Reopen the GUI afterward, and re-select the camera in any app that already had
it open. To iterate quickly on the daemon alone, stop the service and run it in
the foreground:

```bash
systemctl --user stop vinoopeneffects-daemon
RUST_LOG=debug cargo run -p vinoopeneffects-daemon -- --start
```

Run `scripts/uninstall.sh` to remove the installation, user models, and compile
cache (add `--purge` to also remove configuration and backgrounds).

## Tests

```bash
cargo test --workspace                    # full test suite
cargo test -p vinoopeneffects-core        # pure logic, no media stack
cargo test -p vinoopeneffects-core gesture   # single module
```

Tests never require a camera, NPU, audio server, or running daemon. Tests that
depend on models, ONNX Runtime, or an NPU print a `skipping: ...` line and pass
when unavailable; D-Bus round-trip tests in `tests/integration` are marked
`#[ignore]` and can be run manually against a live daemon with
`cargo test -- --ignored`.

**Golden frames:** `core/src/effects/golden_tests.rs` runs the full effect chain
on synthetic frames with injected masks and face boxes, pinning pixel outputs
byte-for-byte via SHA-256 digests. Refactors should leave these digests
unchanged; if a change intentionally alters rendering output, update the digest
and call it out in the PR description.

## CI

`ci.yml` runs on every PR and push to `main`:

| Job | What it checks |
|---|---|
| `fmt` | `cargo fmt --all --check` |
| `clippy` | `cargo clippy --workspace --all-targets -- -D warnings` |
| `test` | `cargo test --workspace` under `xvfb-run` so GUI accessibility tests (`gui/src/a11y_tests.rs`) build real widgets; `VINOOPENEFFECTS_REQUIRE_DISPLAY=1` turns GTK startup failures into test failures |
| `core` | Verifies crate boundaries (`core` does not link GStreamer, GTK, or PipeWire; GUI and CLI depend on `shared` only) and runs `core` tests with no media/GUI system packages installed |
| `coverage` | `cargo llvm-cov` (excluding `daemon/src/bench.rs`): prints summary, uploads `lcov.info` artifact and reports to [Coveralls](https://coveralls.io/github/karanshukla/vinoOpenEffects), failing below 57% line coverage (Coveralls outages do not fail the job) |
| `build-npu` | `cargo check` of the daemon both with and without default features |
| `msrv` | `cargo +1.92 test --workspace --no-run` (verifies binaries, tests, and dev-dependencies compile on MSRV) |
| `install` | Runs `shellcheck scripts/*.sh`, verifies `install.sh` rejects an unwritable prefix before building, runs `install.sh --no-models` into a scratch prefix, verifies all installed files, tests upgrading in place, and runs `install.sh --uninstall` from a `file://` fake release (checking prefix cleanup, model/cache deletion in scratch `HOME`, retention of backgrounds/settings, and `--purge` cleanup) |
| `release-bundle` | Creates the same `git archive` bundle as `release.yml` with a synthetic tag, builds the CLI with `--locked`, and checks `--version` reports `dev` by default and the tag when `VINOOPENEFFECTS_VERSION` is set |
| `deny` | `cargo deny check` (`deny.toml`: licenses, advisories, and crate sources) |
| `fedora` | `cargo build` and `cargo test --locked` inside `fedora:latest` with Fedora's `rustc` (non-blocking, since new Fedora releases can introduce independent breakage) |

Without a display (the `fedora` job, or running `cargo test` over SSH), GUI
widget tests print `skipping: no display` and pass while the accompanying CSS
contrast checks still run.

Because `ci.yml` does not download models, model-backed tests are covered by a
separate workflow, `models.yml`. It fetches the models and CPU ONNX Runtime,
runs `cargo test --workspace`, and fails if any test skips. It triggers on PRs
touching models, fetch scripts, inference code, or the daemon pipeline, runs
weekly with caching bypassed (to detect moved upstream files early), and can be
triggered manually. Keeping it separate prevents upstream model-host outages
from blocking unrelated PRs.

All GitHub Actions are pinned to commit SHAs with version comments, and
Dependabot batches Cargo and Action updates weekly after a 7-day cooldown.

## Releases

Releases use sequential numbers (`v1`, `v2`, ...) rather than semver: each tag
marks a snapshot of `main`, and higher numbers are newer. Inherited upstream
tags (`v0.1.x`) count under `0`, so the first fork release is `v1`. Releases are
cut whenever `main` has user-facing changes worth shipping.

```bash
scripts/release.sh --dry-run                               # show next tag and merged PRs since last release
git cliff --unreleased --tag vN --prepend CHANGELOG.md     # vN from dry run; merge via PR
scripts/release.sh                                         # from up-to-date main: tag origin/main and push
```

How tagging and versioning work:

- `release.sh` requires `CHANGELOG.md` on `main` to contain a section for `vN`
  before tagging.
- Pushing the tag runs `.github/workflows/release.yml`, which verifies the tag
  is on `main` and present in `CHANGELOG.md`, then publishes a GitHub Release
  with the changelog section as notes, a source archive
  (`vinoopeneffects-vN-source.tar.gz`), prebuilt binaries
  (`vinoopeneffects-x86_64-linux.tar.gz`, compiled on Ubuntu 26.04),
  `install.sh`, and `SHA256SUMS` ([install.md](install.md#from-a-release)).
- The crate version in `Cargo.toml` is separate from the release number and does
  not bump per release. The version displayed by the GUI About dialog and
  `vinoopeneffects-ctl --version` comes from `shared/src/version.rs`: it shows
  `vN` only when built with `VINOOPENEFFECTS_VERSION` set (as in the release
  workflow's `binaries` job), and `dev` for source checkouts or source bundles
  even at a tagged commit.

### Changelog

`CHANGELOG.md` is generated with [git-cliff](https://git-cliff.org) from
conventional commits (`feat:`, `fix:`, `docs:`, ...) using `cliff.toml`. Merge
commits and commits without a conventional prefix are omitted.

```bash
git cliff --unreleased            # preview unreleased changelog entries
```

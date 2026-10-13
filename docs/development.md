# Development

How the repo is built, tested and released. [CONTRIBUTING.md](../CONTRIBUTING.md)
is the short version.

## Environments

Rust 1.92 or newer (`rust-version` in `Cargo.toml`, checked by the `msrv` CI
job), plus the GStreamer, GTK 4, libadwaita and PipeWire development packages.

```bash
# Fedora (the dev target)
sudo dnf install gcc pkgconf-pkg-config clang-devel \
  gstreamer1-devel gstreamer1-plugins-base-devel gstreamer1-plugins-bad-free-devel \
  gtk4-devel libadwaita-devel pipewire-devel \
  gstreamer1-plugins-base gstreamer1-plugins-good

# Debian / Ubuntu (what CI uses)
sudo apt install libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev \
  libgstreamer-plugins-bad1.0-dev libgtk-4-dev libadwaita-1-dev \
  libpipewire-0.3-dev libspa-0.2-dev gstreamer1.0-plugins-base gstreamer1.0-plugins-good
```

That is enough to build and to run `cargo test --workspace`. Two more things
are needed to run the models, and the tests that use them skip without them:

- **onnxruntime.** The default `npu` build loads `libonnxruntime.so` at runtime,
  for CPU inference as well as NPU. `scripts/fetch-ort.sh DIR` downloads the
  sha256-verified CPU build; point `ORT_DYLIB_PATH` at it or put it in
  `~/.local/lib/vinoopeneffects/ort`. For the NPU, use
  `ovfetch install --prefix ~/.local/lib/vinoopeneffects/ort` instead.
  `cargo build --no-default-features` links a CPU-only runtime statically and
  needs neither.
- **Models.** `scripts/fetch-models.sh` downloads the four bundled models into
  `~/.local/share/vinoopeneffects/models`, checking each against its manifest's
  sha256. It is idempotent.

`core` builds and tests with none of the system packages above; the `core` CI
job proves that on every PR.

## Running your own build

`scripts/install.sh` builds a release and installs it into `~/.local` (binaries,
D-Bus activation file and interfaces, systemd user unit, desktop entry, icon,
models and an onnxruntime), so a checkout can be the install. It doesn't restart
anything that's already running:

```bash
scripts/install.sh --no-models           # rebuild and reinstall; models are already there
systemctl --user restart vinoopeneffects-daemon
```

Then reopen the GUI, and re-select the camera in any app that had it open. For a
quick loop on the daemon alone, run it in a terminal instead:

```bash
systemctl --user stop vinoopeneffects-daemon
RUST_LOG=debug cargo run -p vinoopeneffects-daemon -- --start
```

`scripts/uninstall.sh` removes the install (`--purge` also removes models,
config and backgrounds).

## Tests

```bash
cargo test --workspace                    # everything
cargo test -p vinoopeneffects-core        # pure logic, no media stack
cargo test -p vinoopeneffects-core gesture   # one module
```

Tests never need a camera, an NPU, an audio server or a running daemon.
Anything that does is either skipped at runtime with a `skipping: ...` line
(models, onnxruntime, NPU) or `#[ignore]`d with the reason (the D-Bus round trips
in `tests/integration`, which need a daemon on the session bus). Run those by
hand with `cargo test -- --ignored` against a running daemon.

**Golden frames.** `core/src/effects/golden_tests.rs` runs the whole effect
chain on synthetic frames, with the mask and faces injected, and pins the output
byte for byte as SHA-256 digests. A refactor must leave them unchanged. If a
change is meant to alter the output, update the digest and say so in the PR.

## CI

`ci.yml` runs on every PR and push to `main`:

| Job | What it checks |
|---|---|
| `fmt` | `cargo fmt --all --check` |
| `clippy` | `cargo clippy --workspace --all-targets -- -D warnings` |
| `test` | `cargo test --workspace` |
| `core` | Crate boundaries (`core` links no GStreamer, GTK or PipeWire; the GUI and CLI depend on `shared` only), then `core`'s tests with no system packages installed |
| `coverage` | `cargo llvm-cov`, summary in the job output; fails below 40% line coverage |
| `build-npu` | `cargo check` of the daemon with and without default features |
| `msrv` | `cargo +1.92 check --workspace` |
| `install` | `shellcheck` on the install scripts, then `install.sh --no-models` into a scratch prefix, a check of every installed file, and `uninstall.sh`, which must leave nothing behind |
| `deny` | `cargo deny check` (`deny.toml`: licenses, advisories, sources) |
| `fedora` | `cargo build` and `cargo test --locked` in a `fedora:latest` container with Fedora's own rustc. Not required, since a new Fedora release can break it on its own |

CI has no models, so the model-backed tests skip there. `models.yml` runs them
for real: it fetches the models and the CPU onnxruntime, runs
`cargo test --workspace`, and fails if any test printed a skip. It runs on PRs
that touch the models, the fetch scripts or the inference code, weekly with the
cache bypassed (so a moved upstream file shows up there first), and by hand. It
is separate from `ci.yml` so a model-host outage never blocks an unrelated PR.

Every action is pinned to a commit SHA with a version comment, and Dependabot
groups cargo and action updates weekly after a 7-day cooldown.

## Releases

Releases are numbered consecutively: `v1`, `v2`, and so on. A release is a tag
and a changelog entry. There is no release workflow and no packages yet:
vinoOpenEffects installs from source ([install.md](install.md)).

```bash
git cliff --unreleased --tag v2 --prepend CHANGELOG.md   # file pending commits under v2
git commit -am "docs(changelog): v2"
git tag v2 && git push && git push origin v2
```

The first release has no earlier `vN` tag to start from, so it uses a range
from the last inherited upstream tag instead:
`git cliff v0.1.4..HEAD --tag v1 --prepend CHANGELOG.md`.

The crate version in `[workspace.package]` in `Cargo.toml` is separate from the
release number. `scripts/bump-version.sh` still takes a semver `x.y.z` and tags
`vx.y.z`, so don't use it to cut a consecutive release.

### Changelog

`CHANGELOG.md` is generated by [git-cliff](https://git-cliff.org) from
conventional commits (`feat:`, `fix:`, `docs:`, ...), using `cliff.toml`. Merge
commits and commits without a conventional prefix are left out, so the prefix
is what gets a change into the changelog.

```bash
git cliff --unreleased            # preview what the next release would list
```

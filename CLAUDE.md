# CLAUDE.md

Guidance for Claude Code (claude.ai/code) when working in this repository.

## What this is

A Linux webcam effects engine (portrait blur, background replace, Studio Light,
Center Stage, gesture reactions) published as a PipeWire virtual camera. Rust
workspace: a daemon that owns the camera, the models and PipeWire, plus a GTK4
GUI and a CLI that are stateless D-Bus clients. A hard fork of
`funinkina/openeffects`.

User docs live in `docs/`; README.md is only the pitch, quick start and an
index. Don't duplicate them here, and when a change alters user-visible
behaviour, update the page that owns it. How it works is in
`docs/architecture.md`.

## Fork rules

- PRs go to `karanshukla/vinoOpenEffects`. Always pass
  `--repo karanshukla/vinoOpenEffects` to `gh`; the `upstream` remote is
  `funinkina/openeffects` and nothing is ever pushed or PR'd there.
- Commits are conventional (`feat:`, `fix:`, `docs:`, `refactor:`, `test:`,
  `ci:`, `chore:`); `CHANGELOG.md` is generated from them with `git cliff`.
- Releases are consecutive tags (`v1`, `v2`), not semver, cut with
  `scripts/release.sh` after the changelog section is merged
  (`docs/development.md#releases`).

## Build and test

```bash
cargo build --workspace
cargo test --workspace
cargo test -p vinoopeneffects-core            # pure logic, no GStreamer/GTK/PipeWire
cargo test -p vinoopeneffects-core gesture    # one module

# What CI gates on, besides tests
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo deny check
cargo +1.92 check --workspace                 # MSRV
```

Crate names: `vinoopeneffects-shared`, `vinoopeneffects-core`,
`vinoopeneffects-daemon`, `vinoopeneffects-ctl` (CLI), `vinoopeneffects` (GUI),
`vinoopeneffects-integration-tests`.

To exercise a change for real: `scripts/install.sh` (it skips verified models
and keeps a verified onnxruntime), then
`systemctl --user restart vinoopeneffects-daemon`, then open the virtual camera
in an app (or the GUI preview). `vinoopeneffects-ctl doctor` checks the install.

## CI

`ci.yml` runs `fmt`, `clippy`, `test`, `core`, `coverage`, `build-npu`, `msrv`,
`install`, `deny` and `fedora` on every PR; `docs/development.md#ci` says what
each one checks. The ones that bite:

- `core` fails if `vinoopeneffects-core` gains a normal dependency on GStreamer,
  GTK, libadwaita or PipeWire, or if the GUI or CLI depends on `core` or the
  daemon. Clients talk to the daemon over D-Bus only, through `shared`.
- `coverage` fails below 40% line coverage.
- `msrv` builds with Rust 1.92; keep it in step with `rust-version` in
  `Cargo.toml`.
- `install` runs `install.sh`/`uninstall.sh` into a scratch prefix and fails on
  any missing or leftover file, so new installed files go in both scripts.
- `models.yml` (not in `ci.yml`) is the only place the model-backed tests
  actually run, and it fails if any of them skipped.

## Gotchas

- **The default `npu` feature loads onnxruntime at runtime, for CPU too.** No
  `libonnxruntime.so` (via `ORT_DYLIB_PATH` or `~/.local/lib/vinoopeneffects/ort`)
  means no model runs at all. ovfetch is the only source of that runtime
  (`ovfetch install --prefix DIR`); `install.sh` and both workflows use it, and
  the required version is `ovfetch_min` in `install.sh`. Don't add another
  download path. `--no-default-features` links a CPU-only runtime statically.
- **Model-backed tests pass by skipping** when models or onnxruntime are
  missing, printing `skipping: ...`. A green local run proves nothing about
  inference unless `scripts/fetch-models.sh` and an onnxruntime are in place.
- **Golden frames.** `core/src/effects/golden_tests.rs` pins the whole effect
  chain's output as SHA-256 digests. A refactor must keep them; a deliberate
  output change updates the digest and says so in the PR.
- **The provide node's stream flags are load-bearing.** `DRIVER`, `RT_PROCESS`
  and the `SPA_META_Header` param each fix a consumer crash or a black frame
  (`docs/architecture.md#the-provide-node`). Don't drop them to "simplify".
- **The camera only opens while a consumer streams.** To test capture, open the
  virtual camera in an app or the GUI; `vinoopeneffects-ctl start` alone only
  advertises the node.
- **D-Bus XML is the contract.** `data/dbus/*.xml` generates the GUI's proxies
  (`gui/build.rs`, `zbus_xmlgen`); change the XML, the daemon's `dbus_server.rs`
  and `docs/cli.md` together.
- **Tests never need hardware.** No camera, NPU, audio server or running daemon.
  NPU-only tests skip on `/dev/accel`; daemon round trips are `#[ignore]`d.
- **Wayland only.** X11 is not supported.

## Conventions

- Code comments are one line, only where the code would invite a wrong fix.
  Longer reasoning goes in `docs/`, linked from the comment.
- Measured claims carry a date and the machine (`docs/hardware.md`).
- Every Actions `uses:` is pinned to a commit SHA with a version comment.
- A `deny.toml` advisory ignore needs a written reason next to it.

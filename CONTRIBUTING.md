# Contributing

This is developed on one Fedora laptop with an Intel NPU and one integrated
webcam. The most useful thing you can bring is what it does on *your* machine,
since that is what I can't test from here.

## Most useful

1. **Other cameras.** UVC webcams, external cameras, anything that misbehaves
   in the virtual camera (wrong mode, frame rate, colours). Attach
   `vinoopeneffects-ctl doctor --json` and `v4l2-ctl --list-formats-ext -d /dev/videoN`.
2. **Other consumers.** Zoom, Teams, OBS, Firefox, Chromium and Flatpak builds
   of them all negotiate the virtual camera slightly differently. Say which app
   and version, and include `journalctl --user -u vinoopeneffects-daemon`.
3. **Other hardware and distros.** Other NPU generations, the OpenVINO GPU
   path, CPU-only machines, and `scripts/install.sh` on distros other than
   Fedora.

## Setup

```bash
# system packages: see docs/development.md#environments
cargo build --workspace
cargo test --workspace
```

That runs without models, onnxruntime or a camera. To run a real install from
your checkout, see
[Running your own build](docs/development.md#running-your-own-build).

## Before you push

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
```

- Tests must not need a camera, an NPU, an audio server or a running daemon.
  If a change can only be checked on hardware, mark the test (skip with a
  `skipping: ...` line, or `#[ignore = "why"]`) and say in the PR what you ran by
  hand. Don't fake it.
- A failing `golden_frames_are_unchanged` means the effect output changed. If
  that was the point, update the digest and say so in the PR.
- Code comments are one line, and only where the code would invite a wrong
  fix. Longer reasoning goes in `docs/`, and the comment links to it.
- Measured claims (latency, frame rate, power) carry a date and the machine.
- Commits are conventional (`feat:`, `fix:`, `docs:`, `refactor:`, `test:`,
  `ci:`, `chore:`); the changelog is generated from them.
- When a change alters user-visible behaviour, update the page in `docs/` that
  owns it. The README links rather than repeats.

[docs/development.md](docs/development.md) has the rest: what CI runs, the
golden frames, and how releases are cut.

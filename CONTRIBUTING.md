# Contributing

vinoOpenEffects is developed on a single Fedora laptop with an Intel NPU and one
integrated webcam, so test reports from other hardware and distros are
especially valuable.

## Priority testing areas

1. **Other cameras:** UVC webcams, external cameras, or any mode, frame rate, or
   color issue in the virtual camera. Attach `vinoopeneffects-ctl doctor --json`
   and `v4l2-ctl --list-formats-ext -d /dev/videoN`.
2. **Other video apps:** Zoom, Teams, OBS, Firefox, Chromium, and their Flatpak
   builds all negotiate the virtual camera slightly differently. Include the app
   name, version, and output from `journalctl --user -u vinoopeneffects-daemon`.
3. **Other hardware and distros:** Different NPU generations, the OpenVINO GPU
   path, CPU-only machines, and `scripts/install.sh` on non-Fedora distros.

## Setup

```bash
# Install system packages first: see docs/development.md#environments
cargo build --workspace
cargo test --workspace
```

Building and running tests works without models, ONNX Runtime, or a camera. To
install and test a live build from your checkout, see
[Running your own build](docs/development.md#running-your-own-build).

## Before you push

Run the full local check command:

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
```

Guidelines:

- **Hardware-free tests:** Tests must not require a camera, NPU, audio server,
  or running daemon. If a change can only be verified on hardware, mark the test
  (print `skipping: ...` at runtime or use `#[ignore = "why"]`) and describe
  your manual verification in the PR instead of faking a test.
- **Golden frames:** If `golden_frames_are_unchanged` fails, effect rendering
  output changed. When that is intentional, update the digest and note it in the
  PR.
- **Comments and docs:** Prefer extracting, naming, or pinning a rule with a
  test pair before adding a comment, and link the test from the code it governs.
  See [docs/comment-style.md](docs/comment-style.md);
  `shared/tests/doc_links.rs` checks those links in CI. Longer rationale belongs
  in `docs/`, linked from the code.
- **Measured claims:** Always include the date and machine for latency, frame
  rate, or power measurements.
- **Commits:** Use conventional commit prefixes (`feat:`, `fix:`, `docs:`,
  `refactor:`, `test:`, `ci:`, `chore:`), which drive changelog generation.
- **Documentation updates:** Update the relevant page in `docs/` whenever
  user-visible behavior changes; keep `README.md` concise and link to `docs/`.

See [docs/development.md](docs/development.md) for CI jobs, golden frames, and
the release process.

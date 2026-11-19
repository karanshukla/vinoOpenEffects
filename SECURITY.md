# Security

vinoOpenEffects captures frames from your webcam, runs local ML models on each
frame, and publishes the output as a PipeWire virtual camera. It runs under your
regular user account (never as root) and is controlled over the D-Bus session
bus. Because a bug could expose your camera feed, the pipeline treats video data
as sensitive as the physical camera itself.

## Reporting

Use [private vulnerability reporting](https://github.com/karanshukla/vinoOpenEffects/security/advisories/new)
rather than opening a public issue for exploitable bugs.

If the vulnerability also affects
[upstream openeffects](https://github.com/funinkina/openeffects), mention that
in your report so disclosure can be coordinated before publication.

## Local processing and downloads

All video inference runs locally on your CPU or NPU with no network calls,
telemetry, or cloud inference in the video path. Network access occurs only when
you explicitly run a download script, and every download is integrity-checked:

| Download | Script | Verification |
|---|---|---|
| Four bundled models | `scripts/fetch-models.sh` (invoked by `install.sh`) | SHA-256 checked against committed `data/models/*/manifest.toml`; mismatches fail |
| ONNX Runtime (OpenVINO build, also used for CPU) | `ovfetch install` (invoked by `install.sh`) | `ovfetch`'s reviewed hash ledger with independent source agreement, plus `SHA256SUMS` for `ovfetch verify`; this repo pins the minimum `ovfetch` version rather than individual artifacts |
| IAT checkpoint (experimental) | `scripts/export-iat.py` | Pinned upstream commit, checkpoint SHA-256, and exported model SHA-256 against its manifest |

The daemon verifies each model's SHA-256 against its manifest when resolving it,
and `vinoopeneffects-ctl doctor` repeats the verification on demand.

## Threat model

| Threat | Mitigation |
|---|---|
| Remote machine controlling the camera or effects | The daemon listens only on the D-Bus **session** bus, with no network socket and no system-bus service |
| Camera left active when unused | The physical webcam opens only while a consumer streams from the virtual camera and closes 300 ms after the last consumer disconnects; arming the daemon advertises the node without opening the hardware ([architecture.md](docs/architecture.md#the-camera-opens-only-when-someone-is-watching)) |
| Sandboxed app reading the processed feed | Flatpak and portal-mediated apps can only access the virtual camera through xdg-desktop-portal's camera permission prompt, identical to a physical camera |
| Malicious background image | Decoded as your user by the `image` crate (JPEG and PNG only) with default allocation limits |
| Tampered model file | SHA-256 verified against the committed manifest before loading |
| Compromised dependency or CI action | crates.io only, enforced by `cargo deny` (`deny.toml` denies unknown registries and git sources, enforces a license allowlist, and checks advisories); `Cargo.lock` committed; all CI actions pinned to commit SHAs; Dependabot waits for a 7-day cooldown |

## Out of scope

- **Other processes running as your user:** The D-Bus API uses session-bus
  access control only, so any process under your account can control the daemon,
  switch cameras, toggle effects, or set any readable image as a background.
  Unsandboxed processes under your user can also read the virtual camera or open
  `/dev/video*` directly.
- **A tampered ONNX Runtime binary:** The daemon dynamically loads
  `libonnxruntime.so` from `ORT_DYLIB_PATH` or the first match in its search
  path (starting in `~/.local`). Anything able to write there or modify the
  daemon's environment already has code execution as your user.
- **Apps opening the physical camera directly:** Any app that chooses your real
  webcam instead of the virtual camera receives the raw, unprocessed feed.
- **Segmentation misses:** Portrait blur and background replacement are visual
  effects, not a privacy barrier. Segmentation can miss edges or fast motion and
  briefly reveal the background (while the model loads after opening the camera,
  the entire frame is blurred or replaced rather than shown raw). Do not rely on
  them to conceal sensitive material.
- **Root-level attackers.**

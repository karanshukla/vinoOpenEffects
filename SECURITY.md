# Security

vinoOpenEffects opens your webcam, runs every frame through local models, and
publishes the result as a PipeWire camera. It runs as your user, never as root,
and is controlled over the D-Bus session bus. A bug here exposes what your
camera sees, so the feed is treated as sensitive as the camera itself.

## Reporting

Use [private vulnerability reporting](https://github.com/karanshukla/vinoOpenEffects/security/advisories/new).
Do not open a public issue for anything exploitable.

If the bug is also in [upstream openeffects](https://github.com/funinkina/openeffects),
say so in the report so it can be coordinated before anything is published.

## What it does with your data

Nothing leaves the machine. Frames go from the camera through ONNX Runtime on
your CPU or NPU to the virtual camera, with no network call anywhere in that
path, no telemetry and no cloud inference. The project only touches the network
when you run a script that downloads something, and each download is checked:

| Download | Script | Check |
|---|---|---|
| The four bundled models | `scripts/fetch-models.sh` (run by `install.sh`) | sha256 from the committed `data/models/*/manifest.toml`; a mismatch fails |
| onnxruntime (OpenVINO build, also used for CPU) | `ovfetch install` (run by `install.sh`) | ovfetch's reviewed hash ledger, independent sources agreeing on each hash, and `SHA256SUMS` for `ovfetch verify`; this repo pins the minimum ovfetch version, not the artifact |
| IAT checkpoint (experimental) | `scripts/export-iat.py` | pinned upstream commit, checkpoint sha256, then the export's sha256 against its manifest |

The daemon checks each model against its manifest's sha256 when it resolves it,
and `vinoopeneffects-ctl doctor` repeats the check on demand.

## Threat model

| Threat | Defence |
|---|---|
| A remote machine drives the camera or the effects | The daemon listens only on the D-Bus **session** bus. There is no network listener and no system-bus service |
| Camera left on when no one is watching | The real camera opens only while a consumer is streaming from the virtual camera, and closes 300 ms after the last one leaves. Arming the daemon advertises the node without opening the device ([architecture.md](docs/architecture.md#the-camera-opens-only-when-someone-is-watching)) |
| A sandboxed app reads the processed feed | Flatpak and other portal-mediated apps reach the virtual camera only through xdg-desktop-portal's camera permission, the same as a real camera |
| A crafted background image | Decoded by the `image` crate (JPEG and PNG only) with its default allocation limits, as your user |
| A tampered model | sha256-checked against the committed manifest before use |
| A compromised dependency or action | crates.io only, enforced by `cargo deny` (`deny.toml`: unknown registries and git sources denied, a license allow-list, advisories), `Cargo.lock` committed, every CI action pinned to a commit SHA, Dependabot updates after a 7-day cooldown |

## What it does not defend against

- **Anything running as your user.** The D-Bus API has no authentication beyond
  the session bus: any process of yours can start and stop the daemon, switch
  cameras, change effects, or point the background at any image you can read.
  Any unsandboxed process of yours can also consume the virtual camera, or open
  `/dev/video*` directly.
- **A malicious onnxruntime library.** The daemon loads `libonnxruntime.so` as
  code, from `ORT_DYLIB_PATH` or the first match in its search path, which
  starts with your own `~/.local` directories. Whoever can write there, or set
  the daemon's environment, can run code as you; that is already you.
- **Apps that open the real camera.** An app that picks your webcam instead of
  the virtual camera gets the raw feed, with no effects.
- **The mask missing.** Background blur and replacement are cosmetic, not a
  privacy guarantee. Segmentation can miss at the edges or in fast motion, and
  briefly show what is behind you. (While the model loads after the camera
  opens, the whole frame is blurred or replaced rather than shown.)
  Don't rely on them to hide something that must not be seen.
- **Anyone with root.**

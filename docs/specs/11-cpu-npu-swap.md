# Swap between CPU and NPU inference

Spec for [#11](https://github.com/karanshukla/vinoOpenEffects/issues/11). Builds on #9, which added the `npu` cargo feature and `OE_EP`. Today the execution provider is fixed at process start and invisible to the user.

## Current state

| Piece | Where | Behavior |
|---|---|---|
| Provider choice | `inference::requested_ep()` | Reads `OE_EP` env once per session build. Default `cpu`. |
| Session build | `inference::build_session()` | Registers OpenVINO EP when `npu` feature is on and `OE_EP` is `npu`/`gpu`. Errors, never silently falls back. |
| Model lifetime | `oe_effects.rs` `Engine` | Lazy load on first use, one-shot `*_failed` memo, never reloaded. |
| Reporting | `DaemonState::capabilities()` | `models_ready`, `output_sink`, `virtual_camera`, `error`. No provider. |
| Config | `shared::config::AppState` | No inference settings. |

Consequence: switching means editing a systemd drop-in and restarting the daemon, and nothing tells you which one you got.

## Goal

Choose CPU, NPU or auto from the GUI, CLI or D-Bus. Persist it. Apply it without restarting the daemon or dropping the camera. Report what is actually running.

## Behavior

### Provider values

| Value | Meaning |
|---|---|
| `auto` (default) | NPU if available, else CPU. |
| `cpu` | CPU EP. Always available. |
| `npu` | OpenVINO EP, `device_type=NPU`. Errors if unavailable, never silently CPU. |

`gpu` stays an env-only developer option (`OE_EP=gpu`); not exposed in the UI. `OE_EP`, when set, overrides config and is reported as the source.

### Availability

NPU is available when all hold:

1. Built with `--features npu`.
2. An onnxruntime with the OpenVINO EP loads from `ORT_DYLIB_PATH`.
3. An NPU device exists (`/dev/accel/accel0`, or OpenVINO's device list).

Probe once at startup, cache, expose the reason when unavailable. A failed probe must never prevent the daemon from starting in CPU mode.

### Applying a change

- Config holds the requested provider. The daemon holds the active one per model.
- On change, build new sessions for the loaded models **off the streaming thread**, then swap atomically. Keep serving frames on the old sessions until the new ones are ready. NPU compile is about 0.5 s for palm and hand on first load; that must not stall frames.
- If building a new session fails, keep the old one, report the error, and leave the config value as requested so the UI shows the mismatch.
- Per-model fallback: if one model won't compile for the NPU, run that model on CPU, log once, report which.

### Reporting

New capability keys:

| Key | Type | Meaning |
|---|---|---|
| `inference_requested` | s | Config value. |
| `inference_active` | s | `cpu`, `npu`, or `mixed` when a model fell back. |
| `inference_available` | as | Providers usable right now. |
| `inference_detail` | s | Reason for unavailable or fallback; absent when fine. |

## Interfaces

**Config** (`state.toml`):

```toml
[inference]
provider = "auto"   # auto | cpu | npu
```

Missing table means `auto`, so old state files load unchanged (`#[serde(default)]`).

**D-Bus** (`in.co.funinkina.Daemon1`):

- Method `SetInferenceProvider(s provider)`. Unknown value is `InvalidArgs`. Requested-but-unavailable is accepted and reported, not an error, so scripts don't race hardware.
- Signal `InferenceChanged(s active, s detail)`.
- Update `data/dbus/in.co.funinkina.Daemon1.xml` and the capabilities comment.

**CLI**:

```
openeffectsctl inference            # print requested, active, available
openeffectsctl inference npu        # set
openeffectsctl status               # includes "inference: npu"
```

`docs/cli.md` already describes `status` as showing the execution provider; this makes that true.

**GUI**: a row in settings, Auto / CPU / NPU. NPU is insensitive with the detail as tooltip when unavailable. Shows the active provider next to the selector. Applies live.

## Implementation notes

- `build_session()` takes a provider argument; stop reading the env inside it. `requested_ep()` becomes the env-override layer only.
- `Engine` gains a generation counter and a swap path. Today's `*_failed` memos must reset on a provider change, or a CPU failure would pin the NPU path off.
- Build constraint: a `--features npu` binary uses `load-dynamic`, so it needs an onnxruntime at startup **even for CPU**. The OpenVINO build also contains the CPU EP, so one library serves both. Without a library the daemon can't run at all. Decide library discovery before shipping the feature (see Open questions).
- Don't add providers to the default build. Default stays CPU, bundled, no env needed.

## Tests

| Test | Kind |
|---|---|
| Config round-trip with and without `[inference]` | unit |
| Provider string parse, unknown rejected | unit |
| `cpu` requested on a CPU-only build is active `cpu` | unit |
| `npu` requested on a CPU-only build reports unavailable, daemon keeps running | unit |
| Swap keeps frames flowing (no gap over N frames) | integration, `videotestsrc` + CPU to CPU generation bump |
| `SetInferenceProvider` / capabilities round trip | integration (`dbus_roundtrip`) |
| CPU to NPU to CPU on real hardware, `npu_busy_time_us` moves | manual, `#[ignore]`, gated on `/dev/accel` |

## Acceptance

- Selecting NPU in the GUI on this machine moves `npu_busy_time_us` within a few seconds, with no camera dropout and no daemon restart.
- Selecting CPU stops NPU busy time from advancing.
- On a CPU-only build, NPU is shown unavailable with a reason, and everything else works.
- Killing the NPU path (unplug the lib) leaves effects working on CPU and reports why.

## Not in this change

- GPU as a user option.
- Per-model provider choice.
- Automatic benchmarking to pick a provider.
- Packaging the `npu` build or an ORT bundle.

## Open questions

- **Library discovery.** `ORT_DYLIB_PATH` only, or also search `$XDG_DATA_HOME/openeffects/ort/` and a system path? A user-level dir fits `ovfetch install --prefix`.
- **Should `auto` pick NPU unconditionally?** Measured here, NPU is 2.9x faster at about a tenth of the power, but that is one machine. Other NPU generations may differ.
- **Unavailable-but-requested persistence.** Keep the requested value (current proposal), or snap back to `auto`?
- **Where does the swap UI live** relative to the existing GUI settings layout? Needs a look at `gui/src/app.rs` before building.

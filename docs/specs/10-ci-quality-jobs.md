# CI quality jobs

Spec for [#10](https://github.com/karanshukla/vinoOpenEffects/issues/10). The repo has no `.github/`, so nothing checks that a change builds, passes tests, or is formatted. That includes the opt-in `npu` feature added in #9, which only compiles because `ort` was bumped to rc.13.

## Scope

One workflow, `ci.yml`, on `pull_request` and `push` to `main`. Quality gates only. Release and packaging workflows are separate work.

## Jobs

| Job | Command | Notes |
|---|---|---|
| `fmt` | `cargo fmt --all --check` | |
| `clippy` | `cargo clippy --workspace --all-targets -- -D warnings` | Run for default features. See Lint baseline. |
| `test` | `cargo test --workspace` | Model-backed tests no-op without models, by design (`engine.rs`). |
| `build-npu` | `cargo check -p openeffectsd --features npu` | `load-dynamic` needs no ORT at build time, so no ORT install. This is the job that would have caught the rc.12 break. |
| `msrv` | `cargo check --workspace` on 1.88 | `rust-version` in `Cargo.toml`. |
| `deny` | `cargo deny check` | Advisories, licenses (project is GPL-3.0-or-later), duplicate bans warn only. Needs a `deny.toml`. |

Required-check names for the ruleset: `fmt`, `clippy`, `test`, `build-npu`, `msrv`, `deny`. Keep them stable once required.

## Runner dependencies

Ubuntu LTS runner, apt:

```
libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev libgstreamer-plugins-bad1.0-dev
libgtk-4-dev libadwaita-1-dev libpipewire-0.3-dev libspa-0.2-dev
gstreamer1.0-plugins-base gstreamer1.0-plugins-good
```

`pipeline_passthrough` uses `videotestsrc`, which is in `-base`. `gstreamer-gl` is a workspace dependency, so the GL dev headers come with `-bad` and `-base` dev; verify against the first run rather than trusting this list.

## Model-backed tests

Two options, pick one:

1. **Skip.** Tests already no-op without models. CI proves compile and logic, not inference. Cheapest, and the default for the first PR.
2. **Fetch and cache.** Run `scripts/fetch-models.sh` with the model dir cached under a key of the `data/models/*/manifest.toml` hashes. Adds a job that exercises real ORT inference. The script already verifies sha256.

Start with 1. Move to 2 as a separate job so a model-host outage can't block unrelated PRs.

## Hardening

Matches the other repos:

- `permissions: contents: read` at workflow level, nothing broader per job.
- Third-party actions pinned to a full commit SHA, with the tag in a trailing comment.
- `concurrency` group per ref with `cancel-in-progress` on PRs.
- No `pull_request_target`, no secrets in these jobs.
- Dependabot for `cargo` and `github-actions`, 7-day cooldown, no auto-merge.
- Cargo cache via a pinned cache action, restore-only on PRs from forks.

## Lint baseline

Clippy has never run on this tree. Expect warnings. Two ways to land it:

- Fix them in the same PR (preferred if under about 30).
- Land `clippy` as non-required with `-D warnings` off, then ratchet.

Say which in the PR description; don't silently `#[allow]` the crate.

## Not in this change

- Release or packaging workflow (nfpm, Flatpak, Arch).
- Hardware tests. NPU runs need a self-hosted runner and stay manual (`scripts/bench-video.sh`).
- CodeQL. Worth adding after this lands; Rust support is newer and needs its own look.
- Branch ruleset configuration (settings, not code).

## Local gotcha

`rust-lld` from Rust 1.98 segfaults linking this workspace on Fedora 45. Local workaround is `RUSTFLAGS="-C link-arg=-fuse-ld=bfd"`. Not relevant to CI on Ubuntu, but don't commit that flag to `.cargo/config.toml`.

## Acceptance

- A PR with a formatting error, a clippy warning, a failing test, or a broken `--features npu` build fails the matching job.
- A PR touching only docs passes in under 5 minutes with a warm cache.
- All third-party actions are SHA-pinned.

## Open questions

- Is `gstreamer-gl` actually required to compile, or only at runtime? Decides whether `-bad` dev is needed.
- Keep `msrv` at 1.88, or let it float with `stable` and drop the job?
- Fedora job as well? Dev and packaging target is Fedora; the runner is not.

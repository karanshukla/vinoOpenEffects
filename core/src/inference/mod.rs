pub mod engine;
pub mod hand;
pub mod manifest;
pub mod models;
pub mod registry;

pub use shared::npu_failure as failure;

const INTRA_THREADS: usize = 2;

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

/// Set once the NPU/GPU path has failed in `auto` mode, so later model loads go
/// straight to CPU instead of paying for another failed compile each.
static ACCEL_FAILED: AtomicBool = AtomicBool::new(false);

pub fn reset_accel_failed() {
    ACCEL_FAILED.store(false, Ordering::Relaxed);
}

const COMPILE_FIX: &str = "check the NPU driver and compiler with `ovfetch detect`, then run `vinoopeneffects-ctl inference --retry`";
#[cfg(feature = "npu")]
const RUNTIME_FIX: &str =
    "run scripts/install.sh, which installs the OpenVINO onnxruntime with `ovfetch install`";

fn fingerprint() -> failure::Fingerprint {
    failure::current(loaded_runtime())
}

/// Why `auto` skips the NPU: a compile failure recorded on this driver and runtime.
fn recorded_failure() -> Option<String> {
    let record = failure::read(&failure::default_path()?, &fingerprint())?;
    Some(format!(
        "NPU skipped after an earlier failure ({}); run `vinoopeneffects-ctl inference --retry` to try it again",
        record.reason
    ))
}

fn remember_failure(reason: &str) {
    let Some(path) = failure::default_path() else {
        return;
    };
    let record = failure::Record {
        device: "npu".into(),
        reason: reason.into(),
        fingerprint: fingerprint(),
    };
    if let Err(err) = failure::write(&path, &record) {
        tracing::warn!(%err, path = %path.display(), "could not save the NPU failure record");
    }
}

/// Forget every NPU failure, in this process and on disk; true if one was recorded.
pub fn forget_failures() -> bool {
    let was_failed = ACCEL_FAILED.swap(false, Ordering::Relaxed);
    let on_disk = failure::default_path().is_some_and(|p| failure::clear(&p));
    was_failed || on_disk
}

/// An NPU exists but `active` isn't it, for a provider that would use it.
pub fn degraded(provider: &str, active: &str) -> bool {
    degraded_on(npu_present(), provider, active)
}

fn degraded_on(npu_present: bool, provider: &str, active: &str) -> bool {
    npu_present && matches!(provider, "auto" | "npu") && active != "npu"
}

#[derive(Debug, Clone)]
pub struct NpuProbe {
    pub available: bool,
    pub reason: Option<String>,
}

#[cfg(feature = "npu")]
pub fn probe_npu() -> NpuProbe {
    use ort::ep::ExecutionProvider;
    if !npu_present() {
        return NpuProbe {
            available: false,
            reason: Some("no Intel NPU hardware detected (/dev/accel has no accel nodes)".into()),
        };
    }
    if shared::accel::probe().denied() {
        return NpuProbe {
            available: false,
            reason: Some(format!(
                "no permission to open the NPU in /dev/accel; {}",
                shared::accel::PERMISSION_FIX
            )),
        };
    }
    if let Err(e) = ensure_runtime() {
        return NpuProbe {
            available: false,
            reason: Some(format!("onnxruntime library unavailable: {e}")),
        };
    }
    let ep = ort::ep::OpenVINO::default().with_device_type("NPU");
    if !ep.is_available().unwrap_or(false) {
        return NpuProbe {
            available: false,
            reason: Some(format!(
                "onnxruntime has no OpenVINO execution provider; {RUNTIME_FIX}"
            )),
        };
    }
    NpuProbe {
        available: true,
        reason: None,
    }
}

#[cfg(not(feature = "npu"))]
pub fn probe_npu() -> NpuProbe {
    NpuProbe {
        available: false,
        reason: Some("built without `npu` feature (CPU-only build)".into()),
    }
}

static PROBED_NPU: std::sync::OnceLock<NpuProbe> = std::sync::OnceLock::new();

pub fn probe_npu_cached() -> &'static NpuProbe {
    PROBED_NPU.get_or_init(probe_npu)
}

pub fn available_providers() -> Vec<&'static str> {
    if probe_npu_cached().available {
        vec!["cpu", "npu"]
    } else {
        vec!["cpu"]
    }
}

pub fn env_ep() -> Option<String> {
    std::env::var("VINOOPENEFFECTS_EP")
        .map(|v| v.trim().to_ascii_lowercase())
        .ok()
        .filter(|v| !v.is_empty())
}

pub fn requested_ep() -> String {
    env_ep().unwrap_or_else(|| "auto".into())
}

pub fn effective_ep(configured: &str) -> String {
    env_ep().unwrap_or_else(|| configured.trim().to_ascii_lowercase())
}

/// The provider models will land on for `provider` before any is loaded, plus
/// why it isn't the NPU when one exists.
pub fn predicted_active(provider: &str) -> (&'static str, Option<String>) {
    match provider {
        "npu" | "auto" => match npu_blocker(provider) {
            None => ("npu", None),
            Some(reason) => ("cpu", reason),
        },
        "gpu" => ("gpu", None),
        _ => ("cpu", None),
    }
}

/// `None` if `provider` (`auto` or `npu`) should try the NPU, else why not:
/// `Some(None)` when there is simply no NPU and `auto` shouldn't mention it.
fn npu_blocker(provider: &str) -> Option<Option<String>> {
    let probe = probe_npu_cached();
    if !probe.available {
        let quiet = provider == "auto" && !npu_present();
        return Some(if quiet { None } else { probe.reason.clone() });
    }
    if provider == "auto" {
        if ACCEL_FAILED.load(Ordering::Relaxed) {
            return Some(Some(format!(
                "NPU failed earlier in this run; {COMPILE_FIX}"
            )));
        }
        if let Some(reason) = recorded_failure() {
            return Some(Some(reason));
        }
    }
    None
}

/// A session builder with the shared CPU settings and no accelerator attached.
fn base_builder() -> anyhow::Result<ort::session::builder::SessionBuilder> {
    use ort::memory::{AllocationDevice, AllocatorType, MemoryInfo, MemoryType};
    ensure_runtime()?;
    let cpu = MemoryInfo::new(
        AllocationDevice::CPU,
        0,
        AllocatorType::Device,
        MemoryType::Default,
    )
    .map_err(|e| anyhow::anyhow!("create CPU memory info: {e}"))?;
    ort::session::Session::builder()
        .map_err(|e| anyhow::anyhow!("create ORT session builder: {e}"))?
        .with_intra_threads(INTRA_THREADS)
        .map_err(|e| anyhow::anyhow!("set intra-op threads: {e}"))?
        .with_intra_op_spinning(false)
        .map_err(|e| anyhow::anyhow!("disable intra-op spinning: {e}"))?
        .with_memory_pattern(false)
        .map_err(|e| anyhow::anyhow!("disable memory pattern: {e}"))?
        .with_allocator(cpu)
        .map_err(|e| anyhow::anyhow!("set non-arena CPU allocator: {e}"))
}

pub fn build_session(provider: &str) -> anyhow::Result<ort::session::builder::SessionBuilder> {
    with_execution_provider(base_builder()?, provider)
}

pub struct LoadedSession {
    pub session: ort::session::Session,
    pub active_ep: &'static str,
    pub fallback: Option<String>,
}

/// Load `model` on the requested execution provider (`auto`, `cpu`, `npu`, `gpu`).
/// `auto` and `npu` fall back to CPU when the NPU is missing or fails to compile
/// the model, and say why in `fallback`; a compile failure is remembered so
/// `auto` skips the NPU until `--retry` or a driver or runtime change. An
/// explicit `npu` always tries, and a success clears the record. Only `gpu`
/// fails outright.
pub fn load_session(model: &Path, provider: &str) -> anyhow::Result<LoadedSession> {
    use anyhow::Context;
    let load = |mut builder: ort::session::builder::SessionBuilder| {
        builder
            .commit_from_file(model)
            .with_context(|| format!("load {}", model.display()))
    };
    let on_cpu = |fallback: Option<String>| {
        Ok(LoadedSession {
            session: load(base_builder()?)?,
            active_ep: "cpu",
            fallback,
        })
    };

    let target = provider.trim().to_ascii_lowercase();
    match target.as_str() {
        "cpu" => on_cpu(None),
        "gpu" => {
            let session = load(build_session("gpu")?)?;
            Ok(LoadedSession {
                session,
                active_ep: "gpu",
                fallback: None,
            })
        }
        "auto" | "npu" => {
            if let Some(reason) = npu_blocker(&target) {
                return on_cpu(reason);
            }
            match build_session("npu").and_then(load) {
                Ok(session) => {
                    if target == "npu" && forget_failures() {
                        tracing::info!(
                            "NPU compiled on an explicit request; cleared the failure record"
                        );
                    }
                    Ok(LoadedSession {
                        session,
                        active_ep: "npu",
                        fallback: None,
                    })
                }
                Err(err) => {
                    let name = model
                        .file_name()
                        .unwrap_or(model.as_os_str())
                        .to_string_lossy();
                    let reason = format!("NPU compile failed for {name}: {err:#}");
                    if !ACCEL_FAILED.swap(true, Ordering::Relaxed) {
                        tracing::error!("{reason}; running on CPU. {COMPILE_FIX}");
                        remember_failure(&reason);
                    }
                    on_cpu(Some(format!("{reason}; {COMPILE_FIX}")))
                }
            }
        }
        other => {
            anyhow::bail!("unknown execution provider '{other}': expected auto, cpu, npu or gpu")
        }
    }
}

/// Load `model` on the NPU or fail: for models that are only usable there
/// (too slow on CPU), where CPU fallback would be a trap.
#[cfg(feature = "npu")]
pub fn load_session_npu(model: &Path, provider: &str) -> anyhow::Result<LoadedSession> {
    use anyhow::Context;

    let target = provider.trim().to_ascii_lowercase();
    anyhow::ensure!(
        matches!(target.as_str(), "auto" | "npu"),
        "needs the NPU, but provider is {target}"
    );
    let probe = probe_npu_cached();
    anyhow::ensure!(
        probe.available,
        "NPU unavailable: {}",
        probe.reason.as_deref().unwrap_or("unknown reason")
    );
    let session = build_session("npu")?
        .commit_from_file(model)
        .with_context(|| format!("load {} on the NPU", model.display()))?;
    Ok(LoadedSession {
        session,
        active_ep: "npu",
        fallback: None,
    })
}

#[cfg(not(feature = "npu"))]
pub fn load_session_npu(_model: &Path, _provider: &str) -> anyhow::Result<LoadedSession> {
    anyhow::bail!("needs a build with --features npu")
}

#[cfg(feature = "npu")]
/// Whether the kernel exposes an Intel NPU (`intel_vpu` registers `accel` nodes).
pub(crate) fn npu_present() -> bool {
    shared::accel::probe().total > 0
}

#[cfg(not(feature = "npu"))]
pub(crate) fn npu_present() -> bool {
    false
}

/// Directories searched for `libonnxruntime.so` when `ORT_DYLIB_PATH` is unset,
/// most specific first: user data directory, user local lib, executable prefix,
/// and system library paths.
#[cfg(feature = "npu")]
fn runtime_dirs() -> Vec<std::path::PathBuf> {
    let mut dirs = Vec::new();
    if let Some(data) = dirs::data_dir() {
        dirs.push(data.join("vinoopeneffects/ort"));
    }
    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join(".local/lib/vinoopeneffects/ort"));
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(prefix) = exe.parent().and_then(Path::parent)
    {
        dirs.push(prefix.join("lib/vinoopeneffects/ort"));
        dirs.push(prefix.join("lib64/vinoopeneffects/ort"));
    }
    dirs.push("/usr/lib/vinoopeneffects/ort".into());
    dirs.push("/usr/lib64/vinoopeneffects/ort".into());
    dirs.push("/usr/local/lib/vinoopeneffects/ort".into());
    dirs
}

/// Load the onnxruntime shared library once, failing with a message instead of
/// ort's panic when none can be found.
#[cfg(feature = "npu")]
pub(crate) fn ensure_runtime() -> anyhow::Result<()> {
    use std::path::PathBuf;
    use std::sync::OnceLock;
    static LOADED: OnceLock<Result<PathBuf, String>> = OnceLock::new();
    let loaded = LOADED.get_or_init(|| {
        let path = match std::env::var_os("ORT_DYLIB_PATH").filter(|v| !v.is_empty()) {
            Some(p) => PathBuf::from(p),
            None => runtime_dirs()
                .into_iter()
                .map(|d| d.join("libonnxruntime.so"))
                .find(|p| p.exists())
                .unwrap_or_else(|| "libonnxruntime.so".into()),
        };
        match ort::init_from(&path) {
            Ok(builder) => {
                builder.commit();
                tracing::info!(runtime = %path.display(), "loaded onnxruntime");
                let _ = LOADED_RUNTIME.set(path.clone());
                Ok(path)
            }
            Err(e) => Err(format!(
                "cannot load onnxruntime ({}): {e}; set ORT_DYLIB_PATH or install libonnxruntime.so \
                 to ~/.local/lib/vinoopeneffects/ort",
                path.display()
            )),
        }
    });
    loaded
        .as_ref()
        .map(|_| ())
        .map_err(|e| anyhow::anyhow!("{e}"))
}

#[cfg(feature = "npu")]
static LOADED_RUNTIME: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

/// The onnxruntime library this process loaded, if any.
#[cfg(feature = "npu")]
fn loaded_runtime() -> Option<&'static Path> {
    LOADED_RUNTIME.get().map(std::path::PathBuf::as_path)
}

#[cfg(not(feature = "npu"))]
fn loaded_runtime() -> Option<&'static Path> {
    None
}

/// Where OpenVINO keeps compiled blobs, so later starts skip the NPU compile.
#[cfg(feature = "npu")]
fn compile_cache_dir() -> Option<String> {
    let dir = dirs::cache_dir()?.join("vinoopeneffects/openvino");
    std::fs::create_dir_all(&dir).ok()?;
    dir.to_str().map(str::to_owned)
}

#[cfg(not(feature = "npu"))]
pub(crate) fn ensure_runtime() -> anyhow::Result<()> {
    Ok(())
}

#[cfg(feature = "npu")]
fn with_execution_provider(
    builder: ort::session::builder::SessionBuilder,
    requested: &str,
) -> anyhow::Result<ort::session::builder::SessionBuilder> {
    use ort::ep::{ExecutionProvider, OpenVINO};

    let (device, strict) = match requested {
        "cpu" => return Ok(builder),
        "auto" => {
            if ACCEL_FAILED.load(Ordering::Relaxed) || !probe_npu_cached().available {
                return Ok(builder);
            }
            ("NPU", false)
        }
        "npu" => {
            let probe = probe_npu_cached();
            if !probe.available {
                anyhow::bail!(
                    "NPU requested but unavailable: {}",
                    probe.reason.as_deref().unwrap_or("unknown error")
                );
            }
            ("NPU", true)
        }
        "gpu" => ("GPU", true),
        other => {
            anyhow::bail!("unknown execution provider '{other}': expected auto, cpu, npu or gpu")
        }
    };
    let mut ep = OpenVINO::default().with_device_type(device);
    if let Some(dir) = compile_cache_dir() {
        ep = ep.with_cache_dir(dir);
    }
    if !ep.is_available().unwrap_or(false) {
        anyhow::ensure!(
            !strict,
            "OpenVINO execution provider unavailable (is ORT_DYLIB_PATH an onnxruntime-openvino build?)"
        );
        ACCEL_FAILED.store(true, Ordering::Relaxed);
        tracing::info!("onnxruntime has no OpenVINO EP; using CPU");
        return Ok(builder);
    }
    let registered = if strict {
        ep.build().error_on_failure()
    } else {
        ep.build()
    };
    builder
        .with_execution_providers([registered])
        .map_err(|e| anyhow::anyhow!("register OpenVINO EP ({device}): {e}"))
}

#[cfg(not(feature = "npu"))]
fn with_execution_provider(
    builder: ort::session::builder::SessionBuilder,
    requested: &str,
) -> anyhow::Result<ort::session::builder::SessionBuilder> {
    match requested {
        "cpu" | "auto" => Ok(builder),
        other => anyhow::bail!("execution provider '{other}' needs a build with --features npu"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn degraded_means_an_npu_exists_and_is_not_used() {
        assert!(!degraded_on(false, "auto", "cpu"), "no NPU is just CPU");
        assert!(!degraded_on(false, "npu", "cpu"));
        assert!(degraded_on(true, "auto", "cpu"));
        assert!(degraded_on(true, "npu", "mixed"));
        assert!(!degraded_on(true, "auto", "npu"));
        assert!(
            !degraded_on(true, "cpu", "cpu"),
            "an explicit cpu is never degraded"
        );
        assert!(!degraded_on(true, "gpu", "gpu"));
    }
}

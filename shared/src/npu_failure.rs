//! The persisted record of an NPU that failed to compile a model, so `auto`
//! skips it on later starts until the driver or runtime changes.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// What the failure depends on: a new NPU driver or OpenVINO runtime may fix it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fingerprint {
    pub driver: Option<String>,
    pub runtime: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    pub device: String,
    pub reason: String,
    pub fingerprint: Fingerprint,
}

pub fn default_path() -> Option<PathBuf> {
    dirs::state_dir().map(|d| d.join("vinoopeneffects/npu-failure.toml"))
}

/// The record at `path` if it was made on `current`; a stale one is deleted.
pub fn read(path: &Path, current: &Fingerprint) -> Option<Record> {
    let text = std::fs::read_to_string(path).ok()?;
    match toml::from_str::<Record>(&text) {
        Ok(record) if record.fingerprint == *current => Some(record),
        _ => {
            let _ = std::fs::remove_file(path);
            None
        }
    }
}

pub fn write(path: &Path, record: &Record) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, toml::to_string(record)?)?;
    Ok(())
}

/// Delete the record; true if there was one.
pub fn clear(path: &Path) -> bool {
    std::fs::remove_file(path).is_ok()
}

const DRIVER_LIB: &str = "libze_intel_npu.so.1";
const DRIVER_DIRS: &[&str] = &[
    "/usr/lib64",
    "/usr/lib",
    "/usr/lib/x86_64-linux-gnu",
    "/usr/local/lib",
];

/// The fingerprint for this machine's NPU driver and the runtime at `ort_lib`.
pub fn current(ort_lib: Option<&Path>) -> Fingerprint {
    let runtime_dir = ort_lib
        .and_then(|lib| std::fs::canonicalize(lib).ok())
        .and_then(|lib| lib.parent().map(Path::to_path_buf));
    Fingerprint {
        driver: driver_version(),
        runtime: runtime_dir.and_then(|d| runtime_version(&d)),
    }
}

/// The user-space NPU driver version, from the file its soname links to.
pub fn driver_version() -> Option<String> {
    DRIVER_DIRS.iter().find_map(|dir| {
        let target = std::fs::canonicalize(Path::new(dir).join(DRIVER_LIB)).ok()?;
        driver_version_from(target.file_name()?.to_str()?)
    })
}

fn driver_version_from(file_name: &str) -> Option<String> {
    file_name
        .strip_prefix("libze_intel_npu.so.")
        .filter(|v| !v.is_empty())
        .map(str::to_owned)
}

/// The onnxruntime and OpenVINO versions in `dir`, from their versioned file names.
pub fn runtime_version(dir: &Path) -> Option<String> {
    let names = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok());
    runtime_version_from(names)
}

fn runtime_version_from(names: impl Iterator<Item = String>) -> Option<String> {
    let mut versions = std::collections::BTreeMap::<String, String>::new();
    for name in names {
        let Some((lib, version)) = name.split_once(".so.") else {
            continue;
        };
        let Some(lib) = lib
            .strip_prefix("lib")
            .filter(|l| *l == "onnxruntime" || *l == "openvino")
        else {
            continue;
        };
        let best = versions.entry(lib.to_owned()).or_default();
        if version.len() > best.len() {
            *best = version.to_owned();
        }
    }
    let tags: Vec<String> = versions.iter().map(|(l, v)| format!("{l}-{v}")).collect();
    (!tags.is_empty()).then(|| tags.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fp(driver: &str) -> Fingerprint {
        Fingerprint {
            driver: Some(driver.into()),
            runtime: Some("onnxruntime-1.24.1 openvino-2641".into()),
        }
    }

    fn record(driver: &str) -> Record {
        Record {
            device: "npu".into(),
            reason: "NPU compile failed for yunet.onnx".into(),
            fingerprint: fp(driver),
        }
    }

    #[test]
    fn record_round_trips_on_the_same_versions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state/npu-failure.toml");
        write(&path, &record("1.35.0")).unwrap();
        assert_eq!(read(&path, &fp("1.35.0")), Some(record("1.35.0")));
        assert!(clear(&path));
        assert!(!clear(&path));
        assert_eq!(read(&path, &fp("1.35.0")), None);
    }

    #[test]
    fn version_change_drops_the_record() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("npu-failure.toml");
        write(&path, &record("1.35.0")).unwrap();
        assert_eq!(read(&path, &fp("1.36.0")), None);
        assert!(!path.exists());
    }

    #[test]
    fn unreadable_record_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("npu-failure.toml");
        std::fs::write(&path, "not = [toml").unwrap();
        assert_eq!(read(&path, &fp("1.35.0")), None);
        assert!(!path.exists());
    }

    #[test]
    fn driver_version_comes_from_the_soname_target() {
        assert_eq!(
            driver_version_from("libze_intel_npu.so.1.35.0").as_deref(),
            Some("1.35.0")
        );
        assert_eq!(driver_version_from("libze_intel_npu.so."), None);
        assert_eq!(driver_version_from("libze_loader.so.1"), None);
    }

    #[test]
    fn runtime_version_keeps_the_most_specific_names() {
        let names = [
            "libonnxruntime.so",
            "libonnxruntime.so.1",
            "libonnxruntime.so.1.24.1",
            "libopenvino.so.2641",
            "libopenvino_c.so.2641",
            "libtbb.so.12",
            "ovfetch.lock.json",
        ]
        .map(String::from);
        assert_eq!(
            runtime_version_from(names.into_iter()).as_deref(),
            Some("onnxruntime-1.24.1 openvino-2641")
        );
        assert_eq!(runtime_version_from(std::iter::empty()), None);
    }
}

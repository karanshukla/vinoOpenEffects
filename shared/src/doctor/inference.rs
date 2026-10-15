use std::path::{Path, PathBuf};

use super::types::{Check, Status};

pub fn build_feature_verdict(built_with_npu: bool) -> Check {
    if built_with_npu {
        Check::new(
            "inference",
            "build",
            Status::Ok,
            "built with `npu` feature (OpenVINO EP enabled)",
            None,
        )
    } else {
        Check::new(
            "inference",
            "build",
            Status::Warn,
            "built without `npu` feature (CPU-only build)",
            Some("rebuild with default features or `--features npu` to enable Intel NPU acceleration".into()),
        )
    }
}

pub fn probe_built_with_npu() -> bool {
    cfg!(feature = "npu")
}

pub fn ort_library_verdict(resolved: Option<&Path>, has_openvino: bool) -> Check {
    match resolved {
        Some(path) => {
            if has_openvino {
                Check::new(
                    "inference",
                    "onnxruntime",
                    Status::Ok,
                    format!("{} (OpenVINO EP available)", path.display()),
                    None,
                )
            } else {
                Check::new(
                    "inference",
                    "onnxruntime",
                    Status::Warn,
                    format!("{} (CPU-only, OpenVINO EP not present)", path.display()),
                    Some("install OpenVINO runtime with `ovfetch install --prefix ~/.local/lib/vinoopeneffects/ort`".into()),
                )
            }
        }
        None => Check::new(
            "inference",
            "onnxruntime",
            Status::Fail,
            "libonnxruntime.so not found in runtime search paths",
            Some("install onnxruntime with `ovfetch install --prefix ~/.local/lib/vinoopeneffects/ort` or set ORT_DYLIB_PATH".into()),
        ),
    }
}

pub fn npu_failure_verdict(record: Option<&crate::npu_failure::Record>) -> Check {
    match record {
        None => Check::new(
            "inference",
            "npu-failure",
            Status::Ok,
            "no NPU compile failure recorded",
            None,
        ),
        Some(r) => Check::new(
            "inference",
            "npu-failure",
            Status::Warn,
            format!("auto skips the NPU after a recorded failure: {}", r.reason),
            Some("check the NPU driver and compiler with `ovfetch detect`, then run `vinoopeneffects-ctl inference --retry`".into()),
        ),
    }
}

/// The recorded NPU failure, if it was made on this driver and `ort_lib`'s runtime.
pub fn probe_npu_failure(ort_lib: Option<&Path>) -> Option<crate::npu_failure::Record> {
    use crate::npu_failure as failure;
    failure::read(&failure::default_path()?, &failure::current(ort_lib))
}

pub fn npu_hardware_verdict(accel_count: usize) -> Check {
    if accel_count > 0 {
        Check::new(
            "inference",
            "npu",
            Status::Ok,
            format!("{accel_count} Intel NPU device(s) found in /dev/accel"),
            None,
        )
    } else {
        Check::new(
            "inference",
            "npu",
            Status::Info,
            "no Intel NPU hardware detected (/dev/accel has no accel nodes; models run on CPU)",
            None,
        )
    }
}

pub fn runtime_dirs(home: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(h) = home {
        dirs.push(h.join(".local/lib/vinoopeneffects/ort"));
    }
    dirs.push(PathBuf::from("/usr/lib/vinoopeneffects/ort"));
    dirs.push(PathBuf::from("/usr/lib64/vinoopeneffects/ort"));
    dirs.push(PathBuf::from("/usr/local/lib/vinoopeneffects/ort"));
    dirs
}

pub fn probe_ort_runtime(home: Option<&Path>) -> (Option<PathBuf>, bool) {
    if let Some(env_path) = std::env::var_os("ORT_DYLIB_PATH").filter(|v| !v.is_empty()) {
        let p = PathBuf::from(env_path);
        let has_ov = p
            .parent()
            .map(|dir| dir.join("libonnxruntime_providers_openvino.so").exists())
            .unwrap_or(false);
        return (Some(p), has_ov);
    }

    for dir in runtime_dirs(home) {
        let lib = dir.join("libonnxruntime.so");
        if lib.exists() {
            let has_ov = dir.join("libonnxruntime_providers_openvino.so").exists();
            return (Some(lib), has_ov);
        }
    }
    (None, false)
}

pub fn probe_npu_devices() -> usize {
    std::fs::read_dir("/dev/accel")
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| e.file_name().to_string_lossy().starts_with("accel"))
                .count()
        })
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn npu_failure_verdict_warns_with_the_retry_command() {
        assert_eq!(npu_failure_verdict(None).status, Status::Ok);
        let record = crate::npu_failure::Record {
            device: "npu".into(),
            reason: "NPU compile failed for yunet.onnx".into(),
            fingerprint: Default::default(),
        };
        let check = npu_failure_verdict(Some(&record));
        assert_eq!(check.status, Status::Warn);
        assert!(check.detail.contains("yunet.onnx"));
        assert!(check.fix.as_deref().unwrap().contains("inference --retry"));
    }

    #[test]
    fn test_build_feature_verdict() {
        assert_eq!(build_feature_verdict(true).status, Status::Ok);
        let warn = build_feature_verdict(false);
        assert_eq!(warn.status, Status::Warn);
        assert!(warn.fix.is_some());
    }

    #[test]
    fn test_ort_library_verdict_missing() {
        let c = ort_library_verdict(None, false);
        assert_eq!(c.status, Status::Fail);
        assert!(c.fix.is_some());
    }

    #[test]
    fn test_ort_library_verdict_cpu_only() {
        let path = PathBuf::from("/usr/lib64/vinoopeneffects/ort/libonnxruntime.so");
        let c = ort_library_verdict(Some(&path), false);
        assert_eq!(c.status, Status::Warn);
        assert!(c.detail.contains("CPU-only"));
    }

    #[test]
    fn test_ort_library_verdict_openvino() {
        let path = PathBuf::from("/usr/lib64/vinoopeneffects/ort/libonnxruntime.so");
        let c = ort_library_verdict(Some(&path), true);
        assert_eq!(c.status, Status::Ok);
        assert!(c.detail.contains("OpenVINO"));
    }

    #[test]
    fn test_npu_hardware_verdict() {
        let ok = npu_hardware_verdict(1);
        assert_eq!(ok.status, Status::Ok);

        let none = npu_hardware_verdict(0);
        assert_eq!(none.status, Status::Info);
    }
}

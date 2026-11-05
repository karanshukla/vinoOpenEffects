//! The Intel NPU's `/dev/accel` nodes, for the daemon's provider choice and `doctor`.

use std::path::Path;

pub const DIR: &str = "/dev/accel";
pub const SYSFS: &str = "/sys/class/accel";
/// Other accel drivers (AMD's `amdxdna`, Habana) register `accel` nodes too.
pub const INTEL_DRIVER: &str = "intel_vpu";

/// Debian/Ubuntu make accel nodes `root:render 0660` with no uaccess tag; Fedora makes them 0666.
pub const PERMISSION_FIX: &str = "sudo usermod -aG render $USER, then log out and back in (the NPU's /dev/accel nodes belong to the render group)";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Nodes {
    /// `accel*` nodes bound to `intel_vpu`.
    pub total: usize,
    /// Of those, ones this process may open read-write.
    pub usable: usize,
}

impl Nodes {
    /// Hardware is there, but no node can be opened.
    pub fn denied(&self) -> bool {
        self.total > 0 && self.usable == 0
    }
}

pub fn probe() -> Nodes {
    probe_in(Path::new(DIR), Path::new(SYSFS))
}

pub fn probe_in(dir: &Path, sysfs: &Path) -> Nodes {
    let mut nodes = Nodes::default();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return nodes;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if !name.to_string_lossy().starts_with("accel") || !is_intel(&sysfs.join(&name)) {
            continue;
        }
        nodes.total += 1;
        if may_open_rw(&entry.path()) {
            nodes.usable += 1;
        }
    }
    nodes
}

fn is_intel(class_dir: &Path) -> bool {
    match std::fs::read_link(class_dir.join("device/driver")) {
        Ok(driver) => driver.file_name().is_some_and(|n| n == INTEL_DRIVER),
        // No sysfs to read (some containers): count it, as before drivers were checked.
        Err(_) => true,
    }
}

fn may_open_rw(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(c) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    // SAFETY: `c` is a valid NUL-terminated path for the duration of the call.
    unsafe { libc::access(c.as_ptr(), libc::R_OK | libc::W_OK) == 0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn missing_dir_has_no_nodes() {
        let n = probe_in(
            Path::new("/nonexistent/accel"),
            Path::new("/nonexistent/sys"),
        );
        assert_eq!(n, Nodes::default());
        assert!(!n.denied());
    }

    #[test]
    fn counts_accel_nodes_and_which_are_usable() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("accel0"), b"").unwrap();
        std::fs::write(dir.path().join("other"), b"").unwrap();
        let n = probe_in(dir.path(), &dir.path().join("sys"));
        assert_eq!(
            n,
            Nodes {
                total: 1,
                usable: 1
            }
        );
        assert!(!n.denied());
    }

    /// Fakes `/sys/class/accel/<node>/device/driver` pointing at `driver`.
    fn bind(sysfs: &Path, node: &str, driver: &str) {
        let device = sysfs.join(node).join("device");
        std::fs::create_dir_all(&device).unwrap();
        let target = format!("../../../bus/pci/drivers/{driver}");
        std::os::unix::fs::symlink(target, device.join("driver")).unwrap();
    }

    #[test]
    fn counts_only_intel_vpu_nodes() {
        let dir = tempfile::tempdir().unwrap();
        let dev = dir.path().join("dev");
        let sys = dir.path().join("sys");
        std::fs::create_dir(&dev).unwrap();
        std::fs::write(dev.join("accel0"), b"").unwrap();
        std::fs::write(dev.join("accel1"), b"").unwrap();
        bind(&sys, "accel0", "amdxdna");
        bind(&sys, "accel1", INTEL_DRIVER);
        assert_eq!(probe_in(&dev, &sys).total, 1);
    }

    #[test]
    fn amd_npu_alone_is_not_an_intel_npu() {
        let dir = tempfile::tempdir().unwrap();
        let dev = dir.path().join("dev");
        let sys = dir.path().join("sys");
        std::fs::create_dir(&dev).unwrap();
        std::fs::write(dev.join("accel0"), b"").unwrap();
        bind(&sys, "accel0", "amdxdna");
        let n = probe_in(&dev, &sys);
        assert_eq!(n, Nodes::default());
        assert!(!n.denied());
    }

    #[test]
    fn unreadable_node_is_denied() {
        // root ignores mode bits, so this can only be checked as a normal user.
        if unsafe { libc::geteuid() } == 0 {
            eprintln!("skipping: running as root");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let node = dir.path().join("accel0");
        std::fs::write(&node, b"").unwrap();
        std::fs::set_permissions(&node, std::fs::Permissions::from_mode(0o000)).unwrap();
        let n = probe_in(dir.path(), &dir.path().join("sys"));
        assert_eq!(
            n,
            Nodes {
                total: 1,
                usable: 0
            }
        );
        assert!(n.denied());
    }
}

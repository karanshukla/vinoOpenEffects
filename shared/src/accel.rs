//! The Intel NPU's `/dev/accel` nodes, for the daemon's provider choice and `doctor`.

use std::path::Path;

pub const DIR: &str = "/dev/accel";

/// Debian/Ubuntu make accel nodes `root:render 0660` with no uaccess tag; Fedora makes them 0666.
pub const PERMISSION_FIX: &str = "sudo usermod -aG render $USER, then log out and back in (the NPU's /dev/accel nodes belong to the render group)";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Nodes {
    /// `accel*` nodes present.
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
    probe_in(Path::new(DIR))
}

pub fn probe_in(dir: &Path) -> Nodes {
    let mut nodes = Nodes::default();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return nodes;
    };
    for entry in entries.flatten() {
        if !entry.file_name().to_string_lossy().starts_with("accel") {
            continue;
        }
        nodes.total += 1;
        if may_open_rw(&entry.path()) {
            nodes.usable += 1;
        }
    }
    nodes
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
        let n = probe_in(Path::new("/nonexistent/accel"));
        assert_eq!(n, Nodes::default());
        assert!(!n.denied());
    }

    #[test]
    fn counts_accel_nodes_and_which_are_usable() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("accel0"), b"").unwrap();
        std::fs::write(dir.path().join("other"), b"").unwrap();
        let n = probe_in(dir.path());
        assert_eq!(
            n,
            Nodes {
                total: 1,
                usable: 1
            }
        );
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
        let n = probe_in(dir.path());
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

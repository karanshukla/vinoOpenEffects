use std::path::Path;

use super::types::{Check, Status};

pub struct DistroInfo {
    pub id: String,
    pub id_like: String,
    /// Booted from an rpm-ostree image (`/run/ostree-booted`): Silverblue, Kinoite, Bazzite.
    pub ostree: bool,
}

pub fn detect_distro(os_release: &str) -> DistroInfo {
    let mut id = String::new();
    let mut id_like = String::new();
    for line in os_release.lines() {
        let line = line.trim();
        if let Some(val) = line.strip_prefix("ID=") {
            id = val.trim_matches('"').to_ascii_lowercase();
        } else if let Some(val) = line.strip_prefix("ID_LIKE=") {
            id_like = val.trim_matches('"').to_ascii_lowercase();
        }
    }
    DistroInfo {
        id,
        id_like,
        ostree: false,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    Fedora,
    Debian,
    Arch,
    Suse,
    Other,
}

fn family(d: &DistroInfo) -> Family {
    let (id, like) = (d.id.as_str(), d.id_like.as_str());
    let is = |name: &str| id == name || like.split_whitespace().any(|l| l == name);
    if is("fedora") || is("rhel") || is("centos") {
        Family::Fedora
    } else if is("debian") || is("ubuntu") {
        Family::Debian
    } else if is("arch") {
        Family::Arch
    } else if id.starts_with("opensuse") || is("suse") || is("opensuse") {
        Family::Suse
    } else {
        Family::Other
    }
}

/// openSUSE's image-based editions, where zypper can't change the read-only root.
fn transactional_suse(d: &DistroInfo) -> bool {
    ["microos", "aeon", "kalpa"]
        .iter()
        .any(|name| d.id.contains(name))
}

/// One GStreamer element and the package that ships it in each distro family.
pub struct GstPlugin {
    pub element: &'static str,
    /// Plugin file name, for the fallback probe when `gst-inspect-1.0` isn't installed.
    pub file: &'static str,
    fedora: &'static str,
    debian: &'static str,
    arch: &'static str,
    suse: &'static str,
}

pub const GTK4_SINK: GstPlugin = GstPlugin {
    element: "gtk4paintablesink",
    file: "libgstgtk4.so",
    fedora: "gstreamer1-plugin-gtk4",
    debian: "gstreamer1.0-gtk4",
    arch: "gst-plugin-gtk4",
    // openSUSE ships the gtk4 sink inside the gst-plugins-rs bundle, not on its own.
    suse: "gstreamer-plugins-rs",
};

pub const PIPEWIRE_SRC: GstPlugin = GstPlugin {
    element: "pipewiresrc",
    file: "libgstpipewire.so",
    fedora: "pipewire-gstreamer",
    debian: "gstreamer1.0-pipewire",
    arch: "gst-plugin-pipewire",
    suse: "gstreamer-plugin-pipewire",
};

/// The package that provides `plugin` on this distro, and the command that installs it.
pub fn package_remedy(plugin: &GstPlugin, distro: Option<&DistroInfo>) -> (String, String) {
    let Some(d) = distro else {
        return generic_remedy(plugin);
    };
    match family(d) {
        Family::Fedora if d.ostree => (
            plugin.fedora.into(),
            format!("rpm-ostree install {}, then reboot", plugin.fedora),
        ),
        Family::Fedora => (
            plugin.fedora.into(),
            format!("sudo dnf install {}", plugin.fedora),
        ),
        Family::Debian => (
            plugin.debian.into(),
            format!("sudo apt install {}", plugin.debian),
        ),
        Family::Arch => (
            plugin.arch.into(),
            format!("sudo pacman -S {}", plugin.arch),
        ),
        Family::Suse if transactional_suse(d) => (
            plugin.suse.into(),
            format!(
                "sudo transactional-update pkg install {}, then reboot",
                plugin.suse
            ),
        ),
        Family::Suse => (
            plugin.suse.into(),
            format!("sudo zypper install {}", plugin.suse),
        ),
        Family::Other => generic_remedy(plugin),
    }
}

fn generic_remedy(plugin: &GstPlugin) -> (String, String) {
    (
        format!("the GStreamer plugin that provides {}", plugin.element),
        format!(
            "install your distro's package for {} ({} on Fedora, {} on Debian/Ubuntu)",
            plugin.element, plugin.fedora, plugin.debian
        ),
    )
}

pub fn gtk4_sink_verdict(sink_found: bool, distro: Option<&DistroInfo>) -> Check {
    if sink_found {
        Check::new(
            "gui",
            "sink",
            Status::Ok,
            "gtk4paintablesink element available for live preview",
            None,
        )
    } else {
        let (pkg, cmd) = package_remedy(&GTK4_SINK, distro);
        Check::new(
            "gui",
            "sink",
            Status::Fail,
            format!(
                "gtk4paintablesink element missing (needs {pkg}); live preview cannot render frames"
            ),
            Some(cmd),
        )
    }
}

/// The GUI preview reads the virtual camera with `pipewiresrc`; some distros package it apart from PipeWire.
pub fn pipewire_src_verdict(found: bool, distro: Option<&DistroInfo>) -> Check {
    if found {
        Check::new(
            "gui",
            "pipewiresrc",
            Status::Ok,
            "pipewiresrc element available for live preview",
            None,
        )
    } else {
        let (pkg, cmd) = package_remedy(&PIPEWIRE_SRC, distro);
        Check::new(
            "gui",
            "pipewiresrc",
            Status::Fail,
            format!(
                "pipewiresrc element missing (needs {pkg}); the live preview cannot read the virtual camera"
            ),
            Some(cmd),
        )
    }
}

/// The oldest libadwaita the GUI builds against (`v1_7` in `gui/Cargo.toml`).
const MIN_ADW: (u32, u32) = (1, 7);

fn major_minor(version: &str) -> Option<(u32, u32)> {
    let mut parts = version.split('.').map(|p| p.parse::<u32>().ok());
    Some((parts.next()??, parts.next()??))
}

pub fn gtk_versions_verdict(gtk_ver: Option<&str>, adw_ver: Option<&str>) -> Check {
    match (gtk_ver, adw_ver) {
        (Some(gtk), Some(adw)) if major_minor(adw).is_some_and(|v| v < MIN_ADW) => Check::new(
            "gui",
            "libraries",
            Status::Warn,
            format!(
                "GTK {gtk}, Libadwaita {adw}: the GUI needs Libadwaita {}.{} or newer",
                MIN_ADW.0, MIN_ADW.1
            ),
            Some(
                "use the CLI (scripts/install.sh --no-gui), or a newer distro release for the GUI"
                    .into(),
            ),
        ),
        (Some(gtk), Some(adw)) => Check::new(
            "gui",
            "libraries",
            Status::Ok,
            format!("GTK {gtk}, Libadwaita {adw}"),
            None,
        ),
        (Some(gtk), None) => Check::new("gui", "libraries", Status::Ok, format!("GTK {gtk}"), None),
        _ => Check::new(
            "gui",
            "libraries",
            Status::Info,
            "GTK / Libadwaita versions not queried",
            None,
        ),
    }
}

pub fn probe_gtk4_sink() -> bool {
    probe_gst_plugin(&GTK4_SINK)
}

pub fn probe_pipewire_src() -> bool {
    probe_gst_plugin(&PIPEWIRE_SRC)
}

fn probe_gst_plugin(plugin: &GstPlugin) -> bool {
    use std::process::Command;

    if Command::new("gst-inspect-1.0")
        .arg(plugin.element)
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
    {
        return true;
    }

    let plugin_dirs = [
        "/usr/lib64/gstreamer-1.0",
        "/usr/lib/gstreamer-1.0",
        "/usr/lib/x86_64-linux-gnu/gstreamer-1.0",
        "/usr/lib/aarch64-linux-gnu/gstreamer-1.0",
        "/usr/local/lib/gstreamer-1.0",
    ];
    plugin_dirs
        .iter()
        .any(|dir| Path::new(dir).join(plugin.file).exists())
}

pub fn probe_distro() -> Option<DistroInfo> {
    std::fs::read_to_string("/etc/os-release")
        .or_else(|_| std::fs::read_to_string("/usr/lib/os-release"))
        .ok()
        .map(|c| DistroInfo {
            ostree: Path::new("/run/ostree-booted").exists(),
            ..detect_distro(&c)
        })
}

pub fn probe_gtk_versions() -> (Option<String>, Option<String>) {
    use std::process::Command;

    let gtk = Command::new("pkg-config")
        .args(["--modversion", "gtk4"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());

    let adw = Command::new("pkg-config")
        .args(["--modversion", "libadwaita-1"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());

    (gtk, adw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_distro_detection() {
        let fedora = "NAME=\"Fedora Linux\"\nID=fedora\nVERSION_ID=41\n";
        let info = detect_distro(fedora);
        assert_eq!(info.id, "fedora");

        let ubuntu = "NAME=\"Ubuntu\"\nID=ubuntu\nID_LIKE=debian\n";
        let info = detect_distro(ubuntu);
        assert_eq!(info.id, "ubuntu");
        assert_eq!(info.id_like, "debian");
    }

    #[test]
    fn test_gtk4_sink_verdict_fedora_names_gstreamer1_plugin_gtk4() {
        let distro = DistroInfo {
            id: "fedora".into(),
            id_like: "".into(),
            ostree: false,
        };
        let c = gtk4_sink_verdict(false, Some(&distro));
        assert_eq!(c.status, Status::Fail);
        assert!(c.detail.contains("gstreamer1-plugin-gtk4"));
        assert_eq!(
            c.fix,
            Some("sudo dnf install gstreamer1-plugin-gtk4".into())
        );
    }

    #[test]
    fn test_gtk4_sink_verdict_ubuntu_names_pkg() {
        let distro = DistroInfo {
            id: "ubuntu".into(),
            id_like: "debian".into(),
            ostree: false,
        };
        let c = gtk4_sink_verdict(false, Some(&distro));
        assert_eq!(c.status, Status::Fail);
        assert!(c.detail.contains("gstreamer1.0-gtk4"));
        assert_eq!(c.fix, Some("sudo apt install gstreamer1.0-gtk4".into()));
    }

    fn distro(os_release: &str, ostree: bool) -> DistroInfo {
        DistroInfo {
            ostree,
            ..detect_distro(os_release)
        }
    }

    #[test]
    fn atomic_fedora_layers_with_rpm_ostree() {
        let kinoite = distro("ID=fedora\nVARIANT_ID=kinoite\n", true);
        let (_, cmd) = package_remedy(&GTK4_SINK, Some(&kinoite));
        assert_eq!(
            cmd,
            "rpm-ostree install gstreamer1-plugin-gtk4, then reboot"
        );
        let bazzite = distro("ID=bazzite\nID_LIKE=\"fedora\"\n", true);
        let (_, cmd) = package_remedy(&PIPEWIRE_SRC, Some(&bazzite));
        assert_eq!(cmd, "rpm-ostree install pipewire-gstreamer, then reboot");
    }

    #[test]
    fn opensuse_names_the_rs_bundle_and_transactional_update_on_aeon() {
        let tw = distro(
            "ID=\"opensuse-tumbleweed\"\nID_LIKE=\"opensuse suse\"\n",
            false,
        );
        let (pkg, cmd) = package_remedy(&GTK4_SINK, Some(&tw));
        assert_eq!(pkg, "gstreamer-plugins-rs");
        assert_eq!(cmd, "sudo zypper install gstreamer-plugins-rs");
        let aeon = distro("ID=\"opensuse-aeon\"\nID_LIKE=\"suse opensuse\"\n", false);
        let (_, cmd) = package_remedy(&PIPEWIRE_SRC, Some(&aeon));
        assert_eq!(
            cmd,
            "sudo transactional-update pkg install gstreamer-plugin-pipewire, then reboot"
        );
    }

    #[test]
    fn arch_and_debian_name_their_pipewire_plugin() {
        let arch = distro("ID=arch\n", false);
        assert_eq!(
            package_remedy(&PIPEWIRE_SRC, Some(&arch)).1,
            "sudo pacman -S gst-plugin-pipewire"
        );
        let pop = distro("ID=pop\nID_LIKE=\"ubuntu debian\"\n", false);
        assert_eq!(
            package_remedy(&PIPEWIRE_SRC, Some(&pop)).1,
            "sudo apt install gstreamer1.0-pipewire"
        );
    }

    #[test]
    fn unknown_distro_names_the_element_not_a_package_manager() {
        let void = distro("ID=void\n", false);
        let (_, cmd) = package_remedy(&GTK4_SINK, Some(&void));
        assert!(cmd.contains("gtk4paintablesink"), "{cmd}");
        assert!(!cmd.starts_with("sudo"), "{cmd}");
    }

    #[test]
    fn pipewire_src_verdict_fails_with_a_fix() {
        assert_eq!(pipewire_src_verdict(true, None).status, Status::Ok);
        let c = pipewire_src_verdict(false, Some(&distro("ID=fedora\n", false)));
        assert_eq!(c.status, Status::Fail);
        assert_eq!(
            c.fix.as_deref(),
            Some("sudo dnf install pipewire-gstreamer")
        );
    }

    #[test]
    fn old_libadwaita_warns_that_the_gui_will_not_run() {
        let c = gtk_versions_verdict(Some("4.14.5"), Some("1.5.0"));
        assert_eq!(c.status, Status::Warn);
        assert!(c.detail.contains("1.7"), "{}", c.detail);
        assert_eq!(
            gtk_versions_verdict(Some("4.20.1"), Some("1.7.6")).status,
            Status::Ok
        );
        assert_eq!(
            gtk_versions_verdict(Some("4.20.1"), Some("1.10.0")).status,
            Status::Ok
        );
    }

    #[test]
    fn test_gtk4_sink_verdict_ok() {
        let c = gtk4_sink_verdict(true, None);
        assert_eq!(c.status, Status::Ok);
    }
}

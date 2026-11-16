use super::types::{CameraClass, CameraNode, Check, Status};

pub const COLOR_FORMATS: &[&str] = &[
    "MJPG", "JPEG", "YUYV", "NV12", "NV21", "RGB3", "BGR3", "RGB4", "BGR4", "UYVY", "YV12", "YU12",
];

pub const IR_FORMATS: &[&str] = &["GREY", "Y16", "Y10", "Y8", "Z16", "INZI"];

pub fn classify_formats(formats: &[String], is_meta: bool) -> CameraClass {
    if is_meta || formats.is_empty() {
        return CameraClass::Metadata;
    }
    if formats.iter().any(|f| COLOR_FORMATS.contains(&f.as_str())) {
        return CameraClass::Rgb;
    }
    if formats.iter().any(|f| IR_FORMATS.contains(&f.as_str())) {
        return CameraClass::Ir;
    }
    CameraClass::Unknown
}

pub fn camera_list_verdict(cameras: &[CameraNode]) -> Check {
    if cameras.is_empty() {
        return Check::new(
            "camera",
            "devices",
            Status::Fail,
            "no cameras detected under /dev/video*",
            Some("connect a camera or verify camera permissions".into()),
        );
    }

    let summary: Vec<String> = cameras
        .iter()
        .map(|c| format!("{} [{}]", c.path, class_tag(c.class)))
        .collect();

    Check::new(
        "camera",
        "devices",
        Status::Ok,
        format!("{} device(s): {}", cameras.len(), summary.join(", ")),
        None,
    )
}

fn class_tag(class: CameraClass) -> &'static str {
    match class {
        CameraClass::Rgb => "RGB",
        CameraClass::Ir => "IR",
        CameraClass::Metadata => "meta",
        CameraClass::Unknown => "unknown",
    }
}

pub fn selected_camera_verdict(selected: &str, cameras: &[CameraNode]) -> Check {
    let rgb_alt = cameras.iter().find(|c| c.class == CameraClass::Rgb);

    if selected.is_empty() {
        return auto_select_verdict(rgb_alt, cameras.is_empty());
    }

    let found = cameras
        .iter()
        .find(|c| c.path == selected || c.name == selected);

    match found {
        Some(node) => configured_camera_verdict(node, rgb_alt),
        None => selected_check(
            Status::Warn,
            format!("configured camera '{selected}' was not found in enumerated devices"),
            Some("vinoopeneffects-ctl camera list".into()),
        ),
    }
}

fn selected_check(status: Status, detail: impl Into<String>, fix: Option<String>) -> Check {
    Check::new("camera", "selected", status, detail, fix)
}

fn auto_select_verdict(rgb: Option<&CameraNode>, no_cameras: bool) -> Check {
    match rgb {
        Some(rgb) => selected_check(
            Status::Ok,
            format!("auto-select (picks RGB camera {})", rgb.path),
            None,
        ),
        None if no_cameras => selected_check(
            Status::Fail,
            "no camera available to auto-select",
            Some("connect a USB or integrated webcam".into()),
        ),
        None => selected_check(
            Status::Warn,
            "auto-select found no RGB cameras (only IR/metadata nodes present)",
            None,
        ),
    }
}

fn configured_camera_verdict(node: &CameraNode, rgb_alt: Option<&CameraNode>) -> Check {
    let switch_to_rgb = || rgb_alt.map(|alt| format!("vinoopeneffects-ctl camera {}", alt.path));
    match node.class {
        CameraClass::Rgb => selected_check(
            Status::Ok,
            format!("{} ({}) is RGB", node.path, node.name),
            None,
        ),
        CameraClass::Ir => {
            let detail = match rgb_alt {
                Some(alt) => format!(
                    "{} is an IR camera (greyscale); prefer RGB alternative ({})",
                    node.path, alt.path
                ),
                None => format!(
                    "{} is an IR camera (greyscale); no RGB alternative detected",
                    node.path
                ),
            };
            selected_check(Status::Warn, detail, switch_to_rgb())
        }
        CameraClass::Metadata => selected_check(
            Status::Fail,
            format!(
                "{} is a metadata node, not a video capture device",
                node.path
            ),
            switch_to_rgb(),
        ),
        CameraClass::Unknown => selected_check(
            Status::Warn,
            format!(
                "{} has unrecognized formats ({})",
                node.path,
                node.formats.join(", ")
            ),
            None,
        ),
    }
}

const V4L2_CAP_META_CAPTURE: u32 = 0x00800000;
const V4L2_CAP_DEVICE_CAPS: u32 = 0x80000000;
const MAX_ENUMERATED_FORMATS: u32 = 128;

fn is_meta_capture(capabilities: u32, device_caps: u32) -> bool {
    let effective = if capabilities & V4L2_CAP_DEVICE_CAPS != 0 {
        device_caps
    } else {
        capabilities
    };
    effective & V4L2_CAP_META_CAPTURE != 0
}

fn fourcc_name(pixelformat: u32) -> String {
    let fourcc: String = pixelformat
        .to_le_bytes()
        .iter()
        .map(|&b| {
            if b.is_ascii_graphic() || b == b' ' {
                b as char
            } else {
                '?'
            }
        })
        .collect();
    fourcc.trim().to_string()
}

fn push_distinct_format(formats: &mut Vec<String>, name: String) {
    if !name.is_empty() && !formats.contains(&name) {
        formats.push(name);
    }
}

#[cfg(target_os = "linux")]
pub fn probe_cameras() -> Vec<CameraNode> {
    use std::fs;

    let mut nodes = Vec::new();
    let Ok(entries) = fs::read_dir("/sys/class/video4linux") else {
        return nodes;
    };

    let mut dev_names: Vec<String> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("video"))
        .collect();

    dev_names.sort_by_key(|n| {
        n.trim_start_matches("video")
            .parse::<u32>()
            .unwrap_or(u32::MAX)
    });

    for dev in dev_names {
        let path = format!("/dev/{dev}");
        let sys_name = fs::read_to_string(format!("/sys/class/video4linux/{dev}/name"))
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|_| "Unknown".into());

        let (is_meta, formats) = query_v4l2_caps_and_formats(&path);
        let class = classify_formats(&formats, is_meta);

        nodes.push(CameraNode {
            path,
            name: sys_name,
            class,
            formats,
        });
    }

    nodes
}

#[cfg(target_os = "linux")]
fn query_v4l2_caps_and_formats(device_path: &str) -> (bool, Vec<String>) {
    use std::fs::OpenOptions;
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::OpenOptionsExt;

    #[repr(C)]
    struct v4l2_capability {
        driver: [u8; 16],
        card: [u8; 32],
        bus_info: [u8; 32],
        version: u32,
        capabilities: u32,
        device_caps: u32,
        reserved: [u32; 3],
    }

    #[repr(C)]
    struct v4l2_fmtdesc {
        index: u32,
        type_: u32,
        flags: u32,
        description: [u8; 32],
        pixelformat: u32,
        mbus_code: u32,
        reserved: [u32; 3],
    }

    const VIDIOC_QUERYCAP: libc::c_ulong = 0x80685600;
    const VIDIOC_ENUM_FMT: libc::c_ulong = 0xc0405602;
    const V4L2_BUF_TYPE_VIDEO_CAPTURE: u32 = 1;

    let Ok(file) = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(device_path)
    else {
        return (false, Vec::new());
    };
    let fd = file.as_raw_fd();

    let mut cap: v4l2_capability = unsafe { std::mem::zeroed() };
    let is_meta = unsafe { libc::ioctl(fd, VIDIOC_QUERYCAP, &mut cap) } == 0
        && is_meta_capture(cap.capabilities, cap.device_caps);

    let mut formats = Vec::new();
    let mut idx = 0u32;
    loop {
        let mut fmt: v4l2_fmtdesc = unsafe { std::mem::zeroed() };
        fmt.index = idx;
        fmt.type_ = V4L2_BUF_TYPE_VIDEO_CAPTURE;
        if unsafe { libc::ioctl(fd, VIDIOC_ENUM_FMT, &mut fmt) } != 0 {
            break;
        }
        push_distinct_format(&mut formats, fourcc_name(fmt.pixelformat));
        idx += 1;
        if idx > MAX_ENUMERATED_FORMATS {
            break;
        }
    }

    (is_meta, formats)
}

#[cfg(not(target_os = "linux"))]
pub fn probe_cameras() -> Vec<CameraNode> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(path: &str, class: CameraClass) -> CameraNode {
        let formats = match class {
            CameraClass::Rgb => vec!["YUYV".into()],
            CameraClass::Ir => vec!["GREY".into()],
            CameraClass::Metadata => Vec::new(),
            CameraClass::Unknown => vec!["FOOO".into()],
        };
        CameraNode {
            path: path.into(),
            name: format!("name of {path}"),
            class,
            formats,
        }
    }

    #[test]
    fn no_cameras_fails_the_device_list() {
        let check = camera_list_verdict(&[]);
        assert_eq!(check.status, Status::Fail);
        assert!(check.fix.is_some());
    }

    #[test]
    fn one_camera_of_any_class_passes_the_device_list() {
        let check = camera_list_verdict(&[node("/dev/video3", CameraClass::Metadata)]);
        assert_eq!(check.status, Status::Ok);
        assert_eq!(check.detail, "1 device(s): /dev/video3 [meta]");
        assert_eq!(check.fix, None);
    }

    #[test]
    fn the_device_list_tags_every_class() {
        let cameras = [
            node("/dev/video0", CameraClass::Rgb),
            node("/dev/video2", CameraClass::Ir),
            node("/dev/video4", CameraClass::Unknown),
        ];
        assert_eq!(
            camera_list_verdict(&cameras).detail,
            "3 device(s): /dev/video0 [RGB], /dev/video2 [IR], /dev/video4 [unknown]"
        );
    }

    #[test]
    fn auto_select_with_an_rgb_camera_is_ok() {
        let cameras = [
            node("/dev/video2", CameraClass::Ir),
            node("/dev/video0", CameraClass::Rgb),
        ];
        let check = selected_camera_verdict("", &cameras);
        assert_eq!(check.status, Status::Ok);
        assert!(check.detail.contains("/dev/video0"));
    }

    #[test]
    fn auto_select_without_an_rgb_camera_warns() {
        let check = selected_camera_verdict("", &[node("/dev/video2", CameraClass::Ir)]);
        assert_eq!(check.status, Status::Warn);
        assert_eq!(check.fix, None);
    }

    #[test]
    fn auto_select_with_no_camera_at_all_fails() {
        let check = selected_camera_verdict("", &[]);
        assert_eq!(check.status, Status::Fail);
        assert!(check.fix.is_some());
    }

    #[test]
    fn a_configured_camera_is_found_by_path_or_by_name() {
        let cameras = [node("/dev/video0", CameraClass::Rgb)];
        for selected in ["/dev/video0", "name of /dev/video0"] {
            assert_eq!(
                selected_camera_verdict(selected, &cameras).status,
                Status::Ok
            );
        }
    }

    #[test]
    fn a_configured_camera_that_is_not_enumerated_warns() {
        let cameras = [node("/dev/video0", CameraClass::Rgb)];
        let check = selected_camera_verdict("/dev/video9", &cameras);
        assert_eq!(check.status, Status::Warn);
        assert!(check.detail.contains("/dev/video9"));
        assert_eq!(check.fix, Some("vinoopeneffects-ctl camera list".into()));
    }

    #[test]
    fn an_ir_camera_without_an_rgb_alternative_warns_with_no_fix() {
        let check = selected_camera_verdict("/dev/video2", &[node("/dev/video2", CameraClass::Ir)]);
        assert_eq!(check.status, Status::Warn);
        assert!(check.detail.contains("no RGB alternative"));
        assert_eq!(check.fix, None);
    }

    #[test]
    fn a_metadata_node_fails_and_offers_the_rgb_camera_when_there_is_one() {
        let alone = [node("/dev/video1", CameraClass::Metadata)];
        let check = selected_camera_verdict("/dev/video1", &alone);
        assert_eq!(check.status, Status::Fail);
        assert_eq!(check.fix, None);

        let with_rgb = [
            node("/dev/video0", CameraClass::Rgb),
            node("/dev/video1", CameraClass::Metadata),
        ];
        let check = selected_camera_verdict("/dev/video1", &with_rgb);
        assert_eq!(check.status, Status::Fail);
        assert_eq!(
            check.fix,
            Some("vinoopeneffects-ctl camera /dev/video0".into())
        );
    }

    #[test]
    fn unrecognised_formats_warn_and_are_listed() {
        let cameras = [
            node("/dev/video0", CameraClass::Rgb),
            node("/dev/video4", CameraClass::Unknown),
        ];
        let check = selected_camera_verdict("/dev/video4", &cameras);
        assert_eq!(check.status, Status::Warn);
        assert!(check.detail.contains("FOOO"));
        assert_eq!(check.fix, None);
    }

    #[test]
    fn device_caps_decide_meta_capture_only_when_the_driver_reports_them() {
        let meta = V4L2_CAP_META_CAPTURE;
        let has_device_caps = V4L2_CAP_DEVICE_CAPS;
        assert!(is_meta_capture(has_device_caps, meta));
        assert!(!is_meta_capture(has_device_caps | meta, 0));
        assert!(is_meta_capture(meta, 0));
        assert!(!is_meta_capture(0, meta));
    }

    #[test]
    fn a_fourcc_reads_low_byte_first_and_drops_padding() {
        assert_eq!(fourcc_name(u32::from_le_bytes(*b"MJPG")), "MJPG");
        assert_eq!(fourcc_name(u32::from_le_bytes(*b"Y16 ")), "Y16");
        assert_eq!(fourcc_name(u32::from_le_bytes(*b"    ")), "");
    }

    #[test]
    fn a_fourcc_byte_outside_printable_ascii_becomes_a_question_mark() {
        assert_eq!(
            fourcc_name(u32::from_le_bytes([b'Y', b'~', 0x7f, 0])),
            "Y~??"
        );
        assert_eq!(
            fourcc_name(u32::from_le_bytes([b'!', 0x1f, b'8', 0x80])),
            "!?8?"
        );
    }

    #[test]
    fn formats_are_listed_once_and_blank_ones_skipped() {
        let mut formats = Vec::new();
        for name in ["YUYV", "", "MJPG", "YUYV"] {
            push_distinct_format(&mut formats, name.to_string());
        }
        assert_eq!(formats, ["YUYV", "MJPG"]);
    }

    #[test]
    fn test_format_classification() {
        assert_eq!(
            classify_formats(&["YUYV".into(), "MJPG".into()], false),
            CameraClass::Rgb
        );
        assert_eq!(classify_formats(&["GREY".into()], false), CameraClass::Ir);
        assert_eq!(classify_formats(&["Y16".into()], false), CameraClass::Ir);
        assert_eq!(classify_formats(&[], true), CameraClass::Metadata);
        assert_eq!(classify_formats(&[], false), CameraClass::Metadata);
        assert_eq!(
            classify_formats(&["FOOO".into()], false),
            CameraClass::Unknown
        );
    }

    #[test]
    fn test_selected_camera_ir_warns_with_rgb_alt() {
        let cameras = vec![
            CameraNode {
                path: "/dev/video0".into(),
                name: "Integrated_Webcam_FHD".into(),
                class: CameraClass::Rgb,
                formats: vec!["YUYV".into()],
            },
            CameraNode {
                path: "/dev/video2".into(),
                name: "Integrated_Webcam_FHD".into(),
                class: CameraClass::Ir,
                formats: vec!["GREY".into()],
            },
        ];

        let check = selected_camera_verdict("/dev/video2", &cameras);
        assert_eq!(check.status, Status::Warn);
        assert!(check.detail.contains("IR camera"));
        assert!(check.detail.contains("/dev/video0"));
        assert_eq!(
            check.fix,
            Some("vinoopeneffects-ctl camera /dev/video0".into())
        );
    }

    #[test]
    fn test_selected_camera_rgb_ok() {
        let cameras = vec![CameraNode {
            path: "/dev/video0".into(),
            name: "Cam".into(),
            class: CameraClass::Rgb,
            formats: vec!["MJPG".into()],
        }];
        let check = selected_camera_verdict("/dev/video0", &cameras);
        assert_eq!(check.status, Status::Ok);
    }
}

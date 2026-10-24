pub mod camera;
pub mod config;
pub mod daemon;
pub mod gui;
pub mod inference;
pub mod models;
pub mod pipeline;
pub mod types;

pub use types::{CameraClass, CameraNode, Check, Status};

/// Run all doctor probes on the current system.
///
/// `dbus_status`: Pass `Ok(status_str)` if `Daemon1` replied over D-Bus, or `Err(err_str)`.
pub fn run_all(dbus_status: Result<&str, &str>) -> Vec<Check> {
    let mut checks = Vec::new();
    let home = dirs::home_dir();
    let home_ref = home.as_deref();

    let act_files = daemon::probe_activation_files(home_ref);
    let (unit_state, unit_enabled) = daemon::probe_systemd_unit();
    checks.push(daemon::dbus_verdict(
        dbus_status,
        Some(&act_files),
        Some(unit_state),
    ));
    checks.push(daemon::activation_file_verdict(&act_files));
    checks.push(daemon::systemd_unit_verdict(unit_state, unit_enabled));
    checks.push(daemon::session_target_verdict(
        crate::systemd::session_target_active(),
        unit_enabled,
    ));

    checks.push(inference::build_feature_verdict(
        inference::probe_built_with_npu(),
    ));
    let (ort_lib, has_openvino) = inference::probe_ort_runtime(home_ref);
    checks.push(inference::ort_library_verdict(
        ort_lib.as_deref(),
        has_openvino,
    ));
    let accel_count = inference::probe_npu_devices();
    checks.push(inference::npu_hardware_verdict(accel_count));
    let record = inference::probe_npu_failure(ort_lib.as_deref());
    checks.push(inference::npu_failure_verdict(record.as_ref()));

    let search_dirs = models::model_search_dirs(home_ref);
    for m in models::KNOWN_MODELS {
        let (manifest_found, file_found, sha_match, model_path) =
            models::probe_model(&search_dirs, m.id);
        checks.push(models::model_verdict(
            m.id,
            m.name,
            m.required,
            manifest_found,
            file_found,
            sha_match,
            model_path.as_deref(),
        ));
    }

    let cameras = camera::probe_cameras();
    checks.push(camera::camera_list_verdict(&cameras));
    let (_cfg_path, cfg_read) = config::probe_config(home_ref);
    let selected_cam = cfg_read
        .as_ref()
        .ok()
        .and_then(|text| toml::from_str::<crate::config::AppState>(text).ok())
        .map(|s| s.camera.selected)
        .unwrap_or_default();
    checks.push(camera::selected_camera_verdict(&selected_cam, &cameras));

    let pw_running = pipeline::probe_pipewire(None);
    checks.push(pipeline::pipewire_verdict(pw_running));
    let vcam_present = pipeline::probe_virtual_camera();
    checks.push(pipeline::virtual_camera_verdict(vcam_present));

    let distro = gui::probe_distro();
    let sink_found = gui::probe_gtk4_sink();
    checks.push(gui::gtk4_sink_verdict(sink_found, distro.as_ref()));
    let pw_src_found = gui::probe_pipewire_src();
    checks.push(gui::pipewire_src_verdict(pw_src_found, distro.as_ref()));
    let (gtk_ver, adw_ver) = gui::probe_gtk_versions();
    checks.push(gui::gtk_versions_verdict(
        gtk_ver.as_deref(),
        adw_ver.as_deref(),
    ));

    let (cfg_path, cfg_res) = config::probe_config(home_ref);
    checks.push(config::config_verdict(&cfg_path, cfg_res.as_deref()));

    checks
}

/// Print the human-readable report to stdout and return the exit code: 1 if any check failed, 0 otherwise.
pub fn print_report(checks: &[Check]) -> i32 {
    for c in checks {
        let tag = c.status.tag();
        let target = format!("{}: {}", c.group, c.name);
        println!("[{tag}] {:<28} {}", target, c.detail);
        if let Some(fix) = &c.fix {
            println!("       fix: {fix}");
        }
    }

    let fails = checks.iter().filter(|c| c.status == Status::Fail).count();
    let warns = checks.iter().filter(|c| c.status == Status::Warn).count();

    println!("\n{fails} failed, {warns} warning(s)");
    (fails > 0) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(status: Status) -> Check {
        Check::new("group", "name", status, "detail", None)
    }

    #[test]
    fn print_report_exit_code_tracks_failures() {
        assert_eq!(print_report(&[]), 0);
        assert_eq!(
            print_report(&[check(Status::Ok), check(Status::Warn), check(Status::Info)]),
            0
        );
        assert_eq!(print_report(&[check(Status::Ok), check(Status::Fail)]), 1);
    }

    #[test]
    fn check_json_shape_is_stable() {
        let json = serde_json::to_value(check(Status::Warn)).unwrap();
        assert_eq!(json["status"], "warn");
        assert!(json.get("fix").is_none(), "fix omitted when None");

        let with_fix = Check::new("g", "n", Status::Fail, "d", Some("do x".into()));
        let json = serde_json::to_value(with_fix).unwrap();
        assert_eq!(json["status"], "fail");
        assert_eq!(json["fix"], "do x");
    }

    #[test]
    fn status_tags_line_up() {
        for s in [Status::Ok, Status::Warn, Status::Fail, Status::Info] {
            assert_eq!(s.tag().len(), 4, "{s:?}");
        }
    }

    #[test]
    fn run_all_produces_a_check_per_probe() {
        let checks = run_all(Err("daemon is not running"));
        assert_eq!(checks.len(), 15 + models::KNOWN_MODELS.len());
        for c in &checks {
            assert!(!c.group.is_empty() && !c.name.is_empty(), "{c:?}");
        }
    }
}

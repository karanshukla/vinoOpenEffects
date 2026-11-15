use shared::dbus::{SERVICE_NAME, VariantMap, value_as_bool};
use zbus::Connection;

use crate::args::{AutostartCommand, CameraCommand};
use crate::bus::{daemon_proxy, devices_proxy};
use crate::values::{format_params, simple_value};

pub(crate) async fn start(conn: &Connection) -> anyhow::Result<()> {
    daemon_proxy(conn)
        .await?
        .call::<_, _, ()>("Start", &())
        .await?;
    Ok(())
}

pub(crate) async fn stop(conn: &Connection) -> anyhow::Result<()> {
    daemon_proxy(conn)
        .await?
        .call::<_, _, ()>("Stop", &())
        .await?;
    Ok(())
}

pub(crate) async fn status(conn: &Connection, json: bool, short: bool) -> anyhow::Result<()> {
    let daemon = daemon_proxy(conn).await?;
    let status = daemon.get_property::<String>("Status").await?;
    if short {
        println!("{status}");
        return Ok(());
    }
    let caps = daemon
        .get_property::<VariantMap>("Capabilities")
        .await
        .unwrap_or_default();
    let inference = simple_value(&caps, "inference_active").unwrap_or_else(|| "cpu".into());
    let models = match simple_value(&caps, "models_ready").as_deref() {
        Some("true") => "ready",
        _ => "missing",
    };
    let sink = simple_value(&caps, "output_sink").unwrap_or_else(|| "unknown".into());
    let degraded = caps
        .get("inference_degraded")
        .and_then(value_as_bool)
        .unwrap_or(false);
    if json {
        let requested = simple_value(&caps, "inference_requested");
        let available = caps
            .get("inference_available")
            .and_then(shared::dbus::value_as_str_array);
        let detail = simple_value(&caps, "inference_detail");
        println!(
            "{}",
            serde_json::json!({
                "status": status,
                "inference": inference,
                "inference_requested": requested,
                "inference_available": available,
                "inference_degraded": degraded,
                "inference_detail": detail,
                "models": models,
                "sink": sink,
            })
        );
    } else {
        println!("{status} (inference: {inference}, models: {models}, sink: {sink})");
        if degraded {
            let why = simple_value(&caps, "inference_detail").unwrap_or_default();
            println!("warning: an NPU is present but effects run on {inference}: {why}");
        }
    }
    Ok(())
}

pub(crate) async fn inference(
    conn: &Connection,
    provider: Option<String>,
    retry: bool,
    json: bool,
) -> anyhow::Result<()> {
    let daemon = daemon_proxy(conn).await?;
    if retry {
        let cleared: bool = daemon.call("RetryInference", &()).await?;
        if !json {
            println!(
                "{}",
                if cleared {
                    "cleared the NPU failure record; reloading"
                } else {
                    "no NPU failure recorded; reloading"
                }
            );
        }
    }
    if let Some(target) = provider {
        daemon
            .call::<_, _, ()>("SetInferenceProvider", &(target.as_str()))
            .await?;
    }
    let caps = daemon
        .get_property::<VariantMap>("Capabilities")
        .await
        .unwrap_or_default();
    let requested = simple_value(&caps, "inference_requested").unwrap_or_else(|| "auto".into());
    let active = simple_value(&caps, "inference_active").unwrap_or_else(|| "cpu".into());
    let available = caps
        .get("inference_available")
        .and_then(shared::dbus::value_as_str_array)
        .unwrap_or_else(|| vec!["cpu".into()]);
    let detail = simple_value(&caps, "inference_detail");
    let degraded = caps
        .get("inference_degraded")
        .and_then(value_as_bool)
        .unwrap_or(false);

    if json {
        println!(
            "{}",
            serde_json::json!({
                "requested": requested,
                "active": active,
                "available": available,
                "degraded": degraded,
                "detail": detail,
            })
        );
    } else {
        println!("requested: {requested}");
        println!(
            "active:    {active}{}",
            if degraded {
                " (degraded: an NPU is present but unused)"
            } else {
                ""
            }
        );
        println!("available: {}", available.join(", "));
        if let Some(d) = detail {
            println!("detail:    {d}");
        }
    }
    Ok(())
}

pub(crate) async fn camera(conn: &Connection, command: CameraCommand) -> anyhow::Result<()> {
    let devices = devices_proxy(conn).await?;
    match command {
        CameraCommand::List => {
            let cameras: Vec<VariantMap> = devices.call("ListCameras", &()).await?;
            for camera in cameras {
                let id = simple_value(&camera, "id").unwrap_or_default();
                let name = simple_value(&camera, "name").unwrap_or_else(|| id.clone());
                let active = camera
                    .get("active")
                    .and_then(value_as_bool)
                    .unwrap_or(false);
                let mark = if active { "* " } else { "  " };
                println!("{mark}{id}\t{name}");
            }
        }
        CameraCommand::Select { id } => {
            devices
                .call::<_, _, ()>("SelectCamera", &(id.as_str()))
                .await?
        }
        CameraCommand::Info => {
            let info = devices
                .get_property::<VariantMap>("VirtualCameraInfo")
                .await
                .unwrap_or_default();
            println!("{}", format_params(&info));
        }
    }
    Ok(())
}

pub(crate) async fn doctor(json: bool) -> anyhow::Result<()> {
    let dbus_status_res: Result<String, String> = async {
        let conn = Connection::session().await.map_err(|e| e.to_string())?;
        let dbus = zbus::fdo::DBusProxy::new(&conn)
            .await
            .map_err(|e| e.to_string())?;
        let name = zbus::names::BusName::try_from(SERVICE_NAME).map_err(|e| e.to_string())?;
        let has_owner = dbus.name_has_owner(name).await.map_err(|e| e.to_string())?;
        if !has_owner {
            return Err("daemon is not running".to_string());
        }
        let daemon = daemon_proxy(&conn).await.map_err(|e| e.to_string())?;
        let status = daemon
            .get_property::<String>("Status")
            .await
            .map_err(|e| e.to_string())?;
        Ok(status)
    }
    .await;

    let dbus_status_ref = match &dbus_status_res {
        Ok(s) => Ok(s.as_str()),
        Err(e) => Err(e.as_str()),
    };

    let checks = shared::doctor::run_all(dbus_status_ref);

    if json {
        let json_str = serde_json::to_string_pretty(&checks)?;
        println!("{json_str}");
    } else {
        shared::doctor::print_report(&checks);
    }

    if checks
        .iter()
        .any(|c| c.status == shared::doctor::Status::Fail)
    {
        std::process::exit(1);
    }

    Ok(())
}

/// Enable/disable/report the daemon's systemd user unit.
pub(crate) fn autostart(command: &AutostartCommand) -> anyhow::Result<()> {
    match command {
        AutostartCommand::Enable => {
            shared::systemd::set_enabled(true).map_err(|e| anyhow::anyhow!(e))?;
            println!(
                "autostart enabled: {} will start with the graphical session",
                shared::systemd::UNIT
            );
            if shared::systemd::session_target_active() == Some(false) {
                eprintln!(
                    "note: {} isn't active in this session, so this won't fire here; {}",
                    shared::systemd::SESSION_TARGET,
                    shared::systemd::NO_SESSION_TARGET_FIX
                );
            }
        }
        AutostartCommand::Disable => {
            shared::systemd::set_enabled(false).map_err(|e| anyhow::anyhow!(e))?;
            println!("autostart disabled");
        }
        AutostartCommand::Status => {
            println!(
                "autostart: {}",
                if shared::systemd::is_enabled() {
                    "enabled"
                } else {
                    "disabled"
                }
            );
            println!(
                "daemon:    {}",
                if shared::systemd::is_active() {
                    "running"
                } else {
                    "stopped"
                }
            );
            if shared::systemd::session_target_active() == Some(false) {
                println!(
                    "session:   {} inactive, so autostart won't fire; {}",
                    shared::systemd::SESSION_TARGET,
                    shared::systemd::NO_SESSION_TARGET_FIX
                );
            }
        }
    }
    Ok(())
}

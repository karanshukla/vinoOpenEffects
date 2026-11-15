use shared::dbus::{
    DAEMON_INTERFACE, DEVICES_INTERFACE, EFFECTS_INTERFACE, OBJECT_PATH, SERVICE_NAME,
};
use zbus::{Connection, Proxy};

pub(crate) async fn daemon_proxy(conn: &Connection) -> zbus::Result<Proxy<'_>> {
    Proxy::new(conn, SERVICE_NAME, OBJECT_PATH, DAEMON_INTERFACE).await
}

pub(crate) async fn effects_proxy(conn: &Connection) -> zbus::Result<Proxy<'_>> {
    Proxy::new(conn, SERVICE_NAME, OBJECT_PATH, EFFECTS_INTERFACE).await
}

pub(crate) async fn devices_proxy(conn: &Connection) -> zbus::Result<Proxy<'_>> {
    Proxy::new(conn, SERVICE_NAME, OBJECT_PATH, DEVICES_INTERFACE).await
}

/// Start the daemon if its bus name has no owner. Prefers the systemd unit;
/// if that fails (e.g. unit not installed in a dev tree), proceeds and lets the
/// subsequent D-Bus method call auto-activate it.
pub(crate) async fn ensure_daemon(conn: &Connection) {
    let Ok(dbus) = zbus::fdo::DBusProxy::new(conn).await else {
        return;
    };
    let Ok(name) = SERVICE_NAME.try_into() else {
        return;
    };
    if dbus.name_has_owner(name).await.unwrap_or(false) {
        return;
    }
    if let Err(err) = shared::systemd::start() {
        tracing::warn!(%err, "systemctl start failed; relying on D-Bus activation");
    }
}

use std::sync::Arc;

use shared::dbus::{EFFECT_IDS, VariantMap};
use tokio::sync::{RwLock, mpsc};
use zbus::{fdo, interface, object_server::SignalEmitter};
use zvariant::OwnedValue;

use crate::{
    pipeline::{self, PipelineCommand},
    state::{DaemonState, DaemonStatus},
};

pub type SharedDaemonState = Arc<RwLock<DaemonState>>;

#[derive(Clone)]
pub struct Daemon1Iface {
    state: SharedDaemonState,
    pipeline_tx: mpsc::Sender<PipelineCommand>,
    shutdown_tx: mpsc::Sender<()>,
}

impl Daemon1Iface {
    pub fn new(
        state: SharedDaemonState,
        pipeline_tx: mpsc::Sender<PipelineCommand>,
        shutdown_tx: mpsc::Sender<()>,
    ) -> Self {
        Self {
            state,
            pipeline_tx,
            shutdown_tx,
        }
    }
}

#[interface(name = "io.github.karanshukla.vinoopeneffects.Daemon1")]
impl Daemon1Iface {
    async fn start(&self, #[zbus(signal_emitter)] emitter: SignalEmitter<'_>) -> fdo::Result<()> {
        let app = {
            let mut state = self.state.write().await;
            if !state.status.can_start() {
                return Ok(());
            }
            state.status = DaemonStatus::Starting;
            state.last_error = None;
            state.app.clone()
        };

        Self::daemon_status_changed(&emitter, DaemonStatus::Starting.as_str())
            .await
            .map_err(to_fdo_error)?;
        self.pipeline_tx
            .send(PipelineCommand::Start(app))
            .await
            .map_err(|err| fdo::Error::Failed(err.to_string()))?;
        Ok(())
    }

    async fn stop(&self, #[zbus(signal_emitter)] emitter: SignalEmitter<'_>) -> fdo::Result<()> {
        {
            let mut state = self.state.write().await;
            if !state.status.can_stop() {
                return Ok(());
            }
            state.status = DaemonStatus::Stopped;
        }
        self.pipeline_tx
            .send(PipelineCommand::Stop)
            .await
            .map_err(|err| fdo::Error::Failed(err.to_string()))?;
        Self::daemon_status_changed(&emitter, DaemonStatus::Stopped.as_str())
            .await
            .map_err(to_fdo_error)?;
        Ok(())
    }

    /// Tear down the pipeline and exit the process. Best-effort: the main loop
    /// performs the actual shutdown + clean exit when it receives the signal.
    async fn quit(&self) -> fdo::Result<()> {
        let _ = self.shutdown_tx.send(()).await;
        Ok(())
    }

    async fn set_inference_provider(&self, provider: String) -> fdo::Result<()> {
        let parsed: shared::config::InferenceProvider = provider
            .parse()
            .map_err(|e: anyhow::Error| fdo::Error::InvalidArgs(e.to_string()))?;
        {
            let mut state = self.state.write().await;
            state.app.inference.provider = parsed;
            let _ = state.sync_inference_from_config();
            if let Err(e) = state.save_config() {
                tracing::warn!(%e, "failed to persist inference config");
            }
        }
        self.pipeline_tx
            .send(PipelineCommand::SetInferenceProvider(parsed))
            .await
            .map_err(|err| fdo::Error::Failed(err.to_string()))?;
        Ok(())
    }

    /// Forget recorded NPU failures and reload the models on the current provider.
    async fn retry_inference(&self) -> fdo::Result<bool> {
        let cleared = oe_core::inference::forget_failures();
        let provider = {
            let mut state = self.state.write().await;
            let _ = state.sync_inference_from_config();
            state.app.inference.provider
        };
        self.pipeline_tx
            .send(PipelineCommand::SetInferenceProvider(provider))
            .await
            .map_err(|err| fdo::Error::Failed(err.to_string()))?;
        Ok(cleared)
    }

    #[zbus(property)]
    async fn status(&self) -> String {
        self.state.read().await.status.as_str().into()
    }

    #[zbus(property)]
    async fn capabilities(&self) -> VariantMap {
        self.state.read().await.capabilities()
    }

    #[zbus(signal)]
    #[zbus(name = "StatusChanged")]
    pub async fn daemon_status_changed(
        emitter: &SignalEmitter<'_>,
        new_status: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    #[zbus(name = "InferenceChanged")]
    pub async fn inference_changed(
        emitter: &SignalEmitter<'_>,
        active: &str,
        detail: &str,
    ) -> zbus::Result<()>;
}

#[derive(Clone)]
pub struct Effects1Iface {
    state: SharedDaemonState,
    pipeline_tx: mpsc::Sender<PipelineCommand>,
}

impl Effects1Iface {
    pub fn new(state: SharedDaemonState, pipeline_tx: mpsc::Sender<PipelineCommand>) -> Self {
        Self { state, pipeline_tx }
    }
}

#[interface(name = "io.github.karanshukla.vinoopeneffects.Effects1")]
impl Effects1Iface {
    async fn list_effects(&self) -> Vec<String> {
        EFFECT_IDS.iter().map(|id| (*id).to_string()).collect()
    }

    async fn set_enabled(
        &self,
        id: &str,
        on: bool,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> fdo::Result<()> {
        let (params, app) = {
            let mut state = self.state.write().await;
            if !state.set_enabled(id, on) {
                return Err(fdo::Error::InvalidArgs(format!("unknown effect '{id}'")));
            }
            state
                .save_config()
                .map_err(|err| fdo::Error::Failed(err.to_string()))?;
            (
                state.effect_params(id).unwrap_or_default(),
                state.app.clone(),
            )
        };

        let _ = self
            .pipeline_tx
            .send(PipelineCommand::ApplyEffects(app))
            .await;
        Self::effect_changed(&emitter, id, params)
            .await
            .map_err(to_fdo_error)?;
        Ok(())
    }

    async fn set_param(
        &self,
        id: &str,
        key: &str,
        value: OwnedValue,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> fdo::Result<()> {
        let (params, app) = {
            let mut state = self.state.write().await;
            if !state.set_param(id, key, &value) {
                return Err(fdo::Error::InvalidArgs(format!(
                    "unknown effect parameter '{id}.{key}'"
                )));
            }
            state
                .save_config()
                .map_err(|err| fdo::Error::Failed(err.to_string()))?;
            (
                state.effect_params(id).unwrap_or_default(),
                state.app.clone(),
            )
        };

        let _ = self
            .pipeline_tx
            .send(PipelineCommand::ApplyEffects(app))
            .await;
        Self::effect_changed(&emitter, id, params)
            .await
            .map_err(to_fdo_error)?;
        Ok(())
    }

    async fn set_bypass(
        &self,
        on: bool,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> fdo::Result<()> {
        let app = {
            let mut state = self.state.write().await;
            if state.bypass() == on {
                return Ok(());
            }
            state.set_bypass(on);
            state
                .save_config()
                .map_err(|err| fdo::Error::Failed(err.to_string()))?;
            state.app.clone()
        };

        let _ = self
            .pipeline_tx
            .send(PipelineCommand::ApplyEffects(app))
            .await;
        Self::bypass_changed(&emitter, on)
            .await
            .map_err(to_fdo_error)?;
        Ok(())
    }

    async fn get_bypass(&self) -> bool {
        self.state.read().await.bypass()
    }

    /// Fire a one-shot reaction burst. Stateless: no config is changed and no
    /// signal is emitted — it just hands the pipeline a trigger.
    async fn trigger_reaction(&self, id: &str) -> fdo::Result<()> {
        if !shared::dbus::REACTION_IDS.contains(&id) {
            return Err(fdo::Error::InvalidArgs(format!("unknown reaction '{id}'")));
        }
        self.pipeline_tx
            .send(PipelineCommand::TriggerReaction(id.to_string()))
            .await
            .map_err(|err| fdo::Error::Failed(err.to_string()))?;
        Ok(())
    }

    async fn get_params(&self, id: &str) -> fdo::Result<VariantMap> {
        self.state
            .read()
            .await
            .effect_params(id)
            .ok_or_else(|| fdo::Error::InvalidArgs(format!("unknown effect '{id}'")))
    }

    async fn get_all_state(&self) -> VariantMap {
        self.state.read().await.all_effect_state()
    }

    #[zbus(signal)]
    async fn effect_changed(
        emitter: &SignalEmitter<'_>,
        id: &str,
        params: VariantMap,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn bypass_changed(emitter: &SignalEmitter<'_>, on: bool) -> zbus::Result<()>;
}

#[derive(Clone)]
pub struct Devices1Iface {
    state: SharedDaemonState,
    pipeline_tx: mpsc::Sender<PipelineCommand>,
}

impl Devices1Iface {
    pub fn new(state: SharedDaemonState, pipeline_tx: mpsc::Sender<PipelineCommand>) -> Self {
        Self { state, pipeline_tx }
    }
}

#[interface(name = "io.github.karanshukla.vinoopeneffects.Devices1")]
impl Devices1Iface {
    async fn list_cameras(&self) -> Vec<VariantMap> {
        let active = self.state.read().await.active_camera.clone();
        pipeline::cameras::enumerate()
            .into_iter()
            .map(|c| {
                let mut m = VariantMap::new();
                m.insert("id".into(), shared::dbus::str_value(&c.id));
                m.insert("name".into(), shared::dbus::str_value(&c.name));
                m.insert("path".into(), shared::dbus::str_value(&c.path));
                m.insert("api".into(), shared::dbus::str_value(&c.api));
                m.insert("active".into(), shared::dbus::bool_value(c.id == active));
                m.insert("width".into(), shared::dbus::u32_value(1280));
                m.insert("height".into(), shared::dbus::u32_value(720));
                m.insert("fps".into(), shared::dbus::u32_value(30));
                m
            })
            .collect()
    }

    async fn select_camera(&self, id: &str) -> fdo::Result<()> {
        let (was_running, app) = {
            let mut state = self.state.write().await;
            state.active_camera = id.into();
            state.app.camera.selected = id.into();
            state
                .save_config()
                .map_err(|err| fdo::Error::Failed(err.to_string()))?;
            (
                matches!(
                    state.status,
                    DaemonStatus::Running | DaemonStatus::Idle | DaemonStatus::Starting
                ),
                state.app.clone(),
            )
        };
        if was_running {
            let _ = self.pipeline_tx.send(PipelineCommand::Start(app)).await;
        }
        Ok(())
    }

    #[zbus(property)]
    async fn virtual_camera_info(&self) -> VariantMap {
        self.state.read().await.virtual_camera_info()
    }

    #[zbus(property)]
    async fn active_camera(&self) -> String {
        self.state.read().await.active_camera.clone()
    }
}

fn to_fdo_error(error: zbus::Error) -> fdo::Error {
    fdo::Error::Failed(error.to_string())
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeSet, path::PathBuf, time::Duration};

    use futures_util::StreamExt;
    use shared::{
        config::{AppState, InferenceProvider},
        dbus::{DAEMON_INTERFACE, DEVICES_INTERFACE, EFFECTS_INTERFACE, OBJECT_PATH},
    };
    use zvariant::Value;

    use super::*;

    const INVALID_ARGS: &str = "org.freedesktop.DBus.Error.InvalidArgs";

    /// The three interfaces served on a peer-to-peer connection, so no bus,
    /// GStreamer or camera is needed; the pipeline is the receiving end of `rx`.
    struct Harness {
        client: zbus::Connection,
        _server: zbus::Connection,
        state: SharedDaemonState,
        rx: mpsc::Receiver<PipelineCommand>,
        shutdown_rx: mpsc::Receiver<()>,
        config: PathBuf,
        _dir: tempfile::TempDir,
    }

    async fn harness() -> Harness {
        let dir = tempfile::TempDir::new().unwrap();
        let config = dir.path().join("state.toml");
        let mut daemon = DaemonState::new(AppState::default(), false);
        daemon.config_path = Some(config.clone());
        let state = Arc::new(RwLock::new(daemon));
        let (tx, rx) = mpsc::channel(8);
        let (shutdown_tx, shutdown_rx) = mpsc::channel(1);

        let (a, b) = tokio::net::UnixStream::pair().unwrap();
        let server = zbus::connection::Builder::unix_stream(a)
            .server(zbus::Guid::generate())
            .unwrap()
            .p2p()
            .serve_at(
                OBJECT_PATH,
                Daemon1Iface::new(state.clone(), tx.clone(), shutdown_tx),
            )
            .unwrap()
            .serve_at(OBJECT_PATH, Effects1Iface::new(state.clone(), tx.clone()))
            .unwrap()
            .serve_at(OBJECT_PATH, Devices1Iface::new(state.clone(), tx))
            .unwrap()
            .build();
        let client = zbus::connection::Builder::unix_stream(b).p2p().build();
        let (server, client) = tokio::try_join!(server, client).unwrap();
        Harness {
            client,
            _server: server,
            state,
            rx,
            shutdown_rx,
            config,
            _dir: dir,
        }
    }

    async fn call<B>(
        h: &Harness,
        iface: &str,
        method: &str,
        body: &B,
    ) -> zbus::Result<zbus::Message>
    where
        B: serde::Serialize + zvariant::DynamicType,
    {
        h.client
            .call_method(None::<&str>, OBJECT_PATH, Some(iface), method, body)
            .await
    }

    fn error_name(result: zbus::Result<zbus::Message>) -> String {
        match result {
            Ok(_) => panic!("call succeeded, expected an error"),
            Err(zbus::Error::MethodError(name, _, _)) => name.to_string(),
            Err(err) => panic!("expected a D-Bus method error, got {err}"),
        }
    }

    /// The next signal named `member`, or `None` if none arrives in `wait`.
    async fn signal(
        stream: &mut zbus::MessageStream,
        member: &str,
        wait: Duration,
    ) -> Option<zbus::Message> {
        tokio::time::timeout(wait, async {
            while let Some(Ok(msg)) = stream.next().await {
                let header = msg.header();
                if header.message_type() == zbus::message::Type::Signal
                    && header.member().is_some_and(|m| m.as_str() == member)
                {
                    return Some(msg);
                }
            }
            None
        })
        .await
        .ok()
        .flatten()
    }

    const SIGNAL_WAIT: Duration = Duration::from_secs(2);
    const NO_SIGNAL_WAIT: Duration = Duration::from_millis(200);

    fn saved(h: &Harness) -> AppState {
        AppState::load_or_default_from(h.config.clone()).unwrap()
    }

    #[tokio::test]
    async fn set_param_clamps_persists_and_applies() {
        let mut h = harness().await;
        let mut stream = zbus::MessageStream::from(&h.client);

        call(
            &h,
            EFFECTS_INTERFACE,
            "SetParam",
            &("portrait_blur", "strength", Value::from(250u32)),
        )
        .await
        .unwrap();

        assert_eq!(h.state.read().await.app.effects.portrait_blur.strength, 100);
        assert_eq!(saved(&h).effects.portrait_blur.strength, 100);
        match h.rx.try_recv() {
            Ok(PipelineCommand::ApplyEffects(app)) => {
                assert_eq!(app.effects.portrait_blur.strength, 100)
            }
            other => panic!("expected ApplyEffects, got {other:?}"),
        }
        let msg = signal(&mut stream, "EffectChanged", SIGNAL_WAIT)
            .await
            .expect("EffectChanged");
        let (id, _params): (String, VariantMap) = msg.body().deserialize().unwrap();
        assert_eq!(id, "portrait_blur");
    }

    #[tokio::test]
    async fn bad_input_is_rejected_before_anything_is_saved_or_sent() {
        let mut h = harness().await;
        let cases: Vec<(&str, &str, zbus::Result<zbus::Message>)> = vec![
            (
                "unknown effect",
                "SetParam",
                call(
                    &h,
                    EFFECTS_INTERFACE,
                    "SetParam",
                    &("nope", "strength", Value::from(1u32)),
                )
                .await,
            ),
            (
                "unknown param",
                "SetParam",
                call(
                    &h,
                    EFFECTS_INTERFACE,
                    "SetParam",
                    &("portrait_blur", "nope", Value::from(1u32)),
                )
                .await,
            ),
            (
                "wrong value type",
                "SetParam",
                call(
                    &h,
                    EFFECTS_INTERFACE,
                    "SetParam",
                    &("portrait_blur", "strength", Value::from("loud")),
                )
                .await,
            ),
            (
                "unknown effect",
                "SetEnabled",
                call(&h, EFFECTS_INTERFACE, "SetEnabled", &("nope", true)).await,
            ),
            (
                "unknown effect",
                "GetParams",
                call(&h, EFFECTS_INTERFACE, "GetParams", &("nope",)).await,
            ),
            (
                "unknown reaction",
                "TriggerReaction",
                call(&h, EFFECTS_INTERFACE, "TriggerReaction", &("nope",)).await,
            ),
            (
                "unknown provider",
                "SetInferenceProvider",
                call(&h, DAEMON_INTERFACE, "SetInferenceProvider", &("bogus",)).await,
            ),
        ];
        for (what, method, result) in cases {
            assert_eq!(error_name(result), INVALID_ARGS, "{method}: {what}");
        }

        assert!(!h.config.exists(), "rejected calls wrote the config");
        assert!(
            h.rx.try_recv().is_err(),
            "rejected calls reached the pipeline"
        );
        assert_eq!(h.state.read().await.app.effects.portrait_blur.strength, 50);
    }

    #[tokio::test]
    async fn set_enabled_persists_and_applies() {
        let mut h = harness().await;
        call(&h, EFFECTS_INTERFACE, "SetEnabled", &("studio_light", true))
            .await
            .unwrap();
        assert!(saved(&h).effects.studio_light.enabled);
        assert!(matches!(
            h.rx.try_recv(),
            Ok(PipelineCommand::ApplyEffects(app)) if app.effects.studio_light.enabled
        ));
    }

    #[tokio::test]
    async fn start_from_stopped_signals_and_sends_start() {
        let mut h = harness().await;
        let mut stream = zbus::MessageStream::from(&h.client);

        call(&h, DAEMON_INTERFACE, "Start", &()).await.unwrap();

        assert_eq!(h.state.read().await.status, DaemonStatus::Starting);
        assert!(matches!(h.rx.try_recv(), Ok(PipelineCommand::Start(_))));
        let msg = signal(&mut stream, "StatusChanged", SIGNAL_WAIT)
            .await
            .expect("StatusChanged");
        assert_eq!(msg.body().deserialize::<String>().unwrap(), "starting");
    }

    #[tokio::test]
    async fn start_while_running_is_a_no_op() {
        let mut h = harness().await;
        h.state.write().await.status = DaemonStatus::Running;
        let mut stream = zbus::MessageStream::from(&h.client);

        call(&h, DAEMON_INTERFACE, "Start", &()).await.unwrap();

        assert_eq!(h.state.read().await.status, DaemonStatus::Running);
        assert!(h.rx.try_recv().is_err());
        assert!(
            signal(&mut stream, "StatusChanged", NO_SIGNAL_WAIT)
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn stop_sends_stop_and_signals_only_when_running() {
        let mut h = harness().await;
        let mut stream = zbus::MessageStream::from(&h.client);

        call(&h, DAEMON_INTERFACE, "Stop", &()).await.unwrap();
        assert!(
            h.rx.try_recv().is_err(),
            "Stop while stopped reached the pipeline"
        );

        h.state.write().await.status = DaemonStatus::Running;
        call(&h, DAEMON_INTERFACE, "Stop", &()).await.unwrap();
        assert_eq!(h.state.read().await.status, DaemonStatus::Stopped);
        assert!(matches!(h.rx.try_recv(), Ok(PipelineCommand::Stop)));
        let msg = signal(&mut stream, "StatusChanged", SIGNAL_WAIT)
            .await
            .expect("StatusChanged");
        assert_eq!(msg.body().deserialize::<String>().unwrap(), "stopped");
    }

    #[tokio::test]
    async fn a_dead_pipeline_worker_is_reported_as_failed() {
        let mut h = harness().await;
        let (_, closed) = mpsc::channel(1);
        drop(std::mem::replace(&mut h.rx, closed));
        assert_eq!(
            error_name(call(&h, DAEMON_INTERFACE, "Start", &()).await),
            "org.freedesktop.DBus.Error.Failed"
        );
        assert_eq!(
            error_name(
                call(
                    &h,
                    EFFECTS_INTERFACE,
                    "TriggerReaction",
                    &(shared::dbus::REACTION_IDS[0],)
                )
                .await
            ),
            "org.freedesktop.DBus.Error.Failed"
        );
    }

    #[tokio::test]
    async fn set_bypass_is_idempotent() {
        let mut h = harness().await;
        let mut stream = zbus::MessageStream::from(&h.client);

        call(&h, EFFECTS_INTERFACE, "SetBypass", &(true,))
            .await
            .unwrap();
        assert!(saved(&h).bypass);
        assert!(matches!(h.rx.try_recv(), Ok(PipelineCommand::ApplyEffects(app)) if app.bypass));
        let msg = signal(&mut stream, "BypassChanged", SIGNAL_WAIT)
            .await
            .expect("BypassChanged");
        assert!(msg.body().deserialize::<bool>().unwrap());

        call(&h, EFFECTS_INTERFACE, "SetBypass", &(true,))
            .await
            .unwrap();
        assert!(h.rx.try_recv().is_err());
        assert!(
            signal(&mut stream, "BypassChanged", NO_SIGNAL_WAIT)
                .await
                .is_none()
        );

        let reply = call(&h, EFFECTS_INTERFACE, "GetBypass", &()).await.unwrap();
        assert!(reply.body().deserialize::<bool>().unwrap());
    }

    #[tokio::test]
    async fn set_inference_provider_persists_and_reaches_the_worker() {
        let mut h = harness().await;
        call(&h, DAEMON_INTERFACE, "SetInferenceProvider", &("cpu",))
            .await
            .unwrap();
        assert_eq!(saved(&h).inference.provider, InferenceProvider::Cpu);
        assert!(matches!(
            h.rx.try_recv(),
            Ok(PipelineCommand::SetInferenceProvider(
                InferenceProvider::Cpu
            ))
        ));
    }

    #[tokio::test]
    async fn select_camera_persists_and_restarts_only_a_running_pipeline() {
        let mut h = harness().await;

        call(&h, DEVICES_INTERFACE, "SelectCamera", &("/dev/video4",))
            .await
            .unwrap();
        assert_eq!(saved(&h).camera.selected, "/dev/video4");
        assert_eq!(h.state.read().await.active_camera, "/dev/video4");
        assert!(h.rx.try_recv().is_err(), "a stopped pipeline was restarted");

        h.state.write().await.status = DaemonStatus::Running;
        call(&h, DEVICES_INTERFACE, "SelectCamera", &("/dev/video6",))
            .await
            .unwrap();
        assert!(matches!(
            h.rx.try_recv(),
            Ok(PipelineCommand::Start(app)) if app.camera.selected == "/dev/video6"
        ));
    }

    #[tokio::test]
    async fn quit_signals_the_main_loop() {
        let mut h = harness().await;
        call(&h, DAEMON_INTERFACE, "Quit", &()).await.unwrap();
        assert!(h.shutdown_rx.try_recv().is_ok());
    }

    #[tokio::test]
    async fn list_effects_matches_the_shared_ids() {
        let h = harness().await;
        let reply = call(&h, EFFECTS_INTERFACE, "ListEffects", &())
            .await
            .unwrap();
        let ids: Vec<String> = reply.body().deserialize().unwrap();
        assert_eq!(ids, EFFECT_IDS);
    }

    /// Methods, signals and properties of one interface, by name and signature.
    /// Argument names are left out: zbus takes them from the Rust parameters.
    fn members(iface: &zbus_xml::Interface<'_>) -> BTreeSet<String> {
        // A method arg with no direction is "in"; signal args are always "out".
        let args = |args: &[zbus_xml::Arg], signal: bool| {
            args.iter()
                .map(|a| {
                    let out = signal || a.direction() == Some(zbus_xml::ArgDirection::Out);
                    format!("{}:{}", if out { "out" } else { "in" }, a.ty())
                })
                .collect::<Vec<_>>()
                .join(",")
        };
        let mut out = BTreeSet::new();
        for m in iface.methods() {
            out.insert(format!("method {}({})", m.name(), args(m.args(), false)));
        }
        for s in iface.signals() {
            out.insert(format!("signal {}({})", s.name(), args(s.args(), true)));
        }
        for p in iface.properties() {
            out.insert(format!("property {} {} {:?}", p.name(), p.ty(), p.access()));
        }
        out
    }

    #[tokio::test]
    async fn served_interfaces_match_the_published_xml() {
        let h = harness().await;
        let reply = h
            .client
            .call_method(
                None::<&str>,
                OBJECT_PATH,
                Some("org.freedesktop.DBus.Introspectable"),
                "Introspect",
                &(),
            )
            .await
            .unwrap();
        let served_xml: String = reply.body().deserialize().unwrap();
        let served = zbus_xml::Node::from_reader(served_xml.as_bytes()).unwrap();

        for (name, xml) in [
            (
                DAEMON_INTERFACE,
                include_str!("../../data/dbus/io.github.karanshukla.vinoopeneffects.Daemon1.xml"),
            ),
            (
                EFFECTS_INTERFACE,
                include_str!("../../data/dbus/io.github.karanshukla.vinoopeneffects.Effects1.xml"),
            ),
            (
                DEVICES_INTERFACE,
                include_str!("../../data/dbus/io.github.karanshukla.vinoopeneffects.Devices1.xml"),
            ),
        ] {
            let published = zbus_xml::Node::from_reader(xml.as_bytes()).unwrap();
            let find = |node: &zbus_xml::Node<'_>| {
                node.interfaces()
                    .iter()
                    .find(|i| i.name().as_str() == name)
                    .map(members)
                    .unwrap_or_else(|| panic!("{name} missing"))
            };
            assert_eq!(
                find(&served),
                find(&published),
                "{name} drifted from its XML"
            );
        }
    }
}

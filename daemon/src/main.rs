mod bench;
mod dbus_server;
mod pipeline;
mod state;

use std::sync::Arc;

use anyhow::Context;
use shared::{
    config::AppState,
    dbus::{OBJECT_PATH, SERVICE_NAME},
};
use tokio::sync::{RwLock, mpsc};
use tracing::{error, info, warn};
use zbus::object_server::SignalEmitter;

use crate::{
    dbus_server::{Daemon1Iface, Devices1Iface, Effects1Iface},
    pipeline::{PipelineCommand, PipelineEvent},
    state::{DaemonState, DaemonStatus},
};

fn main() -> anyhow::Result<()> {
    if let Some(i) = std::env::args().position(|a| a == "--bench-inference") {
        let rest: Vec<String> = std::env::args().skip(i + 1).collect();
        return bench::run(&rest);
    }
    if let Some(i) = std::env::args().position(|a| a == "--bench-video") {
        let rest: Vec<String> = std::env::args().skip(i + 1).collect();
        return bench::run_video(&rest);
    }
    unsafe { libc::mallopt(libc::M_ARENA_MAX, 2) };

    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?
        .block_on(run())
}

async fn run() -> anyhow::Result<()> {
    init_tracing();

    // Must run before config and models are read.
    shared::migrate::migrate_legacy_paths().log();

    let start_pipeline = std::env::args().any(|arg| arg == "--start");
    let app = load_app_state();
    let models_ready = effect_models_present();

    let warm_models = pipeline::effects::wanted_models(&app);
    let daemon_state = DaemonState::new(app, models_ready);
    let warm_provider = daemon_state.requested_inference_provider();
    if daemon_state.inference_degraded() {
        warn!(
            detail = daemon_state.inference_detail.as_deref().unwrap_or(""),
            "an NPU is present but models will run on CPU"
        );
    }
    let state = Arc::new(RwLock::new(daemon_state));
    let (pipeline_tx, pipeline_rx) = mpsc::channel(32);
    let (event_tx, event_rx) = mpsc::channel(32);
    spawn_warm_up(warm_provider, warm_models, event_tx.clone());
    let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);
    let worker = pipeline::spawn_worker(pipeline_rx, event_tx);

    let conn = serve_on_session_bus(&state, &pipeline_tx, shutdown_tx).await?;
    let status_emitter = SignalEmitter::new(&conn, OBJECT_PATH)?.into_owned();
    tokio::spawn(announce_pipeline_events(
        Arc::clone(&state),
        event_rx,
        status_emitter,
    ));

    if start_pipeline {
        start_pipeline_now(&state, &pipeline_tx).await?;
    }

    info!("vinoopeneffects-daemon is running on the session bus");
    wait_for_shutdown(&mut shutdown_rx).await?;
    let _ = pipeline_tx.send(PipelineCommand::Shutdown).await;
    let _ = worker.join();
    if !oe_core::inference::engine::wait_for_loads(std::time::Duration::from_secs(5)) {
        warn!("exiting with a model load still running");
    }
    Ok(())
}

fn init_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
}

fn load_app_state() -> AppState {
    let mut app = match AppState::load_or_recover() {
        Ok((app, None)) => app,
        Ok((app, Some(bad))) => {
            error!(moved_to = %bad.display(), "config did not parse, using defaults");
            app
        }
        Err(err) => {
            error!(%err, "failed to load config, using defaults");
            AppState::default()
        }
    };
    if app.camera.selected.trim().is_empty() {
        select_default_camera(&mut app);
    }
    app
}

fn select_default_camera(app: &mut AppState) {
    if let Err(err) = gstreamer::init() {
        error!(%err, "failed to initialize GStreamer for camera autodetect");
        return;
    }
    if let Some(camera) = pipeline::cameras::autodetect() {
        app.camera.selected = camera.id;
        if let Err(err) = app.save() {
            error!(%err, "failed to persist default camera selection");
        }
    }
}

fn effect_models_present() -> bool {
    let present = oe_core::inference::registry::find_model("yunet").is_some()
        && oe_core::inference::registry::find_model("selfie_segmentation").is_some();
    if !present {
        warn!(
            "ML models not found; run scripts/fetch-models.sh to enable Portrait Blur and Center Stage"
        );
    }
    present
}

async fn serve_on_session_bus(
    state: &Arc<RwLock<DaemonState>>,
    pipeline_tx: &mpsc::Sender<PipelineCommand>,
    shutdown_tx: mpsc::Sender<()>,
) -> anyhow::Result<zbus::Connection> {
    zbus::connection::Builder::session()?
        .serve_at(
            OBJECT_PATH,
            Daemon1Iface::new(Arc::clone(state), pipeline_tx.clone(), shutdown_tx),
        )?
        .serve_at(
            OBJECT_PATH,
            Effects1Iface::new(Arc::clone(state), pipeline_tx.clone()),
        )?
        .serve_at(
            OBJECT_PATH,
            Devices1Iface::new(Arc::clone(state), pipeline_tx.clone()),
        )?
        .name(SERVICE_NAME)?
        .build()
        .await
        .context("connect to session bus")
}

async fn announce_pipeline_events(
    state: Arc<RwLock<DaemonState>>,
    mut events: mpsc::Receiver<PipelineEvent>,
    emitter: SignalEmitter<'static>,
) {
    while let Some(event) = events.recv().await {
        let (announcements, degraded) = {
            let mut state = state.write().await;
            let announcements = state.apply_pipeline_event(event);
            (announcements, state.inference_degraded())
        };

        if let Some(status) = announcements.status
            && let Err(err) = Daemon1Iface::daemon_status_changed(&emitter, status.as_str()).await
        {
            error!(%err, "failed to emit StatusChanged");
        }

        if let Some((active, detail)) = announcements.inference {
            let detail_str = detail.as_deref().unwrap_or("");
            if degraded {
                warn!(
                    detail = detail_str,
                    "an NPU is present but models run on {active}"
                );
            }
            if let Err(err) = Daemon1Iface::inference_changed(&emitter, &active, detail_str).await {
                error!(%err, "failed to emit InferenceChanged");
            }
        }
    }
}

async fn start_pipeline_now(
    state: &RwLock<DaemonState>,
    pipeline_tx: &mpsc::Sender<PipelineCommand>,
) -> anyhow::Result<()> {
    let app = state.read().await.app.clone();
    {
        let mut state = state.write().await;
        state.status = DaemonStatus::Starting;
    }
    pipeline_tx.send(PipelineCommand::Start(app)).await?;
    Ok(())
}

async fn wait_for_shutdown(quit: &mut mpsc::Receiver<()>) -> anyhow::Result<()> {
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .context("install SIGTERM handler")?;
    tokio::select! {
        result = tokio::signal::ctrl_c() => {
            result?;
            info!("received ctrl-c, shutting down");
        }
        _ = sigterm.recv() => {
            info!("received SIGTERM, shutting down");
        }
        _ = quit.recv() => {
            info!("received Quit, shutting down");
        }
    }
    Ok(())
}

/// Compile the enabled effects' models at start, off every frame path, so the
/// first stream loads them from OpenVINO's cache; a failure is reported now.
fn spawn_warm_up(
    provider: String,
    models: oe_core::inference::engine::LoadedModels,
    events: mpsc::Sender<PipelineEvent>,
) {
    if !models.any() {
        return;
    }
    std::thread::spawn(move || {
        let started = std::time::Instant::now();
        let (active, detail) = oe_core::inference::engine::warm(&provider, models);
        info!(%active, elapsed = ?started.elapsed(), ?models, "warmed models");
        if oe_core::inference::degraded(&provider, active) {
            let _ = events.blocking_send(PipelineEvent::InferenceChanged {
                active: active.into(),
                detail,
            });
        }
    });
}

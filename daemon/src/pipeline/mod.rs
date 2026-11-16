pub mod bridge;
pub mod builder;
pub mod cameras;
pub mod effects;
pub mod filters;
pub mod probe;
pub mod provider;

use std::sync::Arc;
use std::sync::mpsc as std_mpsc;
use std::time::{Duration, Instant};

use pipewire as pw;
use shared::config::AppState;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use crate::pipeline::bridge::Bridge;
use crate::pipeline::probe::PIPEWIRE_NODE_NAME;

pub const WIDTH: u32 = 1280;
pub const HEIGHT: u32 = 720;
pub const FPS: i32 = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PipelineFormat {
    pub width: u32,
    pub height: u32,
    pub fps: i32,
}

impl PipelineFormat {
    pub const DEFAULT: PipelineFormat = PipelineFormat {
        width: WIDTH,
        height: HEIGHT,
        fps: FPS,
    };

    /// I420 Y-plane stride.
    pub fn stride(&self) -> u32 {
        self.width
    }

    /// I420 frame size = W*H (Y) + 2 * (W/2 * H/2) (U,V) = W*H*3/2.
    pub fn frame_size(&self) -> usize {
        (self.width as usize * self.height as usize * 3) / 2
    }
}

/// Grace period before releasing the real camera once the provide node leaves
/// STREAMING. Consumers that go through xdg-desktop-portal (browsers) probe the
/// node with rapid connect/disconnect blips and retry if the first frames are
/// the warmup placeholder; tearing the capture pipeline on every blip would
/// thrash the camera (open takes ~250 ms) and never deliver a stable stream.
/// Holding briefly keeps the camera warm across blips while still releasing it
/// promptly — and the LED off — when the consumer is really gone.
const CAPTURE_RELEASE_GRACE: Duration = Duration::from_millis(300);

/// How often to look for the camera again after it failed mid-stream (unplugged),
/// while a consumer is still streaming.
const CAPTURE_RETRY: Duration = Duration::from_secs(2);

/// A camera that delivers nothing for this long counts as failed, for stalls that
/// post no bus error. Well above the frame gap of a camera throttling in low light.
const CAPTURE_STALL: Duration = Duration::from_secs(5);
const STALL_CHECK: Duration = Duration::from_secs(1);

#[derive(Debug)]
pub enum PipelineCommand {
    Start(AppState),
    Stop,
    /// Replace the worker's stored config and push it onto the live capture
    /// elements. Sent after any effect toggle or param change, carrying the
    /// daemon's canonical `AppState` so the worker never re-parses per-key
    /// values (a duplicated mapping here once let param changes go stale until
    /// a restart).
    ApplyEffects(AppState),
    /// Fire a one-shot reaction burst (a [`shared::dbus::REACTION_IDS`] id) into
    /// the live `oe_effects` element. A no-op when no capture pipeline is up
    /// (nothing is consuming the virtual camera).
    TriggerReaction(String),
    /// Persisted choice; `VINOOPENEFFECTS_EP`, when set, still wins.
    SetInferenceProvider(shared::config::InferenceProvider),
    Shutdown,
}

#[derive(Debug, Clone)]
pub enum PipelineEvent {
    Advertised {
        sink: String,
    },
    Started {
        sink: String,
    },
    Idle,
    Stopped,
    Error(String),
    VirtualCameraVerified {
        node_name: String,
        found: bool,
    },
    InferenceChanged {
        active: String,
        detail: Option<String>,
    },
}

/// Internal gating signal: the provide node's consumer state drives the capture
/// pipeline's lifecycle. Emitted from the provider thread's `state_changed`.
#[derive(Debug)]
pub(crate) enum CaptureCmd {
    Start,
    Stop,
}

/// Everything the worker reacts to arrives on one inbox, so it can block until
/// there is work instead of polling: a daemon command, or a consumer-state
/// change from the provide node.
pub(crate) enum WorkerMsg {
    Command(PipelineCommand),
    Capture(CaptureCmd),
    /// The capture pipeline with this [`builder::BuiltCapture::id`] errored or hit EOS after it was
    /// PLAYING, e.g. the camera was unplugged.
    CaptureFailed {
        id: u64,
        error: String,
    },
}

/// Handle to the running native provide node (its own `pw_main_loop` thread).
struct ProviderHandle {
    quit: pw::channel::Sender<()>,
    join: std::thread::JoinHandle<()>,
}

impl ProviderHandle {
    fn stop(self) {
        let _ = self.quit.send(());
        let _ = self.join.join();
    }
}

pub fn spawn_worker(
    mut commands: mpsc::Receiver<PipelineCommand>,
    events: mpsc::Sender<PipelineEvent>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        if let Err(err) = gstreamer::init() {
            let _ = events.blocking_send(PipelineEvent::Error(err.to_string()));
            return;
        }
        if let Err(err) = filters::register() {
            let _ = events.blocking_send(PipelineEvent::Error(err.to_string()));
            return;
        }
        let (inbox_tx, inbox) = std_mpsc::channel::<WorkerMsg>();
        let forward_tx = inbox_tx.clone();
        std::thread::spawn(move || {
            while let Some(cmd) = commands.blocking_recv() {
                if forward_tx.send(WorkerMsg::Command(cmd)).is_err() {
                    return;
                }
            }
            let _ = forward_tx.send(WorkerMsg::Command(PipelineCommand::Shutdown));
        });
        let bridge = Arc::new(Bridge::new());
        worker_loop(inbox, inbox_tx, events, bridge);
    })
}

fn worker_loop(
    inbox: std_mpsc::Receiver<WorkerMsg>,
    capture_tx: std_mpsc::Sender<WorkerMsg>,
    events: mpsc::Sender<PipelineEvent>,
    bridge: Arc<Bridge>,
) {
    let mut stored_app: Option<AppState> = None;
    let mut provider: Option<ProviderHandle> = None;
    let mut capture: Option<builder::BuiltCapture> = None;
    let mut current_fmt = PipelineFormat::DEFAULT;
    let mut consumer_streaming = false;
    let mut stop_deadline: Option<Instant> = None;
    // Set while a consumer streams but there is no capture; the next rebuild attempt.
    let mut retry_at: Option<Instant> = None;

    loop {
        let watchdog =
            (consumer_streaming && capture.is_some()).then(|| Instant::now() + STALL_CHECK);
        let wake = earliest([stop_deadline, retry_at, watchdog]);
        let msg = match wake {
            Some(deadline) => {
                match inbox.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                    Ok(msg) => Some(msg),
                    Err(std_mpsc::RecvTimeoutError::Timeout) => None,
                    Err(std_mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
            None => match inbox.recv() {
                Ok(msg) => Some(msg),
                Err(_) => break,
            },
        };

        match msg {
            Some(WorkerMsg::Command(PipelineCommand::Start(app))) => {
                if let Some(c) = capture.take() {
                    c.stop();
                }
                bridge.clear();
                stop_deadline = None;
                retry_at = None;

                let fmt = builder::probe_format(&app);
                stored_app = Some(app);
                if provider.is_some() && fmt != current_fmt {
                    info!(?fmt, "virtual camera format changed; re-advertising");
                    if let Some(p) = provider.take() {
                        p.stop();
                    }
                    consumer_streaming = false;
                }
                current_fmt = fmt;

                if provider.is_none() {
                    provider = Some(spawn_provider(&bridge, current_fmt, &capture_tx, &events));
                    let _ = events.blocking_send(PipelineEvent::Advertised {
                        sink: format!("pipewire:{PIPEWIRE_NODE_NAME}"),
                    });
                }

                if consumer_streaming && let Some(app) = stored_app.clone() {
                    start_capture(
                        &app,
                        current_fmt,
                        &bridge,
                        &capture_tx,
                        &events,
                        &mut capture,
                        false,
                    );
                }
            }
            Some(WorkerMsg::Command(PipelineCommand::Stop)) => {
                if let Some(c) = capture.take() {
                    c.stop();
                }
                bridge.clear();
                consumer_streaming = false;
                stop_deadline = None;
                retry_at = None;
                if let Some(p) = provider.take() {
                    p.stop();
                }
                let _ = events.blocking_send(PipelineEvent::Stopped);
            }
            Some(WorkerMsg::Command(PipelineCommand::ApplyEffects(app))) => {
                if let Some(c) = capture.as_ref() {
                    c.apply_app_state(&app);
                }
                stored_app = Some(app);
            }
            Some(WorkerMsg::Command(PipelineCommand::TriggerReaction(id))) => {
                if let Some(c) = capture.as_ref() {
                    c.trigger_reaction(&id);
                }
            }
            Some(WorkerMsg::Command(PipelineCommand::SetInferenceProvider(choice))) => {
                if let Some(app) = stored_app.as_mut() {
                    app.inference.provider = choice;
                }
                let provider = oe_core::inference::effective_ep(choice.as_str());
                let (active, detail) = match capture.as_ref() {
                    Some(c) => match c.swap_inference(&provider) {
                        Ok(status) => status,
                        Err(err) => {
                            warn!(%err, "runtime inference swap failed; keeping current provider");
                            let active = c.inference_active().unwrap_or_else(|| "cpu".into());
                            (
                                active,
                                Some(format!("switch to {provider} failed: {err:#}")),
                            )
                        }
                    },
                    None => {
                        let (active, detail) = oe_core::inference::predicted_active(&provider);
                        (active.to_string(), detail)
                    }
                };
                let _ = events.blocking_send(PipelineEvent::InferenceChanged { active, detail });
            }
            Some(WorkerMsg::Command(PipelineCommand::Shutdown)) => {
                if let Some(c) = capture.take() {
                    c.stop();
                }
                if let Some(p) = provider.take() {
                    p.stop();
                }
                break;
            }
            Some(WorkerMsg::Capture(CaptureCmd::Start)) => {
                consumer_streaming = true;
                stop_deadline = None;
                retry_at = None;
                if capture.is_none()
                    && let Some(app) = stored_app.clone()
                {
                    start_capture(
                        &app,
                        current_fmt,
                        &bridge,
                        &capture_tx,
                        &events,
                        &mut capture,
                        false,
                    );
                }
            }
            Some(WorkerMsg::Capture(CaptureCmd::Stop)) => {
                consumer_streaming = false;
                if retry_at.take().is_some() {
                    let _ = events.blocking_send(PipelineEvent::Idle);
                }
                if capture.is_some() && stop_deadline.is_none() {
                    stop_deadline = Some(Instant::now() + CAPTURE_RELEASE_GRACE);
                }
            }
            Some(WorkerMsg::CaptureFailed { id, error }) => {
                if let Some(c) = capture.take_if(|c| c.id() == id) {
                    c.stop();
                    // Black frames rather than the last one frozen.
                    bridge.clear();
                    stop_deadline = None;
                    error!(%error, "capture failed mid-stream; camera released");
                    let _ = events
                        .blocking_send(PipelineEvent::Error(format!("camera stopped: {error}")));
                }
            }
            None => {}
        }

        if consumer_streaming
            && let Some(c) = capture.as_ref()
            && let Some(quiet) = stalled_for(bridge.age(), c.age())
        {
            let _ = capture_tx.send(WorkerMsg::CaptureFailed {
                id: c.id(),
                error: format!("no frame from the camera for {}s", quiet.as_secs()),
            });
        }

        if let Some(at) = retry_at
            && Instant::now() >= at
        {
            retry_at = None;
            if consumer_streaming
                && capture.is_none()
                && let Some(app) = stored_app.clone()
            {
                // No camera would mean videotestsrc's colour bars; keep serving black instead.
                if builder::resolve_camera(&app).is_some() {
                    start_capture(
                        &app,
                        current_fmt,
                        &bridge,
                        &capture_tx,
                        &events,
                        &mut capture,
                        true,
                    );
                }
            }
        }

        if retry_at.is_none()
            && consumer_lacks_camera(consumer_streaming, capture.is_some(), stored_app.is_some())
        {
            retry_at = Some(Instant::now() + CAPTURE_RETRY);
        }

        if let Some(deadline) = stop_deadline {
            if !consumer_streaming
                && Instant::now() >= deadline
                && let Some(c) = capture.take()
            {
                c.stop();
                bridge.clear();
                unsafe { libc::malloc_trim(0) };
                info!("capture stopped: no consumer, camera released");
                let _ = events.blocking_send(PipelineEvent::Idle);
            }
            if consumer_streaming || Instant::now() >= deadline {
                stop_deadline = None;
            }
        }
    }
}

fn earliest(deadlines: [Option<Instant>; 3]) -> Option<Instant> {
    deadlines.into_iter().flatten().min()
}

/// How long the camera has delivered nothing, once that reaches `CAPTURE_STALL`.
fn stalled_for(last_frame_age: Option<Duration>, capture_age: Duration) -> Option<Duration> {
    let quiet = last_frame_age.map_or(capture_age, |a| a.min(capture_age));
    (quiet >= CAPTURE_STALL).then_some(quiet)
}

/// A consumer is waiting but the camera failed to open or died mid-stream.
fn consumer_lacks_camera(consumer_streaming: bool, capturing: bool, started: bool) -> bool {
    consumer_streaming && !capturing && started
}

fn spawn_provider(
    bridge: &Arc<Bridge>,
    fmt: PipelineFormat,
    capture_tx: &std_mpsc::Sender<WorkerMsg>,
    events: &mpsc::Sender<PipelineEvent>,
) -> ProviderHandle {
    let (quit_tx, quit_rx) = pw::channel::channel::<()>();
    let bridge = Arc::clone(bridge);
    let capture_tx = capture_tx.clone();
    let events = events.clone();
    let join = std::thread::spawn(move || {
        provider::run(bridge, fmt, capture_tx, events, quit_rx);
    });
    ProviderHandle {
        quit: quit_tx,
        join,
    }
}

/// Build + start the capture pipeline (opens the real camera) and report status.
fn start_capture(
    app: &AppState,
    fmt: PipelineFormat,
    bridge: &Arc<Bridge>,
    worker: &std_mpsc::Sender<WorkerMsg>,
    events: &mpsc::Sender<PipelineEvent>,
    capture: &mut Option<builder::BuiltCapture>,
    retrying: bool,
) {
    match builder::build_capture_pipeline(app, fmt, Arc::clone(bridge)).and_then(|c| {
        c.watch_inference(events.clone());
        c.start()?;
        c.watch_bus(worker.clone());
        Ok(c)
    }) {
        Ok(c) => {
            info!("capture started: consumer connected, camera opened");
            *capture = Some(c);
            let _ = events.blocking_send(PipelineEvent::Started {
                sink: format!("pipewire:{PIPEWIRE_NODE_NAME}"),
            });
        }
        // Expected until the camera is back; the status already says why.
        Err(err) if retrying => debug!(%err, "camera still unavailable"),
        Err(err) => {
            error!(%err, "capture start failed");
            let _ = events.blocking_send(PipelineEvent::Error(err.to_string()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const JUST_UNDER_STALL: Duration = CAPTURE_STALL.saturating_sub(Duration::from_millis(1));

    #[test]
    fn the_worker_wakes_for_the_earliest_pending_deadline() {
        let now = Instant::now();
        let later = now + Duration::from_secs(1);
        assert_eq!(earliest([Some(later), None, Some(now)]), Some(now));
        assert_eq!(earliest([None, Some(later), None]), Some(later));
    }

    #[test]
    fn the_worker_blocks_without_a_timeout_when_nothing_is_pending() {
        assert_eq!(earliest([None, None, None]), None);
    }

    #[test]
    fn a_capture_quiet_for_the_stall_limit_has_stalled() {
        assert_eq!(
            stalled_for(Some(CAPTURE_STALL), CAPTURE_STALL * 2),
            Some(CAPTURE_STALL)
        );
        assert_eq!(stalled_for(Some(JUST_UNDER_STALL), CAPTURE_STALL * 2), None);
    }

    #[test]
    fn a_capture_younger_than_the_stall_limit_has_not_stalled_on_an_old_frame() {
        assert_eq!(stalled_for(Some(CAPTURE_STALL * 2), JUST_UNDER_STALL), None);
    }

    #[test]
    fn a_capture_that_never_delivered_a_frame_stalls_on_its_own_age() {
        assert_eq!(stalled_for(None, CAPTURE_STALL), Some(CAPTURE_STALL));
        assert_eq!(stalled_for(None, JUST_UNDER_STALL), None);
    }

    #[test]
    fn only_a_streaming_consumer_of_a_started_daemon_with_no_capture_lacks_a_camera() {
        assert!(consumer_lacks_camera(true, false, true));
        assert!(!consumer_lacks_camera(false, false, true));
        assert!(!consumer_lacks_camera(true, true, true));
        assert!(!consumer_lacks_camera(true, false, false));
    }
}

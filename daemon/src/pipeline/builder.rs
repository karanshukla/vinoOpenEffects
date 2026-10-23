use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Sender as StdSender;

use anyhow::{Context, Result};
use gst::prelude::*;
use gstreamer as gst;
use gstreamer_app as gst_app;
use shared::config::AppState;
use tracing::{info, warn};

use super::bridge::Bridge;
use super::{PipelineFormat, WorkerMsg, cameras, effects};

/// The GStreamer half of the virtual camera: it captures the real webcam, runs
/// the effects bin, and hands each processed frame to the [`Bridge`] via an
/// appsink. It carries no PipeWire output sink — the native provide node
/// ([`super::provider`]) publishes the result. This pipeline only exists while a
/// consumer is connected; building it opens the camera, dropping it releases it.
pub struct BuiltCapture {
    pipeline: gst::Pipeline,
    id: u64,
    built_at: std::time::Instant,
}

static NEXT_CAPTURE_ID: AtomicU64 = AtomicU64::new(0);

const PLAYING_TIMEOUT: gst::ClockTime = gst::ClockTime::from_seconds(5);

impl BuiltCapture {
    pub fn start(&self) -> Result<()> {
        if let Err(err) = self.pipeline.set_state(gst::State::Playing) {
            let bus_err = drain_bus_error(&self.pipeline);
            let _ = self.pipeline.set_state(gst::State::Null);
            if let Some(detail) = bus_err {
                return Err(anyhow::anyhow!("set pipeline to playing: {err}: {detail}"));
            }
            return Err(anyhow::Error::new(err).context("set pipeline to playing"));
        }

        let (result, _current, _pending) = self.pipeline.state(Some(PLAYING_TIMEOUT));
        if result.is_err() {
            let bus_err = drain_bus_error(&self.pipeline);
            let _ = self.pipeline.set_state(gst::State::Null);
            if let Some(detail) = bus_err {
                return Err(anyhow::anyhow!(
                    "pipeline failed to reach PLAYING: {detail}"
                ));
            }
            return Err(anyhow::anyhow!("pipeline failed to reach PLAYING"));
        }
        Ok(())
    }

    /// Tells [`WorkerMsg::CaptureFailed`] from an earlier, already replaced pipeline apart.
    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn age(&self) -> std::time::Duration {
        self.built_at.elapsed()
    }

    /// After [`Self::start`]: report the first runtime error or EOS (the camera
    /// unplugged mid-stream) to the worker, and drop every other bus message so
    /// the unwatched bus doesn't queue them for the pipeline's lifetime.
    pub fn watch_bus(&self, worker: StdSender<WorkerMsg>) {
        let Some(bus) = self.pipeline.bus() else {
            return;
        };
        let id = self.id;
        let fired = AtomicBool::new(false);
        let report = move |msg: &gst::Message| {
            let Some(error) = runtime_failure(msg) else {
                return;
            };
            if !fired.swap(true, Ordering::Relaxed) {
                let _ = worker.send(WorkerMsg::CaptureFailed { id, error });
            }
        };
        let report = Arc::new(report);
        let handler = Arc::clone(&report);
        bus.set_sync_handler(move |_, msg| {
            handler(msg);
            gst::BusSyncReply::Drop
        });
        // Anything posted before the handler went in is still queued.
        while let Some(msg) = bus.pop() {
            report(&msg);
        }
    }

    pub fn stop(self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }

    pub fn apply_app_state(&self, app: &AppState) {
        if let Some(effects_el) = self.pipeline.by_name("oe_effects") {
            effects::apply_app_state_to_elements(&effects_el, app);
        }
    }

    /// Fire a one-shot reaction burst into the live effects element via its
    /// write-only `trigger-reaction` property.
    pub fn trigger_reaction(&self, id: &str) {
        if let Some(effects_el) = self.pipeline.by_name("oe_effects") {
            effects_el.set_property("trigger-reaction", id);
        }
    }

    pub fn swap_inference(&self, provider: &str) -> anyhow::Result<(String, Option<String>)> {
        match self.oe_effects() {
            Some(oe) => oe.swap_provider(provider),
            None => {
                let (active, detail) = oe_core::inference::predicted_active(provider);
                Ok((active.to_string(), detail))
            }
        }
    }

    /// Report the live models' provider each time a background load changes it.
    pub fn watch_inference(&self, events: tokio::sync::mpsc::Sender<super::PipelineEvent>) {
        let Some(oe) = self.oe_effects() else { return };
        oe.connect_notify(Some("inference-active"), move |el, _| {
            let _ = events.try_send(super::PipelineEvent::InferenceChanged {
                active: el.property("inference-active"),
                detail: el.property("inference-detail"),
            });
        });
    }

    /// The provider the live models are on, e.g. after a failed swap left them
    /// where they were.
    pub fn inference_active(&self) -> Option<String> {
        self.oe_effects()
            .map(|oe| oe.property::<String>("inference-active"))
    }

    fn oe_effects(&self) -> Option<crate::pipeline::filters::oe_effects::OeEffects> {
        self.pipeline.by_name("oe_effects")?.downcast().ok()
    }
}

/// The output caps: every processed frame the appsink delivers (and thus every
/// frame the provide node serves) is I420 at the camera's native mode.
fn output_caps(fmt: PipelineFormat) -> gst::Caps {
    gst::Caps::builder("video/x-raw")
        .field("format", "I420")
        .field("width", fmt.width as i32)
        .field("height", fmt.height as i32)
        .field("framerate", gst::Fraction::new(fmt.fps, 1))
        .build()
}

/// Caps pinned directly on the camera source: same dimensions/rate as the
/// output, in either raw or MJPEG (decodebin decodes the latter). This forces
/// the camera to capture *at* the virtual-camera mode instead of letting
/// negotiation pick an arbitrary default (e.g. raw 640x480 on cameras that only
/// do 720p in MJPEG), which videoscale would then stretch to the output size
/// and distort the aspect ratio.
fn source_caps(fmt: PipelineFormat) -> gst::Caps {
    let dims = |name: &str| {
        gst::Structure::builder(name)
            .field("width", fmt.width as i32)
            .field("height", fmt.height as i32)
            .field("framerate", gst::Fraction::new(fmt.fps, 1))
            .build()
    };
    gst::Caps::builder_full()
        .structure(dims("video/x-raw"))
        .structure(dims("image/jpeg"))
        .build()
}

/// The format the virtual camera should advertise for the currently selected
/// camera: its preferred native mode, or the default if nothing can be probed
/// (no camera -> `videotestsrc`, which renders any mode).
pub fn probe_format(app: &AppState) -> PipelineFormat {
    resolve_camera(app)
        .and_then(|info| cameras::preferred_format(&info))
        .unwrap_or(PipelineFormat::DEFAULT)
}

pub fn build_capture_pipeline(
    app: &AppState,
    fmt: PipelineFormat,
    bridge: Arc<Bridge>,
) -> Result<BuiltCapture> {
    let pipeline = gst::Pipeline::new();
    let source = build_source(app)?;
    let src_filter = gst::ElementFactory::make("capsfilter")
        .name("oe_src_caps")
        .property("caps", source_caps(fmt))
        .build()
        .context("create source capsfilter")?;

    let decoder = gst::ElementFactory::make("decodebin")
        .name("oe_in_decode")
        .build()
        .context("create input decodebin")?;

    let convert = gst::ElementFactory::make("videoconvert")
        .name("oe_in_convert")
        .build()
        .context("create input videoconvert")?;
    let scale = gst::ElementFactory::make("videoscale")
        .name("oe_in_scale")
        .build()
        .context("create input videoscale")?;
    let caps = gst::ElementFactory::make("capsfilter")
        .property("caps", output_caps(fmt))
        .build()
        .context("create capsfilter")?;

    let effects_bin = effects::build_effects_bin(app)?;

    let appsink = gst_app::AppSink::builder()
        .name("oe_appsink")
        .caps(&output_caps(fmt))
        .max_buffers(1)
        .drop(true)
        .build();

    appsink.set_callbacks(
        gst_app::AppSinkCallbacks::builder()
            .new_sample(move |sink| {
                let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                let buffer = sample.buffer().ok_or(gst::FlowError::Error)?;
                let map = buffer.map_readable().map_err(|_| gst::FlowError::Error)?;
                bridge.store(map.as_slice().to_vec());
                Ok(gst::FlowSuccess::Ok)
            })
            .build(),
    );

    pipeline.add_many([
        &source,
        &src_filter,
        &decoder,
        &convert,
        &scale,
        &caps,
        effects_bin.upcast_ref(),
        appsink.upcast_ref(),
    ])?;

    gst::Element::link_many([&source, &src_filter, &decoder])
        .context("link source -> src caps -> decoder")?;
    let convert_weak = convert.downgrade();
    decoder.connect_pad_added(move |_dbin, src_pad| {
        let Some(convert) = convert_weak.upgrade() else {
            return;
        };
        let Some(sink_pad) = convert.static_pad("sink") else {
            return;
        };
        if sink_pad.is_linked() {
            return;
        }
        if let Err(err) = src_pad.link(&sink_pad) {
            tracing::error!(?err, "failed to link decodebin -> videoconvert");
        }
    });
    gst::Element::link_many([
        &convert,
        &scale,
        &caps,
        effects_bin.upcast_ref(),
        appsink.upcast_ref(),
    ])?;

    Ok(BuiltCapture {
        pipeline,
        id: NEXT_CAPTURE_ID.fetch_add(1, Ordering::Relaxed),
        built_at: std::time::Instant::now(),
    })
}

fn runtime_failure(msg: &gst::Message) -> Option<String> {
    match msg.view() {
        gst::MessageView::Error(err) => Some(describe_error(err)),
        gst::MessageView::Eos(_) => Some("camera stream ended".into()),
        _ => None,
    }
}

fn describe_error(err: &gst::message::Error) -> String {
    let src = err
        .src()
        .map(|s| s.path_string().to_string())
        .unwrap_or_else(|| "<unknown>".into());
    let debug = err.debug().map(|d| format!(" [{d}]")).unwrap_or_default();
    format!("{src}: {}{debug}", err.error())
}

fn drain_bus_error(pipeline: &gst::Pipeline) -> Option<String> {
    let bus = pipeline.bus()?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(200);
    while std::time::Instant::now() < deadline {
        let Some(msg) = bus.timed_pop(Some(gst::ClockTime::from_mseconds(50))) else {
            continue;
        };
        if let gst::MessageView::Error(err) = msg.view() {
            return Some(describe_error(err));
        }
    }
    None
}

/// Resolve which physical camera the pipeline should use: the configured one
/// when it exists, otherwise autodetect. `None` means no camera at all
/// (`videotestsrc` fallback). Shared by `probe_format` and `build_source` so
/// the probed mode always belongs to the camera that is actually opened.
pub(crate) fn resolve_camera(app: &AppState) -> Option<cameras::CameraInfo> {
    let selected = app.camera.selected.trim();

    if !selected.is_empty() {
        if let Some(info) = cameras::enumerate().into_iter().find(|c| c.id == selected) {
            return Some(info);
        }
        if selected.starts_with("/dev/video") {
            return Some(cameras::CameraInfo {
                id: selected.to_string(),
                name: selected.to_string(),
                path: selected.to_string(),
                api: String::from("v4l2"),
            });
        }
        warn!(
            selected,
            "configured camera not found; falling back to autodetect"
        );
    }

    cameras::autodetect()
}

fn build_source(app: &AppState) -> Result<gst::Element> {
    if let Some(info) = resolve_camera(app) {
        info!(id = %info.id, name = %info.name, api = %info.api, "using camera");
        return cameras::build_source_for(&info);
    }

    warn!("no camera available; using videotestsrc fallback");
    gst::ElementFactory::make("videotestsrc")
        .property("is-live", true)
        .property_from_str("pattern", "smpte")
        .build()
        .context("create videotestsrc fallback")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failure(rx: &std::sync::mpsc::Receiver<WorkerMsg>) -> Option<(u64, String)> {
        match rx.try_recv().ok()? {
            WorkerMsg::CaptureFailed { id, error } => Some((id, error)),
            _ => None,
        }
    }

    #[test]
    fn bus_watch_reports_the_first_runtime_error_once() {
        gst::init().unwrap();
        let capture = BuiltCapture {
            pipeline: gst::Pipeline::new(),
            id: 7,
            built_at: std::time::Instant::now(),
        };
        let bus = capture.pipeline.bus().unwrap();
        let post_error = || {
            let msg = gst::message::Error::builder(gst::ResourceError::Read, "No such device")
                .src(&capture.pipeline)
                .build();
            bus.post(msg).unwrap();
        };
        // Posted before the watch: still reported.
        post_error();
        let (tx, rx) = std::sync::mpsc::channel();
        capture.watch_bus(tx);
        let (id, error) = failure(&rx).expect("queued error was not reported");
        assert_eq!(id, 7);
        assert!(error.contains("No such device"), "{error}");

        post_error();
        assert!(failure(&rx).is_none(), "a second error was reported");
        assert!(bus.pop().is_none(), "the watched bus still queues messages");
    }

    #[test]
    fn bus_watch_reports_eos() {
        gst::init().unwrap();
        let capture = BuiltCapture {
            pipeline: gst::Pipeline::new(),
            id: 1,
            built_at: std::time::Instant::now(),
        };
        let (tx, rx) = std::sync::mpsc::channel();
        capture.watch_bus(tx);
        let bus = capture.pipeline.bus().unwrap();
        bus.post(gst::message::Latency::new()).unwrap();
        assert!(failure(&rx).is_none(), "latency counted as a failure");
        bus.post(gst::message::Eos::new()).unwrap();
        assert!(failure(&rx).is_some(), "EOS was not reported");
    }
}

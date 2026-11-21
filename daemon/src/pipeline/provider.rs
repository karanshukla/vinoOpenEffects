//! The native libpipewire `Video/Source` node consumers see as the camera.
//! It waits in `PAUSED` with the real camera closed; a consumer moving it to
//! `STREAMING` makes the worker open the capture pipeline, and leaving closes
//! it again. Graph laziness alone doesn't gate a hardware capture stream, so
//! this `state_changed` gating is what keeps the webcam LED off until an app
//! opens the camera.
//!
//! libpipewire objects aren't `Send`, so the node runs a `pw_main_loop` on its
//! own thread and talks to the daemon only through channels and the shared
//! [`Bridge`](super::bridge::Bridge).

use std::io::Cursor;
use std::sync::Arc;
use std::sync::mpsc::Sender as StdSender;
use std::time::Duration;

use pipewire as pw;
use pw::spa::pod::Pod;
use pw::{properties::properties, spa};
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use super::bridge::Bridge;
use super::probe::PIPEWIRE_NODE_NAME;
use super::{CaptureCmd, PipelineEvent, PipelineFormat, WorkerMsg};

/// State carried through the stream callbacks.
struct UserData {
    /// Emit `VirtualCameraVerified` only once, on first registration.
    announced: bool,
    capture_tx: StdSender<WorkerMsg>,
    /// Arms (`true`) or parks (`false`) the frame tick thread.
    tick_gate: StdSender<bool>,
    events: mpsc::Sender<PipelineEvent>,
    bridge: Arc<Bridge>,
    /// The advertised video format (the selected camera's native mode).
    fmt: PipelineFormat,
    /// Monotonic frame counter for the buffer header meta (`seq`/`pts`).
    frame_count: u64,
}

/// Run the provide node until `quit_rx` receives. A setup failure (no
/// PipeWire, say) emits an `Error` event and returns.
pub fn run(
    bridge: Arc<Bridge>,
    fmt: PipelineFormat,
    capture_tx: StdSender<WorkerMsg>,
    events: mpsc::Sender<PipelineEvent>,
    quit_rx: pw::channel::Receiver<()>,
) {
    if let Err(err) = run_inner(bridge, fmt, capture_tx, events.clone(), quit_rx) {
        error!(%err, "provide node failed");
        let _ = events.blocking_send(PipelineEvent::Error(format!("provide node: {err}")));
    }
}

fn run_inner(
    bridge: Arc<Bridge>,
    fmt: PipelineFormat,
    capture_tx: StdSender<WorkerMsg>,
    events: mpsc::Sender<PipelineEvent>,
    quit_rx: pw::channel::Receiver<()>,
) -> Result<(), pw::Error> {
    pw::init();

    let mainloop = pw::main_loop::MainLoopRc::new(None)?;
    let context = pw::context::ContextRc::new(&mainloop, None)?;
    let core = context.connect_rc(None)?;

    let _quit = quit_rx.attach(mainloop.loop_(), {
        let mainloop = mainloop.clone();
        move |_| mainloop.quit()
    });

    let (tick_tx, tick_rx) = pw::channel::channel::<()>();
    let (tick_gate, gate_rx) = std::sync::mpsc::channel::<bool>();
    spawn_tick_thread(
        Duration::from_nanos(frame_duration_ns(fmt)),
        gate_rx,
        tick_tx,
    );

    let data = UserData {
        announced: false,
        capture_tx,
        tick_gate,
        events,
        bridge,
        fmt,
        frame_count: 0,
    };

    let stream = pw::stream::StreamRc::new(
        core,
        "vinoopeneffects-source",
        properties! {
            *pw::keys::MEDIA_TYPE => "Video",
            *pw::keys::MEDIA_CATEGORY => "Source",
            *pw::keys::MEDIA_ROLE => "Camera",
            // WirePlumber only offers `Video/Source` nodes to consumers as cameras.
            *pw::keys::MEDIA_CLASS => "Video/Source",
            *pw::keys::NODE_NAME => PIPEWIRE_NODE_NAME,
            *pw::keys::NODE_DESCRIPTION => "vinoOpenEffects Virtual Camera",
        },
    )?;

    let _listener = stream
        .add_local_listener_with_user_data(data)
        .state_changed(|_stream, ud, old, new| {
            use pw::stream::StreamState;
            info!(?old, ?new, "provide node state changed");
            if matches!(new, StreamState::Paused) && !ud.announced {
                ud.announced = true;
                let _ = ud
                    .events
                    .blocking_send(PipelineEvent::VirtualCameraVerified {
                        node_name: PIPEWIRE_NODE_NAME.into(),
                        found: true,
                    });
            }
            if let Some(streaming) = consumer_streaming(&new) {
                let capture = if streaming {
                    CaptureCmd::Start
                } else {
                    CaptureCmd::Stop
                };
                let _ = ud.tick_gate.send(streaming);
                let _ = ud.capture_tx.send(WorkerMsg::Capture(capture));
            }
        })
        .param_changed(|stream, ud, id, param| {
            if param.is_none() || id != pw::spa::param::ParamType::Format.as_raw() {
                return;
            }
            let buffers = build_buffers_pod(ud.fmt);
            let meta_header = build_meta_header_pod();
            let (Some(buffers_pod), Some(meta_pod)) =
                (Pod::from_bytes(&buffers), Pod::from_bytes(&meta_header))
            else {
                return;
            };
            let mut params = [buffers_pod, meta_pod];
            if let Err(err) = stream.update_params(&mut params) {
                warn!(%err, "update_params(Buffers, Meta) failed");
            }
        })
        .process(|stream, ud| {
            let Some(buffer) = Dequeued::new(stream) else {
                return;
            };
            {
                let datas = buffer.datas_mut();
                if datas.is_empty() {
                    return;
                }
                let data = &mut datas[0];

                let written = if let Some(slice) = data.data() {
                    match ud.bridge.copy_latest(slice) {
                        Some(n) => n,
                        None => {
                            let n = ud.fmt.frame_size().min(slice.len());
                            fill_black_i420(&mut slice[..n], ud.fmt);
                            n
                        }
                    }
                } else {
                    0
                };

                let chunk = data.chunk_mut();
                *chunk.offset_mut() = 0;
                *chunk.stride_mut() = ud.fmt.stride() as i32;
                *chunk.size_mut() = written as u32;
            }

            if let Some(h) = buffer.header_meta() {
                // SAFETY: `h` points into this dequeued buffer's meta region, which the
                // producer may write until it queues the buffer back, and nothing else
                // borrows it.
                unsafe {
                    (*h).flags = 0;
                    (*h).offset = 0;
                    (*h).pts = (ud.frame_count * frame_duration_ns(ud.fmt)) as i64;
                    (*h).dts_offset = 0;
                    (*h).seq = ud.frame_count;
                }
                ud.frame_count += 1;
            }
        })
        .register()?;

    let format = build_format_pod(fmt);
    let Some(format_pod) = Pod::from_bytes(&format) else {
        return Err(pw::Error::CreationFailed);
    };
    let mut params = [format_pod];

    // Camera consumers never cycle a graph whose source isn't DRIVER, and without RT_PROCESS
    // PipeWire >= 1.2 schedules the driver async, which crash-loops Chromium's capture service.
    stream.connect(
        spa::utils::Direction::Output,
        None,
        pw::stream::StreamFlags::DRIVER
            | pw::stream::StreamFlags::MAP_BUFFERS
            | pw::stream::StreamFlags::RT_PROCESS,
        &mut params,
    )?;

    let _tick = tick_rx.attach(mainloop.loop_(), {
        let stream = stream.clone();
        move |()| {
            if stream.is_driving() {
                let _ = stream.trigger_process();
            }
        }
    });

    info!(
        node = PIPEWIRE_NODE_NAME,
        "virtual camera advertised; waiting for consumers"
    );
    mainloop.run();
    info!("provide node loop exited");
    Ok(())
}

/// A buffer dequeued from `stream`, queued back when dropped. pipewire-rs's
/// `Buffer` only hands out the header meta as `&MetaHeader`, and writing
/// through a shared reference is undefined behaviour, so this keeps the raw
/// `spa_buffer` to reach the meta through a pointer that may write.
struct Dequeued<'s> {
    stream: &'s pw::stream::Stream,
    buf: std::ptr::NonNull<pw::sys::pw_buffer>,
}

impl<'s> Dequeued<'s> {
    fn new(stream: &'s pw::stream::Stream) -> Option<Self> {
        // SAFETY: a pointer from `dequeue_raw_buffer` is queued back exactly
        // once, in `drop`.
        let buf = std::ptr::NonNull::new(unsafe { stream.dequeue_raw_buffer() })?;
        Some(Self { stream, buf })
    }

    fn spa_buffer(&self) -> *mut spa::sys::spa_buffer {
        // SAFETY: `buf` is a live dequeued `pw_buffer` until `drop`.
        unsafe { self.buf.as_ref().buffer }
    }

    #[allow(clippy::mut_from_ref)] // the slice aliases the buffer's own memory, not `self`
    fn datas_mut(&self) -> &mut [spa::buffer::Data] {
        let b = self.spa_buffer();
        // SAFETY: as pipewire-rs's `Buffer::datas_mut`: `Data` is a transparent
        // `spa_data`, and the array lives as long as the dequeued buffer. One
        // call per cycle, so no two slices alias.
        unsafe {
            if b.is_null() || (*b).n_datas == 0 || (*b).datas.is_null() {
                return &mut [];
            }
            std::slice::from_raw_parts_mut(
                (*b).datas.cast::<spa::buffer::Data>(),
                (*b).n_datas as usize,
            )
        }
    }

    /// The buffer's `SPA_META_Header`, if it carries one big enough.
    fn header_meta(&self) -> Option<*mut spa::sys::spa_meta_header> {
        let b = self.spa_buffer();
        if b.is_null() {
            return None;
        }
        // SAFETY: `metas` holds `n_metas` entries for as long as the buffer is dequeued.
        unsafe {
            if (*b).metas.is_null() {
                return None;
            }
            std::slice::from_raw_parts((*b).metas, (*b).n_metas as usize)
                .iter()
                .find(|m| {
                    m.type_ == spa::sys::SPA_META_Header
                        && m.size as usize >= std::mem::size_of::<spa::sys::spa_meta_header>()
                        && !m.data.is_null()
                })
                .map(|m| m.data.cast::<spa::sys::spa_meta_header>())
        }
    }
}

impl Drop for Dequeued<'_> {
    fn drop(&mut self) {
        // SAFETY: `buf` came from this stream's `dequeue_raw_buffer` and is queued once.
        unsafe { self.stream.queue_raw_buffer(self.buf.as_ptr()) }
    }
}

/// Whether a consumer is streaming in this state; `None` while it has yet to settle.
fn consumer_streaming(state: &pw::stream::StreamState) -> Option<bool> {
    use pw::stream::StreamState;
    match state {
        StreamState::Streaming => Some(true),
        StreamState::Paused | StreamState::Unconnected | StreamState::Error(_) => Some(false),
        StreamState::Connecting => None,
    }
}

fn frame_duration_ns(fmt: PipelineFormat) -> u64 {
    1_000_000_000 / fmt.fps as u64
}

/// Sends a tick every `interval` while the gate is open and blocks while it is
/// closed. Exits once the gate sender or the tick receiver is gone.
fn spawn_tick_thread(
    interval: Duration,
    gate: std::sync::mpsc::Receiver<bool>,
    ticks: pw::channel::Sender<()>,
) {
    std::thread::spawn(move || {
        let mut open = false;
        loop {
            if !open {
                match gate.recv() {
                    Ok(v) => open = v,
                    Err(_) => return,
                }
                continue;
            }
            let mut next = std::time::Instant::now() + interval;
            while open {
                match gate.recv_timeout(next.saturating_duration_since(std::time::Instant::now())) {
                    Ok(v) => open = v,
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                        if ticks.send(()).is_err() {
                            return;
                        }
                        next += interval;
                    }
                }
            }
        }
    });
}

/// The one format consumers are offered: I420 at the camera's native mode,
/// what the capture pipeline delivers.
fn build_format_pod(fmt: PipelineFormat) -> Vec<u8> {
    let obj = pw::spa::pod::object!(
        pw::spa::utils::SpaTypes::ObjectParamFormat,
        pw::spa::param::ParamType::EnumFormat,
        pw::spa::pod::property!(
            pw::spa::param::format::FormatProperties::MediaType,
            Id,
            pw::spa::param::format::MediaType::Video
        ),
        pw::spa::pod::property!(
            pw::spa::param::format::FormatProperties::MediaSubtype,
            Id,
            pw::spa::param::format::MediaSubtype::Raw
        ),
        pw::spa::pod::property!(
            pw::spa::param::format::FormatProperties::VideoFormat,
            Id,
            pw::spa::param::video::VideoFormat::I420
        ),
        pw::spa::pod::property!(
            pw::spa::param::format::FormatProperties::VideoSize,
            Rectangle,
            pw::spa::utils::Rectangle {
                width: fmt.width,
                height: fmt.height
            }
        ),
        pw::spa::pod::property!(
            pw::spa::param::format::FormatProperties::VideoFramerate,
            Fraction,
            pw::spa::utils::Fraction {
                num: fmt.fps as u32,
                denom: 1
            }
        ),
    );
    pw::spa::pod::serialize::PodSerializer::serialize(
        Cursor::new(Vec::new()),
        &pw::spa::pod::Value::Object(obj),
    )
    .unwrap()
    .0
    .into_inner()
}

/// Buffer requirements for format negotiation, from raw `spa_sys` constants:
/// libspa-rs has no typed enum for these keys.
fn build_buffers_pod(fmt: PipelineFormat) -> Vec<u8> {
    use pw::spa::pod::{ChoiceValue, Object, Property, Value};
    use pw::spa::sys;
    use pw::spa::utils::{Choice, ChoiceEnum, ChoiceFlags};

    let mem_ptr = 1i32 << sys::SPA_DATA_MemPtr;

    let obj = Object {
        type_: sys::SPA_TYPE_OBJECT_ParamBuffers,
        id: sys::SPA_PARAM_Buffers,
        properties: vec![
            Property::new(
                sys::SPA_PARAM_BUFFERS_buffers,
                Value::Choice(ChoiceValue::Int(Choice(
                    ChoiceFlags::empty(),
                    ChoiceEnum::Range {
                        default: 8,
                        min: 2,
                        max: 16,
                    },
                ))),
            ),
            Property::new(sys::SPA_PARAM_BUFFERS_blocks, Value::Int(1)),
            Property::new(
                sys::SPA_PARAM_BUFFERS_size,
                Value::Int(fmt.frame_size() as i32),
            ),
            Property::new(
                sys::SPA_PARAM_BUFFERS_stride,
                Value::Int(fmt.stride() as i32),
            ),
            Property::new(
                sys::SPA_PARAM_BUFFERS_dataType,
                Value::Choice(ChoiceValue::Int(Choice(
                    ChoiceFlags::empty(),
                    ChoiceEnum::Flags {
                        default: mem_ptr,
                        flags: vec![mem_ptr],
                    },
                ))),
            ),
        ],
    };
    pw::spa::pod::serialize::PodSerializer::serialize(Cursor::new(Vec::new()), &Value::Object(obj))
        .unwrap()
        .0
        .into_inner()
}

/// Announce support for the buffer header meta (`SPA_META_Header`). WebRTC
/// consumers request it and crash if the negotiated buffers don't carry it; the
/// region is only allocated when the producer side lists it too.
fn build_meta_header_pod() -> Vec<u8> {
    use pw::spa::pod::{Object, Property, Value};
    use pw::spa::sys;
    use pw::spa::utils::Id;

    let obj = Object {
        type_: sys::SPA_TYPE_OBJECT_ParamMeta,
        id: sys::SPA_PARAM_Meta,
        properties: vec![
            Property::new(
                sys::SPA_PARAM_META_type,
                Value::Id(Id(sys::SPA_META_Header)),
            ),
            Property::new(
                sys::SPA_PARAM_META_size,
                Value::Int(std::mem::size_of::<sys::spa_meta_header>() as i32),
            ),
        ],
    };
    pw::spa::pod::serialize::PodSerializer::serialize(Cursor::new(Vec::new()), &Value::Object(obj))
        .unwrap()
        .0
        .into_inner()
}

/// Fill an I420 buffer with limited-range black (Y=16, Cb=Cr=128).
fn fill_black_i420(buf: &mut [u8], fmt: PipelineFormat) {
    let y_size = (fmt.width as usize) * (fmt.height as usize);
    let chroma = y_size / 4;
    let end = (y_size + 2 * chroma).min(buf.len());
    let y_end = y_size.min(buf.len());
    for b in &mut buf[..y_end] {
        *b = 16;
    }
    if y_end < end {
        for b in &mut buf[y_end..end] {
            *b = 128;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pw::stream::StreamState;

    const TINY: PipelineFormat = PipelineFormat {
        width: 4,
        height: 2,
        fps: 30,
    };

    #[test]
    fn only_a_streaming_node_opens_the_camera() {
        assert_eq!(consumer_streaming(&StreamState::Streaming), Some(true));
    }

    #[test]
    fn a_paused_unconnected_or_failed_node_closes_the_camera() {
        for state in [
            StreamState::Paused,
            StreamState::Unconnected,
            StreamState::Error("gone".into()),
        ] {
            assert_eq!(consumer_streaming(&state), Some(false));
        }
    }

    #[test]
    fn a_connecting_node_leaves_the_camera_as_it_is() {
        assert_eq!(consumer_streaming(&StreamState::Connecting), None);
    }

    #[test]
    fn a_frame_lasts_a_whole_number_of_nanoseconds_at_the_advertised_rate() {
        assert_eq!(frame_duration_ns(TINY), 33_333_333);
        assert_eq!(
            frame_duration_ns(PipelineFormat { fps: 60, ..TINY }),
            16_666_666
        );
    }

    #[test]
    fn black_is_limited_range_luma_then_neutral_chroma() {
        let mut frame = vec![0u8; TINY.frame_size()];
        fill_black_i420(&mut frame, TINY);
        assert_eq!(frame, [16, 16, 16, 16, 16, 16, 16, 16, 128, 128, 128, 128]);
    }

    #[test]
    fn black_fills_only_as_far_as_a_short_or_long_buffer_allows() {
        let mut short = vec![0u8; 9];
        fill_black_i420(&mut short, TINY);
        assert_eq!(short, [16, 16, 16, 16, 16, 16, 16, 16, 128]);

        let mut luma_only = vec![0u8; 8];
        fill_black_i420(&mut luma_only, TINY);
        assert_eq!(luma_only, [16; 8]);

        let mut long = vec![0u8; 13];
        fill_black_i420(&mut long, TINY);
        assert_eq!(long[11..], [128, 0]);
    }
}

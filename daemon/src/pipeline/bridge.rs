//! The latest-frame slot between the GStreamer pipeline's appsink, which
//! writes, and the PipeWire provide node, which reads. Only the newest frame
//! is kept: a live camera never queues stale frames.

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Latest processed frame in the fixed pipeline format (I420, see
/// [`super::PipelineFormat::frame_size`]). `None` until the first frame arrives, or after
/// [`Bridge::clear`].
#[derive(Default)]
pub struct Bridge {
    latest: Mutex<Option<Vec<u8>>>,
    stored_at: Mutex<Option<Instant>>,
}

impl Bridge {
    pub fn new() -> Self {
        Self {
            latest: Mutex::new(None),
            stored_at: Mutex::new(None),
        }
    }

    /// Store the newest processed frame, dropping any previous one.
    pub fn store(&self, frame: Vec<u8>) {
        *self.latest.lock().unwrap() = Some(frame);
        *self.stored_at.lock().unwrap() = Some(Instant::now());
    }

    /// Time since the last [`Self::store`], or `None` if nothing was stored since the last clear.
    pub fn age(&self) -> Option<Duration> {
        self.stored_at.lock().unwrap().map(|t| t.elapsed())
    }

    /// Copy the latest frame into `dst`; the bytes written, or `None` with no frame.
    pub fn copy_latest(&self, dst: &mut [u8]) -> Option<usize> {
        let guard = self.latest.lock().unwrap();
        let frame = guard.as_ref()?;
        let n = frame.len().min(dst.len());
        dst[..n].copy_from_slice(&frame[..n]);
        Some(n)
    }

    /// Drop the held frame, so a reconnect never serves the last session's.
    pub fn clear(&self) {
        *self.latest.lock().unwrap() = None;
        *self.stored_at.lock().unwrap() = None;
    }
}

#[cfg(test)]
mod tests {
    use super::Bridge;

    #[test]
    fn store_then_copy_roundtrips() {
        let bridge = Bridge::new();
        let mut dst = [0u8; 4];
        assert_eq!(bridge.copy_latest(&mut dst), None);

        bridge.store(vec![1, 2, 3, 4]);
        assert_eq!(bridge.copy_latest(&mut dst), Some(4));
        assert_eq!(dst, [1, 2, 3, 4]);

        assert!(bridge.age().is_some());

        bridge.clear();
        assert_eq!(bridge.copy_latest(&mut dst), None);
        assert_eq!(bridge.age(), None);
    }

    #[test]
    fn store_overwrites_previous_frame() {
        let bridge = Bridge::new();
        bridge.store(vec![9, 9]);
        bridge.store(vec![7, 7]);
        let mut dst = [0u8; 2];
        bridge.copy_latest(&mut dst);
        assert_eq!(dst, [7, 7]);
    }
}

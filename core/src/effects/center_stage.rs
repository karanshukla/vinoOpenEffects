//! Center Stage: a YuNet face box drives an EMA-smoothed, deadzoned crop+zoom
//! that keeps the face centered without distorting the aspect ratio.

use super::resample::sample_bilinear;
use super::settings::Settings;
use crate::inference::engine::Engine;
use crate::inference::models::Face;

/// Minimum YuNet score to accept a face for framing.
const FACE_SCORE_THRESH: f32 = 0.6;
/// Run YuNet every Nth frame; the comfort zone absorbs the staleness.
const FACE_INTERVAL: u64 = 4;
/// EMA weight for the crop (about 12 frames to 90%).
const CS_ALPHA: f32 = 0.18;
/// Tightest crop as a fraction of each axis, capping zoom at 4×.
const CS_MIN_CROP: f32 = 0.25;
/// Framing stays locked while the subject drifts less than this fraction of
/// the crop span (docs/architecture.md#center-stage).
const CS_COMFORT_POS: f32 = 0.20;
/// Framing stays locked while the ideal/committed zoom ratio is inside this band.
const CS_COMFORT_ZOOM_LO: f32 = 0.78;
const CS_COMFORT_ZOOM_HI: f32 = 1.30;
/// A drift past this fraction of the span, or a zoom ratio outside this band,
/// reframes after `CS_FAST_CONFIRM` frames instead of `CS_CONFIRM`.
const CS_BREACH_POS: f32 = 0.45;
const CS_BREACH_ZOOM_LO: f32 = 0.55;
const CS_BREACH_ZOOM_HI: f32 = 1.85;
/// Consecutive out-of-comfort frames before reframing: about 1 s, or 0.3 s for a breach.
const CS_CONFIRM: u32 = 30;
const CS_FAST_CONFIRM: u32 = 9;
const EYES_ABOVE_FACE_CENTER: f32 = 0.25;
/// A crop at least this wide is the whole frame: nothing to resample.
const FULL_FRAME: f32 = 0.999;

/// A crop window in normalized frame coordinates. `zf` is the fraction of
/// each axis it spans (1.0 = full frame); the same fraction on both axes keeps
/// the frame's aspect ratio.
#[derive(Clone, Copy)]
pub(crate) struct CropState {
    cx: f32,
    cy: f32,
    zf: f32,
}

impl Default for CropState {
    fn default() -> Self {
        Self {
            cx: 0.5,
            cy: 0.5,
            zf: 1.0,
        }
    }
}

#[derive(Default)]
pub(crate) struct CenterStage {
    /// EMA-smoothed crop actually applied this frame.
    crop: CropState,
    /// Committed framing target the crop eases toward.
    target: CropState,
    /// Consecutive frames the ideal crop has been outside `target`'s comfort zone.
    out_frames: u32,
    pub(crate) faces: Vec<Face>,
}

impl CenterStage {
    /// Back to the full frame (on a resolution change). Faces are kept.
    pub(crate) fn reset(&mut self) {
        self.crop = CropState::default();
        self.target = CropState::default();
        self.out_frames = 0;
    }

    /// This frame's crop, decided on the full frame.
    pub(crate) fn track(
        &mut self,
        engine: &mut Engine,
        settings: &Settings,
        frame: u64,
        img: &[u8],
        w: usize,
        h: usize,
    ) -> CropState {
        if frame % FACE_INTERVAL == 1 {
            let faces = if let Some(model) = engine.yunet() {
                model
                    .detect(img, w, h, w * 4, FACE_SCORE_THRESH)
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            self.faces = faces;
        }

        if let Some(subject) = subject_box(&self.faces, &settings.cs_mode) {
            self.update_target(crop_target(subject, &settings.cs_zoom));
        }

        let t = self.target;
        let crop = &mut self.crop;
        crop.zf += (t.zf - crop.zf) * CS_ALPHA;
        crop.cx += (t.cx - crop.cx) * CS_ALPHA;
        crop.cy += (t.cy - crop.cy) * CS_ALPHA;
        *crop
    }

    /// Commit `raw` as the target only once it has stayed outside the comfort
    /// zone for the confirmation window (pinned by
    /// `tests::drift_inside_the_comfort_zone_never_reframes`,
    /// `tests::sustained_drift_reframes_after_the_confirm_window`,
    /// `tests::a_drastic_move_reframes_on_the_fast_path`,
    /// `tests::returning_in_time_resets_the_count`).
    fn update_target(&mut self, raw: CropState) {
        let t = self.target;
        let span = t.zf.max(CS_MIN_CROP);
        let ex = (raw.cx - t.cx).abs() / span;
        let ey = (raw.cy - t.cy).abs() / span;
        let rz = raw.zf / t.zf;

        let comfortable = ex < CS_COMFORT_POS
            && ey < CS_COMFORT_POS
            && (CS_COMFORT_ZOOM_LO..CS_COMFORT_ZOOM_HI).contains(&rz);
        if comfortable {
            self.out_frames = 0;
            return;
        }

        self.out_frames += 1;
        let drastic = ex > CS_BREACH_POS
            || ey > CS_BREACH_POS
            || !(CS_BREACH_ZOOM_LO..=CS_BREACH_ZOOM_HI).contains(&rz);
        let confirm = if drastic { CS_FAST_CONFIRM } else { CS_CONFIRM };
        if self.out_frames >= confirm {
            self.target = raw;
            self.out_frames = 0;
        }
    }
}

/// The crop window copied out at whole-pixel bounds, plus where the exact
/// crop sits inside it, so the frame is resampled only once, on the way back
/// up (pinned by `tests::roi_round_trip_resamples_once`).
pub(crate) struct Roi {
    pub(crate) buf: Vec<u8>,
    pub(crate) w: usize,
    pub(crate) h: usize,
    window: Window,
}

/// A fractional rectangle of a source image, in pixels.
#[derive(Clone, Copy)]
struct Window {
    x0: f32,
    y0: f32,
    span_x: f32,
    span_y: f32,
}

/// The pixel rectangle `crop` covers in a `w`×`h` frame.
fn pixel_window(crop: CropState, zf: f32, w: usize, h: usize) -> Window {
    Window {
        x0: (crop.cx - zf / 2.0) * w as f32,
        y0: (crop.cy - zf / 2.0) * h as f32,
        span_x: zf * w as f32,
        span_y: zf * h as f32,
    }
}

/// Fill the `w`×`h` `dst` with `window` of `src`, bilinearly.
fn resample_window(
    dst: &mut [u8],
    w: usize,
    h: usize,
    src: &[u8],
    sw: usize,
    sh: usize,
    window: Window,
) {
    for y in 0..h {
        let sv = window.y0 + (y as f32 + 0.5) / h as f32 * window.span_y - 0.5;
        for x in 0..w {
            let su = window.x0 + (x as f32 + 0.5) / w as f32 * window.span_x - 0.5;
            let dp = (y * w + x) * 4;
            sample_bilinear(src, sw, sh, su, sv, &mut dst[dp..dp + 4]);
        }
    }
}

/// Copy the crop window into a tight RGBA buffer so segmentation, blur and
/// composite run on ROI-sized pixels. `None` when the crop spans the frame.
pub(crate) fn extract_roi(img: &[u8], w: usize, h: usize, crop: CropState) -> Option<Roi> {
    let zf = crop.zf.clamp(CS_MIN_CROP, 1.0);
    if zf >= FULL_FRAME {
        return None;
    }
    let exact = pixel_window(crop, zf, w, h);
    let (fx, fy) = (exact.x0, exact.y0);
    // One pixel of margin each side: bilinear at the crop's edge reads its neighbour.
    let ix = ((fx.floor() - 1.0).max(0.0) as usize).min(w - 1);
    let iy = ((fy.floor() - 1.0).max(0.0) as usize).min(h - 1);
    let ex = (((fx + exact.span_x).ceil() + 1.0) as usize).clamp(ix + 1, w);
    let ey = (((fy + exact.span_y).ceil() + 1.0) as usize).clamp(iy + 1, h);
    let (rw, rh) = (ex - ix, ey - iy);
    let mut buf = Vec::with_capacity(rw * rh * 4);
    for y in iy..ey {
        buf.extend_from_slice(&img[(y * w + ix) * 4..(y * w + ex) * 4]);
    }
    Some(Roi {
        buf,
        w: rw,
        h: rh,
        window: Window {
            x0: fx - ix as f32,
            y0: fy - iy as f32,
            ..exact
        },
    })
}

/// Upscale the composited ROI's exact crop back into the full-frame output.
pub(crate) fn upscale_roi(img: &mut [u8], w: usize, h: usize, roi: &Roi) {
    resample_window(img, w, h, &roi.buf, roi.w, roi.h, roi.window);
}

/// Resample the crop window back up to the full frame, in place.
pub(crate) fn resample_crop(img: &mut [u8], w: usize, h: usize, crop: CropState) {
    if crop.zf >= FULL_FRAME {
        return;
    }
    let src = img.to_vec();
    resample_window(img, w, h, &src, w, h, pixel_window(crop, crop.zf, w, h));
}

/// What the framing `span` was derived from, which sets its margin scale.
#[derive(Clone, Copy, Debug, PartialEq)]
enum SubjectKind {
    /// A single face's bbox height.
    Face,
    /// The union of all face boxes.
    Group,
}

/// What to frame on, normalized.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Subject {
    cx: f32,
    cy: f32,
    span: f32,
    kind: SubjectKind,
}

/// One face: centered on the eyes and sized by the bbox height, which a
/// smile doesn't change (bbox center without landmarks). Group: the union
/// box, padded by the mean face height.
fn subject_box(faces: &[Face], mode: &str) -> Option<Subject> {
    if faces.is_empty() {
        return None;
    }
    if mode == "group" {
        let (mut x0, mut y0, mut x1, mut y1) = (1.0f32, 1.0f32, 0.0f32, 0.0f32);
        let mut height_sum = 0.0f32;
        for f in faces {
            x0 = x0.min(f.cx - f.w / 2.0);
            y0 = y0.min(f.cy - f.h / 2.0);
            x1 = x1.max(f.cx + f.w / 2.0);
            y1 = y1.max(f.cy + f.h / 2.0);
            height_sum += f.h;
        }
        let pad = height_sum / faces.len() as f32;
        return Some(Subject {
            cx: (x0 + x1) / 2.0,
            cy: (y0 + y1) / 2.0,
            span: (x1 - x0).max(y1 - y0) + pad,
            kind: SubjectKind::Group,
        });
    }
    let best = faces.iter().max_by(|a, b| a.score.total_cmp(&b.score))?;
    let (cx, cy) = if best.eye_dist > 0.0 {
        (best.eye_cx, best.eye_cy + best.h * EYES_ABOVE_FACE_CENTER)
    } else {
        (best.cx, best.cy)
    };
    Some(Subject {
        cx,
        cy,
        span: best.h,
        kind: SubjectKind::Face,
    })
}

/// Crop span = subject span × margin. Every margin is above 1, so even
/// "tight" never cuts the face (pinned by `tests::every_zoom_encloses_the_subject`).
fn zoom_margin(zoom: &str, kind: SubjectKind) -> f32 {
    match kind {
        SubjectKind::Face => match zoom {
            "subtle" => 3.4,
            "tight" => 2.1,
            _ => 2.7,
        },
        SubjectKind::Group => match zoom {
            "subtle" => 1.6,
            "tight" => 1.15,
            _ => 1.4,
        },
    }
}

/// The ideal crop around `subject`, clamped inside the frame.
fn crop_target(subject: Subject, zoom: &str) -> CropState {
    let zf = (subject.span * zoom_margin(zoom, subject.kind)).clamp(CS_MIN_CROP, 1.0);
    let half = zf / 2.0;
    CropState {
        cx: subject.cx.clamp(half, 1.0 - half),
        cy: subject.cy.clamp(half, 1.0 - half),
        zf,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roi_round_trip_resamples_once() {
        let (w, h) = (64, 48);
        let img: Vec<u8> = (0..w * h)
            .flat_map(|i| {
                let (x, y) = (i % w, i / w);
                [(x * 4) as u8, (y * 5) as u8, ((x * y) % 251) as u8, 255]
            })
            .collect();
        let crop = CropState {
            cx: 0.537,
            cy: 0.461,
            zf: 0.413,
        };
        let mut direct = img.clone();
        resample_crop(&mut direct, w, h, crop);

        let roi = extract_roi(&img, w, h, crop).unwrap();
        let mut via_roi = img.clone();
        upscale_roi(&mut via_roi, w, h, &roi);
        assert_eq!(via_roi, direct);
    }

    #[test]
    fn full_frame_crop_has_no_roi() {
        assert!(extract_roi(&[0; 16], 2, 2, CropState::default()).is_none());
    }

    fn crop(cx: f32, zf: f32) -> CropState {
        CropState { cx, cy: 0.5, zf }
    }

    /// A stage committed to a half-frame crop at the center.
    fn settled() -> CenterStage {
        let mut stage = CenterStage::default();
        stage.target = crop(0.5, 0.5);
        stage
    }

    /// Feed `raw` for `frames` frames; whether the target moved to it.
    fn reframes_after(stage: &mut CenterStage, raw: CropState, frames: u32) -> bool {
        for _ in 0..frames {
            stage.update_target(raw);
        }
        stage.target.cx == raw.cx
    }

    #[test]
    fn drift_inside_the_comfort_zone_never_reframes() {
        let mut stage = settled();
        let drift = 0.5 + 0.5 * CS_COMFORT_POS * 0.9;
        assert!(!reframes_after(
            &mut stage,
            crop(drift, 0.5),
            10 * CS_CONFIRM
        ));
    }

    #[test]
    fn sustained_drift_reframes_after_the_confirm_window() {
        let drift = crop(0.5 + 0.5 * (CS_COMFORT_POS + CS_BREACH_POS) / 2.0, 0.5);
        assert!(!reframes_after(&mut settled(), drift, CS_CONFIRM - 1));
        assert!(reframes_after(&mut settled(), drift, CS_CONFIRM));
    }

    #[test]
    fn a_drastic_move_reframes_on_the_fast_path() {
        let breach = crop(0.5 + 0.5 * CS_BREACH_POS * 1.1, 0.5);
        assert!(!reframes_after(&mut settled(), breach, CS_FAST_CONFIRM - 1));
        assert!(reframes_after(&mut settled(), breach, CS_FAST_CONFIRM));
    }

    #[test]
    fn returning_in_time_resets_the_count() {
        let mut stage = settled();
        let drift = crop(0.5 + 0.5 * (CS_COMFORT_POS + CS_BREACH_POS) / 2.0, 0.5);
        reframes_after(&mut stage, drift, CS_CONFIRM - 1);
        stage.update_target(stage.target);
        assert!(!reframes_after(&mut stage, drift, CS_CONFIRM - 1));
    }

    #[test]
    fn every_zoom_encloses_the_subject() {
        for kind in [SubjectKind::Face, SubjectKind::Group] {
            for zoom in ["subtle", "tight", "normal"] {
                assert!(zoom_margin(zoom, kind) > 1.0, "{zoom} {kind:?}");
            }
        }
    }

    #[test]
    fn a_single_face_is_framed_on_its_eyes_and_height() {
        let face = Face {
            cx: 0.5,
            cy: 0.5,
            w: 0.3,
            h: 0.2,
            score: 0.9,
            eye_cx: 0.45,
            eye_cy: 0.4,
            eye_dist: 0.1,
        };
        let subject = subject_box(&[face], "single").unwrap();
        assert_eq!(subject.cx, 0.45);
        assert_eq!(subject.cy, 0.4 + 0.2 * EYES_ABOVE_FACE_CENTER);
        assert_eq!(subject.span, 0.2);
    }
}

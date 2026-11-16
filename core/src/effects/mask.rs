//! The shared person mask: segmented on a cadence, temporally smoothed, with
//! tracked hands painted in, then refined per frame into per-pixel weights.

use tracing::warn;

use crate::gesture::tracking::HandTracker;
use crate::inference::engine::Engine;
use crate::inference::models::Mask;

use super::resample::{LUMA_X256_MAX, luma_x256};

/// Segment every Nth frame and reuse the mask in between (docs/architecture.md#the-person-mask).
const SEG_INTERVAL: u64 = 2;
/// Temporal EMA weight of each fresh segmentation; lower is steadier but laggier.
const SEG_EMA_ALPHA: f32 = 0.5;
/// Guided-filter coefficients are computed at 1/s² of the pixels.
const GF_SUBSAMPLE: usize = 4;
/// Guided-filter regularization; lower snaps harder to luma edges and is noisier.
const GF_EPS: f32 = 1e-4;
/// Half-width of the smoothstep alpha band around 0.5; smaller is a harder edge.
const GF_EDGE_WIDTH: f32 = 0.06;
const GF_RADIUS_DIVISOR: usize = 96;

#[derive(Default)]
pub(crate) struct PersonMask {
    pub(crate) mask: Option<Mask>,
    /// Temporal EMA of the raw 256² model output, against boundary flicker.
    ema: Option<Vec<f32>>,
    /// Whether `mask` was segmented from the center-stage ROI; a mismatch
    /// forces a re-segment, so a mask is never sampled in the wrong space.
    is_roi: bool,
    /// Bumped on every fresh segmentation.
    pub(crate) generation: u64,
    /// Per-pixel weights (0–255) at canvas resolution, so the effects read one
    /// byte per pixel instead of sampling the 256² mask.
    lut: Vec<u8>,
    lut_w: usize,
    lut_h: usize,
    gf: GuidedFilter,
    /// Hands tracked in the buffer the mask is segmented from (full frame or
    /// ROI), so kept apart from the gesture pipeline's.
    hands: HandTracker,
}

impl PersonMask {
    /// Drop the mask and everything sized to the old frame.
    pub(crate) fn reset(&mut self) {
        self.mask = None;
        self.ema = None;
        self.lut.clear();
        self.lut_w = 0;
        self.lut_h = 0;
        self.gf.low_w = 0;
        self.gf.low_h = 0;
        self.hands.clear();
    }

    /// The refined per-pixel weights, if they match a `w`×`h` canvas.
    pub(crate) fn lut(&self, w: usize, h: usize) -> Option<&[u8]> {
        (self.lut_w == w && self.lut_h == h).then_some(&self.lut[..])
    }

    /// Segment on the cadence, or whenever no mask is cached for this
    /// coordinate space (full frame or ROI).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn ensure(
        &mut self,
        engine: &mut Engine,
        frame: u64,
        buf: &[u8],
        w: usize,
        h: usize,
        stride: usize,
        is_roi: bool,
    ) {
        let space_changed = self.is_roi != is_roi;
        let stale = self.mask.is_none() || space_changed || frame.is_multiple_of(SEG_INTERVAL);
        if !stale {
            return;
        }
        if space_changed {
            self.ema = None;
            self.hands.clear();
        }
        let Some(model) = engine.selfie() else {
            return;
        };
        match model.segment(buf, w, h, stride) {
            Ok(mut mask) => {
                self.paint_hands(engine, frame, buf, w, h, stride, &mut mask);
                self.smooth(&mut mask);
                self.mask = Some(mask);
                self.is_roi = is_roi;
                self.generation = self.generation.wrapping_add(1);
            }
            Err(err) => warn!(%err, "segmentation failed this frame"),
        }
    }

    /// Blend a fresh `mask` into the temporal EMA and replace it with the result.
    fn smooth(&mut self, mask: &mut Mask) {
        let a = SEG_EMA_ALPHA;
        match &mut self.ema {
            Some(e) if e.len() == mask.values.len() => {
                for (e, &n) in e.iter_mut().zip(&mask.values) {
                    *e = a * n + (1.0 - a) * *e;
                }
                mask.values.copy_from_slice(e);
            }
            _ => self.ema = Some(mask.values.clone()),
        }
    }

    /// Fill tracked hands into a fresh segmentation: the selfie model drops
    /// hands held away from the body, and the union only adds.
    #[allow(clippy::too_many_arguments)]
    fn paint_hands(
        &mut self,
        engine: &mut Engine,
        frame: u64,
        buf: &[u8],
        w: usize,
        h: usize,
        stride: usize,
        mask: &mut Mask,
    ) {
        for lm in self.hands.step(engine, frame, buf, w, h, stride) {
            lm.paint_silhouette(&mut mask.values, mask.size, w, h);
        }
    }

    /// Rasterize the mask into per-pixel weights through the guided filter,
    /// refitting it only on a fresh mask or a new size.
    pub(crate) fn refine(&mut self, img: &[u8], w: usize, h: usize) {
        let Some(mask) = self.mask.as_ref() else {
            return;
        };
        let lw = w.div_ceil(GF_SUBSAMPLE);
        let lh = h.div_ceil(GF_SUBSAMPLE);
        let gf = &mut self.gf;
        if gf.coef_gen != self.generation || gf.low_w != lw || gf.low_h != lh {
            gf.downsample(mask, img, w, h, lw, lh);
            gf.fit(lw, lh);
            gf.coef_gen = self.generation;
        }
        gf.rasterize(img, w, h, &mut self.lut);
        self.lut_w = w;
        self.lut_h = h;
    }
}

/// A fast guided filter: linear coefficients `a`, `b` fitted at
/// 1/`GF_SUBSAMPLE`² of the pixels, applied as `q = a·I + b` to the live
/// frame's full-resolution luma every frame, so the edge snaps to the person
/// and tracks motion while the coarse mask is held.
#[derive(Default)]
struct GuidedFilter {
    a_low: Vec<f32>,
    b_low: Vec<f32>,
    /// The mask and the luma at low resolution.
    p_low: Vec<f32>,
    i_low: Vec<f32>,
    boxes: BoxMean,
    low_w: usize,
    low_h: usize,
    /// `PersonMask::generation` the coefficients were fitted for.
    coef_gen: u64,
}

impl GuidedFilter {
    /// Sample the mask and box-average the luma into the `lw`×`lh` planes.
    fn downsample(&mut self, mask: &Mask, img: &[u8], w: usize, h: usize, lw: usize, lh: usize) {
        let s = GF_SUBSAMPLE;
        let n = lw * lh;
        self.p_low.resize(n, 0.0);
        self.i_low.resize(n, 0.0);
        for ly in 0..lh {
            let v = (ly as f32 + 0.5) / lh as f32;
            let (y0, y1) = (ly * s, (ly * s + s).min(h));
            for lx in 0..lw {
                let u = (lx as f32 + 0.5) / lw as f32;
                self.p_low[ly * lw + lx] = mask.sample(u, v).clamp(0.0, 1.0);
                let (x0, x1) = (lx * s, (lx * s + s).min(w));
                let mut acc = 0u32;
                for yy in y0..y1 {
                    let row = yy * w;
                    for xx in x0..x1 {
                        acc += luma_x256(&img[(row + xx) * 4..]);
                    }
                }
                let cnt = ((y1 - y0) * (x1 - x0)) as f32;
                self.i_low[ly * lw + lx] = acc as f32 / (cnt * LUMA_X256_MAX);
            }
        }
    }

    /// Fit `a`, `b` per low-res pixel from the box statistics of mask and luma.
    fn fit(&mut self, lw: usize, lh: usize) {
        let n = lw * lh;
        let r = (lw.min(lh) / GF_RADIUS_DIVISOR).max(1);
        let mut mean_i = vec![0f32; n];
        let mut mean_p = vec![0f32; n];
        let mut corr_i = vec![0f32; n];
        let mut corr_ip = vec![0f32; n];
        let mut prod = vec![0f32; n];
        self.boxes.mean(&self.i_low, &mut mean_i, lw, lh, r);
        self.boxes.mean(&self.p_low, &mut mean_p, lw, lh, r);
        for (p, &a) in prod.iter_mut().zip(&self.i_low).take(n) {
            *p = a * a;
        }
        self.boxes.mean(&prod, &mut corr_i, lw, lh, r);
        for (p, (&a, &b)) in prod
            .iter_mut()
            .zip(self.i_low.iter().zip(&self.p_low))
            .take(n)
        {
            *p = a * b;
        }
        self.boxes.mean(&prod, &mut corr_ip, lw, lh, r);

        let mut a = vec![0f32; n];
        let mut b = vec![0f32; n];
        for i in 0..n {
            let var_i = corr_i[i] - mean_i[i] * mean_i[i];
            let cov = corr_ip[i] - mean_i[i] * mean_p[i];
            a[i] = cov / (var_i + GF_EPS);
            b[i] = mean_p[i] - a[i] * mean_i[i];
        }
        self.boxes.mean(&a, &mut self.a_low, lw, lh, r);
        self.boxes.mean(&b, &mut self.b_low, lw, lh, r);
        self.low_w = lw;
        self.low_h = lh;
    }

    /// Apply the coefficients to `img`'s luma, smoothstepped into `lut`.
    fn rasterize(&self, img: &[u8], w: usize, h: usize, lut: &mut Vec<u8>) {
        let s = GF_SUBSAMPLE;
        let (lw, lh) = (self.low_w, self.low_h);
        lut.resize(w * h, 0);
        for y in 0..h {
            let fy = ((y as f32 + 0.5) / s as f32 - 0.5).clamp(0.0, lh as f32 - 1.0);
            let y0 = fy.floor() as usize;
            let y1 = (y0 + 1).min(lh - 1);
            let ty = fy - y0 as f32;
            for x in 0..w {
                let fx = ((x as f32 + 0.5) / s as f32 - 0.5).clamp(0.0, lw as f32 - 1.0);
                let x0 = fx.floor() as usize;
                let x1 = (x0 + 1).min(lw - 1);
                let tx = fx - x0 as f32;
                let a = bilerp(&self.a_low, lw, x0, x1, y0, y1, tx, ty);
                let b = bilerp(&self.b_low, lw, x0, x1, y0, y1, tx, ty);
                let lum = luma_x256(&img[(y * w + x) * 4..]) as f32 / LUMA_X256_MAX;
                let q = a * lum + b;
                let t = ((q - (0.5 - GF_EDGE_WIDTH)) / (2.0 * GF_EDGE_WIDTH)).clamp(0.0, 1.0);
                let alpha = t * t * (3.0 - 2.0 * t);
                lut[y * w + x] = (alpha * 255.0) as u8;
            }
        }
    }
}

/// Scratch for a separable box mean over an f32 plane.
#[derive(Default)]
struct BoxMean {
    /// The horizontal pass.
    tmp: Vec<f32>,
    /// Row or column prefix sums; `prefix[0]` is never written and stays 0,
    /// which the window subtraction at `lo == 0` relies on.
    prefix: Vec<f32>,
}

impl BoxMean {
    /// Mean of `src` over a `(2r+1)²` window (clipped at the edges) into
    /// `dst`, at a cost independent of `r`.
    fn mean(&mut self, src: &[f32], dst: &mut Vec<f32>, w: usize, h: usize, r: usize) {
        let (tmp, prefix) = (&mut self.tmp, &mut self.prefix);
        let n = w * h;
        dst.resize(n, 0.0);
        tmp.resize(n, 0.0);
        prefix.resize(w.max(h) + 1, 0.0);
        for y in 0..h {
            let base = y * w;
            for x in 0..w {
                prefix[x + 1] = prefix[x] + src[base + x];
            }
            for x in 0..w {
                let lo = x.saturating_sub(r);
                let hi = (x + r + 1).min(w);
                tmp[base + x] = (prefix[hi] - prefix[lo]) / (hi - lo) as f32;
            }
        }
        for x in 0..w {
            for y in 0..h {
                prefix[y + 1] = prefix[y] + tmp[y * w + x];
            }
            for y in 0..h {
                let lo = y.saturating_sub(r);
                let hi = (y + r + 1).min(h);
                dst[y * w + x] = (prefix[hi] - prefix[lo]) / (hi - lo) as f32;
            }
        }
    }
}

/// Bilinear lookup into an `lw`-stride f32 plane at the four pre-resolved
/// corner indices with fractional weights `tx`,`ty`.
#[allow(clippy::too_many_arguments)]
fn bilerp(
    buf: &[f32],
    lw: usize,
    x0: usize,
    x1: usize,
    y0: usize,
    y1: usize,
    tx: f32,
    ty: f32,
) -> f32 {
    let top = buf[y0 * lw + x0] + (buf[y0 * lw + x1] - buf[y0 * lw + x0]) * tx;
    let bot = buf[y1 * lw + x0] + (buf[y1 * lw + x1] - buf[y1 * lw + x0]) * tx;
    top + (bot - top) * ty
}

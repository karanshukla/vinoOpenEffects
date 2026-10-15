//! Temporal denoise: a motion-adaptive recursive average against the previous
//! output, run before auto light and the center-stage zoom so they amplify a
//! clean frame instead of sensor noise. See docs/architecture.md#denoise.

/// Side of the square block motion is judged over; noise averages out across it, motion doesn't.
const BLOCK: usize = 8;
/// Mean signed luma change of a block (u8 levels, ×16) below which it is still,
/// and at which it is moving and passes through unaveraged.
const STILL: i32 = 24;
const MOVING: i32 = 40;
/// Per-channel change that passes a pixel through whatever its block says, for thin edges.
const PIXEL_MOVING: i32 = 24;
/// History weight on a fully still pixel, in 1/256ths; higher is cleaner but trails more.
const MAX_HISTORY: i32 = 192;
/// Fractional bits the history keeps, so gain applied after it doesn't stretch u8 steps.
pub(crate) const FRAC_BITS: u32 = 8;

#[derive(Default)]
pub(crate) struct Denoise {
    /// Previous output per RGBA byte, in fixed point with `FRAC_BITS`.
    acc: Vec<u16>,
    /// Per-block history weight for this frame, then dilated into `weight`.
    motion: Vec<i32>,
    weight: Vec<i32>,
    /// Frame the history was last updated on; a gap means it is stale.
    last_frame: Option<u64>,
}

impl Denoise {
    pub(crate) fn reset(&mut self) {
        self.acc.clear();
        self.last_frame = None;
    }

    /// Denoise `img` in place and return the unrounded result, which
    /// [`super::auto_light`] reads so the gain rounds to u8 only once.
    pub(crate) fn apply(&mut self, frame: u64, img: &mut [u8], w: usize, h: usize) -> &[u16] {
        let fresh = self.acc.len() == img.len()
            && self.last_frame.is_some_and(|f| f.wrapping_add(1) == frame);
        self.last_frame = Some(frame);
        if !fresh {
            self.acc.clear();
            self.acc
                .extend(img.iter().map(|&p| (p as u16) << FRAC_BITS));
            return &self.acc;
        }
        let (bw, bh) = (w.div_ceil(BLOCK), h.div_ceil(BLOCK));
        self.block_weights(img, w, h, bw, bh);
        for y in 0..h {
            let row = y * w * 4..(y + 1) * w * 4;
            let block_row = &self.weight[(y / BLOCK) * bw..(y / BLOCK + 1) * bw];
            let pixels = img[row.clone()].as_chunks_mut::<4>().0.iter_mut();
            let history = self.acc[row].as_chunks_mut::<4>().0.iter_mut();
            for (x, (px, acc)) in pixels.zip(history).enumerate() {
                let moved = (0..3)
                    .any(|c| (px[c] as i32 - (acc[c] as i32 >> FRAC_BITS)).abs() >= PIXEL_MOVING);
                let k = if moved { 0 } else { block_row[x / BLOCK] };
                for c in 0..3 {
                    let cur = (px[c] as i32) << FRAC_BITS;
                    let out = cur + (((acc[c] as i32 - cur) * k) >> 8);
                    acc[c] = out as u16;
                    px[c] = ((out + (1 << (FRAC_BITS - 1))) >> FRAC_BITS).min(255) as u8;
                }
                acc[3] = (px[3] as u16) << FRAC_BITS;
            }
        }
        &self.acc
    }

    /// History weight per block from its mean signed luma change, taking the
    /// most-moving neighbour so a block next to motion doesn't smear into it.
    fn block_weights(&mut self, img: &[u8], w: usize, h: usize, bw: usize, bh: usize) {
        self.motion.clear();
        self.motion.resize(bw * bh, 0);
        let mut count = vec![0i32; bw * bh];
        for y in 0..h {
            let base = (y / BLOCK) * bw;
            let pixels = img[y * w * 4..(y + 1) * w * 4].as_chunks::<4>().0;
            let history = self.acc[y * w * 4..(y + 1) * w * 4].as_chunks::<4>().0;
            for (x, (px, acc)) in pixels.iter().zip(history).enumerate() {
                let d = |c: usize| ((px[c] as i32) << FRAC_BITS) - acc[c] as i32;
                let dy = (77 * d(0) + 150 * d(1) + 29 * d(2)) >> 8;
                self.motion[base + x / BLOCK] += dy;
                count[base + x / BLOCK] += 1;
            }
        }
        for (m, n) in self.motion.iter_mut().zip(&count) {
            *m = (*m / *n).abs() >> (FRAC_BITS - 4);
        }
        self.weight.clear();
        self.weight.resize(bw * bh, 0);
        for by in 0..bh {
            for bx in 0..bw {
                let mut worst = 0;
                for ny in by.saturating_sub(1)..(by + 2).min(bh) {
                    for nx in bx.saturating_sub(1)..(bx + 2).min(bw) {
                        worst = worst.max(self.motion[ny * bw + nx]);
                    }
                }
                self.weight[by * bw + bx] = history_weight(worst);
            }
        }
    }
}

/// History weight for a block's motion: `MAX_HISTORY` when still, falling
/// linearly to 0 at `MOVING`.
fn history_weight(motion: i32) -> i32 {
    MAX_HISTORY * (MOVING - motion).clamp(0, MOVING - STILL) / (MOVING - STILL)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noisy(base: u8, seed: u32, n: usize) -> Vec<u8> {
        let mut s = seed;
        let mut img = Vec::with_capacity(n * 4);
        for _ in 0..n {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let v = base as i32 + (s >> 24) as i32 % 13 - 6;
            img.extend_from_slice(&[v as u8, v as u8, v as u8, 255]);
        }
        img
    }

    fn spread(img: &[u8], base: u8) -> f32 {
        let n = img.len() / 4;
        let sq: i64 = img
            .chunks(4)
            .map(|p| (p[0] as i64 - base as i64).pow(2))
            .sum();
        (sq as f32 / n as f32).sqrt()
    }

    #[test]
    fn static_noise_is_reduced() {
        let mut dn = Denoise::default();
        let mut last = Vec::new();
        for f in 1..=20 {
            last = noisy(100, f, 4096);
            dn.apply(f as u64, &mut last, 64, 64);
        }
        let raw = spread(&noisy(100, 99, 4096), 100);
        let out = spread(&last, 100);
        assert!(out < raw * 0.6, "{out} vs raw {raw}");
    }

    #[test]
    fn motion_passes_through() {
        let mut dn = Denoise::default();
        let mut img = vec![50, 50, 50, 255];
        dn.apply(1, &mut img, 1, 1);
        let mut img = vec![200, 200, 200, 255];
        dn.apply(2, &mut img, 1, 1);
        assert_eq!(img, [200, 200, 200, 255]);
    }

    #[test]
    fn slow_gradient_motion_does_not_trail() {
        let (w, h) = (64, 16);
        let ramp = |shift: usize| -> Vec<u8> {
            (0..w * h)
                .flat_map(|i| {
                    let v = (((i % w) + 64 - shift) % 64 * 3 + 30) as u8;
                    [v, v, v, 255]
                })
                .collect()
        };
        let mut dn = Denoise::default();
        let mut img = Vec::new();
        for f in 0..12 {
            img = ramp(f);
            dn.apply(f as u64 + 1, &mut img, w, h);
        }
        let want = ramp(11);
        let worst = img
            .iter()
            .zip(&want)
            .skip(4 * 8)
            .map(|(a, b)| (*a as i32 - *b as i32).abs())
            .filter(|&d| d < 100)
            .max()
            .unwrap();
        assert!(worst <= 1, "trails by {worst} levels");
    }

    #[test]
    fn skipped_frame_reseeds_history() {
        let mut dn = Denoise::default();
        let mut img = vec![100, 100, 100, 255];
        dn.apply(1, &mut img, 1, 1);
        let mut img = vec![110, 110, 110, 255];
        dn.apply(5, &mut img, 1, 1);
        assert_eq!(img, [110, 110, 110, 255]);
    }

    #[test]
    fn history_keeps_fractions() {
        let mut dn = Denoise::default();
        let mut img = vec![100, 100, 100, 255];
        dn.apply(1, &mut img, 1, 1);
        let mut img = vec![101, 101, 101, 255];
        let acc = dn.apply(2, &mut img, 1, 1);
        let v = acc[0] as f32 / (1 << FRAC_BITS) as f32;
        assert!(v > 100.0 && v < 101.0, "{v}");
    }
}

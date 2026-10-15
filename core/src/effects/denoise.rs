//! Temporal denoise: a motion-adaptive recursive average against the previous
//! output, run before auto light and the center-stage zoom so they amplify a
//! clean frame instead of sensor noise. See docs/architecture.md#denoise.

/// Per-channel difference from the history at which a pixel counts as moving
/// and passes through unaveraged.
const MOTION_THRESH: i32 = 24;
/// History weight on a fully static pixel, in 1/256ths; higher is cleaner but trails more.
const MAX_HISTORY: i32 = 192;
/// Fractional bits the history keeps, so gain applied after it doesn't stretch u8 steps.
pub(crate) const FRAC_BITS: u32 = 8;

#[derive(Default)]
pub(crate) struct Denoise {
    /// Previous output per RGBA byte, in fixed point with `FRAC_BITS`.
    acc: Vec<u16>,
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
    pub(crate) fn apply(&mut self, frame: u64, img: &mut [u8]) -> &[u16] {
        let fresh = self.acc.len() == img.len()
            && self.last_frame.is_some_and(|f| f.wrapping_add(1) == frame);
        self.last_frame = Some(frame);
        if !fresh {
            self.acc.clear();
            self.acc
                .extend(img.iter().map(|&p| (p as u16) << FRAC_BITS));
            return &self.acc;
        }
        let weights = history_weights();
        for (px, acc) in img
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(self.acc.as_chunks_mut::<4>().0)
        {
            for c in 0..3 {
                let cur = (px[c] as i32) << FRAC_BITS;
                let prev = acc[c] as i32;
                let diff = (px[c] as i32 - (prev >> FRAC_BITS)).unsigned_abs() as usize;
                let k = weights[diff.min(255)];
                let out = cur + (((prev - cur) * k) >> 8);
                acc[c] = out as u16;
                px[c] = ((out + (1 << (FRAC_BITS - 1))) >> FRAC_BITS).min(255) as u8;
            }
            acc[3] = (px[3] as u16) << FRAC_BITS;
        }
        &self.acc
    }
}

/// History weight by per-channel difference: `MAX_HISTORY` when identical,
/// falling linearly to 0 at `MOTION_THRESH`.
fn history_weights() -> [i32; 256] {
    let mut lut = [0i32; 256];
    for (d, k) in lut.iter_mut().enumerate().take(MOTION_THRESH as usize) {
        *k = MAX_HISTORY * (MOTION_THRESH - d as i32) / MOTION_THRESH;
    }
    lut
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
            dn.apply(f as u64, &mut last);
        }
        let raw = spread(&noisy(100, 99, 4096), 100);
        let out = spread(&last, 100);
        assert!(out < raw * 0.6, "{out} vs raw {raw}");
    }

    #[test]
    fn motion_passes_through() {
        let mut dn = Denoise::default();
        let mut img = vec![50, 50, 50, 255];
        dn.apply(1, &mut img);
        let mut img = vec![200, 200, 200, 255];
        dn.apply(2, &mut img);
        assert_eq!(img, [200, 200, 200, 255]);
    }

    #[test]
    fn skipped_frame_reseeds_history() {
        let mut dn = Denoise::default();
        let mut img = vec![100, 100, 100, 255];
        dn.apply(1, &mut img);
        let mut img = vec![110, 110, 110, 255];
        dn.apply(5, &mut img);
        assert_eq!(img, [110, 110, 110, 255]);
    }

    #[test]
    fn history_keeps_fractions() {
        let mut dn = Denoise::default();
        let mut img = vec![100, 100, 100, 255];
        dn.apply(1, &mut img);
        let mut img = vec![101, 101, 101, 255];
        let acc = dn.apply(2, &mut img);
        let v = acc[0] as f32 / (1 << FRAC_BITS) as f32;
        assert!(v > 100.0 && v < 101.0, "{v}");
    }
}

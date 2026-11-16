/// Rec. 601 luma of the RGBA pixel at the start of `px`, scaled by 256.
pub(crate) fn luma_x256(px: &[u8]) -> u32 {
    77 * px[0] as u32 + 150 * px[1] as u32 + 29 * px[2] as u32
}

/// The largest [`luma_x256`]: 255 × 256.
pub(crate) const LUMA_X256_MAX: f32 = 65280.0;

/// Bilinearly sample the RGBA pixel at fractional `fx`,`fy` (clamped to the
/// image) into `out`.
pub(crate) fn sample_bilinear(src: &[u8], w: usize, h: usize, fx: f32, fy: f32, out: &mut [u8]) {
    let fx = fx.clamp(0.0, w as f32 - 1.0);
    let fy = fy.clamp(0.0, h as f32 - 1.0);
    let x0 = fx.floor() as usize;
    let y0 = fy.floor() as usize;
    let x1 = (x0 + 1).min(w - 1);
    let y1 = (y0 + 1).min(h - 1);
    let tx = fx - x0 as f32;
    let ty = fy - y0 as f32;
    for c in 0..4 {
        let a = src[(y0 * w + x0) * 4 + c] as f32;
        let b = src[(y0 * w + x1) * 4 + c] as f32;
        let cc = src[(y1 * w + x0) * 4 + c] as f32;
        let d = src[(y1 * w + x1) * 4 + c] as f32;
        let top = a + (b - a) * tx;
        let bot = cc + (d - cc) * tx;
        out[c] = (top + (bot - top) * ty).round() as u8;
    }
}

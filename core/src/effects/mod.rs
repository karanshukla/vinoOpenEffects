//! Every ML-driven effect, run over tight RGBA frames in system memory:
//!
//! 1. **Studio Light** — a `videobalance`-style brightness/contrast lift
//!    blended by the person mask, so only the subject is lit (the background
//!    keeps its original exposure).
//! 2. **Portrait blur** / **Background replace** — `out = fg·mask + bg·(1-mask)`
//!    where `bg` is a user image/color, or — for blur — a box blur of the
//!    frame *weighted by the background mask* so the subject's own pixels are
//!    excluded from it (otherwise the person's colors bleed outward under the
//!    blur and leave a halo ring around the composited subject).
//! 3. **Center Stage** — a YuNet face box drives an EMA-smoothed, deadzoned
//!    crop+zoom that keeps the face centered without distorting the aspect ratio.
//!
//! Studio Light, Portrait blur and Background replace all need the selfie
//! segmentation mask; it is segmented **once per frame** and shared by all
//! three, so enabling several together costs no extra ONNX work.
//!
//! Two paths, chosen per frame:
//!
//! - **Center stage on**: framing is computed on the full frame, then the crop
//!   ROI is extracted and the masked effects run on *just that ROI* (the only
//!   pixels the zoom keeps) before it's upscaled to the output. Compositing the
//!   discarded border would be wasted work, so center stage is the source of
//!   truth for the mask canvas.
//! - **Center stage off**: the masked effects run over the full frame.
//!
//! Energy: selfie segmentation (the per-frame ONNX cost) runs on a cadence
//! (`SEG_INTERVAL`) with the mask reused in between, and YuNet detection on a
//! coarser one (`FACE_INTERVAL`) — the reframe deadzone absorbs the staleness.
//!
//! Before any of them, a temporal denoise (see [`denoise`]) cleans the frame
//! whenever an effect would amplify its noise.
//!
//! 4. **Reactions** — reverse-rain emoji bursts (see [`reactions`]), composited
//!    last over the final canvas. Triggered manually or, when enabled,
//!    automatically by the hand-gesture pipeline.

mod auto_light;
mod blur;
mod center_stage;
mod composite;
mod denoise;
mod mask;
pub mod reactions;
mod resample;
pub mod settings;
mod studio_light;

#[cfg(test)]
mod golden_tests;

pub use settings::Settings;

use crate::gesture::detector::GestureDetector;
use crate::inference::engine::Engine;
use auto_light::AutoLightState;
use blur::BlurScratch;
use center_stage::{CenterStage, extract_roi, resample_crop, upscale_roi};
use composite::Background;
use denoise::Denoise;
use mask::PersonMask;
use reactions::Reactions;
use studio_light::{StudioPivot, apply_studio};

/// Per-stream effect state: settings, the model engine, and everything
/// carried from one frame to the next.
#[derive(Default)]
pub struct Effects {
    pub settings: Settings,
    pub engine: Engine,
    frame: u64,
    center_stage: CenterStage,
    mask: PersonMask,
    blur: BlurScratch,
    bg: Background,
    studio_pivot: StudioPivot,
    auto_light: AutoLightState,
    denoise: Denoise,
    gestures: GestureDetector,
    /// Reverse-rain emoji particle system (manual + gesture triggers).
    reactions: Reactions,
}

impl Effects {
    /// Whether frames need processing at all: an effect is on, or a manual
    /// burst is still in flight.
    pub fn busy(&self) -> bool {
        self.settings.active() || self.reactions.active()
    }

    pub fn set_bg_path(&mut self, path: String) {
        self.settings.bg_path = path;
        self.bg.clear();
    }

    /// Fire a burst of reaction `id` now.
    pub fn trigger_reaction(&mut self, id: &str) {
        self.reactions.trigger(id, self.frame);
    }

    /// A resolution change invalidates the smoothed crop, cached mask, and
    /// pre-scaled background, and normalized hand crops don't survive it.
    pub fn reset_for_new_size(&mut self) {
        self.center_stage.reset();
        self.mask.reset();
        self.bg.clear();
        self.auto_light.gain = None;
        self.denoise.reset();
        self.gestures.reset();
    }

    /// Run every enabled effect over one tight `w`×`h` RGBA frame, in place.
    pub fn process(&mut self, img: &mut [u8], w: usize, h: usize) {
        self.frame = self.frame.wrapping_add(1);

        // Gestures must see the original frame, before auto light and the center-stage crop rewrite
        // it in place.
        if self.settings.reactions_enabled
            && let Some(g) = self.gestures.step(&mut self.engine, self.frame, img, w, h)
        {
            self.reactions.trigger(g.reaction_id(), self.frame);
        }

        let precise = self
            .settings
            .needs_denoise()
            .then(|| self.denoise.apply(self.frame, img, w, h));
        if self.settings.auto_light_enabled {
            let strength = self.settings.auto_light_strength;
            self.auto_light
                .apply(&mut self.engine, self.frame, strength, img, w, h, precise);
        }

        if self.settings.center_stage_active() {
            self.center_stage(img, w, h);
        } else if self.settings.needs_mask() {
            self.masked_effects(img, w, h, false);
        }

        if self.settings.reactions_enabled || self.reactions.active() {
            self.reactions.step_and_render(img, w, h, self.frame);
        }
    }

    /// Center stage: decide framing on the full frame, then crop to the ROI and
    /// run blur / bg-replace on just that ROI before upscaling it to the output.
    fn center_stage(&mut self, img: &mut [u8], w: usize, h: usize) {
        let crop = self
            .center_stage
            .track(&mut self.engine, &self.settings, self.frame, img, w, h);

        if !self.settings.needs_mask() {
            resample_crop(img, w, h, crop);
            return;
        }

        let Some(mut roi) = extract_roi(img, w, h, crop) else {
            self.masked_effects(img, w, h, false);
            return;
        };
        self.masked_effects(&mut roi.buf, roi.w, roi.h, true);
        upscale_roi(img, w, h, &roi);
    }

    /// Run every mask-gated effect over a tight RGBA buffer: segment **once**,
    /// then (in order) studio-light the person, and blur / bg-replace the
    /// background. `is_roi` selects the mask coordinate space (full vs crop ROI).
    fn masked_effects(&mut self, img: &mut [u8], w: usize, h: usize, is_roi: bool) {
        self.mask
            .ensure(&mut self.engine, self.frame, img, w, h, w * 4, is_roi);
        self.mask.refine(img, w, h);
        let Some(lut) = self.mask.lut(w, h) else {
            return;
        };
        let s = &self.settings;

        if s.studio_enabled {
            let pivot = self.studio_pivot.update(img, lut);
            apply_studio(
                img,
                lut,
                pivot,
                s.studio_brightness,
                s.studio_contrast,
                s.studio_bg_brightness,
            );
        }
        if !s.needs_composite() {
            return;
        }
        if s.bg_enabled {
            self.bg.composite(img, lut, &s.bg_path, w, h);
        } else {
            self.blur.blur_background(img, lut, w, h, s.blur_strength);
        }
    }
}

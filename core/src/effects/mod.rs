//! Every ML-driven effect, run over tight RGBA frames in system memory, in
//! order: temporal denoise (when an effect would amplify noise), auto light,
//! Center Stage, the masked effects (Studio Light, then portrait blur or
//! background replace), and reactions on top. The person mask is segmented
//! once per frame on a cadence and shared by every masked effect. With
//! Center Stage on, the masked effects run on just the crop it keeps.
//! docs/architecture.md has the reasoning.

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
    reactions: Reactions,
    /// An all-background mask, for hiding the frame until a real one exists.
    hidden: Vec<u8>,
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

    /// Frames stopped reaching [`Self::process`] (the filter went passthrough),
    /// so the next one isn't continuous with the denoise history.
    pub fn pause(&mut self) {
        self.denoise.reset();
    }

    /// Run every enabled effect over one tight `w`×`h` RGBA frame, in place.
    pub fn process(&mut self, img: &mut [u8], w: usize, h: usize) {
        self.frame = self.frame.wrapping_add(1);

        // Before auto light and the crop rewrite the frame in place.
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

    /// Frame on the full image, then run the masked effects on just the crop.
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

    /// Segment once, then studio-light the person and blur or replace the
    /// background. `is_roi`: `img` is the center-stage crop, not the frame.
    fn masked_effects(&mut self, img: &mut [u8], w: usize, h: usize, is_roi: bool) {
        self.mask
            .ensure(&mut self.engine, self.frame, img, w, h, w * 4, is_roi);
        self.mask.refine(img, w, h);
        let Some(lut) = self.mask.lut(w, h) else {
            // No mask yet: hide the frame unless the model failed for good
            // (pinned by `golden_tests::blur_hides_the_frame_until_the_mask_exists`,
            // `golden_tests::a_failed_model_still_turns_blur_off`).
            if self.settings.needs_composite() && !self.engine.selfie_failed() {
                self.hidden.clear();
                self.hidden.resize(w * h, 0);
                composite(
                    &mut self.bg,
                    &mut self.blur,
                    &self.settings,
                    img,
                    &self.hidden,
                    w,
                    h,
                );
            }
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
        if s.needs_composite() {
            composite(&mut self.bg, &mut self.blur, s, img, lut, w, h);
        }
    }
}

/// Background replace if it's on, else portrait blur, over `lut`'s background.
fn composite(
    bg: &mut Background,
    blur: &mut BlurScratch,
    s: &Settings,
    img: &mut [u8],
    lut: &[u8],
    w: usize,
    h: usize,
) {
    if s.bg_enabled {
        bg.composite(img, lut, &s.bg_path, w, h);
    } else {
        blur.blur_background(img, lut, w, h, s.blur_strength);
    }
}

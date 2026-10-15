//! The set of lazily-loaded model sessions the effects run on. A provider swap
//! builds a whole new [`Engine`] off the frame path and replaces the old one.

use std::time::{Duration, Instant};

use tracing::warn;

use super::hand::{HandLandmarker, PalmDetector};
use super::models::{AutoLight, SelfieSeg, YuNet};

/// A lazily-loaded model, with a one-shot failure memo so a missing model
/// is logged once rather than retried every frame, and a last-use stamp so
/// an idle model can be dropped and reloaded on demand.
struct Slot<T> {
    model: Option<T>,
    active_ep: Option<&'static str>,
    failed: bool,
    /// Queued for, or being loaded by, a background loader.
    loading: bool,
    used: Instant,
}

impl<T> Default for Slot<T> {
    fn default() -> Self {
        Self {
            model: None,
            active_ep: None,
            failed: false,
            loading: false,
            used: Instant::now(),
        }
    }
}

/// What a model accessor did with an empty slot.
#[derive(PartialEq)]
enum Touch {
    Nothing,
    Loaded,
    Queued,
}

impl<T> Slot<T> {
    /// Load an empty slot now, or in background mode queue it and return.
    fn touch(
        &mut self,
        background: bool,
        provider: &str,
        load: impl FnOnce(&str) -> anyhow::Result<(T, &'static str, Option<String>)>,
        unavailable: &str,
        fallback_out: &mut Option<String>,
    ) -> Touch {
        if self.model.is_some() || self.failed || self.loading {
            return Touch::Nothing;
        }
        if background {
            self.loading = true;
            return Touch::Queued;
        }
        self.ensure_loaded(provider, load, unavailable, fallback_out);
        Touch::Loaded
    }

    /// Take a background load's result into a slot still waiting for it.
    fn adopt(&mut self, from: Slot<T>) -> bool {
        if !self.loading {
            return false;
        }
        self.model = from.model;
        self.active_ep = from.active_ep;
        self.failed = from.failed;
        self.loading = false;
        self.used = Instant::now();
        true
    }

    fn ensure_loaded(
        &mut self,
        provider: &str,
        load: impl FnOnce(&str) -> anyhow::Result<(T, &'static str, Option<String>)>,
        unavailable: &str,
        fallback_out: &mut Option<String>,
    ) {
        if self.model.is_none() && !self.failed {
            match load(provider) {
                Ok((m, ep, fallback)) => {
                    self.model = Some(m);
                    self.active_ep = Some(ep);
                    if let Some(fb) = fallback {
                        *fallback_out = Some(fb);
                    }
                }
                Err(err) => {
                    warn!(%err, "{unavailable}");
                    self.failed = true;
                }
            }
        }
    }

    fn as_mut(&mut self) -> Option<&mut T> {
        self.used = Instant::now();
        self.model.as_mut()
    }

    #[cfg(test)]
    fn get_simple(
        &mut self,
        load: impl FnOnce() -> anyhow::Result<T>,
        unavailable: &str,
    ) -> Option<&mut T> {
        let mut fb = None;
        self.ensure_loaded(
            "cpu",
            |_| load().map(|m| (m, "cpu", None)),
            unavailable,
            &mut fb,
        );
        self.as_mut()
    }

    fn is_loaded(&self) -> bool {
        self.model.is_some()
    }

    /// A slot holding a model already loaded for a new provider, or an empty
    /// one that loads lazily on first use.
    fn preloaded(loaded: Option<(T, &'static str)>) -> Self {
        let mut slot = Self::default();
        if let Some((model, ep)) = loaded {
            slot.model = Some(model);
            slot.active_ep = Some(ep);
        }
        slot
    }

    fn unload_idle(&mut self, max_idle: Duration) -> bool {
        if self.model.is_some() && self.used.elapsed() >= max_idle {
            self.model = None;
            self.active_ep = None;
            return true;
        }
        false
    }
}

/// A set of models: which an [`Engine`] has loaded, so a replacement loads the
/// same set, or which a background load or warm-up should compile.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LoadedModels {
    pub selfie: bool,
    pub yunet: bool,
    pub palm: bool,
    pub hand: bool,
    pub auto_light: bool,
}

impl LoadedModels {
    pub fn any(&self) -> bool {
        self.selfie || self.yunet || self.palm || self.hand || self.auto_light
    }
}

/// Models the frame path asked for in background mode, to load off its thread.
#[derive(Debug)]
pub struct LoadRequest {
    pub provider: String,
    pub generation: u64,
    pub models: LoadedModels,
}

impl LoadRequest {
    /// Compile the requested models. Slow; run it on its own thread, then hand
    /// the result to [`Engine::adopt`].
    pub fn run(self) -> Loaded {
        let mut engine = Engine::with_provider(&self.provider);
        let m = self.models;
        if m.selfie {
            let _ = engine.selfie();
        }
        if m.yunet {
            let _ = engine.yunet();
        }
        if m.palm {
            let _ = engine.palm();
        }
        if m.hand {
            let _ = engine.hand();
        }
        if m.auto_light {
            let _ = engine.auto_light();
        }
        Loaded {
            generation: self.generation,
            engine,
        }
    }
}

/// A background load's result, for the engine generation that asked for it.
pub struct Loaded {
    generation: u64,
    engine: Engine,
}

/// Compile `models` on `provider` and drop them, so OpenVINO's compile cache
/// and the failure record are ready before the first frame needs them.
pub fn warm(provider: &str, models: LoadedModels) -> (&'static str, Option<String>) {
    let loaded = LoadRequest {
        provider: provider.to_owned(),
        generation: 0,
        models,
    }
    .run();
    (loaded.engine.active, loaded.engine.detail)
}

pub struct Engine {
    pub generation: u64,
    pub provider: String,
    pub active: &'static str,
    pub detail: Option<String>,
    /// Queue model loads for [`take_request`](Self::take_request) instead of
    /// compiling on the caller's (frame) thread.
    pub background: bool,
    queued: LoadedModels,
    selfie: Slot<SelfieSeg>,
    yunet: Slot<YuNet>,
    palm: Slot<PalmDetector>,
    hand: Slot<HandLandmarker>,
    auto_light: Slot<AutoLight>,
}

impl Default for Engine {
    fn default() -> Self {
        Self::with_provider(&super::requested_ep())
    }
}

impl Engine {
    fn with_provider(provider: &str) -> Self {
        let mut eng = Self {
            generation: 0,
            provider: provider.to_owned(),
            active: "cpu",
            detail: None,
            background: false,
            queued: LoadedModels::default(),
            selfie: Slot::default(),
            yunet: Slot::default(),
            palm: Slot::default(),
            hand: Slot::default(),
            auto_light: Slot::default(),
        };
        eng.update_active_status();
        eng
    }

    /// An NPU exists but these models aren't (all) on it.
    pub fn degraded(&self) -> bool {
        super::degraded(&self.provider, self.active)
    }

    /// The models queued since the last call, to load with [`LoadRequest::run`].
    pub fn take_request(&mut self) -> Option<LoadRequest> {
        let models = std::mem::take(&mut self.queued);
        models.any().then(|| LoadRequest {
            provider: self.provider.clone(),
            generation: self.generation,
            models,
        })
    }

    /// Install a background load's models; false if a provider swap made it stale.
    pub fn adopt(&mut self, loaded: Loaded) -> bool {
        if loaded.generation != self.generation {
            return false;
        }
        let from = loaded.engine;
        let mut any = self.selfie.adopt(from.selfie);
        any |= self.yunet.adopt(from.yunet);
        any |= self.palm.adopt(from.palm);
        any |= self.hand.adopt(from.hand);
        any |= self.auto_light.adopt(from.auto_light);
        if any {
            if from.detail.is_some() {
                self.detail = from.detail;
            }
            self.update_active_status();
        }
        any
    }

    /// Load `wanted` on `provider` into a fresh engine. Slow (each model
    /// compiles for the provider), so callers run it without holding the
    /// frame lock and then [`replace`](Self::replace) the live engine.
    pub fn load(provider: &str, wanted: LoadedModels) -> anyhow::Result<Self> {
        // Reset first: a stale accelerator-failed memo from the old provider would pin `auto` to
        // CPU.
        super::reset_accel_failed();
        let mut detail = None;
        let selfie = reload(
            wanted.selfie,
            SelfieSeg::load_with_provider,
            provider,
            &mut detail,
        )?;
        let yunet = reload(
            wanted.yunet,
            YuNet::load_with_provider,
            provider,
            &mut detail,
        )?;
        let palm = reload(
            wanted.palm,
            PalmDetector::load_with_provider,
            provider,
            &mut detail,
        )?;
        let hand = reload(
            wanted.hand,
            HandLandmarker::load_with_provider,
            provider,
            &mut detail,
        )?;
        let auto_light = reload(
            wanted.auto_light,
            AutoLight::load_with_provider,
            provider,
            &mut detail,
        )
        .unwrap_or_default();

        let mut eng = Self {
            generation: 0,
            provider: provider.to_string(),
            active: "cpu",
            detail,
            background: false,
            queued: LoadedModels::default(),
            selfie: Slot::preloaded(selfie),
            yunet: Slot::preloaded(yunet),
            palm: Slot::preloaded(palm),
            hand: Slot::preloaded(hand),
            auto_light: Slot::preloaded(auto_light),
        };
        eng.update_active_status();
        Ok(eng)
    }

    pub fn loaded(&self) -> LoadedModels {
        LoadedModels {
            selfie: self.selfie.is_loaded(),
            yunet: self.yunet.is_loaded(),
            palm: self.palm.is_loaded(),
            hand: self.hand.is_loaded(),
            auto_light: self.auto_light.is_loaded(),
        }
    }

    /// Swap in `next` as the next generation. A model the old engine loaded
    /// after `next` was built is dropped with it and reloads lazily.
    pub fn replace(&mut self, next: Engine) {
        let generation = self.generation + 1;
        let background = self.background;
        *self = next;
        self.generation = generation;
        self.background = background;
        super::reset_accel_failed();
    }

    pub fn update_active_status(&mut self) {
        let eps: Vec<&'static str> = [
            self.selfie.active_ep,
            self.yunet.active_ep,
            self.palm.active_ep,
            self.hand.active_ep,
            self.auto_light.active_ep,
        ]
        .into_iter()
        .flatten()
        .collect();
        self.active = match eps.split_first() {
            None => {
                let (active, detail) = super::predicted_active(&self.provider);
                self.detail = detail;
                active
            }
            Some((first, rest)) if rest.iter().all(|ep| ep == first) => first,
            Some(_) => "mixed",
        };
    }

    pub fn selfie(&mut self) -> Option<&mut SelfieSeg> {
        let mut fb = None;
        match self.selfie.touch(
            self.background,
            &self.provider,
            SelfieSeg::load_with_provider,
            "selfie segmentation unavailable; portrait blur / bg replace disabled",
            &mut fb,
        ) {
            Touch::Nothing => {}
            Touch::Queued => self.queued.selfie = true,
            Touch::Loaded => {
                if fb.is_some() {
                    self.detail = fb;
                }
                self.update_active_status();
            }
        }
        self.selfie.as_mut()
    }

    pub fn yunet(&mut self) -> Option<&mut YuNet> {
        let mut fb = None;
        match self.yunet.touch(
            self.background,
            &self.provider,
            YuNet::load_with_provider,
            "yunet unavailable; center stage cannot track a subject",
            &mut fb,
        ) {
            Touch::Nothing => {}
            Touch::Queued => self.queued.yunet = true,
            Touch::Loaded => {
                if fb.is_some() {
                    self.detail = fb;
                }
                self.update_active_status();
            }
        }
        self.yunet.as_mut()
    }

    pub fn palm(&mut self) -> Option<&mut PalmDetector> {
        let mut fb = None;
        match self.palm.touch(
            self.background,
            &self.provider,
            PalmDetector::load_with_provider,
            "palm detection unavailable; gesture reactions disabled",
            &mut fb,
        ) {
            Touch::Nothing => {}
            Touch::Queued => self.queued.palm = true,
            Touch::Loaded => {
                if fb.is_some() {
                    self.detail = fb;
                }
                self.update_active_status();
            }
        }
        self.palm.as_mut()
    }

    pub fn hand(&mut self) -> Option<&mut HandLandmarker> {
        let mut fb = None;
        match self.hand.touch(
            self.background,
            &self.provider,
            HandLandmarker::load_with_provider,
            "hand landmark model unavailable; gesture reactions disabled",
            &mut fb,
        ) {
            Touch::Nothing => {}
            Touch::Queued => self.queued.hand = true,
            Touch::Loaded => {
                if fb.is_some() {
                    self.detail = fb;
                }
                self.update_active_status();
            }
        }
        self.hand.as_mut()
    }

    pub fn auto_light(&mut self) -> Option<&mut AutoLight> {
        let mut fb = None;
        match self.auto_light.touch(
            self.background,
            &self.provider,
            AutoLight::load_with_provider,
            "auto light unavailable (needs an NPU and the iat_lowlight model)",
            &mut fb,
        ) {
            Touch::Nothing => {}
            Touch::Queued => self.queued.auto_light = true,
            Touch::Loaded => {
                if fb.is_some() {
                    self.detail = fb;
                }
                self.update_active_status();
            }
        }
        self.auto_light.as_mut()
    }

    /// Drop every model unused for `max_idle`; true if any was freed.
    pub fn unload_idle(&mut self, max_idle: Duration) -> bool {
        let selfie = self.selfie.unload_idle(max_idle);
        let yunet = self.yunet.unload_idle(max_idle);
        let palm = self.palm.unload_idle(max_idle);
        let hand = self.hand.unload_idle(max_idle);
        let auto = self.auto_light.unload_idle(max_idle);
        let freed = selfie || yunet || palm || hand || auto;
        if freed {
            self.update_active_status();
        }
        freed
    }

    /// An engine whose every model has already failed, so nothing loads.
    #[cfg(test)]
    pub(crate) fn offline() -> Self {
        let mut eng = Self::default();
        eng.selfie.failed = true;
        eng.yunet.failed = true;
        eng.palm.failed = true;
        eng.hand.failed = true;
        eng.auto_light.failed = true;
        eng
    }
}

/// Load a model on `provider` if `wanted`, recording any CPU fallback reason.
fn reload<T>(
    wanted: bool,
    load: impl FnOnce(&str) -> anyhow::Result<(T, &'static str, Option<String>)>,
    provider: &str,
    detail: &mut Option<String>,
) -> anyhow::Result<Option<(T, &'static str)>> {
    if !wanted {
        return Ok(None);
    }
    let (model, ep, fallback) = load(provider)?;
    if fallback.is_some() {
        *detail = fallback;
    }
    Ok(Some((model, ep)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_slot_unloads_then_reloads_on_demand() {
        let mut slot: Slot<u32> = Slot::default();
        assert_eq!(slot.get_simple(|| Ok(7), "x"), Some(&mut 7));
        assert!(!slot.unload_idle(Duration::from_secs(60)));
        assert!(slot.unload_idle(Duration::ZERO));
        assert!(!slot.unload_idle(Duration::ZERO));
        assert_eq!(slot.get_simple(|| Ok(8), "x"), Some(&mut 8));
    }

    #[test]
    fn failed_slot_is_not_retried() {
        let mut slot: Slot<u32> = Slot::default();
        assert!(slot.get_simple(|| anyhow::bail!("missing"), "x").is_none());
        assert!(slot.get_simple(|| Ok(1), "x").is_none());
    }

    #[test]
    fn background_mode_queues_loads_once_and_adopts_them() {
        let mut eng = Engine::with_provider("cpu");
        eng.background = true;
        assert!(eng.selfie().is_none());
        assert!(eng.palm().is_none());
        assert!(eng.selfie().is_none());
        let req = eng.take_request().expect("queued");
        assert_eq!(
            req.models,
            LoadedModels {
                selfie: true,
                palm: true,
                ..Default::default()
            }
        );
        assert!(eng.take_request().is_none());
        assert!(eng.selfie().is_none());
        assert!(
            eng.take_request().is_none(),
            "a loading slot is not queued again"
        );

        assert!(eng.adopt(req.run()));
        assert!(!eng.selfie.loading && !eng.palm.loading);
        assert!(eng.selfie.is_loaded() || eng.selfie.failed);
        let _ = eng.selfie();
        assert!(eng.take_request().is_none());
    }

    #[test]
    fn stale_background_load_is_dropped() {
        let mut eng = Engine::with_provider("cpu");
        eng.background = true;
        let _ = eng.yunet();
        let req = eng.take_request().unwrap();
        eng.replace(Engine::with_provider("cpu"));
        assert!(eng.background, "a swap keeps background mode");
        assert!(!eng.adopt(req.run()));
        assert!(!eng.yunet.is_loaded());
    }

    #[test]
    fn replace_bumps_generation_and_clears_failures() {
        let mut eng = Engine::offline();
        let fresh = Engine::load("cpu", eng.loaded()).expect("load on cpu");
        eng.replace(fresh);
        assert_eq!(eng.generation, 1);
        assert!(!eng.selfie.failed && !eng.auto_light.failed);
        assert_eq!((eng.provider.as_str(), eng.active), ("cpu", "cpu"));
        eng.replace(Engine::load("cpu", eng.loaded()).unwrap());
        assert_eq!(eng.generation, 2);
    }
}

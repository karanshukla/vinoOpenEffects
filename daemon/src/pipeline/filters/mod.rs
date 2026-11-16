pub mod oe_effects;

pub use oe_effects::OeEffects;

use gst::glib;
use gst::prelude::*;
use gstreamer as gst;

/// Register the `oe_effects` element by name; a no-op if already registered.
pub fn register() -> Result<(), glib::BoolError> {
    if gst::ElementFactory::find("oe_effects").is_none() {
        gst::Element::register(
            None,
            "oe_effects",
            gst::Rank::NONE,
            OeEffects::static_type(),
        )?;
    }
    Ok(())
}

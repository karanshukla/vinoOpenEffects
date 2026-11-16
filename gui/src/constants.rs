//! Static lookup tables shared across the effect cards: app id, toggle-group
//! option lists and built-in presets.

pub const APP_ID: &str = "io.github.karanshukla.vinoopeneffects";

/// The virtual camera's PipeWire node name, which the preview targets. Must
/// match the daemon's `PIPEWIRE_NODE_NAME`.
pub const VCAM_NODE_NAME: &str = "vinoopeneffects";

/// Center Stage framing levels (stored value, display label). "Off" is the
/// card's switch, so it is not a framing choice here.
pub const FRAMING_LEVELS: &[(&str, &str)] = &[
    ("subtle", "Subtle"),
    ("normal", "Normal"),
    ("tight", "Tight"),
];

/// Center Stage tracking mode (stored value, display label).
pub const FRAMING_MODES: &[(&str, &str)] = &[("single", "One Face"), ("group", "Whole Group")];

/// Background card modes, mapped onto the `portrait_blur` / `bg_replace`
/// enable flags. The card's switch is "off".
pub const BG_BLUR: &str = "blur";
pub const BG_REPLACE: &str = "replace";
pub const BG_MODES: &[(&str, &str)] = &[(BG_BLUR, "Blur"), (BG_REPLACE, "Replace")];

/// Blur strength buckets: toggle name (the stored `portrait_blur.strength`
/// value as a string) and display label.
pub const BLUR_LEVELS: &[(&str, &str)] = &[("33", "Low"), ("66", "Medium"), ("100", "High")];

/// Built-in solid-color backgrounds (label, `#RRGGBB`).
pub const BG_PRESETS: &[(&str, &str)] = &[
    ("Charcoal", "#1e1e2e"),
    ("Slate", "#2e3440"),
    ("Deep Blue", "#1b3a5b"),
    ("Forest", "#1f3d2b"),
    ("Plum", "#3b2e4a"),
    ("Warm Gray", "#3a3a3a"),
];

/// A built-in image background. `bg_replace.background` stores a path, so
/// `pages::background::ensure_preset_images` writes it to the data dir first.
pub struct BuiltinImage {
    pub label: &'static str,
    /// File name in the data dir.
    pub file: &'static str,
    pub bytes: &'static [u8],
}

pub const BG_IMAGE_PRESETS: &[BuiltinImage] = &[
    BuiltinImage {
        label: "Modern Living Room",
        file: "modern-living-room.jpg",
        bytes: include_bytes!("../data/backgrounds/modern-living-room.jpg"),
    },
    BuiltinImage {
        label: "Serene Nature",
        file: "serene-nature.jpg",
        bytes: include_bytes!("../data/backgrounds/serene-nature.jpg"),
    },
    BuiltinImage {
        label: "Sunset Room",
        file: "sunset-room.jpg",
        bytes: include_bytes!("../data/backgrounds/sunset-room.jpg"),
    },
    BuiltinImage {
        label: "Wall of Books",
        file: "wall-of-books.jpg",
        bytes: include_bytes!("../data/backgrounds/wall-of-books.jpg"),
    },
];

/// An emoji button on the Reactions card, which fires a burst whatever the
/// gesture switch says.
pub struct ReactionButton {
    /// A `shared::dbus::REACTION_IDS` id.
    pub id: &'static str,
    pub glyph: &'static str,
    pub tooltip: &'static str,
}

/// In `REACTION_IDS` order (pinned by `tests::one_reaction_button_per_id_in_order`).
pub const REACTION_BUTTONS: &[ReactionButton] = &[
    ReactionButton {
        id: "heart",
        glyph: "\u{1F496}",
        tooltip: "Heart",
    },
    ReactionButton {
        id: "thumbs_up",
        glyph: "\u{1F44D}",
        tooltip: "Thumbs Up",
    },
    ReactionButton {
        id: "thumbs_down",
        glyph: "\u{1F44E}",
        tooltip: "Thumbs Down",
    },
    ReactionButton {
        id: "tada",
        glyph: "\u{1F389}",
        tooltip: "Celebrate",
    },
    ReactionButton {
        id: "clap",
        glyph: "\u{1F44F}",
        tooltip: "Applause",
    },
    ReactionButton {
        id: "wave",
        glyph: "\u{1F44B}",
        tooltip: "Wave",
    },
    ReactionButton {
        id: "joy",
        glyph: "\u{1F602}",
        tooltip: "Laughing",
    },
    ReactionButton {
        id: "open_mouth",
        glyph: "\u{1F62E}",
        tooltip: "Wow",
    },
    ReactionButton {
        id: "cry",
        glyph: "\u{1F622}",
        tooltip: "Crying",
    },
    ReactionButton {
        id: "thinking",
        glyph: "\u{1F914}",
        tooltip: "Thinking",
    },
];

/// A model listed in Preferences.
pub struct BundledModel {
    pub id: &'static str,
    pub name: &'static str,
    /// Pango markup.
    pub purpose: &'static str,
}

pub const BUNDLED_MODELS: &[BundledModel] = &[
    BundledModel {
        id: "selfie_segmentation",
        name: "MediaPipe Selfie Segmentation",
        purpose: "Portrait blur &amp; background replace",
    },
    BundledModel {
        id: "yunet",
        name: "YuNet",
        purpose: "Face detection for Center Stage",
    },
    BundledModel {
        id: "palm_detection",
        name: "MediaPipe Palm Detection",
        purpose: "Hand detection for gesture reactions",
    },
    BundledModel {
        id: "hand_landmark",
        name: "MediaPipe Hand Landmark",
        purpose: "Hand keypoints for gesture reactions",
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_reaction_button_per_id_in_order() {
        let ids: Vec<&str> = REACTION_BUTTONS.iter().map(|b| b.id).collect();
        assert_eq!(ids, shared::dbus::REACTION_IDS);
    }
}

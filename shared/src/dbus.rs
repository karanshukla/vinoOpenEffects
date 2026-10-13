use std::collections::HashMap;

use zvariant::{OwnedValue, Str};

pub const SERVICE_NAME: &str = "io.github.karanshukla.vinoopeneffects.Daemon";
pub const OBJECT_PATH: &str = "/io/github/karanshukla/vinoopeneffects/Daemon";

pub const DAEMON_INTERFACE: &str = "io.github.karanshukla.vinoopeneffects.Daemon1";
pub const EFFECTS_INTERFACE: &str = "io.github.karanshukla.vinoopeneffects.Effects1";
pub const DEVICES_INTERFACE: &str = "io.github.karanshukla.vinoopeneffects.Devices1";

pub const EFFECT_IDS: [&str; 5] = [
    "center_stage",
    "portrait_blur",
    "bg_replace",
    "studio_light",
    "reactions",
];

/// Canonical reaction emoji ids. The gesture-mapped ones (heart, thumbs_up,
/// thumbs_down, wave) are recognized by the hand-gesture pipeline; the rest are
/// shown only in the manual picker. Order is parallel to the GUI buttons and the
/// embedded emoji PNGs.
pub const REACTION_IDS: [&str; 10] = [
    "heart",
    "thumbs_up",
    "thumbs_down",
    "tada",
    "clap",
    "wave",
    "joy",
    "open_mouth",
    "cry",
    "thinking",
];

pub type VariantMap = HashMap<String, OwnedValue>;

pub fn str_value(value: impl AsRef<str>) -> OwnedValue {
    OwnedValue::from(Str::from(value.as_ref().to_owned()))
}

pub fn str_array_value<S: AsRef<str>>(values: &[S]) -> OwnedValue {
    let strings: Vec<String> = values.iter().map(|s| s.as_ref().to_owned()).collect();
    OwnedValue::try_from(zvariant::Value::from(strings)).expect("convert str array to OwnedValue")
}

pub fn bool_value(value: bool) -> OwnedValue {
    OwnedValue::from(value)
}

pub fn u32_value(value: u32) -> OwnedValue {
    OwnedValue::from(value)
}

pub fn i32_value(value: i32) -> OwnedValue {
    OwnedValue::from(value)
}

pub fn value_as_bool(value: &OwnedValue) -> Option<bool> {
    bool::try_from(value).ok()
}

pub fn value_as_u32(value: &OwnedValue) -> Option<u32> {
    u32::try_from(value).ok().or_else(|| {
        i32::try_from(value)
            .ok()
            .and_then(|v| u32::try_from(v).ok())
    })
}

pub fn value_as_i32(value: &OwnedValue) -> Option<i32> {
    i32::try_from(value).ok().or_else(|| {
        u32::try_from(value)
            .ok()
            .and_then(|v| i32::try_from(v).ok())
    })
}

pub fn value_as_string(value: &OwnedValue) -> Option<String> {
    value
        .try_clone()
        .ok()
        .and_then(|value| String::try_from(value).ok())
}

pub fn value_as_str_array(value: &OwnedValue) -> Option<Vec<String>> {
    value
        .try_clone()
        .ok()
        .and_then(|value| Vec::<String>::try_from(value).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalar_helpers_round_trip() {
        assert_eq!(value_as_bool(&bool_value(true)), Some(true));
        assert_eq!(value_as_u32(&u32_value(512)), Some(512));
        assert_eq!(value_as_i32(&i32_value(-100)), Some(-100));
        assert_eq!(
            value_as_string(&str_value("tight")).as_deref(),
            Some("tight")
        );
        assert_eq!(
            value_as_str_array(&str_array_value(&["cpu", "npu"])),
            Some(vec!["cpu".to_string(), "npu".to_string()])
        );
    }

    #[test]
    fn integer_helpers_accept_the_other_signedness_when_in_range() {
        assert_eq!(value_as_u32(&i32_value(42)), Some(42));
        assert_eq!(value_as_i32(&u32_value(42)), Some(42));
    }

    #[test]
    fn integer_helpers_reject_out_of_range_values() {
        assert_eq!(value_as_u32(&i32_value(-1)), None);
        assert_eq!(value_as_i32(&u32_value(u32::MAX)), None);
    }

    #[test]
    fn helpers_reject_mismatched_types() {
        assert_eq!(value_as_bool(&u32_value(1)), None);
        assert_eq!(value_as_u32(&str_value("7")), None);
        assert_eq!(value_as_i32(&bool_value(true)), None);
        assert_eq!(value_as_string(&u32_value(7)), None);
        assert_eq!(value_as_str_array(&str_value("cpu")), None);
    }

    #[test]
    fn empty_str_array_round_trips() {
        let empty: [&str; 0] = [];
        assert_eq!(
            value_as_str_array(&str_array_value(&empty)),
            Some(Vec::new())
        );
    }

    #[test]
    fn ids_are_unique() {
        let mut effects = EFFECT_IDS.to_vec();
        effects.sort_unstable();
        effects.dedup();
        assert_eq!(effects.len(), EFFECT_IDS.len());

        let mut reactions = REACTION_IDS.to_vec();
        reactions.sort_unstable();
        reactions.dedup();
        assert_eq!(reactions.len(), REACTION_IDS.len());
    }
}

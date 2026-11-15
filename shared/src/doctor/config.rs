use std::path::{Path, PathBuf};

use super::types::{Check, Status};
use crate::config::AppState;

const TOP_LEVEL_FLAGS: &[&str] = &["bypass"];

const SECTION_KEYS: &[(&str, &[&str])] = &[("inference", &["provider"]), ("camera", &["selected"])];

const EFFECT_KEYS: &[(&str, &[&str])] = &[
    ("center_stage", &["enabled", "zoom", "mode", "crop"]),
    ("portrait_blur", &["enabled", "strength"]),
    ("bg_replace", &["enabled", "background"]),
    (
        "studio_light",
        &[
            "enabled",
            "intensity",
            "brightness",
            "contrast",
            "bg_brightness",
            "auto_light",
            "auto_strength",
        ],
    ),
    ("reactions", &["enabled"]),
];

/// Keys in `state.toml` the daemon would ignore, as dotted paths.
/// `tests::every_key_the_daemon_writes_is_known` keeps the tables in step with `AppState`.
pub fn find_unknown_keys(value: &toml::Value) -> Vec<String> {
    let mut unknown = Vec::new();
    let toml::Value::Table(top) = value else {
        return unknown;
    };
    for (key, section) in top {
        if TOP_LEVEL_FLAGS.contains(&key.as_str()) {
            continue;
        }
        if let Some(known) = known_keys(SECTION_KEYS, key) {
            push_unknown_keys(&mut unknown, key, section, known);
        } else if key == "effects" {
            push_unknown_effect_keys(&mut unknown, section);
        } else {
            unknown.push(key.clone());
        }
    }
    unknown
}

fn known_keys(
    table: &[(&str, &'static [&'static str])],
    name: &str,
) -> Option<&'static [&'static str]> {
    table
        .iter()
        .find_map(|(section, keys)| (*section == name).then_some(*keys))
}

fn push_unknown_effect_keys(unknown: &mut Vec<String>, effects: &toml::Value) {
    let toml::Value::Table(effects) = effects else {
        return;
    };
    for (effect, params) in effects {
        match known_keys(EFFECT_KEYS, effect) {
            Some(known) => push_unknown_keys(unknown, &format!("effects.{effect}"), params, known),
            None => unknown.push(format!("effects.{effect}")),
        }
    }
}

fn push_unknown_keys(unknown: &mut Vec<String>, path: &str, section: &toml::Value, known: &[&str]) {
    let toml::Value::Table(section) = section else {
        return;
    };
    unknown.extend(
        section
            .keys()
            .filter(|key| !known.contains(&key.as_str()))
            .map(|key| format!("{path}.{key}")),
    );
}

pub fn config_verdict(path: &Path, content: Result<&str, &std::io::Error>) -> Check {
    match content {
        Ok(text) => {
            let parsed_app: Result<AppState, _> = toml::from_str(text);
            let parsed_val: Result<toml::Value, _> = toml::from_str(text);

            match (parsed_app, parsed_val) {
                (Ok(_), Ok(val)) => {
                    let unknown = find_unknown_keys(&val);
                    if unknown.is_empty() {
                        Check::new(
                            "config",
                            "state.toml",
                            Status::Ok,
                            format!("{} is valid", path.display()),
                            None,
                        )
                    } else {
                        Check::new(
                            "config",
                            "state.toml",
                            Status::Warn,
                            format!(
                                "{} is valid, but contains unrecognized key(s): {}",
                                path.display(),
                                unknown.join(", ")
                            ),
                            None,
                        )
                    }
                }
                (Err(e), _) => Check::new(
                    "config",
                    "state.toml",
                    Status::Fail,
                    format!("parse error in {}: {e}", path.display()),
                    Some("check state.toml syntax or remove invalid keys".into()),
                ),
                _ => Check::new(
                    "config",
                    "state.toml",
                    Status::Fail,
                    format!("invalid TOML in {}", path.display()),
                    Some("check state.toml syntax".into()),
                ),
            }
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Check::new(
            "config",
            "state.toml",
            Status::Ok,
            format!("default config ({} does not exist yet)", path.display()),
            None,
        ),
        Err(err) => Check::new(
            "config",
            "state.toml",
            Status::Fail,
            format!("could not read {}: {err}", path.display()),
            None,
        ),
    }
}

pub fn probe_config_path(home: Option<&Path>) -> PathBuf {
    match home {
        Some(h) => h.join(".config/vinoopeneffects/state.toml"),
        None => dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("vinoopeneffects/state.toml"),
    }
}

pub fn probe_config(home: Option<&Path>) -> (PathBuf, Result<String, std::io::Error>) {
    let path = probe_config_path(home);
    let res = std::fs::read_to_string(&path);
    (path, res)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_verdict_missing_file() {
        let err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let c = config_verdict(Path::new("/missing/state.toml"), Err(&err));
        assert_eq!(c.status, Status::Ok);
        assert!(c.detail.contains("default config"));
    }

    #[test]
    fn test_config_verdict_valid() {
        let toml = r#"
            bypass = false
            [effects.portrait_blur]
            enabled = true
            strength = 50
        "#;
        let c = config_verdict(Path::new("/config/state.toml"), Ok(toml));
        assert_eq!(c.status, Status::Ok);
        assert!(c.detail.contains("valid"));
    }

    #[test]
    fn test_config_verdict_unknown_keys() {
        let toml = r#"
            unknown_root = true
            [effects.unknown_effect]
            enabled = true
        "#;
        let c = config_verdict(Path::new("/config/state.toml"), Ok(toml));
        assert_eq!(c.status, Status::Warn);
        assert!(c.detail.contains("unknown_root"));
        assert!(c.detail.contains("effects.unknown_effect"));
    }

    #[test]
    fn every_key_the_daemon_writes_is_known() {
        let written = toml::to_string_pretty(&AppState::default()).unwrap();
        let value: toml::Value = toml::from_str(&written).unwrap();
        assert_eq!(find_unknown_keys(&value), Vec::<String>::new());
    }

    #[test]
    fn an_unknown_key_is_reported_with_its_full_path() {
        let toml = r#"
            bypass = true
            extra = 1
            [inference]
            provider = "auto"
            threads = 4
            [camera]
            selected = ""
            fps = 30
            [effects.center_stage]
            zoom = "tight"
            pan = 1
            [effects.center_stage.crop]
            anything = 1
            [effects.portrait_blur]
            strength = 5
            radius = 1
            [effects.bg_replace]
            background = ""
            colour = 1
            [effects.studio_light]
            auto_light = true
            warmth = 1
            [effects.reactions]
            enabled = true
            size = 1
            [effects.sparkle]
            enabled = true
        "#;
        let value: toml::Value = toml::from_str(toml).unwrap();
        let mut unknown = find_unknown_keys(&value);
        unknown.sort();
        assert_eq!(
            unknown,
            [
                "camera.fps",
                "effects.bg_replace.colour",
                "effects.center_stage.pan",
                "effects.portrait_blur.radius",
                "effects.reactions.size",
                "effects.sparkle",
                "effects.studio_light.warmth",
                "extra",
                "inference.threads",
            ]
        );
    }

    #[test]
    fn a_known_key_holding_the_wrong_shape_is_left_to_the_parser() {
        let value: toml::Value = toml::from_str("effects = 1\ninference = \"npu\"").unwrap();
        assert_eq!(find_unknown_keys(&value), Vec::<String>::new());
    }

    #[test]
    fn test_config_verdict_syntax_error() {
        let toml = "this is not valid toml = = =";
        let c = config_verdict(Path::new("/config/state.toml"), Ok(toml));
        assert_eq!(c.status, Status::Fail);
        assert!(c.fix.is_some());
    }
}

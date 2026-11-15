use shared::dbus::VariantMap;
use zvariant::{OwnedValue, Str};

pub(crate) fn parse_value(raw: &str) -> OwnedValue {
    if raw.eq_ignore_ascii_case("true") {
        return OwnedValue::from(true);
    }
    if raw.eq_ignore_ascii_case("false") {
        return OwnedValue::from(false);
    }
    if let Ok(value) = raw.parse::<u32>() {
        return OwnedValue::from(value);
    }
    if let Ok(value) = raw.parse::<i32>() {
        return OwnedValue::from(value);
    }
    OwnedValue::from(Str::from(raw.to_owned()))
}

pub(crate) fn simple_value(map: &VariantMap, key: &str) -> Option<String> {
    map.get(key)
        .and_then(shared::dbus::value_as_string)
        .or_else(|| {
            map.get(key)
                .and_then(shared::dbus::value_as_u32)
                .map(|v| v.to_string())
        })
        .or_else(|| {
            map.get(key)
                .and_then(shared::dbus::value_as_i32)
                .map(|v| v.to_string())
        })
        .or_else(|| {
            map.get(key)
                .and_then(shared::dbus::value_as_bool)
                .map(|v| v.to_string())
        })
}

pub(crate) fn format_params(params: &VariantMap) -> String {
    let mut parts = params
        .iter()
        .map(|(key, value)| {
            let value = simple_value(params, key).unwrap_or_else(|| format!("{value:?}"));
            format!("{key}={value}")
        })
        .collect::<Vec<_>>();
    parts.sort();
    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use shared::dbus::{
        bool_value, i32_value, str_value, u32_value, value_as_bool, value_as_string, value_as_u32,
    };

    use super::*;

    #[test]
    fn parse_value_picks_the_narrowest_type() {
        assert_eq!(value_as_bool(&parse_value("true")), Some(true));
        assert_eq!(value_as_bool(&parse_value("FALSE")), Some(false));

        let unsigned = parse_value("42");
        assert_eq!(u32::try_from(&unsigned).ok(), Some(42));

        let negative = parse_value("-7");
        assert_eq!(i32::try_from(&negative).ok(), Some(-7));
        assert_eq!(value_as_u32(&negative), None);

        assert_eq!(
            value_as_string(&parse_value("#112233")).as_deref(),
            Some("#112233")
        );
        assert_eq!(
            value_as_string(&parse_value("99999999999")).as_deref(),
            Some("99999999999")
        );
    }

    #[test]
    fn simple_value_stringifies_each_scalar_type() {
        let map = VariantMap::from([
            ("s".into(), str_value("hello")),
            ("u".into(), u32_value(7)),
            ("i".into(), i32_value(-3)),
            ("b".into(), bool_value(true)),
        ]);
        assert_eq!(simple_value(&map, "s").as_deref(), Some("hello"));
        assert_eq!(simple_value(&map, "u").as_deref(), Some("7"));
        assert_eq!(simple_value(&map, "i").as_deref(), Some("-3"));
        assert_eq!(simple_value(&map, "b").as_deref(), Some("true"));
        assert_eq!(simple_value(&map, "missing"), None);
    }

    #[test]
    fn format_params_is_sorted_key_value_pairs() {
        let map = VariantMap::from([
            ("strength".into(), u32_value(60)),
            ("enabled".into(), bool_value(false)),
            ("brightness".into(), i32_value(-20)),
        ]);
        assert_eq!(
            format_params(&map),
            "brightness=-20 enabled=false strength=60"
        );
        assert_eq!(format_params(&VariantMap::new()), "");
    }

    #[test]
    fn format_params_falls_back_to_debug_for_non_scalars() {
        let map = VariantMap::from([(
            "available".into(),
            shared::dbus::str_array_value(&["cpu", "npu"]),
        )]);
        let out = format_params(&map);
        assert!(out.starts_with("available="), "{out}");
        assert!(out.contains("cpu") && out.contains("npu"), "{out}");
    }
}

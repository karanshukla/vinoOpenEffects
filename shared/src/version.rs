//! Whether this binary is a release or a source build.

/// What release CI sets `VINOOPENEFFECTS_VERSION` to: the `vN` tag.
const STAMP: Option<&str> = option_env!("VINOOPENEFFECTS_VERSION");

/// The release tag, or `dev` for any build without the stamp, so a binary says
/// what it is wherever it was installed from. Used by `--version`, the About
/// dialog and the update check.
pub const VERSION: &str = version_of(STAMP);

// A stamp that isn't a `vN` release tag must not compile. Releases here are
// consecutive tags; `Cargo.toml`'s version is not the release number.
const _: () = assert!(
    STAMP.is_none() || is_release_tag(VERSION),
    "VINOOPENEFFECTS_VERSION must be a release tag like v7"
);

const fn version_of(stamp: Option<&str>) -> &str {
    match stamp {
        Some(v) => v,
        None => "dev",
    }
}

const fn is_release_tag(tag: &str) -> bool {
    let b = tag.as_bytes();
    if b.len() < 2 || b[0] != b'v' {
        return false;
    }
    let mut i = 1;
    while i < b.len() {
        if !b[i].is_ascii_digit() {
            return false;
        }
        i += 1;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_release_build_reports_its_version() {
        assert_eq!(version_of(Some("v7")), "v7");
    }

    #[test]
    fn an_unstamped_build_reports_dev() {
        assert_eq!(version_of(None), "dev");
    }

    #[test]
    fn a_stamp_must_be_a_release_tag() {
        assert!(is_release_tag("v7"));
        assert!(is_release_tag("v12"));
        assert!(!is_release_tag("7"));
        assert!(!is_release_tag("v"));
        assert!(!is_release_tag("v7-dirty"));
        assert!(!is_release_tag("dev"));
    }

    #[test]
    fn this_build_is_dev_or_a_release_tag() {
        assert!(VERSION == "dev" || is_release_tag(VERSION));
    }
}

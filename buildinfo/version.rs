// Shared by the GUI and CLI build scripts through `include!`.

/// The release shown by `--version` and the About dialog: `VINOOPENEFFECTS_VERSION`
/// if set, else the `VERSION` file a release's source bundle carries, else the
/// checkout's exact `vN` release tag, else `dev`.
fn release_version() -> String {
    use std::process::Command;

    println!("cargo:rerun-if-env-changed=VINOOPENEFFECTS_VERSION");
    if let Ok(v) = std::env::var("VINOOPENEFFECTS_VERSION")
        && !v.trim().is_empty()
    {
        return v.trim().to_string();
    }

    // The release workflow's source bundle has no .git, so it ships the tag in a file.
    let version_file = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../VERSION");
    println!("cargo:rerun-if-changed={}", version_file.display());
    if let Ok(v) = std::fs::read_to_string(&version_file)
        && is_release_tag(v.trim())
    {
        return v.trim().to_string();
    }

    let git = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
    };

    let mut watched = vec![git(&["rev-parse", "--git-path", "HEAD"])];
    if let Some(branch) = git(&["rev-parse", "--symbolic-full-name", "HEAD"]) {
        watched.push(git(&["rev-parse", "--git-path", &branch]));
    }
    watched.push(git(&["rev-parse", "--git-path", "refs/tags"]));
    watched.push(git(&["rev-parse", "--git-path", "packed-refs"]));
    for path in watched.into_iter().flatten() {
        if std::path::Path::new(&path).exists() {
            println!("cargo:rerun-if-changed={path}");
        }
    }

    git(&["describe", "--tags", "--exact-match", "--match", "v[0-9]*", "HEAD"])
        .filter(|tag| is_release_tag(tag))
        .unwrap_or_else(|| "dev".into())
}

fn is_release_tag(tag: &str) -> bool {
    tag.len() > 1 && tag.starts_with('v') && tag[1..].bytes().all(|b| b.is_ascii_digit())
}

//! "Check for Updates", from the primary menu: one anonymous GET to GitHub's
//! releases API through `curl`, only when it's clicked. When a newer release
//! is out it offers to install it with `vinoopeneffects-upgrade`, the release
//! installer `install.sh` leaves behind; a build from a checkout has none and
//! gets the release page instead. The same steps as vino-ui's About row
//! (karanshukla/vinoUserInterface, src/update.rs), which this workspace can't
//! depend on (it needs Rust 1.95); keep them in step.

use std::path::PathBuf;
use std::process::{Command, Stdio};

use adw::prelude::*;
use gtk::{gio, glib};

const REPO: &str = "karanshukla/vinoOpenEffects";
const CURRENT: &str = env!("VINOOPENEFFECTS_VERSION");

/// What a check found.
#[derive(Debug, PartialEq)]
enum Found {
    Newer(String),
    Latest(String),
}

/// A tag or version as numbers: `v3` is [3]. Anything else (`dev`) is None
/// and is never offered an update.
fn parts(version: &str) -> Option<Vec<u64>> {
    version
        .strip_prefix('v')
        .unwrap_or(version)
        .split('.')
        .map(|p| p.parse().ok())
        .collect()
}

fn is_newer(current: &str, latest: &str) -> bool {
    let (Some(mut current), Some(mut latest)) = (parts(current), parts(latest)) else {
        return false;
    };
    let len = current.len().max(latest.len());
    current.resize(len, 0);
    latest.resize(len, 0);
    latest > current
}

/// The `"tag_name"` of a releases API response, if it looks like a tag: it
/// ends up in a dialog and in the installer's environment.
fn latest_tag(json: &str) -> Option<String> {
    let after = &json[json.find("\"tag_name\"")? + "\"tag_name\"".len()..];
    let after = after
        .trim_start()
        .strip_prefix(':')?
        .trim_start()
        .strip_prefix('"')?;
    let tag = &after[..after.find('"')?];
    let valid = !tag.is_empty()
        && tag.len() <= 32
        && tag.starts_with('v')
        && tag
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-');
    valid.then(|| tag.to_string())
}

pub(crate) fn upgrade_command() -> Option<PathBuf> {
    let path = dirs::executable_dir()
        .or_else(|| dirs::home_dir().map(|home| home.join(".local/bin")))?
        .join("vinoopeneffects-upgrade");
    path.is_file().then_some(path)
}

fn check() -> Result<Found, String> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let out = Command::new("curl")
        .args(["-fsSL", "--max-time", "10", "--max-filesize", "1048576"])
        .args(["-H", "Accept: application/vnd.github+json", &url])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|_| "curl isn't installed.".to_string())?;
    if !out.status.success() {
        return Err("Couldn't reach GitHub to check. Try again when you're online.".into());
    }
    let latest = latest_tag(&String::from_utf8_lossy(&out.stdout))
        .ok_or_else(|| "GitHub's answer had no release in it.".to_string())?;
    Ok(if is_newer(CURRENT, &latest) {
        Found::Newer(latest)
    } else {
        Found::Latest(latest)
    })
}

/// Runs the upgrade for `tag`; the error is the installer's last line.
fn install(upgrade: PathBuf, tag: String) -> Result<(), String> {
    let out = Command::new(&upgrade)
        .env("VINO_UPDATE_TAG", &tag)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("Couldn't run {}: {e}.", upgrade.display()))?;
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    let mut lines = stderr
        .lines()
        .rev()
        .map(str::trim)
        .filter(|l| !l.is_empty());
    let last = lines.clone().next().map(str::to_owned);
    let line = lines
        .find(|l| l.starts_with("install:") || l.starts_with('✗'))
        .map(str::to_owned)
        .or(last);
    Err(line.unwrap_or_else(|| format!("The update to {tag} failed.")))
}

fn alert(heading: &str, body: &str) -> adw::AlertDialog {
    let dialog = adw::AlertDialog::new(Some(heading), Some(body));
    dialog.add_response("close", "Close");
    dialog.set_close_response("close");
    dialog.set_default_response(Some("close"));
    dialog
}

/// The menu item: check, then say what was found and offer the next step.
pub fn present(parent: &impl IsA<gtk::Widget>) {
    let parent: gtk::Widget = parent.clone().upcast();
    glib::spawn_future_local(async move {
        let found = gio::spawn_blocking(check)
            .await
            .unwrap_or_else(|_| Err("The check stopped.".into()));
        match found {
            Ok(Found::Newer(tag)) => offer(&parent, tag),
            Ok(Found::Latest(latest)) => {
                let body = if parts(CURRENT).is_some() {
                    format!("vinoOpenEffects {CURRENT} is the newest release.")
                } else {
                    format!("The newest release is {latest}. This is a development build.")
                };
                alert("Up to Date", &body).present(Some(&parent));
            }
            Err(why) => alert("Couldn't Check for Updates", &why).present(Some(&parent)),
        }
    });
}

fn offer(parent: &gtk::Widget, tag: String) {
    let upgrade = upgrade_command();
    let body = if upgrade.is_some() {
        format!(
            "{tag} is out. Updating restarts the effects daemon, so the camera pauses for a moment."
        )
    } else {
        format!(
            "{tag} is out. This install wasn't made by the release installer; its release page says how to update."
        )
    };
    let dialog = adw::AlertDialog::new(Some("Update Available"), Some(&body));
    dialog.add_response("cancel", "Not Now");
    let action = if upgrade.is_some() {
        format!("Update to {tag}")
    } else {
        "Open Release Page".into()
    };
    dialog.add_response("go", &action);
    dialog.set_response_appearance("go", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("go"));
    dialog.set_close_response("cancel");
    let window = parent.clone();
    dialog.connect_response(None, move |_, response| {
        if response != "go" {
            return;
        }
        match upgrade.clone() {
            Some(upgrade) => run_install(&window, upgrade, tag.clone()),
            None => {
                let url = format!("https://github.com/{REPO}/releases/tag/{tag}");
                gtk::UriLauncher::new(&url).launch(
                    window.root().and_downcast_ref::<gtk::Window>(),
                    gio::Cancellable::NONE,
                    |_| {},
                );
            }
        }
    });
    dialog.present(Some(parent));
}

fn run_install(parent: &gtk::Widget, upgrade: PathBuf, tag: String) {
    let progress = adw::AlertDialog::new(
        Some(&format!("Installing {tag}")),
        Some("This can take a few minutes."),
    );
    progress.set_can_close(false);
    progress.present(Some(parent));
    let parent = parent.clone();
    glib::spawn_future_local(async move {
        let for_thread = tag.clone();
        let result = gio::spawn_blocking(move || install(upgrade, for_thread))
            .await
            .unwrap_or_else(|_| Err("The update stopped before it finished.".into()));
        progress.set_can_close(true);
        progress.force_close();
        let (heading, body) = match result {
            Ok(()) => (
                "Updated".to_string(),
                format!("Restart vinoOpenEffects to use {tag}."),
            ),
            Err(why) => (format!("Couldn't Install {tag}"), why),
        };
        alert(&heading, &body).present(Some(&parent));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_numbers_compare_and_dev_never_updates() {
        assert!(is_newer("v3", "v4"));
        assert!(!is_newer("v4", "v4"));
        assert!(!is_newer("v10", "v9"));
        assert!(!is_newer("dev", "v9"));
    }

    #[test]
    fn only_a_plausible_tag_is_taken() {
        assert_eq!(
            latest_tag(r#"{"tag_name": "v4","name":"v4"}"#),
            Some("v4".into())
        );
        assert_eq!(latest_tag(r#"{"tag_name":"v4; rm -rf ~"}"#), None);
        assert_eq!(latest_tag(r#"{"message":"Not Found"}"#), None);
    }
}

//! Deleting the downloaded models from Preferences: the per-user models
//! directory and the OpenVINO compile cache. A system-wide install under
//! `/usr/local` or `/usr` is never touched; it belongs to root.

use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// What Delete removes, both in the user's own home.
#[derive(Debug, Clone, PartialEq)]
pub struct UserPaths {
    /// Where the daemon looks first (`registry::search_dirs`) and
    /// `fetch-models.sh` downloads to.
    pub models: PathBuf,
    /// The NPU compile cache and anything else derived from the models.
    pub cache: PathBuf,
}

impl UserPaths {
    pub fn current() -> Option<Self> {
        Some(Self::under(&dirs::data_dir()?, &dirs::cache_dir()?))
    }

    fn under(data: &Path, cache: &Path) -> Self {
        Self {
            models: data.join("vinoopeneffects/models"),
            cache: cache.join("vinoopeneffects"),
        }
    }

    /// Bytes on disk under both directories.
    pub fn size(&self) -> u64 {
        tree_size(&self.models) + tree_size(&self.cache)
    }

    /// True when there is a model left to delete (the cache alone doesn't count).
    pub fn has_models(&self) -> bool {
        has_model(&self.models)
    }

    /// Remove both directories; one that's already gone is fine.
    pub fn delete(&self) -> io::Result<()> {
        for dir in [&self.models, &self.cache] {
            match std::fs::remove_dir_all(dir) {
                Err(err) if err.kind() != io::ErrorKind::NotFound => return Err(err),
                _ => {}
            }
        }
        Ok(())
    }
}

/// Sum of file sizes under `path`, not following symlinks.
fn tree_size(path: &Path) -> u64 {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return 0;
    };
    if !meta.is_dir() {
        return meta.len();
    }
    std::fs::read_dir(path)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| tree_size(&entry.path()))
                .sum()
        })
        .unwrap_or(0)
}

/// A models directory holds at least one `<id>/manifest.toml`.
fn has_model(dir: &Path) -> bool {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .any(|entry| entry.path().join("manifest.toml").is_file())
        })
        .unwrap_or(false)
}

/// System-wide model directories the daemon will still use after a delete.
pub fn system_dirs_with_models(user: &UserPaths) -> Vec<PathBuf> {
    shared::doctor::models::system_model_dirs()
        .into_iter()
        .filter(|dir| *dir != user.models && has_model(dir))
        .collect()
}

/// Make the daemon let go of the models: it loads them once per camera
/// session and keeps them open, so it is stopped and, when the GUI
/// reconnects, D-Bus starts it again without them.
fn stop_daemon() -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        let conn = zbus::Connection::session().await?;
        let bus = zbus::fdo::DBusProxy::new(&conn).await?;
        let name = zbus::names::BusName::try_from(shared::dbus::SERVICE_NAME)?;
        // Checked first: calling Quit on a stopped daemon would start it.
        if !bus.name_has_owner(name.clone()).await? {
            return Ok(());
        }
        conn.call_method(
            Some(shared::dbus::SERVICE_NAME),
            shared::dbus::OBJECT_PATH,
            Some("io.github.karanshukla.vinoopeneffects.Daemon1"),
            "Quit",
            &(),
        )
        .await?;
        for _ in 0..50 {
            if !bus.name_has_owner(name.clone()).await? {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        anyhow::bail!("the effects daemon didn't stop")
    })
}

/// Stop the daemon, then delete. Runs off the main thread.
pub fn delete(paths: &UserPaths) -> Result<(), String> {
    if let Err(err) = stop_daemon() {
        // Deleting still works: the daemon keeps its open copies until it exits.
        tracing::warn!(%err, "couldn't stop the daemon before deleting models");
    }
    paths
        .delete()
        .map_err(|err| format!("Couldn't delete {}: {err}.", paths.models.display()))
}

/// `path` with the home directory shown as `~`.
pub fn tilde(path: &Path) -> String {
    match dirs::home_dir().and_then(|home| path.strip_prefix(home).ok().map(Path::to_owned)) {
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_install(root: &Path) -> UserPaths {
        let paths = UserPaths::under(&root.join("data"), &root.join("cache"));
        let yunet = paths.models.join("yunet");
        std::fs::create_dir_all(&yunet).unwrap();
        std::fs::write(yunet.join("manifest.toml"), "[model]\n").unwrap();
        std::fs::write(yunet.join("model.onnx"), [0u8; 1000]).unwrap();
        let blobs = paths.cache.join("openvino");
        std::fs::create_dir_all(&blobs).unwrap();
        std::fs::write(blobs.join("blob"), [0u8; 24]).unwrap();
        let backgrounds = root.join("data/vinoopeneffects/backgrounds");
        std::fs::create_dir_all(&backgrounds).unwrap();
        std::fs::write(backgrounds.join("mine.jpg"), b"jpg").unwrap();
        paths
    }

    #[test]
    fn delete_removes_models_and_cache_but_keeps_backgrounds() {
        let root = tempfile::tempdir().unwrap();
        let paths = fake_install(root.path());
        assert!(paths.has_models());
        assert_eq!(paths.size(), 1000 + "[model]\n".len() as u64 + 24);

        paths.delete().unwrap();

        assert!(!paths.models.exists());
        assert!(!paths.cache.exists());
        assert!(!paths.has_models());
        assert_eq!(paths.size(), 0);
        assert!(
            root.path()
                .join("data/vinoopeneffects/backgrounds/mine.jpg")
                .is_file()
        );
    }

    #[test]
    fn deleting_nothing_is_fine() {
        let root = tempfile::tempdir().unwrap();
        let paths = UserPaths::under(&root.path().join("data"), &root.path().join("cache"));
        assert!(!paths.has_models());
        paths.delete().unwrap();
    }

    #[test]
    fn size_does_not_follow_symlinks_out_of_the_tree() {
        let root = tempfile::tempdir().unwrap();
        let paths = fake_install(root.path());
        let elsewhere = root.path().join("elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::fs::write(elsewhere.join("big"), [0u8; 4096]).unwrap();
        std::os::unix::fs::symlink(&elsewhere, paths.models.join("link")).unwrap();
        assert!(paths.size() < 4096);
        paths.delete().unwrap();
        assert!(elsewhere.join("big").is_file());
    }

    #[test]
    fn the_user_dir_is_never_listed_as_a_system_one() {
        let root = tempfile::tempdir().unwrap();
        let mut paths = fake_install(root.path());
        for dir in shared::doctor::models::system_model_dirs() {
            paths.models = dir;
            assert!(!system_dirs_with_models(&paths).contains(&paths.models));
        }
    }
}

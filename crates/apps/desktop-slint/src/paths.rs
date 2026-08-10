//! Execution-environment path resolution for the Slint desktop runner.

use std::fs;
use std::path::{Path, PathBuf};

use crate::DesktopError;

const APPLICATION_DIRECTORY: &str = "milkdrift";
const LEGACY_APPLICATION_DIRECTORY: &str = "llm-app";
const DATABASE_FILE: &str = "state.redb";

pub fn application_database_path() -> Result<PathBuf, DesktopError> {
    let root = application_data_root().ok_or(DesktopError::MissingDataDirectory)?;
    application_database_path_in(&root)
}

fn application_database_path_in(root: &Path) -> Result<PathBuf, DesktopError> {
    let directory = root.join(APPLICATION_DIRECTORY);
    fs::create_dir_all(&directory).map_err(DesktopError::CreateDataDirectory)?;

    let current = directory.join(DATABASE_FILE);
    if current
        .try_exists()
        .map_err(|source| migration_error(root, &current, source))?
    {
        return Ok(current);
    }

    let legacy = root.join(LEGACY_APPLICATION_DIRECTORY).join(DATABASE_FILE);
    if legacy
        .try_exists()
        .map_err(|source| migration_error(root, &current, source))?
    {
        fs::rename(&legacy, &current).map_err(|source| DesktopError::MigrateApplicationState {
            legacy,
            current: current.clone(),
            source,
        })?;
    }
    Ok(current)
}

fn migration_error(root: &Path, current: &Path, source: std::io::Error) -> DesktopError {
    DesktopError::MigrateApplicationState {
        legacy: root.join(LEGACY_APPLICATION_DIRECTORY).join(DATABASE_FILE),
        current: current.to_path_buf(),
        source,
    }
}

fn application_data_root() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("XDG_DATA_HOME") {
        return Some(PathBuf::from(path));
    }
    if cfg!(target_os = "windows") {
        return std::env::var_os("LOCALAPPDATA")
            .or_else(|| std::env::var_os("APPDATA"))
            .map(PathBuf::from);
    }

    let home = PathBuf::from(std::env::var_os("HOME")?);
    if cfg!(target_os = "macos") {
        Some(home.join("Library/Application Support"))
    } else {
        Some(home.join(".local/share"))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::{APPLICATION_DIRECTORY, DATABASE_FILE, LEGACY_APPLICATION_DIRECTORY};
    use super::{Path, PathBuf, application_database_path_in, fs};

    static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

    struct TestRoot(PathBuf);

    impl TestRoot {
        fn new() -> Result<Self, Box<dyn std::error::Error>> {
            let identifier = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "milkdrift-desktop-paths-{}-{identifier}",
                std::process::id()
            ));
            match fs::remove_dir_all(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
            fs::create_dir_all(&path)?;
            Ok(Self(path))
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestRoot {
        fn drop(&mut self) {
            let _ignored = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn fresh_state_uses_the_milkdrift_directory() -> Result<(), Box<dyn std::error::Error>> {
        let root = TestRoot::new()?;
        let path = application_database_path_in(root.path())?;
        assert_eq!(
            path,
            root.path().join(APPLICATION_DIRECTORY).join(DATABASE_FILE)
        );
        assert!(path.parent().is_some_and(Path::is_dir));
        Ok(())
    }

    #[test]
    fn legacy_state_is_moved_once_without_overwriting_current_state()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = TestRoot::new()?;
        let legacy = root.path().join(LEGACY_APPLICATION_DIRECTORY);
        fs::create_dir_all(&legacy)?;
        let legacy_database = legacy.join(DATABASE_FILE);
        fs::write(&legacy_database, b"legacy")?;

        let current = application_database_path_in(root.path())?;
        assert_eq!(fs::read(&current)?, b"legacy");
        assert!(!legacy_database.try_exists()?);

        fs::write(&current, b"current")?;
        fs::create_dir_all(&legacy)?;
        fs::write(&legacy_database, b"stale legacy")?;
        assert_eq!(application_database_path_in(root.path())?, current);
        assert_eq!(fs::read(&current)?, b"current");
        assert_eq!(fs::read(&legacy_database)?, b"stale legacy");
        Ok(())
    }
}

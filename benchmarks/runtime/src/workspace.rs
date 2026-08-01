//! Unique temporary runtime state rooted under the workspace target directory.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::{BenchmarkError, BenchmarkResult};

const CREATION_ATTEMPTS: u64 = 128;
static NEXT_IDENTIFIER: AtomicU64 = AtomicU64::new(0);

pub(crate) struct OutputWorkspace {
    root: PathBuf,
    cleaned: bool,
}

impl OutputWorkspace {
    pub(crate) fn create(label: &str) -> BenchmarkResult<Self> {
        let parent = repository_root()?.join("target/runtime-benchmarks");
        fs::create_dir_all(&parent).map_err(|error| {
            BenchmarkError::new(format!(
                "could not create runtime benchmark target directory {}: {error}",
                parent.display()
            ))
        })?;
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos());
        let first = NEXT_IDENTIFIER.fetch_add(1, Ordering::Relaxed);
        for attempt in 0..CREATION_ATTEMPTS {
            let identifier = first.wrapping_add(attempt);
            let root = parent.join(format!(
                "{label}-{}-{timestamp}-{identifier}",
                std::process::id()
            ));
            match fs::create_dir(&root) {
                Ok(()) => {
                    return Ok(Self {
                        root,
                        cleaned: false,
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => {
                    return Err(BenchmarkError::new(format!(
                        "could not create temporary runtime workspace {}: {error}",
                        root.display()
                    )));
                }
            }
        }
        Err(BenchmarkError::new(format!(
            "could not create a unique runtime workspace under {} after {CREATION_ATTEMPTS} attempts",
            parent.display()
        )))
    }

    pub(crate) fn database_path(&self, label: &str, ordinal: u32) -> PathBuf {
        self.root.join(format!("{label}-{ordinal}.redb"))
    }

    pub(crate) fn internal_cache_directory(&self) -> BenchmarkResult<PathBuf> {
        let path = self.root.join("download-free-cache");
        fs::create_dir_all(&path).map_err(|error| {
            BenchmarkError::new(format!(
                "could not create temporary application cache directory {}: {error}",
                path.display()
            ))
        })?;
        Ok(path)
    }

    pub(crate) fn cleanup(&mut self) -> BenchmarkResult {
        match fs::remove_dir_all(&self.root) {
            Ok(()) => {
                self.cleaned = true;
                Ok(())
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                self.cleaned = true;
                Ok(())
            }
            Err(error) => Err(BenchmarkError::new(format!(
                "could not remove temporary runtime workspace {}: {error}",
                self.root.display()
            ))),
        }
    }
}

impl Drop for OutputWorkspace {
    fn drop(&mut self) {
        if self.cleaned {
            return;
        }
        if let Err(error) = fs::remove_dir_all(&self.root)
            && error.kind() != io::ErrorKind::NotFound
        {
            eprintln!(
                "runtime benchmark cleanup fallback could not remove {}: {error}",
                self.root.display()
            );
        }
    }
}

pub(crate) fn repository_root() -> BenchmarkResult<PathBuf> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    root.canonicalize().map_err(|error| {
        BenchmarkError::new(format!(
            "could not resolve repository root {}: {error}",
            root.display()
        ))
    })
}

pub(crate) fn canonical_external_cache_directory(
    path: &Path,
    repository_root: &Path,
) -> BenchmarkResult<PathBuf> {
    require_directory(path, "explicit Hugging Face cache directory")?;
    let cache = path.canonicalize().map_err(|error| {
        BenchmarkError::new(format!(
            "could not canonicalize explicit cache directory {}: {error}",
            path.display()
        ))
    })?;
    let target = repository_root.join("target");
    if !cache_location_is_allowed(repository_root, &target, &cache) {
        return Err(BenchmarkError::new(format!(
            "explicit Hugging Face cache {} is inside the source tree but outside the shared root target; use root target/ or a directory outside the repository",
            cache.display()
        )));
    }
    Ok(cache)
}

fn cache_location_is_allowed(repository_root: &Path, target: &Path, cache: &Path) -> bool {
    !cache.starts_with(repository_root) || cache.starts_with(target)
}

pub(crate) fn require_directory(path: &Path, label: &str) -> BenchmarkResult {
    let metadata = fs::metadata(path).map_err(|error| {
        BenchmarkError::new(format!(
            "{label} {} is unavailable: {error}",
            path.display()
        ))
    })?;
    if !metadata.is_dir() {
        return Err(BenchmarkError::new(format!(
            "{label} {} is not a directory",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::cache_location_is_allowed;
    use std::path::Path;

    #[test]
    fn explicit_cache_may_use_root_target_or_an_external_directory() {
        let root = Path::new("/work/milkdrift");
        let target = root.join("target");
        assert!(cache_location_is_allowed(
            root,
            &target,
            &target.join("runtime-benchmarks/hf-cache")
        ));
        assert!(cache_location_is_allowed(
            root,
            &target,
            Path::new("/var/cache/milkdrift")
        ));
    }

    #[test]
    fn explicit_cache_rejects_source_tree_locations() {
        let root = Path::new("/work/milkdrift");
        let target = root.join("target");
        assert!(!cache_location_is_allowed(
            root,
            &target,
            &root.join("benchmarks/runtime/hf-cache")
        ));
        assert!(!cache_location_is_allowed(
            root,
            &target,
            &root.join(".git/model-cache")
        ));
    }
}

//! Declared native directories share one durable conflict owner across attempts and restarts.

use milkdrift_blueprint::TaskPathAccess;
use milkdrift_persistence::{NativePathClaim, NativePathClaimStore, NativePathRoot};
use milkdrift_runtime::ExecutorError;
use std::{path::Path, sync::Arc};

pub(crate) struct NativePathClaims {
    store: Arc<dyn NativePathClaimStore>,
}

impl NativePathClaims {
    pub(crate) fn new(store: Arc<dyn NativePathClaimStore>) -> Self {
        Self { store }
    }

    pub(crate) fn acquire(
        &self,
        owner: &str,
        paths: &[TaskPathAccess],
    ) -> Result<(), ExecutorError> {
        if paths.is_empty() {
            return Ok(());
        }
        if let Some(existing) = self.store.native_path_claim(owner).map_err(storage)? {
            if existing.roots.len() != paths.len()
                || existing.roots.iter().zip(paths).any(|(root, request)| {
                    root.requested != Path::new(&request.path) || root.write != request.write
                })
            {
                return Err(refused(
                    "attempt changed its accepted native directory claims",
                ));
            }
            return self.revalidate(owner);
        }
        let roots = paths
            .iter()
            .map(|path| capture(Path::new(&path.path), path.write))
            .collect::<Result<Vec<_>, _>>()?;
        self.store
            .claim_native_paths(&NativePathClaim {
                owner: owner.to_owned(),
                roots,
            })
            .map_err(storage)?;
        self.revalidate(owner)
    }

    pub(crate) fn revalidate(&self, owner: &str) -> Result<(), ExecutorError> {
        let claim = self
            .store
            .native_path_claim(owner)
            .map_err(storage)?
            .ok_or_else(|| refused("accepted native directory claim is missing; entry refused"))?;
        for root in claim.roots {
            if capture(&root.requested, root.write)? != root {
                return Err(refused(
                    "native directory or symlink target changed after its claim; entry refused",
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn release_settled(&self, owner: &str) -> Result<(), ExecutorError> {
        self.store.release_native_paths(owner).map_err(storage)
    }
}

fn capture(requested: &Path, write: bool) -> Result<NativePathRoot, ExecutorError> {
    if !requested.is_absolute() {
        return Err(refused("native project roots must be absolute paths"));
    }
    let physical = requested
        .canonicalize()
        .map_err(|error| refused(&format!("native project root cannot be resolved: {error}")))?;
    let metadata = physical
        .metadata()
        .map_err(|error| refused(&format!("native project root cannot be inspected: {error}")))?;
    if !metadata.is_dir() {
        return Err(refused("native project root must be an existing directory"));
    }
    let identity = physical_identity(&metadata)?;
    Ok(NativePathRoot {
        requested: requested.to_path_buf(),
        physical,
        identity,
        write,
    })
}

#[cfg(unix)]
fn physical_identity(metadata: &std::fs::Metadata) -> Result<String, ExecutorError> {
    use std::os::unix::fs::MetadataExt;
    Ok(format!("unix:{}:{}", metadata.dev(), metadata.ino()))
}

#[cfg(not(unix))]
fn physical_identity(_metadata: &std::fs::Metadata) -> Result<String, ExecutorError> {
    Err(refused(
        "native project directory identity is not qualified on this platform",
    ))
}

fn refused(message: &str) -> ExecutorError {
    ExecutorError::BoundaryBeforeEntry(message.to_owned())
}
fn storage(error: milkdrift_persistence::PersistenceError) -> ExecutorError {
    refused(&error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use milkdrift_redb_store::RedbStore;

    #[cfg(unix)]
    #[test]
    fn overlapping_writer_survives_host_reopen_until_durable_settlement()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        let project = tempfile::tempdir()?;
        std::fs::create_dir(project.path().join("research"))?;
        let path = |write, path: &Path| TaskPathAccess {
            path: path.to_string_lossy().into_owned(),
            write,
        };
        {
            let host = NativePathClaims::new(Arc::new(RedbStore::open(root.path())?));
            host.acquire("writer", &[path(true, project.path())])?;
            assert!(
                host.acquire("reader", &[path(false, &project.path().join("research"))])
                    .is_err()
            );
        }
        let host = NativePathClaims::new(Arc::new(RedbStore::open(root.path())?));
        assert!(
            host.acquire("new-writer", &[path(true, project.path())])
                .is_err()
        );
        host.acquire("writer", &[path(true, project.path())])?;
        host.release_settled("writer")?;
        host.acquire("reader-a", &[path(false, project.path())])?;
        host.acquire("reader-b", &[path(false, &project.path().join("research"))])?;
        assert!(
            host.acquire("writer", &[path(true, project.path())])
                .is_err()
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn disappearing_claim_refuses_final_entry() -> Result<(), Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        let project = tempfile::tempdir()?;
        let store = Arc::new(RedbStore::open(root.path())?);
        let host = NativePathClaims::new(store.clone());
        host.acquire(
            "writer",
            &[TaskPathAccess {
                path: project.path().to_string_lossy().into_owned(),
                write: true,
            }],
        )?;
        store.release_native_paths("writer")?;
        assert!(host.revalidate("writer").is_err());
        Ok(())
    }

    #[cfg(not(unix))]
    #[test]
    fn unqualified_directory_identity_refuses_native_entry()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        let project = tempfile::tempdir()?;
        let host = NativePathClaims::new(Arc::new(RedbStore::open(root.path())?));
        assert!(
            host.acquire(
                "writer",
                &[TaskPathAccess {
                    path: project.path().to_string_lossy().into_owned(),
                    write: true,
                }]
            )
            .is_err()
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn aliases_conflict_and_changed_symlink_or_directory_refuses_entry()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        let project = tempfile::tempdir()?;
        let other = tempfile::tempdir()?;
        let link = root.path().join("project-alias");
        std::os::unix::fs::symlink(project.path(), &link)?;
        let host = NativePathClaims::new(Arc::new(RedbStore::open(root.path().join("store"))?));
        host.acquire(
            "writer",
            &[TaskPathAccess {
                path: link.to_string_lossy().into_owned(),
                write: true,
            }],
        )?;
        assert!(
            host.acquire(
                "alias-writer",
                &[TaskPathAccess {
                    path: project.path().to_string_lossy().into_owned(),
                    write: true
                }]
            )
            .is_err()
        );
        std::fs::remove_file(&link)?;
        std::os::unix::fs::symlink(other.path(), &link)?;
        assert!(host.revalidate("writer").is_err());
        Ok(())
    }
}

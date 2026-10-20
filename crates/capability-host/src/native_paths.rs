//! Declared native directories share one durable conflict owner across attempts and restarts.

use milkdrift_blueprint::TaskPathAccess;
use milkdrift_persistence::{NativePathClaim, NativePathClaimStore, NativePathRoot};
use milkdrift_runtime::ExecutorError;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

pub(crate) struct NativePathClaims {
    store: Arc<dyn NativePathClaimStore>,
    project_protected_paths: Vec<PathBuf>,
}

impl NativePathClaims {
    pub(crate) fn new(store: Arc<dyn NativePathClaimStore>) -> Self {
        Self {
            store,
            project_protected_paths: Vec::new(),
        }
    }

    pub(crate) fn with_project_protection(mut self, paths: Vec<PathBuf>) -> Self {
        self.project_protected_paths = paths;
        self
    }

    pub(crate) fn verify_project(
        &self,
        binding: &milkdrift_blueprint::TaskProjectBinding,
    ) -> Result<(), ExecutorError> {
        verify_project_binding(binding)?;
        for folder in std::iter::once(&binding.folder).chain(&binding.scopes) {
            let root = Path::new(&folder.physical);
            if self
                .project_protected_paths
                .iter()
                .any(|path| path.starts_with(root) || root.starts_with(path))
            {
                return Err(refused(
                    "accepted project overlaps current protected operator files; external entry refused",
                ));
            }
        }
        Ok(())
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

    pub(crate) fn acquire_serving(
        &self,
        record: &milkdrift_persistence::PeerExecutionRecord,
        paths: &[TaskPathAccess],
    ) -> Result<Option<String>, ExecutorError> {
        if paths.is_empty() {
            return Ok(None);
        }
        let owner = record
            .managed_invocation()
            .map_err(|error| refused(&error.to_string()))?
            .to_string();
        let roots = paths
            .iter()
            .map(|path| capture(Path::new(&path.path), path.write))
            .collect::<Result<Vec<_>, _>>()?;
        self.store
            .claim_serving_native_paths(
                record,
                &NativePathClaim {
                    owner: owner.clone(),
                    roots,
                },
            )
            .map_err(storage)?;
        self.revalidate(&owner)?;
        Ok(Some(owner))
    }

    pub(crate) fn revalidate(&self, owner: &str) -> Result<(), ExecutorError> {
        for root in self.roots(owner)? {
            if capture(&root.requested, root.write)? != root {
                return Err(refused(
                    "native directory or symlink target changed after its claim; entry refused",
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn revalidate_if_present(&self, owner: &str) -> Result<(), ExecutorError> {
        if self
            .store
            .native_path_claim(owner)
            .map_err(storage)?
            .is_some()
        {
            self.revalidate(owner)?;
        }
        Ok(())
    }

    pub(crate) fn verify_configured(
        &self,
        owner: &str,
        paths: &[TaskPathAccess],
    ) -> Result<(), ExecutorError> {
        let owned = self.roots(owner)?;
        self.revalidate(owner)?;
        for path in paths {
            let current = capture(Path::new(&path.path), path.write)?;
            if !owned.iter().any(|root| {
                (!current.write || root.write) && current.physical.starts_with(&root.physical)
            }) {
                return Err(refused(
                    "configured process directory was not included in the parent task's native claim",
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn roots(&self, owner: &str) -> Result<Vec<NativePathRoot>, ExecutorError> {
        self.store
            .native_path_claim(owner)
            .map_err(storage)?
            .map(|claim| claim.roots)
            .ok_or_else(|| refused("accepted native directory claim is missing; entry refused"))
    }

    pub(crate) fn release_settled(&self, owner: &str) -> Result<(), ExecutorError> {
        self.store.release_native_paths(owner).map_err(storage)
    }
}

/// Captures a native directory without creating, relocating or adopting its contents.
///
/// # Errors
/// Refuses unavailable directories and platforms without qualified native identities.
pub fn capture_native_project_root(
    requested: &Path,
    write: bool,
) -> Result<NativePathRoot, ExecutorError> {
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

fn capture(requested: &Path, write: bool) -> Result<NativePathRoot, ExecutorError> {
    capture_native_project_root(requested, write)
}

/// Refuses a missing, relocated or substituted registered folder before project execution.
///
/// # Errors
/// Returns a pre-entry refusal when the current native identity differs from acceptance.
pub fn verify_project_binding(
    binding: &milkdrift_blueprint::TaskProjectBinding,
) -> Result<(), ExecutorError> {
    let current = capture(Path::new(&binding.folder.requested), false)?;
    if current.physical != Path::new(&binding.folder.physical)
        || current.identity != binding.folder.identity
    {
        return Err(refused(
            "registered project folder changed; attach it explicitly before submitting new work",
        ));
    }
    for scope in &binding.scopes {
        let current = capture(Path::new(&scope.requested), false)?;
        if current.physical != Path::new(&scope.physical) || current.identity != scope.identity {
            return Err(refused(
                "accepted project subdirectory changed; entry refused",
            ));
        }
    }
    Ok(())
}

pub(crate) fn pin_project_paths(
    binding: &milkdrift_blueprint::TaskProjectBinding,
) -> Result<Vec<milkdrift_blueprint::TaskProjectFolder>, ExecutorError> {
    verify_project_binding(binding)?;
    if binding.paths.len() > 16 {
        return Err(refused(
            "project task scope exceeds sixteen native directories",
        ));
    }
    binding
        .paths
        .iter()
        .map(|path| {
            let root = capture(Path::new(&path.path), path.write)?;
            if !root.physical.starts_with(&binding.folder.physical) {
                return Err(refused("task path or alias escapes its registered project"));
            }
            Ok(milkdrift_blueprint::TaskProjectFolder {
                requested: root.requested.to_string_lossy().into_owned(),
                physical: root.physical.to_string_lossy().into_owned(),
                identity: root.identity,
            })
        })
        .collect()
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
    fn process_tool_preflight_cannot_switch_to_an_unclaimed_directory()
    -> Result<(), Box<dyn std::error::Error>> {
        let store_directory = tempfile::tempdir()?;
        let first = tempfile::tempdir()?;
        let second = tempfile::tempdir()?;
        let claims = NativePathClaims::new(Arc::new(RedbStore::open(store_directory.path())?));
        let selected = TaskPathAccess {
            path: first.path().to_string_lossy().into_owned(),
            write: true,
        };
        claims.acquire("parent-attempt", std::slice::from_ref(&selected))?;
        claims.verify_configured("parent-attempt", &[selected])?;
        assert!(
            claims
                .verify_configured(
                    "parent-attempt",
                    &[TaskPathAccess {
                        path: second.path().to_string_lossy().into_owned(),
                        write: true,
                    }]
                )
                .is_err()
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn project_scope_pinning_rejects_escaped_aliases_and_replaced_subdirectories()
    -> Result<(), Box<dyn std::error::Error>> {
        use milkdrift_blueprint::{
            ProjectEnvironment, ProjectSettings, TaskProjectAccess, TaskProjectBinding,
            TaskProjectFolder,
        };
        let project = tempfile::tempdir()?;
        let outside = tempfile::tempdir()?;
        let child = project.path().join("research");
        std::fs::create_dir(&child)?;
        let root = capture_native_project_root(project.path(), true)?;
        let mut binding = TaskProjectBinding {
            project_id: "project:test".into(),
            generation: 1,
            folder: TaskProjectFolder {
                requested: root.requested.to_string_lossy().into_owned(),
                physical: root.physical.to_string_lossy().into_owned(),
                identity: root.identity,
            },
            settings: ProjectSettings {
                environment: ProjectEnvironment::Host,
                allow_changes: true,
            },
            access: TaskProjectAccess::Write,
            tools: Vec::new(),
            paths: vec![TaskPathAccess {
                path: child.to_string_lossy().into_owned(),
                write: true,
            }],
            scopes: Vec::new(),
        };
        binding.scopes = pin_project_paths(&binding)?;
        verify_project_binding(&binding)?;
        std::fs::rename(&child, project.path().join("original-research"))?;
        std::fs::create_dir(&child)?;
        assert!(verify_project_binding(&binding).is_err());
        binding.scopes.clear();
        let alias = project.path().join("escaped");
        std::os::unix::fs::symlink(outside.path(), &alias)?;
        binding.paths = vec![TaskPathAccess {
            path: alias.to_string_lossy().into_owned(),
            write: true,
        }];
        assert!(pin_project_paths(&binding).is_err());
        Ok(())
    }

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

    #[cfg(unix)]
    #[test]
    fn independent_research_folders_admit_together_but_parent_capture_conflicts()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        let project = tempfile::tempdir()?;
        let left = project.path().join("research-a");
        let right = project.path().join("research-b");
        std::fs::create_dir(&left)?;
        std::fs::create_dir(&right)?;
        let host = NativePathClaims::new(Arc::new(RedbStore::open(root.path())?));
        let access = |path: &Path, write| TaskPathAccess {
            path: path.to_string_lossy().into_owned(),
            write,
        };
        host.acquire("research-a", &[access(&left, true)])?;
        host.acquire("research-b", &[access(&right, true)])?;
        assert!(
            host.acquire("capture-project", &[access(project.path(), false)])
                .is_err()
        );
        assert!(host.acquire("same-folder", &[access(&left, true)]).is_err());
        host.release_settled("research-a")?;
        assert!(
            host.acquire("capture-project", &[access(project.path(), false)])
                .is_err()
        );
        host.release_settled("research-b")?;
        host.acquire("capture-project", &[access(project.path(), false)])?;
        assert!(host.acquire("research-a", &[access(&left, true)]).is_err());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn simultaneous_capture_and_writer_claims_have_exactly_one_owner()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        let project = tempfile::tempdir()?;
        let store = Arc::new(RedbStore::open(root.path())?);
        let host = NativePathClaims::new(store.clone());
        let starts = std::sync::Barrier::new(2);
        let attempted = std::sync::Barrier::new(2);
        let outcomes = std::thread::scope(|scope| {
            let contender = |owner, write| {
                let path = TaskPathAccess {
                    path: project.path().to_string_lossy().into_owned(),
                    write,
                };
                starts.wait();
                let result = host.acquire(owner, &[path]);
                // Neither side releases a successful claim before the other has attempted entry.
                attempted.wait();
                result
            };
            let reader = scope.spawn(move || contender("capture", false));
            let writer = scope.spawn(move || contender("writer", true));
            (reader.join(), writer.join())
        });
        let reader = outcomes.0.map_err(|_| "capture claimant panicked")?;
        let writer = outcomes.1.map_err(|_| "writer claimant panicked")?;
        assert_ne!(reader.is_ok(), writer.is_ok());
        assert_eq!(
            store.native_path_claim("capture")?.is_some(),
            reader.is_ok()
        );
        assert_eq!(store.native_path_claim("writer")?.is_some(), writer.is_ok());
        Ok(())
    }
}

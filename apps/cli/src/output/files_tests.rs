//! Replacement windows are coordinated at write/publication calls, without timing assumptions.
use super::PendingFile;
use crate::error::CliError;
use std::{fs, io::Write as _};

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn competing_regular_files_and_directories_survive_every_outcome() -> TestResult {
    for directory in [false, true] {
        for outcome in ["failure", "cancel", "publish"] {
            let root = tempfile::tempdir()?;
            let path = root.path().join("result");
            let mut output = PendingFile::create(&path)?;
            output.write_all(b"verified result")?;
            assert!(!path.exists(), "partial output became visible");
            if directory {
                fs::create_dir(&path)?;
                fs::write(path.join("other.txt"), b"unrelated writer")?;
            } else {
                fs::write(&path, b"unrelated writer")?;
            }
            match outcome {
                "failure" => assert!(matches!(
                    output.finish::<()>(Err(CliError::Deadline)),
                    Err(CliError::Deadline)
                )),
                "cancel" => drop(output),
                "publish" => {
                    let result = output.commit().map_err(CliError::from);
                    assert!(result.is_err());
                    assert!(output.finish(result).is_err());
                }
                _ => return Err("unknown outcome".into()),
            }
            let unrelated = if directory {
                path.join("other.txt")
            } else {
                path.clone()
            };
            assert_eq!(fs::read(unrelated)?, b"unrelated writer");
            assert_eq!(fs::read_dir(root.path())?.count(), 1, "staging leaked");
        }
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn competing_symlinks_survive_failure_cancellation_and_publication() -> TestResult {
    use std::os::unix::fs::symlink;
    for outcome in ["failure", "cancel", "publish"] {
        let root = tempfile::tempdir()?;
        let path = root.path().join("result");
        let target = root.path().join("unrelated");
        fs::write(&target, b"do not change")?;
        let mut output = PendingFile::create(&path)?;
        output.write_all(b"verified result")?;
        symlink(&target, &path)?;
        match outcome {
            "failure" => assert!(matches!(
                output.finish::<()>(Err(CliError::Deadline)),
                Err(CliError::Deadline)
            )),
            "cancel" => drop(output),
            "publish" => {
                let result = output.commit().map_err(CliError::from);
                assert!(result.is_err());
                assert!(output.finish(result).is_err());
            }
            _ => return Err("unknown outcome".into()),
        }
        assert!(fs::symlink_metadata(&path)?.file_type().is_symlink());
        assert_eq!(fs::read(target)?, b"do not change");
        assert_eq!(fs::read_dir(root.path())?.count(), 2);
    }
    Ok(())
}

#[test]
fn publication_guarantee_ends_before_a_later_authorized_writer() -> TestResult {
    let root = tempfile::tempdir()?;
    let path = root.path().join("result");
    let mut output = PendingFile::create(&path)?;
    output.write_all(b"verified result")?;
    output.commit()?;
    assert_eq!(fs::read(&path)?, b"verified result");
    // The file was ours when publication succeeded. A subsequent writer may
    // replace it; even a later presentation error must not unlink that name.
    fs::remove_file(&path)?;
    fs::write(&path, b"later writer")?;
    assert!(matches!(
        output.finish::<()>(Err(CliError::Cancelled)),
        Err(CliError::Cancelled)
    ));
    assert_eq!(fs::read(&path)?, b"later writer");
    assert_eq!(fs::read_dir(root.path())?.count(), 1);
    Ok(())
}

#[test]
fn file_and_parent_sync_failures_distinguish_publication() -> TestResult {
    for after_publication in [false, true] {
        let root = tempfile::tempdir()?;
        let path = root.path().join("result");
        let mut output = PendingFile::create(&path)?;
        output.write_all(b"verified result")?;
        let result = output
            .commit_with(
                |file| {
                    if after_publication {
                        file.sync_all()
                    } else {
                        Err(std::io::Error::other("file sync fault"))
                    }
                },
                |_| Err(std::io::Error::other("directory sync fault")),
            )
            .map_err(CliError::from);
        let error = output
            .finish(result)
            .err()
            .ok_or("sync fault was ignored")?;
        assert_eq!(
            matches!(error, CliError::OutputPublished { .. }),
            after_publication
        );
        assert_eq!(path.exists(), after_publication);
        if after_publication {
            assert_eq!(fs::read(&path)?, b"verified result");
        }
        assert_eq!(
            fs::read_dir(root.path())?.count(),
            usize::from(after_publication)
        );
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn stable_parent_aliases_and_private_staging_permissions() -> TestResult {
    use std::os::unix::fs::{PermissionsExt as _, symlink};
    let root = tempfile::tempdir()?;
    let parent = root.path().join("parent");
    fs::create_dir(&parent)?;
    let alias = root.path().join("alias");
    symlink(&parent, &alias)?;
    let output = PendingFile::create(&alias.join("result"))?;
    assert_eq!(
        fs::metadata(output.staging.as_ref().ok_or("staging absent")?.path())?
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        output
            .file
            .as_ref()
            .ok_or("file absent")?
            .as_file()
            .metadata()?
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    output.write_complete(b"complete")?;
    assert_eq!(fs::read(parent.join("result"))?, b"complete");
    assert_eq!(fs::read(alias.join("result"))?, b"complete");
    assert!(PendingFile::create(&root.path().join("missing/result")).is_err());
    Ok(())
}

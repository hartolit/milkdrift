//! Bounded redacted task failures and private child startup logs.
use super::support::*;
use serde_json::{Value, json};
use std::{io::Write as _, path::Path};
const MAX_BUNDLE_BYTES: usize = 65536;
fn retain(root: &Path, mut document: Value) -> TestResult<std::path::PathBuf> {
    let mut bytes = serde_json::to_vec_pretty(&document)?;
    if bytes.len() > MAX_BUNDLE_BYTES {
        document
            .as_object_mut()
            .ok_or("diagnostic document is not an object")?
            .insert("collected".into(), json!({"truncated":"bundle byte limit"}));
        document
            .as_object_mut()
            .ok_or("diagnostic document is not an object")?
            .insert("last".into(), json!({"truncated":"bundle byte limit"}));
        bytes = serde_json::to_vec_pretty(&document)?;
    }
    if bytes.len() > MAX_BUNDLE_BYTES {
        return Err("diagnostic metadata exceeds byte limit".into());
    }
    fs::create_dir_all(root)?;
    let mut file = tempfile::Builder::new()
        .prefix("failure-")
        .suffix(".json")
        .tempfile_in(root)?;
    file.write_all(&bytes)?;
    let path = file.path().to_owned();
    file.persist(&path)?;
    Ok(path)
}

/// Keep bounded raw startup logs private, separate from the redacted CI bundle directory.
pub(super) fn child_failure(directory: &Path) -> String {
    use std::io::Read as _;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/control-plane-private");
    let result = (|| -> TestResult<std::path::PathBuf> {
        fs::create_dir_all(&root)?;
        let mut retained = tempfile::Builder::new()
            .prefix("startup-")
            .suffix(".log")
            .tempfile_in(&root)?;
        for name in ["daemon.stdout", "daemon.stderr"] {
            let mut bytes = vec![];
            match fs::File::open(directory.join(name))
                .and_then(|file| file.take(16_385).read_to_end(&mut bytes))
            {
                Ok(_) => {
                    writeln!(retained, "{name}; truncated={}", bytes.len() > 16_384)?;
                    bytes.truncate(16_384);
                    retained.write_all(&bytes)?;
                }
                Err(error) => writeln!(retained, "{name}; unavailable: {:?}", error.kind())?,
            }
        }
        let path = retained.path().to_owned();
        retained.persist(&path)?;
        Ok(path)
    })();
    let public_root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/control-plane-failures");
    let bundle = retain(
        &public_root,
        json!({
            "test":std::thread::current().name(),
            "source":env!("MILKDRIFT_SOURCE_REVISION"),
            "failure_class":"daemon startup failed before readiness",
            "deadline_ms":5000,
            "child_logs":"raw output withheld; bounded private retention attempted",
            "private_retention_succeeded":result.is_ok(),
            "last":"readiness unavailable; child cleanup attempted before collection"
        }),
    );
    let public = match bundle {
        Ok(path) => format!("diagnostics: {}", path.display()),
        Err(_) => "redacted startup diagnostics unavailable".into(),
    };
    match result {
        Ok(path) => format!("{public}; bounded private startup logs: {}", path.display()),
        Err(_) => format!("{public}; private startup log retention failed"),
    }
}

//! Atomically writes submitted source only against the exact observed file state.
use std::{
    fs,
    io::{Read, Write},
    path::Path,
};

fn replace(path: &Path, expected: &str, source: &str) -> Result<(), Box<dyn std::error::Error>> {
    if source.is_empty() || source.len() > 32768 {
        return Err("source must contain 1..32768 UTF-8 bytes".into());
    }
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if !metadata.is_file() || metadata.len() > 32768 {
                return Err("selected source must be a regular file within its byte bound".into());
            }
            let mut bytes = Vec::new();
            fs::File::open(path)?.take(32769).read_to_end(&mut bytes)?;
            if bytes.len() > 32768 || expected != format!("b3_{}", blake3::hash(&bytes)) {
                return Err(
                    "source differs from the selected evidence or exceeds its bound".into(),
                );
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && expected == "absent" => {}
        Err(error) => return Err(error.into()),
    }
    let parent = path.parent().ok_or("source parent absent")?;
    fs::create_dir_all(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(source.as_bytes())?;
    file.as_file().sync_all()?;
    if expected == "absent" {
        file.persist_noclobber(path)?;
    } else {
        // The managed worker holds the installation's exclusive editing claim.
        file.persist(path)?;
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 2 {
        return Err("expected observed source digest (or absent) and complete new source".into());
    }
    replace(
        Path::new("/workspace/source/slotbook.rs"),
        &args[0],
        &args[1],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_source_bytes_survive_and_refusals_preserve_the_existing_file()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        let path = root.path().join("source/slotbook.rs");
        let source = "fn main() { println!(\"$1 `literal`\\n é\"); }\n";
        replace(&path, "absent", source)?;
        assert_eq!(fs::read_to_string(&path)?, source);
        let digest = format!("b3_{}", blake3::hash(source.as_bytes()));
        for (expected, value) in [
            ("absent", "changed"),
            ("stale", "changed"),
            (&digest, ""),
            (&digest, &"é".repeat(16385)),
        ] {
            assert!(replace(&path, expected, value).is_err());
            assert_eq!(fs::read_to_string(&path)?, source);
        }
        replace(&path, &digest, "fn main() {}\n")?;
        assert_eq!(fs::read_to_string(&path)?, "fn main() {}\n");
        fs::remove_file(&path)?;
        assert!(replace(&path, &digest, source).is_err());
        assert!(!path.exists());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlinks_and_special_files_without_replacing_them()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        let original = root.path().join("original.rs");
        fs::write(&original, "original")?;
        let path = root.path().join("source.rs");
        std::os::unix::fs::symlink(&original, &path)?;
        let digest = format!("b3_{}", blake3::hash(b"original"));
        assert!(replace(&path, &digest, "changed").is_err());
        assert!(fs::symlink_metadata(&path)?.is_symlink());
        assert_eq!(fs::read_to_string(&original)?, "original");
        fs::remove_file(&path)?;
        let _socket = std::os::unix::net::UnixListener::bind(&path)?;
        assert!(replace(&path, "absent", "changed").is_err());
        assert!(!fs::symlink_metadata(&path)?.is_file());
        Ok(())
    }
}

//! Embed source provenance for offline backup manifests; the executable hash pins
//! the actual producer even when the checkout contains uncommitted changes.
use std::{path::Path, process::Command};

fn main() {
    let root = Path::new("../..");
    for path in [
        "Cargo.toml",
        "Cargo.lock",
        ".git/HEAD",
        ".git/index",
        ".git/refs",
    ] {
        println!("cargo:rerun-if-changed={}", root.join(path).display());
    }
    for area in ["crates", "adapters", "apps"] {
        if let Ok(entries) = std::fs::read_dir(root.join(area)) {
            for entry in entries.flatten() {
                for component in ["Cargo.toml", "src"] {
                    println!(
                        "cargo:rerun-if-changed={}",
                        entry.path().join(component).display()
                    );
                }
            }
        }
    }
    let output = Command::new("git")
        .current_dir(root)
        .args(["rev-parse", "HEAD"])
        .output();
    let revision = output
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_owned());
    let status = Command::new("git")
        .current_dir(root)
        .args(["status", "--porcelain", "--untracked-files=all"])
        .output();
    let dirty = status
        .ok()
        .filter(|o| o.status.success())
        .map(|o| !o.stdout.is_empty());
    let source = match (revision, dirty) {
        (Some(revision), Some(dirty)) => format!(
            "{revision}; {}",
            if dirty {
                "dirty checkout; binary digest identifies exact build"
            } else {
                "clean checkout"
            }
        ),
        _ => "unavailable; source tree has no readable Git provenance".into(),
    };
    println!("cargo:rustc-env=MILKDRIFT_SOURCE_REVISION={source}");
}

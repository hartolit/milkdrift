use milkdrift_evidence::{EvidenceResult, application::ensure};
use std::{fmt::Write as _, fs, io::Read as _, path::Path, time::Duration};

pub(super) fn run(path: &Path, delay_ms: u64) -> EvidenceResult {
    ensure(delay_ms <= 10_000, "report delay exceeds fixture bound")?;
    let file = fs::File::open(path)?;
    let mut bytes = Vec::new();
    file.take(1_048_577).read_to_end(&mut bytes)?;
    ensure(bytes.len() <= 1_048_576, "manifest exceeds 1 MiB")?;
    let text = std::str::from_utf8(&bytes)?;
    let manifest: toml::Value = toml::from_str(text)?;
    let result = describe(&manifest, &bytes)?;
    std::thread::sleep(Duration::from_millis(delay_ms));
    fs::write("report.txt", result.as_bytes())?;
    Ok(())
}

fn describe(manifest: &toml::Value, bytes: &[u8]) -> EvidenceResult<String> {
    let mut output = String::from("Cargo project analysis\n======================\n");
    writeln!(output, "Captured manifest bytes: {}", bytes.len())?;
    writeln!(output, "Captured digest: b3_{}", blake3::hash(bytes))?;
    if let Some(package) = manifest.get("package").and_then(toml::Value::as_table) {
        writeln!(
            output,
            "Package: {}",
            package
                .get("name")
                .and_then(toml::Value::as_str)
                .unwrap_or("[inherited or absent]")
        )?;
        for field in ["version", "edition", "rust-version", "description"] {
            if let Some(value) = package.get(field) {
                writeln!(output, "{field}: {value}")?;
            }
        }
    }
    if let Some(workspace) = manifest.get("workspace").and_then(toml::Value::as_table) {
        if let Some(members) = workspace.get("members").and_then(toml::Value::as_array) {
            writeln!(output, "Workspace members: {}", members.len())?;
            for member in members {
                writeln!(output, "  Member: {member}")?;
            }
        }
        dependencies(
            &mut output,
            workspace.get("dependencies"),
            "Workspace dependency",
        )?;
    }
    for (key, label) in [
        ("dependencies", "Dependency"),
        ("dev-dependencies", "Development dependency"),
        ("build-dependencies", "Build dependency"),
    ] {
        dependencies(&mut output, manifest.get(key), label)?;
    }
    if let Some(targets) = manifest.get("target").and_then(toml::Value::as_table) {
        for (target, table) in targets {
            writeln!(output, "Platform selector: {target}")?;
            for key in ["dependencies", "dev-dependencies", "build-dependencies"] {
                dependencies(&mut output, table.get(key), "Platform dependency")?;
            }
        }
    }
    let mut explicit_targets = 0usize;
    for kind in ["lib", "bin", "example", "test", "bench"] {
        if let Some(value) = manifest.get(kind) {
            if let Some(entries) = value.as_array() {
                for entry in entries {
                    writeln!(output, "Declared {kind} target: {entry}")?;
                    explicit_targets += 1;
                }
            } else {
                writeln!(output, "Declared {kind} target: {value}")?;
                explicit_targets += 1;
            }
        }
    }
    writeln!(output, "Explicit target declarations: {explicit_targets}")?;
    if let Some(features) = manifest.get("features").and_then(toml::Value::as_table) {
        for (name, value) in features {
            writeln!(output, "Feature {name}: {value}")?;
        }
    }
    writeln!(
        output,
        "\nScope: this report parses the supplied manifest bytes. It does not resolve a lockfile, access the network, compile code, or infer targets from files that were not supplied."
    )?;
    ensure(
        output.len() <= 65_536,
        "computed manifest report exceeds output bound",
    )?;
    Ok(output)
}

fn dependencies(output: &mut String, value: Option<&toml::Value>, label: &str) -> EvidenceResult {
    if let Some(values) = value.and_then(toml::Value::as_table) {
        for (name, configuration) in values {
            writeln!(output, "{label}: {name} = {configuration}")?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_follows_supplied_manifest_instead_of_returning_fixed_content() -> EvidenceResult {
        let first = "[package]\nname='train-routes'\nversion='0.1.0'\n[dependencies]\nserde='1'\n[[bin]]\nname='routes'\npath='src/main.rs'\n";
        let second = "[package]\nname='learned-animation'\nversion='0.2.0'\n[dependencies]\nndarray='0.16'\n";
        let a = describe(&toml::from_str(first)?, first.as_bytes())?;
        let b = describe(&toml::from_str(second)?, second.as_bytes())?;
        assert!(a.contains("Package: train-routes"));
        assert!(a.contains("Dependency: serde"));
        assert!(a.contains("Explicit target declarations: 1"));
        assert!(b.contains("Package: learned-animation"));
        assert!(b.contains("Dependency: ndarray"));
        assert!(!b.contains("Dependency: serde"));
        assert_ne!(a, b);
        Ok(())
    }
}

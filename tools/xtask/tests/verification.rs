//! Integration coverage for metadata-owned canonical Cargo command planning.

use std::error::Error;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use cargo_metadata::{Metadata, MetadataCommand};
use xtask::{
    CargoCommand, cuda_clippy_command_plan, cuda_clippy_command_plan_for_metadata,
    cuda_compile_command_plan, cuda_compile_command_plan_for_metadata, cuda_hardware_command_plan,
    cuda_hardware_command_plan_for_metadata, portable_command_plan,
    portable_command_plan_for_metadata,
};

static NEXT_FIXTURE_ID: AtomicU64 = AtomicU64::new(0);

struct FixtureWorkspace {
    root: PathBuf,
}

impl FixtureWorkspace {
    fn new(name: &str) -> Result<Self, Box<dyn Error>> {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name);
        let id = NEXT_FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "xtask-verification-{name}-{}-{id}",
            std::process::id()
        ));
        copy_fixture(&source, &root)?;
        Ok(Self { root })
    }

    fn manifest(&self) -> PathBuf {
        self.root.join("Cargo.toml")
    }

    fn read(&self, relative: &str) -> Result<String, Box<dyn Error>> {
        Ok(fs::read_to_string(self.root.join(relative))?)
    }

    fn write(&self, relative: &str, content: &str) -> Result<(), Box<dyn Error>> {
        let path = self.root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, content)?;
        Ok(())
    }

    fn replace(&self, relative: &str, old: &str, new: &str) -> Result<(), Box<dyn Error>> {
        let content = self.read(relative)?;
        if !content.contains(old) {
            return Err(
                format!("fixture {relative} did not contain replacement text `{old}`").into(),
            );
        }
        self.write(relative, &content.replacen(old, new, 1))
    }

    fn append(&self, relative: &str, addition: &str) -> Result<(), Box<dyn Error>> {
        let mut content = self.read(relative)?;
        content.push_str(addition);
        self.write(relative, &content)
    }

    fn refresh_lock(&self) -> Result<(), Box<dyn Error>> {
        let cargo = std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
        let output = Command::new(cargo)
            .args(["generate-lockfile", "--offline", "--manifest-path"])
            .arg(self.manifest())
            .output()?;
        if output.status.success() {
            Ok(())
        } else {
            Err(format!(
                "could not generate fixture lockfile: {}",
                String::from_utf8_lossy(&output.stderr)
            )
            .into())
        }
    }

    fn metadata(&self) -> Result<Metadata, Box<dyn Error>> {
        let mut command = MetadataCommand::new();
        command
            .manifest_path(self.manifest())
            .no_deps()
            .other_options(vec!["--locked".to_owned()]);
        if let Some(cargo) = std::env::var_os("CARGO") {
            command.cargo_path(cargo);
        }
        Ok(command.exec()?)
    }
}

impl Drop for FixtureWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn copy_fixture(source: &Path, destination: &Path) -> Result<(), Box<dyn Error>> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let destination_path = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_fixture(&entry.path(), &destination_path)?;
        } else {
            fs::copy(entry.path(), destination_path)?;
        }
    }
    Ok(())
}

fn strings(arguments: &[&str]) -> Vec<String> {
    arguments
        .iter()
        .map(|argument| (*argument).to_owned())
        .collect()
}

fn command_arguments(commands: &[CargoCommand]) -> Vec<Vec<String>> {
    commands
        .iter()
        .map(|command| command.arguments().to_vec())
        .collect()
}

fn selected_package(command: &CargoCommand) -> Option<&str> {
    command
        .arguments()
        .windows(2)
        .find(|pair| pair.first().is_some_and(|argument| argument == "-p"))
        .and_then(|pair| pair.get(1))
        .map(String::as_str)
}

fn is_hardware_target_command(command: &CargoCommand) -> bool {
    command.arguments().windows(2).any(|pair| {
        pair.first().is_some_and(|argument| argument == "--test")
            && pair
                .get(1)
                .is_some_and(|argument| argument == "cuda_hardware")
    })
}

fn assert_all_cuda_plans_reject(metadata: &Metadata) -> Result<(), Box<dyn Error>> {
    let results = [
        cuda_compile_command_plan_for_metadata(metadata),
        cuda_clippy_command_plan_for_metadata(metadata),
        cuda_hardware_command_plan_for_metadata(metadata),
    ];
    for result in results {
        let Err(error) = result else {
            return Err("invalid CUDA hardware declaration entered a command plan".into());
        };
        assert!(
            error
                .to_string()
                .contains("could not derive exact CUDA hardware ownership")
        );
    }
    Ok(())
}

#[test]
fn help_exposes_metadata_owned_command_surface() -> Result<(), Box<dyn Error>> {
    let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .arg("--help")
        .output()?;
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains("Milkdrift workspace tooling"));
    assert!(stdout.contains("portable <wasm32-unknown-unknown|thumbv7em-none-eabihf>"));
    assert!(stdout.contains("cuda-compile"));
    assert!(stdout.contains("cuda-clippy"));
    assert!(stdout.contains("cuda-hardware"));
    assert!(!stdout.contains("llm-app"));
    Ok(())
}

#[test]
fn portable_plan_automatically_owns_a_new_valid_domain_package() -> Result<(), Box<dyn Error>> {
    let fixture = FixtureWorkspace::new("scalable-policy")?;
    fixture.replace(
        "Cargo.toml",
        "    \"crates/domain/f1-b\",\n",
        "    \"crates/domain/f1-b\",\n    \"crates/domain/f1-c\",\n",
    )?;
    fixture.write(
        "crates/domain/f1-c/Cargo.toml",
        "[package]\nname = \"f1-c\"\nversion = \"0.1.0\"\nedition = \"2024\"\npublish = false\n\n[package.metadata.milkdrift]\nrole = \"domain-feature\"\n",
    )?;
    fixture.write("crates/domain/f1-c/src/lib.rs", "pub fn fixture() {}\n")?;
    fixture.refresh_lock()?;

    let metadata = fixture.metadata()?;
    let pure = portable_command_plan_for_metadata(&metadata, "wasm32-unknown-unknown")?;
    let loaded = portable_command_plan(&fixture.manifest(), "wasm32-unknown-unknown")?;
    assert_eq!(pure, loaded);
    assert_eq!(
        command_arguments(&pure),
        vec![strings(&[
            "check",
            "--locked",
            "--target",
            "wasm32-unknown-unknown",
            "--lib",
            "-p",
            "f0",
            "-p",
            "f1-a",
            "-p",
            "f1-b",
            "-p",
            "f1-c",
        ])]
    );
    Ok(())
}

#[test]
fn portable_plan_rejects_invalid_roles_empty_ownership_and_unknown_targets()
-> Result<(), Box<dyn Error>> {
    let invalid_role = FixtureWorkspace::new("scalable-policy")?;
    invalid_role.replace(
        "crates/domain/f0/Cargo.toml",
        "role = \"domain-foundation\"",
        "role = \"mystery-layer\"",
    )?;
    invalid_role.refresh_lock()?;
    let invalid_metadata = invalid_role.metadata()?;
    let Err(error) = portable_command_plan_for_metadata(&invalid_metadata, "thumbv7em-none-eabihf")
    else {
        return Err("invalid role entered the portable command plan".into());
    };
    assert!(error.to_string().contains("unknown role `mystery-layer`"));

    let no_domain = FixtureWorkspace::new("cuda-policy")?;
    no_domain.refresh_lock()?;
    let no_domain_metadata = no_domain.metadata()?;
    let Err(error) =
        portable_command_plan_for_metadata(&no_domain_metadata, "wasm32-unknown-unknown")
    else {
        return Err("empty domain ownership entered the portable command plan".into());
    };
    assert!(error.to_string().contains("at least one workspace package"));

    let Err(error) =
        portable_command_plan_for_metadata(&no_domain_metadata, "x86_64-unknown-linux-gnu")
    else {
        return Err("unsupported target entered the portable command plan".into());
    };
    assert!(error.to_string().contains("unsupported portable target"));
    Ok(())
}

#[test]
fn cuda_plans_are_sorted_exact_and_keep_hardware_targets_separate() -> Result<(), Box<dyn Error>> {
    let fixture = FixtureWorkspace::new("cuda-policy")?;
    fixture.refresh_lock()?;
    let metadata = fixture.metadata()?;

    let compile = cuda_compile_command_plan_for_metadata(&metadata)?;
    assert_eq!(
        command_arguments(&compile),
        vec![
            strings(&[
                "check",
                "--locked",
                "-p",
                "application-runtime",
                "-p",
                "candle-backend",
                "-p",
                "desktop-slint",
                "-p",
                "inference-runtime",
                "-p",
                "runtime-benchmarks",
                "--all-targets",
                "--features",
                "cuda",
            ]),
            strings(&[
                "test",
                "--locked",
                "-p",
                "application-runtime",
                "-p",
                "candle-backend",
                "-p",
                "desktop-slint",
                "-p",
                "inference-runtime",
                "-p",
                "runtime-benchmarks",
                "--features",
                "cuda",
                "--no-run",
            ]),
            strings(&[
                "test",
                "--locked",
                "-p",
                "application-runtime",
                "--features",
                "cuda-hardware-tests",
                "--test",
                "cuda_hardware",
                "--no-run",
            ]),
            strings(&[
                "test",
                "--locked",
                "-p",
                "candle-backend",
                "--features",
                "cuda-hardware-tests",
                "--test",
                "cuda_hardware",
                "--no-run",
            ]),
            strings(&[
                "test",
                "--locked",
                "-p",
                "inference-runtime",
                "--features",
                "cuda-hardware-tests",
                "--test",
                "cuda_hardware",
                "--no-run",
            ]),
        ]
    );
    assert_eq!(compile, cuda_compile_command_plan(&fixture.manifest())?);

    let clippy = cuda_clippy_command_plan_for_metadata(&metadata)?;
    assert_eq!(
        command_arguments(&clippy),
        vec![
            strings(&[
                "clippy",
                "--locked",
                "-p",
                "application-runtime",
                "-p",
                "candle-backend",
                "-p",
                "desktop-slint",
                "-p",
                "inference-runtime",
                "-p",
                "runtime-benchmarks",
                "--all-targets",
                "--features",
                "cuda",
                "--",
                "-D",
                "warnings",
            ]),
            strings(&[
                "clippy",
                "--locked",
                "-p",
                "application-runtime",
                "--features",
                "cuda-hardware-tests",
                "--test",
                "cuda_hardware",
                "--",
                "-D",
                "warnings",
            ]),
            strings(&[
                "clippy",
                "--locked",
                "-p",
                "candle-backend",
                "--features",
                "cuda-hardware-tests",
                "--test",
                "cuda_hardware",
                "--",
                "-D",
                "warnings",
            ]),
            strings(&[
                "clippy",
                "--locked",
                "-p",
                "inference-runtime",
                "--features",
                "cuda-hardware-tests",
                "--test",
                "cuda_hardware",
                "--",
                "-D",
                "warnings",
            ]),
        ]
    );
    assert_eq!(clippy, cuda_clippy_command_plan(&fixture.manifest())?);

    let hardware = cuda_hardware_command_plan_for_metadata(&metadata)?;
    assert_eq!(
        command_arguments(&hardware),
        vec![
            strings(&[
                "test",
                "--release",
                "--locked",
                "-p",
                "application-runtime",
                "--features",
                "cuda-hardware-tests",
                "--test",
                "cuda_hardware",
            ]),
            strings(&[
                "test",
                "--release",
                "--locked",
                "-p",
                "candle-backend",
                "--features",
                "cuda-hardware-tests",
                "--test",
                "cuda_hardware",
            ]),
            strings(&[
                "test",
                "--release",
                "--locked",
                "-p",
                "inference-runtime",
                "--features",
                "cuda-hardware-tests",
                "--test",
                "cuda_hardware",
            ]),
        ]
    );
    assert_eq!(hardware, cuda_hardware_command_plan(&fixture.manifest())?);
    assert!(hardware.iter().all(|command| {
        is_hardware_target_command(command)
            && command.arguments().iter().all(|argument| argument != "--")
    }));
    Ok(())
}

#[test]
fn adding_a_valid_hardware_suite_automatically_enters_every_hardware_plan()
-> Result<(), Box<dyn Error>> {
    let fixture = FixtureWorkspace::new("cuda-policy")?;
    fixture.replace(
        "crates/apps/desktop-slint/Cargo.toml",
        "cuda = [\"application-runtime/cuda\"]",
        "cuda = [\"application-runtime/cuda\"]\ncuda-hardware-tests = [\"cuda\"]",
    )?;
    fixture.append(
        "crates/apps/desktop-slint/Cargo.toml",
        "\n[[test]]\nname = \"cuda_hardware\"\npath = \"tests/cuda_hardware.rs\"\nharness = false\nrequired-features = [\"cuda-hardware-tests\"]\n",
    )?;
    fixture.write(
        "crates/apps/desktop-slint/tests/cuda_hardware.rs",
        "fn main() {}\n",
    )?;
    fixture.refresh_lock()?;
    let metadata = fixture.metadata()?;

    let compile = cuda_compile_command_plan_for_metadata(&metadata)?;
    let clippy = cuda_clippy_command_plan_for_metadata(&metadata)?;
    let hardware = cuda_hardware_command_plan_for_metadata(&metadata)?;
    for plan in [&compile, &clippy, &hardware] {
        let packages = plan
            .iter()
            .filter(|command| is_hardware_target_command(command))
            .filter_map(selected_package)
            .collect::<Vec<_>>();
        assert_eq!(
            packages,
            vec![
                "application-runtime",
                "candle-backend",
                "desktop-slint",
                "inference-runtime",
            ]
        );
    }
    Ok(())
}

#[test]
fn malformed_hardware_target_declarations_fail_all_plans_closed() -> Result<(), Box<dyn Error>> {
    let mutations = [
        ("harness = false", "harness = true"),
        (
            "required-features = [\"cuda-hardware-tests\"]",
            "required-features = [\"cuda\"]",
        ),
        (
            "path = \"tests/cuda_hardware.rs\"",
            "path = \"tests/not_cuda_hardware.rs\"",
        ),
    ];

    for (old, new) in mutations {
        let fixture = FixtureWorkspace::new("cuda-policy")?;
        fixture.replace("crates/runtime/application-runtime/Cargo.toml", old, new)?;
        fixture.write(
            "crates/runtime/application-runtime/tests/not_cuda_hardware.rs",
            "fn main() {}\n",
        )?;
        fixture.refresh_lock()?;
        assert_all_cuda_plans_reject(&fixture.metadata()?)?;
    }
    Ok(())
}

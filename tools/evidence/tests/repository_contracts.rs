//! Repository-level documentation, dependency-direction, and public-boundary contracts.

use std::{
    collections::BTreeSet,
    error::Error,
    fs,
    path::{Path, PathBuf},
};

use milkdrift_authority::Selection;
use milkdrift_capability::OperationId;
use milkdrift_local_process::ProcessProfileDocument;

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

fn root() -> TestResult<PathBuf> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| {
            std::io::Error::other("evidence package must remain under tools/evidence")
        })?;
    Ok(path.to_path_buf())
}

fn read(path: impl AsRef<Path>) -> TestResult<String> {
    Ok(fs::read_to_string(path)?)
}

fn numeric_const(relative: &str, name: &str) -> TestResult<u64> {
    let source = read(root()?.join(relative))?;
    let marker = format!("const {name}:");
    let line = source
        .lines()
        .find(|line| line.contains(&marker))
        .ok_or_else(|| std::io::Error::other(format!("{relative} has no {name}")))?;
    let literal = line
        .split_once('=')
        .ok_or_else(|| std::io::Error::other("constant must have an initializer"))?
        .1
        .trim()
        .trim_end_matches(';');
    Ok(literal.parse()?)
}

#[test]
fn canonical_entrypoint_and_local_links_resolve() -> TestResult {
    let repository = root()?;
    let canonical = [
        "AGENTS.md",
        "VISION.md",
        "ARCHITECTURE.md",
        "README.md",
        "docs/STATUS.md",
        "docs/ROADMAP.md",
        "docs/DEVELOPMENT.md",
        "docs/reference/public-api-policy.md",
    ];
    for relative in canonical {
        let document_path = repository.join(relative);
        let document = read(&document_path)?;
        for line in document.lines() {
            let mut remainder = line;
            while let Some((_, after_label)) = remainder.split_once("](") {
                let Some((raw_target, after_target)) = after_label.split_once(')') else {
                    break;
                };
                remainder = after_target;
                let target = raw_target.trim().trim_matches(['<', '>']);
                if target.is_empty()
                    || target.starts_with('#')
                    || target.starts_with("http://")
                    || target.starts_with("https://")
                    || target.starts_with("mailto:")
                {
                    continue;
                }
                let path = target.split('#').next().unwrap_or_default();
                let resolved = document_path
                    .parent()
                    .unwrap_or(repository.as_path())
                    .join(path);
                assert!(
                    resolved.exists(),
                    "broken local link in {relative}: {target}"
                );
            }
        }
    }

    let agents = read(repository.join("AGENTS.md"))?;
    let ordered = [
        "1. `AGENTS.md`",
        "2. `VISION.md`",
        "3. `ARCHITECTURE.md`",
        "4. `docs/STATUS.md`",
        "5. `docs/ROADMAP.md`",
        "6. Relevant ADRs, references, source, and tests",
    ];
    let mut prior = 0;
    for item in ordered {
        let position = agents
            .find(item)
            .ok_or_else(|| std::io::Error::other(format!("missing {item}")))?;
        assert!(position >= prior, "reading order changed at {item}");
        prior = position;
    }
    Ok(())
}

#[test]
fn canonical_version_statements_match_source_constants() -> TestResult {
    let control_major = numeric_const("crates/control-protocol/src/lib.rs", "PROTOCOL_MAJOR")?;
    let control_minor = numeric_const("crates/control-protocol/src/lib.rs", "PROTOCOL_MINOR")?;
    let peer_major = numeric_const("crates/peer-protocol/src/session.rs", "PROTOCOL_MAJOR_V1")?;
    let peer_minor = numeric_const("crates/peer-protocol/src/session.rs", "PROTOCOL_MINOR_V1")?;
    let daemon = numeric_const("apps/daemon/src/config.rs", "DAEMON_CONFIG_SCHEMA_VERSION")?;
    let storage = numeric_const(
        "adapters/redb-store/src/schema.rs",
        "STORAGE_SCHEMA_VERSION",
    )?;
    let format = numeric_const(
        "adapters/redb-store/src/schema.rs",
        "INTERNAL_DOCUMENT_FORMAT_VERSION",
    )?;
    let grant = numeric_const(
        "crates/authority/src/document.rs",
        "AUTHORITY_GRANT_SCHEMA_VERSION_V4",
    )?;
    let prompt = numeric_const(
        "crates/prompt-sequence/src/document.rs",
        "PROMPT_SEQUENCE_SCHEMA_VERSION_V2",
    )?;
    let peer_execution = numeric_const(
        "crates/persistence/src/peer.rs",
        "PEER_EXECUTION_RECORD_SCHEMA_VERSION_V3",
    )?;
    let run_event = numeric_const(
        "crates/persistence/src/document.rs",
        "RUN_EVENT_SCHEMA_VERSION_V2",
    )?;
    let resolved_snapshot = numeric_const(
        "crates/capability/src/document.rs",
        "RESOLVED_CAPABILITY_SNAPSHOT_SCHEMA_VERSION_V2",
    )?;

    let facts = [
        format!("protocol {control_major}.{control_minor}"),
        format!("protocol {peer_major}.{peer_minor}"),
        format!("configuration is v{daemon}"),
        format!("physical schema {storage}"),
        format!("internal document format {format}"),
        format!("authority grants are v{grant}"),
        format!("prompt-sequence imports are currently v{prompt}"),
        format!("durable hot peer-execution records are v{peer_execution}"),
        format!("run-event envelopes are v{run_event}"),
        format!("resolved-capability snapshots are v{resolved_snapshot}"),
    ];
    let architecture = read(root()?.join("ARCHITECTURE.md"))?;
    for fact in facts {
        assert!(
            architecture.contains(&fact),
            "architecture is missing {fact}"
        );
    }

    let control = format!("protocol-{control_major}.{control_minor}");
    assert!(read(root()?.join("README.md"))?.contains(&control));
    assert!(
        read(root()?.join("docs/STATUS.md"))?
            .contains("External control / authenticated cursor | 2.2 / 2")
    );
    Ok(())
}

#[test]
fn status_is_current_fact_not_a_pass_diary() -> TestResult {
    let status = read(root()?.join("docs/STATUS.md"))?;
    let headings: Vec<_> = status
        .lines()
        .filter(|line| line.starts_with("## "))
        .collect();
    assert_eq!(
        headings,
        [
            "## Implemented now",
            "## Limitations now",
            "## Current validation/evidence snapshot"
        ]
    );
    assert!(!status.contains("On 2026-"));
    assert!(!status.to_ascii_lowercase().contains("pass 0"));
    Ok(())
}

#[test]
fn repository_evidence_tasks_have_one_cargo_owned_rust_path() -> TestResult {
    let repository = root()?;
    let aliases = read(repository.join(".cargo/config.toml"))?;
    assert!(aliases.contains(
        "external-evidence = \"run --package milkdrift-daemon --bin milkdrift-external-evidence --\""
    ));
    assert!(aliases.contains(
        "mutation-evidence = \"run --package milkdrift-evidence --bin mutation-evidence --\""
    ));
    assert!(
        repository
            .join("tools/evidence/src/bin/mutation-evidence.rs")
            .is_file()
    );
    assert!(
        repository
            .join("apps/daemon/src/bin/milkdrift-external-evidence/main.rs")
            .is_file()
    );
    for obsolete in [
        "scripts/check-mutation-classifications.mjs",
        "scripts/run-mutation-shard.sh",
        "scripts/run-external-evidence.sh",
    ] {
        assert!(
            !repository.join(obsolete).is_file(),
            "obsolete repository task path returned: {obsolete}"
        );
    }

    let workflow = read(repository.join(".github/workflows/mutation.yml"))?;
    assert!(workflow.contains("cargo mutation-evidence \"${{ matrix.shard }}\""));
    for document in [
        "README.md",
        "docs/DEVELOPMENT.md",
        "docs/external-evidence.md",
        "docs/verification-evidence.md",
    ] {
        let contents = read(repository.join(document))?;
        assert!(
            !contents.contains("scripts/"),
            "obsolete script path remains documented in {document}"
        );
    }
    Ok(())
}

#[test]
fn semantic_and_protocol_packages_have_no_ui_inference_or_internal_adapter_edges() -> TestResult {
    let repository = root()?;
    let mut manifests = Vec::new();
    collect_files(&repository, &mut manifests, &|path| {
        path.file_name().and_then(|name| name.to_str()) == Some("Cargo.toml")
    })?;
    let forbidden_product_dependencies = [
        "iced", "egui", "tauri", "slint", "dioxus", "candle", "burn", "llama", "tch", "onnx", "ort",
    ];
    for manifest in &manifests {
        let contents = read(manifest)?.to_ascii_lowercase();
        for dependency in forbidden_product_dependencies {
            let declaration = format!("{dependency} =");
            assert!(
                !contents
                    .lines()
                    .any(|line| line.trim_start().starts_with(&declaration)),
                "forbidden UI/inference dependency {dependency} in {}",
                manifest.display()
            );
        }
    }

    for (relative, forbidden) in [
        (
            "crates/control-protocol/Cargo.toml",
            [
                "milkdrift-runtime",
                "milkdrift-redb-store",
                "axum",
                "tokio",
                "reqwest",
            ],
        ),
        (
            "crates/peer-protocol/Cargo.toml",
            [
                "milkdrift-runtime",
                "milkdrift-redb-store",
                "axum",
                "tokio",
                "reqwest",
            ],
        ),
        (
            "crates/model/Cargo.toml",
            [
                "milkdrift-runtime",
                "milkdrift-redb-store",
                "milkdrift-model-provider",
                "axum",
                "reqwest",
            ],
        ),
    ] {
        let manifest = read(repository.join(relative))?;
        for dependency in forbidden {
            assert!(
                !manifest.contains(dependency),
                "{relative} depends on internal/adapter package {dependency}"
            );
        }
    }
    Ok(())
}

#[test]
fn workspace_owns_internal_package_versions_and_paths() -> TestResult {
    let repository = root()?;
    let workspace = read(repository.join("Cargo.toml"))?;
    assert!(workspace.contains("[workspace.package]\nversion = \"0.1.0\""));

    let mut manifests = Vec::new();
    collect_files(&repository, &mut manifests, &|path| {
        path.file_name().and_then(|name| name.to_str()) == Some("Cargo.toml")
    })?;
    for manifest_path in manifests {
        if manifest_path == repository.join("Cargo.toml") {
            continue;
        }
        let manifest = read(&manifest_path)?;
        assert!(
            manifest
                .lines()
                .any(|line| line == "version.workspace = true"),
            "package version is not workspace-owned in {}",
            manifest_path.display()
        );
        for line in manifest.lines().map(str::trim) {
            if !line.starts_with("milkdrift-") || !line.contains('=') {
                continue;
            }
            assert!(
                !line.contains("path =") && !line.contains("version ="),
                "internal dependency repeats its path/version in {}: {line}",
                manifest_path.display()
            );
            assert!(
                line.contains(".workspace = true") || line.contains("{ workspace = true"),
                "internal dependency is not workspace-owned in {}: {line}",
                manifest_path.display()
            );
        }
    }
    Ok(())
}

#[test]
fn narrowed_exports_and_validating_constructors_remain_narrow() -> TestResult {
    let repository = root()?;
    let persistence = read(repository.join("crates/persistence/src/lib.rs"))?;
    for compatibility_export in [
        "pub use milkdrift_authority::ActorRef",
        "pub use milkdrift_capability::InvocationId",
        "pub use milkdrift_workspace::",
    ] {
        assert!(!persistence.contains(compatibility_export));
    }

    let daemon = read(repository.join("apps/daemon/src/lib.rs"))?;
    assert!(!daemon.contains("pub use http::{router"));
    assert!(!daemon.contains("pub use http::router"));

    let redb = read(repository.join("adapters/redb-store/src/lib.rs"))?;
    assert!(redb.contains(
        "#[cfg(feature = \"test-admin\")]\npub use fault::{FaultInjector, FaultPoint, injected_failure};"
    ));

    let runtime_executor = read(repository.join("crates/runtime/src/executor.rs"))?;
    assert!(runtime_executor.contains("fn execute_streaming("));
    assert!(!runtime_executor.contains("ExecutionReportBatch"));
    assert!(!runtime_executor.contains("bounded synchronous executors"));

    assert!(Selection::<OperationId>::only(BTreeSet::new()).is_err());
    Ok(())
}

#[test]
fn public_reexports_are_explicit_and_reviewable() -> TestResult {
    let repository = root()?;
    let public_use = ["pub", "use"].concat();
    let wildcard = [":", ":", "*"].concat();
    let mut sources = Vec::new();
    collect_files(&repository, &mut sources, &|path| {
        path.extension().and_then(|extension| extension.to_str()) == Some("rs")
    })?;
    for source in sources {
        let contents = read(&source)?;
        let compact: String = contents
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect();
        for declaration in compact.split(';') {
            assert!(
                !(declaration.contains(&public_use) && declaration.contains(&wildcard)),
                "wildcard public re-export in {}",
                source.display()
            );
        }
    }
    Ok(())
}

#[test]
fn rust_modules_use_named_file_children_and_respect_the_size_backstop() -> TestResult {
    const MAXIMUM_SOURCE_LINES: usize = 2_000;

    let repository = root()?;
    let mut sources = Vec::new();
    collect_files(&repository, &mut sources, &|path| {
        path.extension().and_then(|extension| extension.to_str()) == Some("rs")
    })?;
    for source in sources {
        assert_ne!(
            source.file_name().and_then(|name| name.to_str()),
            Some("mod.rs"),
            "module ownership must use file.rs with file/ children: {}",
            source.display()
        );
        let lines = read(&source)?.lines().count();
        assert!(
            lines < MAXIMUM_SOURCE_LINES,
            "{} has {lines} lines; perform the required cohesion review before crossing the {MAXIMUM_SOURCE_LINES}-line backstop",
            source.display()
        );
    }
    Ok(())
}

#[test]
fn exact_current_process_fixture_uses_the_public_reader() -> TestResult {
    let fixtures = root()?.join("adapters/local-process/tests/fixtures");
    let current = fs::read(fixtures.join("process-profile-v2.json"))?;
    ProcessProfileDocument::from_json(&current)?;
    let legacy = fs::read(fixtures.join("process-profile-v1.json"))?;
    assert!(ProcessProfileDocument::from_json(&legacy).is_err());
    Ok(())
}

fn collect_files(
    directory: &Path,
    files: &mut Vec<PathBuf>,
    selects: &impl Fn(&Path) -> bool,
) -> std::io::Result<()> {
    for entry in fs::read_dir(directory)? {
        let path = entry?.path();
        if path.is_dir() {
            if !matches!(
                path.file_name().and_then(|name| name.to_str()),
                Some("target" | ".git")
            ) {
                collect_files(&path, files, selects)?;
            }
        } else if selects(&path) {
            files.push(path);
        }
    }
    Ok(())
}

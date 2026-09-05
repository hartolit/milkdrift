//! Repository-level documentation, dependency-direction, and public-boundary contracts.

use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fs,
    path::{Path, PathBuf},
};

use milkdrift_authority::Selection;
use milkdrift_capability::OperationId;
use milkdrift_local_process::ProcessProfileDocument;

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

const COHESION_REVIEW_LINES: usize = 1_000;
const MAXIMUM_SOURCE_LINES: usize = 2_000;

#[derive(Clone, Copy, Debug)]
struct CohesionException {
    path: &'static str,
    ceiling: usize,
    rationale: &'static str,
}

const PRODUCTION_COHESION_EXCEPTIONS: &[CohesionException] = &[
    CohesionException {
        path: "adapters/local-process/src/config.rs",
        ceiling: 1_152,
        rationale: "one versioned process-profile reader validates the complete external adapter contract",
    },
    CohesionException {
        path: "adapters/model-provider/src/adapter.rs",
        ceiling: 1_291,
        rationale: "one provider adapter owns request mapping, streaming, and bounded response classification",
    },
    CohesionException {
        path: "adapters/peer-http/src/remote.rs",
        ceiling: 1_357,
        rationale: "one remote peer adapter owns authenticated protocol exchange and failure translation",
    },
    CohesionException {
        path: "adapters/redb-store/src/admin/cursor.rs",
        ceiling: 1_072,
        rationale: "one durable cursor owner validates every administrative scan phase transition",
    },
    CohesionException {
        path: "adapters/redb-store/src/application.rs",
        ceiling: 1_152,
        rationale: "one storage owner implements the complete application-state persistence port",
    },
    CohesionException {
        path: "adapters/redb-store/src/controller_account.rs",
        ceiling: 1_178,
        rationale: "one storage owner implements the controller-account durable ledger contract",
    },
    CohesionException {
        path: "adapters/redb-store/src/journal/workspace.rs",
        ceiling: 1_067,
        rationale: "one journal transaction owner keeps workspace accounting and event commit atomic",
    },
    CohesionException {
        path: "adapters/redb-store/src/peer.rs",
        ceiling: 1_561,
        rationale: "one storage owner implements the complete peer-state persistence port",
    },
    CohesionException {
        path: "apps/daemon/src/config.rs",
        ceiling: 1_368,
        rationale: "one daemon configuration reader owns validation across the versioned host document",
    },
    CohesionException {
        path: "apps/daemon/src/host.rs",
        ceiling: 1_830,
        rationale: "one daemon composition owner coordinates lifecycle across focused private host modules",
    },
    CohesionException {
        path: "apps/daemon/src/http.rs",
        ceiling: 1_531,
        rationale: "one transport owner keeps route adaptation and public failure mapping consistent",
    },
    CohesionException {
        path: "crates/blueprint/src/validation.rs",
        ceiling: 1_158,
        rationale: "one validator owns the complete immutable blueprint semantic invariant set",
    },
    CohesionException {
        path: "crates/capability/src/descriptor.rs",
        ceiling: 1_213,
        rationale: "one descriptor owner validates the complete versioned capability declaration contract",
    },
    CohesionException {
        path: "crates/capability/src/invocation.rs",
        ceiling: 1_284,
        rationale: "one invocation owner validates exact values, references, and accounting metadata",
    },
    CohesionException {
        path: "crates/control/src/service.rs",
        ceiling: 1_346,
        rationale: "one control service owns authorization and durable command admission ordering",
    },
    CohesionException {
        path: "crates/persistence/src/controller_account.rs",
        ceiling: 1_966,
        rationale: "one persistence contract owns controller-account ledger types and transition invariants",
    },
];

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

fn manifest_section<'a>(manifest: &'a str, heading: &str) -> &'a str {
    let Some((_, remainder)) = manifest.split_once(heading) else {
        return "";
    };
    remainder
        .split_once("\n[")
        .map_or(remainder, |(section, _)| section)
}

#[test]
fn canonical_entrypoint_and_local_links_resolve() -> TestResult {
    let repository = root()?;
    let root_markdown = fs::read_dir(&repository)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|extension| extension.to_str()) == Some("md"))
        .filter_map(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(str::to_owned)
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        root_markdown,
        BTreeSet::from([
            "AGENTS.md".to_owned(),
            "CONTRIBUTING.md".to_owned(),
            "README.md".to_owned(),
        ]),
        "repository-root Markdown is limited to entry-point documents"
    );

    let canonical = [
        "AGENTS.md",
        "CONTRIBUTING.md",
        "README.md",
        "docs/README.md",
        "docs/product/vision.md",
        "docs/architecture.md",
        "docs/product/status.md",
        "docs/product/roadmap.md",
        "docs/development/workflow.md",
        "docs/development/engineering-rules.md",
        "docs/development/verification-evidence.md",
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
        "2. `docs/product/vision.md`",
        "3. `docs/architecture.md`",
        "4. `docs/product/status.md`",
        "5. `docs/product/roadmap.md`",
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
        "RUN_EVENT_SCHEMA_VERSION_V3",
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
    let architecture = read(root()?.join("docs/architecture.md"))?;
    for fact in facts {
        assert!(
            architecture.contains(&fact),
            "architecture is missing {fact}"
        );
    }

    let control = format!("protocol-{control_major}.{control_minor}");
    assert!(read(root()?.join("README.md"))?.contains(&control));
    assert!(
        read(root()?.join("docs/product/status.md"))?
            .contains("External control / authenticated cursor | 2.3 / 2")
    );
    Ok(())
}

#[test]
fn peer_protocol_version_is_exact_from_config_through_transport() -> TestResult {
    let daemon_config = read(root()?.join("apps/daemon/src/config.rs"))?;
    assert!(
        daemon_config.match_indices("PROTOCOL_MINOR_V1").count() >= 4,
        "daemon peer defaults and both configured bounds must derive from the protocol constant"
    );

    let daemon_host = read(root()?.join("apps/daemon/src/host.rs"))?;
    assert!(daemon_host.contains("let versions = ProtocolVersionRange::default();"));
    assert!(daemon_host.contains("major: PROTOCOL_MAJOR_V1"));
    assert!(
        !daemon_host.contains("minor: 1"),
        "daemon peer composition must not restore the refused v1.1 minor"
    );

    let codec = read(root()?.join("crates/peer-protocol/src/document.rs"))?;
    assert!(codec.contains("protocol != ProtocolVersion::V1_2"));
    let client = read(root()?.join("adapters/peer-http/src/client.rs"))?;
    assert!(
        client.contains("peer response envelope does not match the negotiated protocol version")
    );
    Ok(())
}

#[test]
fn status_is_current_fact_not_a_pass_diary() -> TestResult {
    let status = read(root()?.join("docs/product/status.md"))?;
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
fn prompt_packages_and_pass_history_stay_out_of_canonical_docs() -> TestResult {
    let repository = root()?;
    let docs = repository.join("docs");
    let mut documents = Vec::new();
    collect_files(&docs, &mut documents, &|_| true)?;
    collect_files(&repository.join(".github"), &mut documents, &|_| true)?;
    documents.extend(
        ["AGENTS.md", "CONTRIBUTING.md", "README.md"]
            .into_iter()
            .map(|relative| repository.join(relative)),
    );

    for document in documents {
        let relative = document.strip_prefix(&repository)?;
        for component in relative.components() {
            let name = component.as_os_str().to_string_lossy().to_ascii_lowercase();
            assert!(
                !name.contains("phase-prompt")
                    && !name.contains("pass-history")
                    && !name.contains("cleanup-diary")
                    && name != "codebase-audit.md",
                "obsolete process-history path returned: {}",
                relative.display()
            );
        }

        if document
            .extension()
            .and_then(|extension| extension.to_str())
            != Some("md")
            && !document.starts_with(repository.join(".github"))
        {
            continue;
        }
        let contents = read(&document)?;
        for obsolete in [
            "docs/development/phase-prompts/",
            "docs/development/codebase-audit.md",
        ] {
            assert!(
                !contents.contains(obsolete),
                "stale process-history reference `{obsolete}` in {}",
                relative.display()
            );
        }
    }
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
        "docs/development/workflow.md",
        "docs/guides/external-evidence.md",
        "docs/development/verification-evidence.md",
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
fn contract_dependency_direction_and_semantic_owners_are_exact() -> TestResult {
    let repository = root()?;
    let contracts_manifest = read(repository.join("crates/contracts/Cargo.toml"))?;
    assert!(
        !manifest_section(&contracts_manifest, "[dependencies]").contains("milkdrift-"),
        "shared mechanics must not depend on a Milkdrift domain package"
    );
    let mut contract_sources = Vec::new();
    collect_files(
        &repository.join("crates/contracts/src"),
        &mut contract_sources,
        &|path| path.extension().and_then(|extension| extension.to_str()) == Some("rs"),
    )?;
    for source in contract_sources {
        assert!(
            !read(&source)?.contains("SCHEMA_VERSION"),
            "shared mechanics must not own semantic schema constants in {}",
            source.display()
        );
    }

    let capability_manifest = read(repository.join("crates/capability/Cargo.toml"))?;
    assert!(
        !manifest_section(&capability_manifest, "[dependencies]").contains("milkdrift-blueprint"),
        "capability contracts must not depend on workflow definitions"
    );
    let blueprint_manifest = read(repository.join("crates/blueprint/Cargo.toml"))?;
    let blueprint_dependencies = manifest_section(&blueprint_manifest, "[dependencies]");
    for forbidden in [
        "milkdrift-runtime",
        "milkdrift-capability-host",
        "milkdrift-redb-store",
        "milkdrift-local-process",
        "milkdrift-model-provider",
        "milkdrift-peer-http",
    ] {
        assert!(
            !blueprint_dependencies.contains(forbidden),
            "blueprint imports host/runtime/adapter state through {forbidden}"
        );
    }

    let capability_identity = read(repository.join("crates/capability/src/identity.rs"))?;
    for canonical in ["SchemaId", "ExtensionKey", "TrustZone", "PeerId"] {
        assert!(
            capability_identity.contains(canonical),
            "capability no longer owns {canonical}"
        );
    }
    assert!(
        read(repository.join("crates/capability/src/bounded.rs"))?.contains("struct BoundedJson"),
        "capability no longer owns BoundedJson"
    );
    assert!(
        read(repository.join("crates/blueprint/src/model/contract.rs"))?
            .contains("struct SchemaRef")
    );
    assert!(
        read(repository.join("crates/capability/src/descriptor.rs"))?
            .contains("struct SchemaContract")
    );
    Ok(())
}

#[test]
fn test_support_and_removed_compatibility_paths_stay_out_of_default_surfaces() -> TestResult {
    let repository = root()?;
    let runtime_manifest = read(repository.join("crates/runtime/Cargo.toml"))?;
    let host_manifest = read(repository.join("crates/capability-host/Cargo.toml"))?;
    for (name, manifest) in [
        ("runtime", runtime_manifest.as_str()),
        ("capability-host", host_manifest.as_str()),
    ] {
        assert!(manifest.contains("default = []\ntest-support = []"));
        assert!(
            !manifest_section(manifest, "[dependencies]").contains("test-support"),
            "{name} enables test support for default production dependencies"
        );
    }

    let runtime_root = read(repository.join("crates/runtime/src/lib.rs"))?;
    assert!(
        runtime_root.contains(
            "#[cfg(any(test, feature = \"test-support\"))]\npub use boundary::ManualClock;"
        )
    );
    assert!(runtime_root.contains(
        "#[cfg(any(test, feature = \"test-support\"))]\npub use executor::DeterministicExecutor;"
    ));
    let host_root = read(repository.join("crates/capability-host/src/lib.rs"))?;
    assert!(host_root.contains(
        "#[cfg(any(test, feature = \"test-support\"))]\npub use secret::InMemorySecretResolver;"
    ));

    let runtime_engine = read(repository.join("crates/runtime/src/engine.rs"))?;
    let runtime_effects = read(repository.join("crates/runtime/src/engine/effects.rs"))?;
    let runtime_scheduling = read(repository.join("crates/runtime/src/engine/scheduling.rs"))?;
    for removed in ["EffectTickResult", "effect_tick", "drive_once"] {
        assert!(
            !runtime_engine.contains(removed) && !runtime_effects.contains(removed),
            "removed runtime compatibility API returned: {removed}"
        );
    }
    assert!(!runtime_scheduling.contains("pub fn tick("));
    let query = read(repository.join("crates/persistence/src/journal/query.rs"))?;
    assert!(!query.contains("fn nonterminal_runs("));
    assert!(!query.contains("fn runnable("));

    let mut manifests = Vec::new();
    collect_files(&repository, &mut manifests, &|path| {
        path.file_name().and_then(|name| name.to_str()) == Some("Cargo.toml")
    })?;
    for manifest_path in manifests {
        if manifest_path == repository.join("tools/evidence/Cargo.toml") {
            continue;
        }
        let manifest = read(&manifest_path)?;
        assert!(
            !manifest_section(&manifest, "[dependencies]").contains("test-support"),
            "production dependencies enable test support in {}",
            manifest_path.display()
        );
    }
    Ok(())
}

#[test]
fn shared_text_mechanics_and_canonical_import_paths_do_not_diverge() -> TestResult {
    let repository = root()?;
    let mut sources = Vec::new();
    for directory in ["crates", "adapters", "apps"] {
        collect_files(&repository.join(directory), &mut sources, &|path| {
            path.extension().and_then(|extension| extension.to_str()) == Some("rs")
        })?;
    }
    let shared_text = repository.join("crates/contracts/src/text.rs");
    for source in sources {
        if source == shared_text {
            continue;
        }
        let contents = read(&source)?;
        assert!(
            !contents.contains(".is_char_boundary("),
            "UTF-8 truncation boundary logic escaped shared mechanics in {}",
            source.display()
        );
        assert!(
            !contents.contains(".strip_prefix(\"b3_\")"),
            "canonical BLAKE3 lexical logic escaped shared mechanics in {}",
            source.display()
        );
    }

    for relative in [
        "crates/authority/src/lib.rs",
        "crates/peer-protocol/src/lib.rs",
    ] {
        assert!(
            !read(repository.join(relative))?.contains("pub use milkdrift_capability::PeerId"),
            "PeerId regained an alternate public import through {relative}"
        );
    }
    assert!(
        !read(repository.join("crates/workspace/src/lib.rs"))?
            .contains("pub use milkdrift_capability::BoundedJson"),
        "BoundedJson regained an alternate workspace import"
    );
    assert!(
        !read(repository.join("crates/model/src/lib.rs"))?.contains("pub use milkdrift_capability"),
        "capability-owned context constants regained an alternate model import"
    );

    for consumer in [
        "crates/blueprint/src/model/contract.rs",
        "crates/capability/src/descriptor.rs",
    ] {
        assert!(
            read(repository.join(consumer))?.contains("milkdrift_contracts::deserialize_via!"),
            "shared validated-deserialization mechanics lost production consumer {consumer}"
        );
    }

    let capability_document = read(repository.join("crates/capability/src/document.rs"))?;
    let capability_snapshot = read(repository.join("crates/capability/src/resolved.rs"))?;
    assert_eq!(
        capability_document
            .match_indices("const RESOLVED_CAPABILITY_SNAPSHOT_SCHEMA_VERSION_V2")
            .count(),
        1,
        "resolved-capability schema version must have one implementation owner"
    );
    assert!(
        !capability_snapshot.contains("const RESOLVED_CAPABILITY_SNAPSHOT_SCHEMA_VERSION_V2"),
        "resolved-capability schema version regained a secondary owner"
    );
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
    assert!(runtime_executor.contains("fn prepare_exact_entry<'a>("));
    assert!(!runtime_executor.contains("fn execute_streaming("));
    assert!(!runtime_executor.contains("ExecutionReportBatch"));
    assert!(!runtime_executor.contains("bounded synchronous executors"));

    for (package_root, accidental_export) in [
        (
            "crates/authority/src/lib.rs",
            "AUTHORITY_GRANT_SCHEMA_VERSION",
        ),
        (
            "crates/blueprint/src/lib.rs",
            "pub const BLUEPRINT_SCHEMA_VERSION",
        ),
        ("crates/capability/src/lib.rs", "SCHEMA_VERSION"),
        ("crates/capability/src/lib.rs", "MAX_JSON_DEPTH"),
        ("crates/control-protocol/src/lib.rs", "pub const PROTOCOL_"),
        (
            "crates/control-protocol/src/lib.rs",
            "pub const LAYOUT_SCHEMA_VERSION",
        ),
        ("crates/model/src/lib.rs", "MODEL_CONTRACT_SCHEMA_VERSION"),
        ("crates/model/src/lib.rs", "CONTEXT_MANIFEST_SCHEMA_VERSION"),
        (
            "crates/prompt-sequence/src/lib.rs",
            "PROMPT_SEQUENCE_SCHEMA_VERSION",
        ),
        (
            "adapters/local-process/src/lib.rs",
            "PROCESS_PROFILE_SCHEMA_VERSION",
        ),
        (
            "adapters/model-provider/src/lib.rs",
            "MODEL_ENDPOINT_PROFILE_SCHEMA_VERSION",
        ),
    ] {
        assert!(
            !read(repository.join(package_root))?.contains(accidental_export),
            "unconsumed implementation constant regained a root export in {package_root}: {accidental_export}"
        );
    }

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
fn production_sources_over_the_review_threshold_have_exact_bounded_exceptions() -> TestResult {
    let repository = root()?;
    let mut paths = Vec::new();
    collect_files(&repository, &mut paths, &|path| {
        path.extension().and_then(|extension| extension.to_str()) == Some("rs")
    })?;
    let sources = paths
        .iter()
        .map(|path| {
            let relative = path
                .strip_prefix(&repository)
                .map_err(|_| {
                    std::io::Error::other(format!(
                        "collected source escaped repository root: {}",
                        path.display()
                    ))
                })?
                .to_string_lossy()
                .replace('\\', "/");
            Ok(SourceLineCount {
                production: is_production_source(&relative),
                path: relative,
                lines: read(path)?.lines().count(),
            })
        })
        .collect::<TestResult<Vec<_>>>()?;
    let errors = cohesion_policy_errors(PRODUCTION_COHESION_EXCEPTIONS, &sources);
    assert!(
        errors.is_empty(),
        "production cohesion exception policy failed:\n{}",
        errors.join("\n")
    );
    Ok(())
}

#[test]
fn cohesion_policy_rejects_missing_stale_duplicate_over_broad_and_exceeded_exceptions() {
    let sources = vec![
        SourceLineCount::production("crates/example/src/missing.rs", 1_001),
        SourceLineCount::production("crates/example/src/stale.rs", 900),
        SourceLineCount::production("crates/example/src/duplicate.rs", 1_001),
        SourceLineCount::production("crates/example/src/exceeded.rs", 1_101),
        SourceLineCount {
            path: "crates/example/tests/large.rs".to_owned(),
            lines: 1_500,
            production: false,
        },
    ];
    let exceptions = [
        CohesionException {
            path: "crates/example/src/stale.rs",
            ceiling: 1_050,
            rationale: "this deliberately stale fixture has enough words for policy validation",
        },
        CohesionException {
            path: "crates/example/src/duplicate.rs",
            ceiling: 1_050,
            rationale: "this deliberately duplicate fixture has enough words for policy validation",
        },
        CohesionException {
            path: "crates/example/src/duplicate.rs",
            ceiling: 1_060,
            rationale: "this second duplicate fixture has enough words for policy validation",
        },
        CohesionException {
            path: "crates/example/src/*",
            ceiling: 1_050,
            rationale: "this deliberately broad fixture has enough words for policy validation",
        },
        CohesionException {
            path: "crates/example/src/exceeded.rs",
            ceiling: 1_050,
            rationale: "this deliberately exceeded fixture has enough words for policy validation",
        },
    ];
    let errors = cohesion_policy_errors(&exceptions, &sources).join("\n");
    for category in ["missing", "stale", "duplicate", "over-broad", "exceeded"] {
        assert!(
            errors.contains(category),
            "missing {category} diagnostic: {errors}"
        );
    }
    assert!(
        !errors.contains("crates/example/tests/large.rs"),
        "test and evidence sources must remain separate from production review exceptions"
    );
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

#[derive(Clone, Debug)]
struct SourceLineCount {
    path: String,
    lines: usize,
    production: bool,
}

impl SourceLineCount {
    fn production(path: &str, lines: usize) -> Self {
        Self {
            path: path.to_owned(),
            lines,
            production: true,
        }
    }
}

fn is_production_source(relative: &str) -> bool {
    let mut components = relative.split('/');
    let Some(area) = components.next() else {
        return false;
    };
    if !matches!(area, "adapters" | "apps" | "crates") || !relative.contains("/src/") {
        return false;
    }
    if relative == "apps/daemon/src/bin/milkdrift-external-evidence/main.rs" {
        return false;
    }
    !relative
        .split('/')
        .any(|component| matches!(component, "tests" | "tests.rs" | "testing.rs"))
}

fn cohesion_policy_errors(
    exceptions: &[CohesionException],
    sources: &[SourceLineCount],
) -> Vec<String> {
    let source_by_path = sources
        .iter()
        .map(|source| (source.path.as_str(), source))
        .collect::<BTreeMap<_, _>>();
    let mut exception_by_path = BTreeMap::new();
    let mut errors = Vec::new();
    let mut prior_path = None;
    for exception in exceptions {
        if prior_path.is_some_and(|prior| prior >= exception.path) {
            errors.push(format!(
                "unordered exception: {} must follow exact lexical path order",
                exception.path
            ));
        }
        prior_path = Some(exception.path);
        let exact_path = exception.path.ends_with(".rs")
            && !exception.path.starts_with('/')
            && !exception.path.contains("..")
            && !exception.path.contains(['*', '?', '[', ']'])
            && is_production_source(exception.path);
        if !exact_path {
            errors.push(format!("over-broad exception: {}", exception.path));
            continue;
        }
        if exception.rationale.split_whitespace().count() < 6 {
            errors.push(format!("empty or weak rationale: {}", exception.path));
        }
        if exception.ceiling <= COHESION_REVIEW_LINES || exception.ceiling >= MAXIMUM_SOURCE_LINES {
            errors.push(format!(
                "invalid bounded ceiling for {}: {}",
                exception.path, exception.ceiling
            ));
        }
        if exception_by_path
            .insert(exception.path, exception)
            .is_some()
        {
            errors.push(format!("duplicate exception: {}", exception.path));
        }
    }

    for exception in exception_by_path.values() {
        match source_by_path.get(exception.path) {
            None => errors.push(format!("stale exception path: {}", exception.path)),
            Some(source) if !source.production || source.lines <= COHESION_REVIEW_LINES => errors
                .push(format!(
                    "stale exception below review threshold: {} has {} lines",
                    exception.path, source.lines
                )),
            Some(source) if source.lines > exception.ceiling => errors.push(format!(
                "exceeded exception ceiling: {} has {} lines above {}",
                exception.path, source.lines, exception.ceiling
            )),
            Some(_) => {}
        }
    }
    for source in sources
        .iter()
        .filter(|source| source.production && source.lines > COHESION_REVIEW_LINES)
    {
        if !exception_by_path.contains_key(source.path.as_str()) {
            errors.push(format!(
                "missing exception: {} has {} production lines",
                source.path, source.lines
            ));
        }
    }
    errors
}

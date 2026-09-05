use super::*;

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
    let daemon_config = read(root()?.join("apps/daemon/src/config/wire.rs"))?;
    assert!(
        daemon_config.match_indices("PROTOCOL_MINOR_V1").count() >= 2,
        "daemon peer defaults must derive from the protocol constant"
    );
    let compiler = read(root()?.join("apps/daemon/src/config/compile.rs"))?;
    assert!(compiler.contains("relationship.minimum_minor != PROTOCOL_MINOR_V1"));
    assert!(compiler.contains("relationship.maximum_minor != PROTOCOL_MINOR_V1"));

    let daemon_host = read(root()?.join("apps/daemon/src/host/peers.rs"))?;
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

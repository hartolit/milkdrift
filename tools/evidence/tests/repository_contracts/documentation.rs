use super::{
    BTreeMap, BTreeSet, Path, PathBuf, TestResult, collect_files, fs, numeric_const, read, root,
};

const CANONICAL: &[&str] = &[
    "AGENTS.md",
    "CONTRIBUTING.md",
    "README.md",
    "docs/README.md",
    "docs/product/vision.md",
    "docs/architecture.md",
    "docs/product/status.md",
    "docs/product/roadmap.md",
    "docs/development/workflow.md",
    "docs/development/practices/README.md",
    "docs/development/practices/implementation.md",
    "docs/development/practices/documentation.md",
    "docs/development/verification-evidence.md",
    "docs/reference/public-api-policy.md",
];

fn markdown_files() -> TestResult<Vec<PathBuf>> {
    let mut files = Vec::new();
    collect_files(&root()?, &mut files, &|path| {
        path.extension().is_some_and(|ext| ext == "md")
    })?;
    Ok(files)
}

// Only structural Markdown is checked: prose is free to change without a paragraph snapshot.
fn markdown_structure(document: &str) -> TestResult<(BTreeSet<String>, Vec<String>)> {
    let mut headings = BTreeSet::new();
    let mut duplicates = BTreeMap::<String, usize>::new();
    let mut links = Vec::new();
    let mut fence: Option<(char, usize)> = None;
    for line in document.lines() {
        let line = line.trim_start();
        if let Some(marker @ ('`' | '~')) = line.chars().next() {
            let count = line.chars().take_while(|ch| *ch == marker).count();
            if count >= 3 {
                match fence {
                    Some((opened, length)) if opened == marker && count >= length => fence = None,
                    None => fence = Some((marker, count)),
                    _ => {}
                }
                continue;
            }
        }
        if fence.is_some() {
            continue;
        }
        if line.starts_with('#') {
            let heading = line
                .trim_start_matches('#')
                .trim()
                .trim_end_matches('#')
                .trim();
            let base: String = heading
                .to_lowercase()
                .chars()
                .filter(|ch| ch.is_alphanumeric() || ch.is_whitespace() || matches!(ch, '-' | '_'))
                .map(|ch| if ch.is_whitespace() { '-' } else { ch })
                .collect();
            let count = duplicates.entry(base.clone()).or_default();
            headings.insert(if *count == 0 {
                base
            } else {
                format!("{base}-{count}")
            });
            *count += 1;
        }
        let mut remainder = line;
        while let Some((_, target)) = remainder.split_once("](") {
            let (target, rest) = target
                .split_once(')')
                .ok_or_else(|| std::io::Error::other("unterminated Markdown link"))?;
            links.push(target.trim().trim_matches(['<', '>']).to_owned());
            remainder = rest;
        }
        if line.starts_with('[')
            && let Some((_, target)) = line.split_once("]:")
        {
            links.push(target.trim().trim_matches(['<', '>']).to_owned());
        }
    }
    if fence.is_some() {
        return Err(std::io::Error::other("unclosed Markdown fence").into());
    }
    Ok((headings, links))
}

fn check_link(document: &Path, target: &str) -> TestResult {
    if target.starts_with("https://")
        || target.starts_with("http://")
        || target.starts_with("mailto:")
    {
        return Ok(());
    }
    let (path, fragment) = target
        .split_once('#')
        .map_or((target, None), |(p, f)| (p, Some(f)));
    let resolved = if path.is_empty() {
        document.to_owned()
    } else {
        document
            .parent()
            .ok_or("Markdown parent missing")?
            .join(path.replace("%20", " "))
    };
    if !resolved.exists() {
        return Err(format!("{}: missing link {target}", document.display()).into());
    }
    if resolved.extension().is_some_and(|ext| ext == "md")
        && let Some(fragment) = fragment.filter(|fragment| !fragment.is_empty())
    {
        let (headings, _) = markdown_structure(&read(&resolved)?)?;
        if !headings.contains(fragment) {
            return Err(format!("{}: missing heading {target}", document.display()).into());
        }
    }
    Ok(())
}

#[test]
fn canonical_entrypoint_and_all_local_links_resolve() -> TestResult {
    let repository = root()?;
    for relative in CANONICAL {
        assert!(repository.join(relative).is_file(), "missing {relative}");
    }
    let documents = markdown_files()?;
    let root_documents: BTreeSet<_> = documents
        .iter()
        .filter(|path| path.parent() == Some(repository.as_path()))
        .filter_map(|path| path.file_name().and_then(|name| name.to_str()))
        .collect();
    assert_eq!(
        root_documents,
        BTreeSet::from(["AGENTS.md", "CONTRIBUTING.md", "README.md"])
    );
    for path in documents {
        let (_, links) = markdown_structure(&read(&path)?)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        for target in links {
            check_link(&path, &target)?;
        }
    }
    let agents = read(repository.join("AGENTS.md"))?;
    let mut prior = 0;
    for item in [
        "1. `AGENTS.md`",
        "2. `docs/product/vision.md`",
        "3. `docs/architecture.md`",
        "4. `docs/product/status.md`",
        "5. `docs/product/roadmap.md`",
        "6. Relevant ADRs, references, source, and tests",
    ] {
        let position = agents.find(item).ok_or_else(|| format!("missing {item}"))?;
        assert!(position >= prior, "reading order changed at {item}");
        prior = position;
    }
    Ok(())
}

#[test]
fn markdown_checks_reject_missing_files_headings_and_open_fences() -> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("guide.md");
    fs::write(
        &path,
        "# Guide\n\n## A `bounded` path\n\n## A `bounded` path\n",
    )?;
    check_link(&path, "#a-bounded-path")?;
    check_link(&path, "guide.md#a-bounded-path-1")?;
    assert!(check_link(&path, "missing.md").is_err());
    assert!(check_link(&path, "#obsolete-heading").is_err());
    assert!(markdown_structure("```sh\ncommand\n").is_err());
    let (_, links) = markdown_structure("````text\n```sh\n[x](missing)\n```\n````\n[x](guide.md)")?;
    assert_eq!(links, ["guide.md"]);
    Ok(())
}

#[test]
fn canonical_version_cells_match_all_owning_constants() -> TestResult {
    let mut expected = BTreeMap::new();
    for (family, sources) in [
        (
            "Capability descriptor/events/cancellation / resolved snapshot",
            vec![
                ("crates/capability/src/document.rs", "SCHEMA_VERSION_V1"),
                (
                    "crates/capability/src/document.rs",
                    "RESOLVED_CAPABILITY_SNAPSHOT_SCHEMA_VERSION_V2",
                ),
            ],
        ),
        (
            "Invocation request",
            vec![(
                "crates/capability/src/document.rs",
                "INVOCATION_REQUEST_SCHEMA_VERSION_V2",
            )],
        ),
        (
            "Blueprint revision and mutation",
            vec![("crates/blueprint/src/lib.rs", "BLUEPRINT_SCHEMA_VERSION_V2")],
        ),
        (
            "Context manifest",
            vec![(
                "crates/model/src/context.rs",
                "CONTEXT_MANIFEST_SCHEMA_VERSION_V2",
            )],
        ),
        (
            "Model document / task / response / endpoint profile",
            vec![
                (
                    "crates/model/src/document.rs",
                    "MODEL_CONTRACT_SCHEMA_VERSION_V1",
                ),
                (
                    "crates/model/src/document.rs",
                    "MODEL_CONTRACT_SCHEMA_VERSION_V1",
                ),
                (
                    "crates/model/src/document.rs",
                    "MODEL_CONTRACT_SCHEMA_VERSION_V1",
                ),
                (
                    "adapters/model-provider/src/profile.rs",
                    "MODEL_ENDPOINT_PROFILE_SCHEMA_VERSION_V1",
                ),
            ],
        ),
        (
            "Proposal / workflow-control command / risk policy / controller policy",
            vec![
                (
                    "crates/control/src/document.rs",
                    "PROPOSAL_SCHEMA_VERSION_V1",
                ),
                (
                    "crates/control/src/command.rs",
                    "CONTROL_COMMAND_SCHEMA_VERSION_V1",
                ),
                (
                    "crates/control/src/policy.rs",
                    "CONTROL_RISK_POLICY_VERSION_V1",
                ),
                (
                    "crates/control/src/controller/policy.rs",
                    "CONTROLLER_POLICY_SCHEMA_VERSION_V1",
                ),
            ],
        ),
        (
            "Prompt-sequence import",
            vec![(
                "crates/prompt-sequence/src/document.rs",
                "PROMPT_SEQUENCE_SCHEMA_VERSION_V2",
            )],
        ),
        (
            "Run command / run event",
            vec![
                (
                    "crates/runtime/src/command.rs",
                    "RUN_COMMAND_SCHEMA_VERSION_V1",
                ),
                (
                    "crates/persistence/src/document.rs",
                    "RUN_EVENT_SCHEMA_VERSION_V3",
                ),
            ],
        ),
        (
            "Authority grant / authorization decision",
            vec![
                (
                    "crates/authority/src/document.rs",
                    "AUTHORITY_GRANT_SCHEMA_VERSION_V4",
                ),
                (
                    "crates/authority/src/document.rs",
                    "AUTHORITY_DECISION_SCHEMA_VERSION_V2",
                ),
            ],
        ),
        (
            "Authorized-command wrapper / command result",
            vec![
                (
                    "crates/runtime/src/command.rs",
                    "AUTHORIZED_RUN_COMMAND_SCHEMA_VERSION_V1",
                ),
                (
                    "crates/persistence/src/journal.rs",
                    "COMMAND_RESULT_SCHEMA_VERSION_V2",
                ),
            ],
        ),
        (
            "Projection snapshot envelope / runtime payload",
            vec![
                (
                    "crates/persistence/src/snapshot.rs",
                    "SNAPSHOT_ENVELOPE_SCHEMA_VERSION_V2",
                ),
                (
                    "crates/runtime/src/query.rs",
                    "RUN_PROJECTION_SNAPSHOT_SCHEMA_V4",
                ),
            ],
        ),
        (
            "Administrative integrity cursor",
            vec![(
                "adapters/redb-store/src/admin/cursor.rs",
                "INTEGRITY_CURSOR_VERSION",
            )],
        ),
        (
            "Peer hot record / compact tombstone",
            vec![
                (
                    "crates/persistence/src/peer.rs",
                    "PEER_EXECUTION_RECORD_SCHEMA_VERSION_V3",
                ),
                (
                    "crates/persistence/src/peer.rs",
                    "PEER_EXECUTION_TOMBSTONE_SCHEMA_VERSION_V1",
                ),
            ],
        ),
        (
            "Redb internal document format / physical schema",
            vec![
                (
                    "adapters/redb-store/src/schema.rs",
                    "INTERNAL_DOCUMENT_FORMAT_VERSION",
                ),
                (
                    "adapters/redb-store/src/schema.rs",
                    "STORAGE_SCHEMA_VERSION",
                ),
            ],
        ),
        (
            "Application command receipt / layout record",
            vec![
                (
                    "crates/persistence/src/application.rs",
                    "APPLICATION_COMMAND_RECEIPT_SCHEMA_VERSION_V1",
                ),
                (
                    "crates/persistence/src/application.rs",
                    "APPLICATION_LAYOUT_RECORD_SCHEMA_VERSION_V1",
                ),
            ],
        ),
        (
            "Local-process profile / host materialization",
            vec![
                (
                    "adapters/local-process/src/config.rs",
                    "PROCESS_PROFILE_SCHEMA_VERSION_V2",
                ),
                (
                    "crates/capability-host/src/materialization.rs",
                    "MATERIALIZATION_SCHEMA_VERSION_V1",
                ),
            ],
        ),
        (
            "Daemon configuration",
            vec![("apps/daemon/src/config.rs", "DAEMON_CONFIG_SCHEMA_VERSION")],
        ),
        (
            "Layout document / CLI JSON output",
            vec![
                (
                    "crates/control-protocol/src/lib.rs",
                    "LAYOUT_SCHEMA_VERSION",
                ),
                ("apps/cli/src/output.rs", "JSON_OUTPUT_SCHEMA_VERSION"),
            ],
        ),
    ] {
        let versions = sources
            .into_iter()
            .map(|(path, name)| numeric_const(path, name).map(|value| value.to_string()))
            .collect::<TestResult<Vec<_>>>()?;
        expected.insert(family.to_owned(), versions.join(" / "));
    }
    let control = format!(
        "{}.{}",
        numeric_const("crates/control-protocol/src/lib.rs", "PROTOCOL_MAJOR")?,
        numeric_const("crates/control-protocol/src/lib.rs", "PROTOCOL_MINOR")?
    );
    let peer = format!(
        "{}.{}",
        numeric_const("crates/peer-protocol/src/session.rs", "PROTOCOL_MAJOR_V1")?,
        numeric_const("crates/peer-protocol/src/session.rs", "PROTOCOL_MINOR_V1")?
    );
    expected.insert(
        "External control / authenticated cursor".to_owned(),
        format!(
            "{control} / {}",
            numeric_const(
                "crates/control-protocol/src/lib.rs",
                "AUTHENTICATED_CURSOR_SCHEMA_VERSION"
            )?
        ),
    );
    expected.insert(
        "Peer protocol and catalog messages".to_owned(),
        peer.clone(),
    );
    let status = read(root()?.join("docs/product/status.md"))?;
    let mut documented = BTreeMap::new();
    for line in status.lines().filter(|line| line.starts_with("| ")) {
        let cells: Vec<_> = line.split('|').map(str::trim).collect();
        if cells[1] == "Contract or durable family" || cells[1] == "---" {
            continue;
        }
        assert_eq!(
            cells.len(),
            5,
            "version row needs family/version/read behavior"
        );
        assert!(
            documented
                .insert(cells[1].to_owned(), cells[2].to_owned())
                .is_none(),
            "duplicate version family"
        );
        assert!(!cells[3].is_empty(), "missing read behavior");
    }
    assert_eq!(documented, expected);
    for (path, heading) in [
        (
            "docs/reference/control-api.md",
            format!("# Local control API {control}"),
        ),
        (
            "docs/reference/peer-protocol.md",
            format!("# Peer protocol v{peer}"),
        ),
    ] {
        assert_eq!(
            read(root()?.join(path))?.lines().next(),
            Some(heading.as_str())
        );
    }
    Ok(())
}

#[test]
fn every_maintained_example_has_a_production_reader() -> TestResult {
    let repository = root()?;
    let examples = repository.join("examples");
    let mut files = Vec::new();
    collect_files(&examples, &mut files, &|_| true)?;
    for path in files {
        let relative = path
            .strip_prefix(&examples)?
            .to_string_lossy()
            .replace('\\', "/");
        let bytes = fs::read(&path)?;
        match relative.as_str() {
            "operator/starter.json" | "operator/process.json" | "operator/model.json" => {
                let (document, _) =
                    milkdrift_blueprint::BlueprintRevisionDocument::from_json(&bytes)?;
                assert_eq!(document.to_canonical_json()?, bytes, "{relative}");
            }
            "operator/daemon.toml" => {
                // Compile without starting a host or resolving secrets. Paths belong to a fresh directory.
                let directory = tempfile::tempdir()?;
                let config = directory.path().join("daemon.toml");
                fs::write(&config, &bytes)?;
                milkdrift_daemon::DaemonConfig::load(&config)?;
            }
            "operator/process-profile.example.json"
            | "external-evidence/coding-agent-profile.example.json" => {
                validate_process_template(&bytes)?;
            }
            "local-model/openai-compatible-loopback.example.json"
            | "external-evidence/openai-compatible-profile.example.json"
            | "external-evidence/anthropic-profile.example.json" => {
                milkdrift_model_provider::EndpointProfile::from_json(&bytes)?;
            }
            "headless-dogfood-sequence.md" => {
                milkdrift_prompt_sequence::PromptSequenceDocument::from_bytes(&bytes)?;
            }
            "operator/README.md" | "external-evidence/README.md" => {}
            _ => {
                return Err(
                    format!("maintained example has no production reader: {relative}").into(),
                );
            }
        }
    }
    Ok(())
}

#[test]
fn operator_task_requirements_fit_the_documented_grants() -> TestResult {
    use milkdrift_authority::{CapabilityAuthorityScope, CapabilityAuthorityScopeBuilder};
    use milkdrift_blueprint::{BlueprintRevisionDocument, NodeKind};
    use milkdrift_capability::{
        CapabilityId, OperationId, ProviderProfileRef, SideEffectClass, TrustZone,
    };

    let examples = root()?.join("examples/operator");
    let configuration: milkdrift_daemon::DaemonConfig =
        toml::from_str(&read(examples.join("daemon.toml"))?)?;
    let process_grant = &configuration.actors[0].authority.resources.capability;
    // These are the explicit model overrides in the operator guide, independent of the blueprint.
    let model_grant = CapabilityAuthorityScopeBuilder::new(SideEffectClass::Unknown)
        .only_capabilities(BTreeSet::from([CapabilityId::new("operator-model")?]))?
        .only_operations(BTreeSet::from([OperationId::new("model.generate")?]))?
        .only_provider_profiles(BTreeSet::from([ProviderProfileRef::new(
            "local-model-loopback",
        )?]))?
        .only_trust_zones(BTreeSet::from([TrustZone::new(
            "operator-configured-local-model",
        )?]))?
        .build();
    for (name, grant) in [("process", process_grant), ("model", &model_grant)] {
        let (_, revision) = BlueprintRevisionDocument::from_json(&fs::read(
            examples.join(format!("{name}.json")),
        )?)?;
        let mut tasks = 0;
        for node in revision.semantic().nodes().values() {
            if let NodeKind::Task { config } = node.kind() {
                let requested =
                    CapabilityAuthorityScope::requirement_envelope(config.requirement())?;
                assert!(
                    requested.is_subset_of(grant),
                    "{name} exceeds its operator grant"
                );
                assert!(config.requirement().exact_capability().is_some());
                assert!(!config.requirement().trust_zones().is_empty());
                tasks += 1;
            }
        }
        assert_eq!(
            tasks, 1,
            "operator example must have one ordinary external task"
        );
    }
    Ok(())
}

#[test]
fn control_reference_json_uses_current_wire_readers_and_versions() -> TestResult {
    use milkdrift_control_protocol::{
        CommandRequest, ErrorEnvelope, ProtocolVersion, ResponseEnvelope, VersionRequest,
        decode_json,
    };
    let document = read(root()?.join("docs/reference/control-api.md"))?;
    let mut checked = 0;
    for block in document.split("```json").skip(1) {
        let bytes = block
            .split_once("```")
            .ok_or("unclosed JSON example")?
            .0
            .trim()
            .as_bytes();
        let value: serde_json::Value = decode_json(bytes)?;
        if let Some(protocol) = value.get("protocol") {
            assert_eq!(
                serde_json::from_value::<ProtocolVersion>(protocol.clone())?,
                ProtocolVersion::CURRENT
            );
            if value.get("command").is_some() {
                decode_json::<CommandRequest>(bytes)?;
            } else if value.get("code").is_some() {
                decode_json::<ErrorEnvelope>(bytes)?;
            } else if value.get("value").is_some() {
                decode_json::<ResponseEnvelope<serde_json::Value>>(bytes)?;
            } else {
                decode_json::<VersionRequest>(bytes)?;
            }
        } else {
            assert_eq!(
                value["schema_version"].as_u64(),
                Some(numeric_const(
                    "apps/cli/src/output.rs",
                    "JSON_OUTPUT_SCHEMA_VERSION"
                )?)
            );
        }
        checked += 1;
    }
    assert!(
        checked >= 6,
        "reference examples disappeared from validation"
    );
    Ok(())
}

fn validate_process_template(bytes: &[u8]) -> TestResult {
    // The operator must replace absolute paths and platform claims before using a template on
    // another host. Validate that documented rendering through the production reader, leaving
    // schema, identities, arguments, bounds, trust, effects and authority declarations intact.
    #[cfg(windows)]
    let rendered = {
        let mut value = milkdrift_contracts::parse_json_without_duplicates(bytes)?;
        for pointer in [
            "/profile/executable",
            "/profile/working_directory/path",
            "/profile/filesystem_roots/0/path",
            "/profile/filesystem_roots/1/path",
        ] {
            if let Some(path) = value.pointer_mut(pointer) {
                let unix_path = path.as_str().ok_or("template path must be text")?;
                assert!(
                    unix_path.starts_with("/absolute/"),
                    "unreviewed process template path"
                );
                *path = serde_json::Value::String(format!("C:{unix_path}"));
            }
        }
        for flag in [
            "owned_process_group",
            "descendant_escape_prevention",
            "terminal_group_observation",
        ] {
            *value
                .pointer_mut(&format!("/profile/platform/{flag}"))
                .ok_or("missing platform fact")? = false.into();
        }
        serde_json::to_vec(&value)?
    };
    #[cfg(not(windows))]
    let rendered = bytes.to_vec();
    milkdrift_local_process::ProcessProfileDocument::from_json(&rendered)?;
    Ok(())
}

#[test]
fn status_and_operator_docs_do_not_restore_process_history_or_fixture_setup() -> TestResult {
    let repository = root()?;
    let mut files = Vec::new();
    collect_files(&repository, &mut files, &|_| true)?;
    for path in files {
        let relative = path
            .strip_prefix(&repository)?
            .to_string_lossy()
            .replace('\\', "/");
        for component in relative.split('/') {
            let name = component.to_ascii_lowercase();
            assert!(
                ![
                    "phase-prompt",
                    "pass-history",
                    "cleanup-diary",
                    "pristine-readiness-prompts"
                ]
                .iter()
                .any(|term| name.contains(term))
                    && name != "codebase-audit.md",
                "obsolete history path: {relative}"
            );
        }
    }
    let mut documents = markdown_files()?;
    collect_files(&repository.join(".github"), &mut documents, &|_| true)?;
    for path in documents {
        let relative = path
            .strip_prefix(&repository)?
            .to_string_lossy()
            .replace('\\', "/");
        let contents = read(&path)?;
        for obsolete in [
            "docs/development/phase-prompts/",
            "docs/development/codebase-audit.md",
        ] {
            assert!(
                !contents.contains(obsolete),
                "obsolete reference in {relative}"
            );
        }
        if relative == "README.md"
            || relative.starts_with("docs/operations/")
            || relative.starts_with("docs/guides/")
            || relative.starts_with("examples/operator/")
        {
            assert!(
                !contents.contains("/tests/fixtures"),
                "operator setup references test data: {relative}"
            );
        }
    }
    let status = read(repository.join("docs/product/status.md"))?;
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
    for historical in [
        "this pass",
        "previous pass",
        "pass 0",
        "local repair",
        "source commit `",
        "superseded measurements",
        "on 2026-",
    ] {
        assert!(
            !status.to_lowercase().contains(historical),
            "status contains chronology: {historical}"
        );
    }
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

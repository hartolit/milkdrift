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
            "Capability descriptor / resolved snapshot",
            vec![
                ("crates/capability/src/document.rs", "SCHEMA_VERSION_V1"),
                (
                    "crates/capability/src/document.rs",
                    "RESOLVED_CAPABILITY_SNAPSHOT_SCHEMA_VERSION_V3",
                ),
            ],
        ),
        (
            "Invocation request",
            vec![(
                "crates/capability/src/document.rs",
                "INVOCATION_REQUEST_SCHEMA_VERSION_V3",
            )],
        ),
        (
            "Model document / endpoint profile",
            vec![
                (
                    "crates/model/src/document.rs",
                    "MODEL_CONTRACT_SCHEMA_VERSION_V1",
                ),
                (
                    "adapters/model-provider/src/profile.rs",
                    "MODEL_ENDPOINT_PROFILE_SCHEMA_VERSION_V2",
                ),
            ],
        ),
        (
            "Authority grant / authorization decision",
            vec![
                (
                    "crates/authority/src/document.rs",
                    "AUTHORITY_GRANT_SCHEMA_VERSION_V5",
                ),
                (
                    "crates/authority/src/document.rs",
                    "AUTHORITY_DECISION_SCHEMA_VERSION_V3",
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
                    "SERVING_EXECUTION_RECORD_SCHEMA_VERSION",
                ),
                (
                    "crates/persistence/src/peer.rs",
                    "SERVING_EXECUTION_TOMBSTONE_SCHEMA_VERSION",
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
            "Runtime task workflow document",
            vec![("crates/runtime/src/tasks/duration.rs", "SCHEMA_VERSION")],
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
            "CLI JSON output",
            vec![("apps/cli/src/output.rs", "JSON_OUTPUT_SCHEMA_VERSION")],
        ),
        (
            "Managed resource request / inventory",
            vec![("crates/capability/src/managed.rs", "MANAGED_SCHEMA_VERSION")],
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
    let status = read(root()?.join("docs/reference/versions.md"))?;
    let mut documented = BTreeMap::new();
    for line in status.lines().filter(|line| line.starts_with("| ")) {
        let cells: Vec<_> = line.split('|').map(str::trim).collect();
        let [_, family, version, behavior, _] = cells.as_slice() else {
            return Err("version row needs family/version/read behavior".into());
        };
        if *family == "Contract or durable family" || *family == "---" {
            continue;
        }
        assert!(
            documented
                .insert((*family).to_owned(), (*version).to_owned())
                .is_none(),
            "duplicate version family"
        );
        assert!(!behavior.is_empty(), "missing read behavior");
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
            "operator/starter.json"
            | "operator/process.json"
            | "operator/model.json"
            | "operator/peer-placement.json" => {
                let request: milkdrift_control_protocol::tasks::CreateWorkflowRequest =
                    milkdrift_control_protocol::decode_json(&bytes)?;
                assert!(!request.tasks.is_empty(), "{relative}");
            }
            "operator/daemon.toml" | "operator/execution-only.toml" => {
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
            "local-model/continuation-request.example.json" | "operator/direct-model-task.json" => {
                milkdrift_model::ModelTaskRequestDocument::from_json(&bytes)?;
            }
            "operator/release-notes/draft.txt"
            | "operator/release-notes/review.txt"
            | "operator/release-notes/meeting.txt" => {
                // Raw UTF-8 prompts become one text message. The daemon/CLI authoring test
                // separately sends these exact files through the public construction route.
                let prompt = std::str::from_utf8(&bytes)?;
                let document = serde_json::json!({"schema_version":1,"request":{
                    "messages":[{"role":"user","parts":[{"type":"text","text":prompt}],"tool_call_id":null}],
                    "tools":[],"structured_output":null,"session":{"type":"fresh"},
                    "reasoning":null,"maximum_output_units":512,"streaming":false,"extensions":{}
                }});
                milkdrift_model::ModelTaskRequestDocument::from_json(&serde_json::to_vec(
                    &document,
                )?)?;
            }
            "operator/release-notes/harbor-brief.txt"
            | "operator/release-notes/lantern-brief.txt" => {
                std::str::from_utf8(&bytes)?;
                let upload = milkdrift_control_protocol::InputUploadRequest::from_content(
                    "host:example".into(),
                    "example-brief".into(),
                    "text/plain".into(),
                    "restricted".into(),
                    &bytes,
                )?;
                assert_eq!(upload.content()?, bytes);
            }
            "operator/direct-process-inputs.json" => {
                let inputs: Vec<milkdrift_capability::InputReference> =
                    serde_json::from_slice(&bytes)?;
                assert_eq!(inputs.len(), 1);
            }
            "managed-linux/slotbook.json" => {
                milkdrift_managed_linux::LinuxRecipe::from_json(&bytes)?;
            }
            "managed-linux/Containerfile" | "managed-linux/Containerfile.containerignore" => {
                // Podman owns these formats. The documented real image build reads both together;
                // a local string assertion cannot qualify its build or ignore-file semantics.
                std::str::from_utf8(&bytes)?;
            }
            "operator/README.md" | "external-evidence/README.md" | "managed-linux/README.md" => {}
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
    use milkdrift_authority::CapabilityAuthorityScope;
    use milkdrift_capability::{CapabilityId, CapabilityRequirement, OperationId};
    let examples = root()?.join("examples/operator");
    let configuration: milkdrift_daemon::DaemonConfig =
        toml::from_str(&read(examples.join("daemon.toml"))?)?;
    let grant = &configuration
        .actors
        .first()
        .ok_or("operator actor absent")?
        .authority
        .resources
        .capability;
    let request: milkdrift_control_protocol::tasks::CreateWorkflowRequest =
        milkdrift_control_protocol::decode_json(&fs::read(examples.join("process.json"))?)?;
    for task in request.tasks {
        let requirement =
            CapabilityRequirement::new(OperationId::new(task.definition.capability.operation)?)
                .exact(CapabilityId::new(task.definition.capability.id)?)
                .maximum_side_effect(milkdrift_capability::SideEffectClass::ReadOnly)
                .trust_zone(milkdrift_capability::TrustZone::new("local-process")?);
        assert!(CapabilityAuthorityScope::requirement_envelope(&requirement)?.is_subset_of(grant));
    }
    Ok(())
}

#[test]
fn control_reference_json_uses_current_wire_readers_and_versions() -> TestResult {
    use milkdrift_control_protocol::{
        ErrorEnvelope, ProtocolVersion, ResponseEnvelope, VersionRequest, decode_json,
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
            if value.get("code").is_some() {
                decode_json::<ErrorEnvelope>(bytes)?;
            } else if value.get("value").is_some() {
                decode_json::<ResponseEnvelope<serde_json::Value>>(bytes)?;
            } else {
                decode_json::<VersionRequest>(bytes)?;
            }
        } else if value.get("tasks").is_some() {
            decode_json::<milkdrift_control_protocol::tasks::CreateWorkflowRequest>(bytes)?;
        } else if value.get("changes").is_some() {
            decode_json::<milkdrift_control_protocol::tasks::ChangeWorkflowRequest>(bytes)?;
        } else if value.get("text").is_some() {
            decode_json::<milkdrift_control_protocol::tasks::TaskUploadRequest>(bytes)?;
        } else if value.get("installation").is_some() {
            let request: milkdrift_capability::managed::ManagedRequest = decode_json(bytes)?;
            request.validate()?;
        } else {
            assert_eq!(
                value
                    .get("schema_version")
                    .and_then(serde_json::Value::as_u64),
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
    assert!(codec.contains("protocol != ProtocolVersion::V1_6"));
    let client = read(root()?.join("adapters/peer-http/src/client.rs"))?;
    assert!(
        client.contains("peer response envelope does not match the negotiated protocol version")
    );
    Ok(())
}

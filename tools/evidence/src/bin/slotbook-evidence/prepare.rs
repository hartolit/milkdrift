use super::Prepare;
use milkdrift_evidence::{
    EvidenceResult,
    application::{ensure, run_command, write_private},
};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

// The example contract admits a 4 MiB immutable native candidate and its escaped worker report.
// These are reviewed example allocations, not platform-wide defaults or support ceilings.
pub(super) const CANDIDATE_BYTES: u64 = 4 * 1024 * 1024;
pub(super) const INTERNAL_ARTIFACT_BYTES: u64 = 128 * 1024 * 1024;
pub(super) fn write(root: &Path, name: &str, value: &Value) -> EvidenceResult<PathBuf> {
    let path = root.join(name);
    write_private(&path, &serde_json::to_vec(value)?)?;
    Ok(path)
}
pub(super) fn local(cli: &Path, args: &[String]) -> EvidenceResult<Value> {
    let output = run_command(
        Command::new(cli).arg("--json").args(args),
        None,
        Duration::from_secs(30),
    )?;
    ensure(
        output.status.success(),
        &format!(
            "authoring command failed: {} {}",
            output.stdout, output.stderr
        ),
    )?;
    let result: Value = serde_json::from_str(&output.stdout)?;
    Ok(result["value"].clone())
}
pub(super) fn argument(value: impl AsRef<Path>) -> String {
    value.as_ref().display().to_string()
}
pub(super) fn private_directory(path: &Path) -> EvidenceResult {
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)?;
    Ok(())
}
pub(super) fn credential(path: &Path) -> EvidenceResult {
    use std::io::Read;
    let mut random = [0; 32];
    fs::File::open("/dev/urandom")?.read_exact(&mut random)?;
    let token = blake3::hash(&random).to_hex().to_string();
    write_private(path, token.as_bytes()).map(|_| ())
}
pub(super) fn artifact_schema() -> Value {
    json!({"id":"milkdrift.artifact-reference","version":1})
}
pub(super) fn port(direction: &str, schema: Value, binding: Value, required: bool) -> Value {
    json!({"direction":direction,"schema":schema,"binding":binding,"required":required})
}
pub(super) fn node(identity: &str, kind: Value, incoming: bool, outgoing: bool) -> Value {
    json!({"id":identity,"kind":kind,"control_inputs":if incoming {vec!["in"]} else {vec![]},"control_outputs":if outgoing {vec!["out"]} else {vec![]},"data_inputs":{},"data_outputs":{}})
}
pub(super) fn edge(
    identity: &str,
    source: &str,
    target: &str,
    kind: &str,
    from: &str,
    to: &str,
) -> Value {
    json!({"id":identity,"kind":kind,"source_node":source,"source_port":from,"target_node":target,"target_port":to})
}
pub(super) fn task(
    identity: &str,
    requirement: Value,
    name: &str,
    literal: Value,
    schema: Value,
    outputs: &[&str],
) -> Value {
    let context = json!({"ancestor_depth":null,"artifact_selector":null,"budget":{"max_artifact_bytes":33554432,"max_bytes":262144,"max_items":64,"max_model_input_units":null},"exclude_categories":["raw_progress","tool_trace","verbose_command_output","prior_prompt"],"fail_closed":true,"include_categories":["direct_input"],"include_direct_inputs":true,"ordering":"causal_kind_source","selected_nodes":[],"selected_roles":[],"session":"fresh","truncation":"omit_oversized"});
    let mut result = node(
        identity,
        json!({"type":"task","config":{"requirement":requirement,"context_policy":context}}),
        true,
        true,
    );
    result["data_inputs"][name] = port(
        "Input",
        schema,
        json!({"type":"literal","value":literal}),
        true,
    );
    for name in outputs {
        result["data_outputs"][*name] = port("Output", artifact_schema(), Value::Null, false);
    }
    result
}
pub(super) fn worker(identity: &str, requirement: &Value, argv: &[&str], capture: bool) -> Value {
    task(
        identity,
        requirement.clone(),
        "command",
        json!({"argv":argv,"stdout_artifact":capture}),
        json!({"id":"milkdrift.managed.worker","version":2}),
        &["worker_result", "stdout"],
    )
}
fn effect(
    identity: &str,
    requirement: &Value,
    operation: &str,
    selected: &str,
    version: u64,
) -> Value {
    let mut req = requirement.clone();
    req["categories"] = json!([{"type":"tool"}]);
    req["exact_capability"] = json!("milkdrift.resources");
    req["maximum_side_effect"] = json!("idempotent_write");
    req["operation"] = json!(operation);
    let mut result = task(
        identity,
        req,
        "target",
        json!({"schema_version":3,"command":identity,"installation":"slotbook-test","expected_version":version}),
        json!({"id":"milkdrift.managed.command","version":3}),
        &["resource_result"],
    );
    result["data_inputs"][selected] = port("Input", artifact_schema(), Value::Null, true);
    result
}
pub(super) fn run(args: Prepare) -> EvidenceResult {
    let cli = args.cli.canonicalize()?;
    let verifier = args.verifier.canonicalize()?;
    let parent = args
        .root
        .parent()
        .ok_or("example parent absent")?
        .canonicalize()?;
    let root = parent.join(args.root.file_name().ok_or("example root absent")?);
    private_directory(&root)?;
    credential(&root.join("service.token"))?;
    write_private(&root.join("clock"), b"2027-04-10T09:00:00Z")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(root.join("clock"), fs::Permissions::from_mode(0o444))?;
    }
    // Cargo may hardlink its output into deps. Approve a private immutable copy so rebuilds
    // cannot replace the operator's trusted executable or fail the manager's link-count check.
    let verifier_bytes = fs::read(&verifier)?;
    let verifier = root.join("trusted-verifier");
    write_private(&verifier, &verifier_bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&verifier, fs::Permissions::from_mode(0o555))?;
    }
    let verifier_digest = format!("b3_{}", blake3::hash(&verifier_bytes));
    let policy = json!({"schema_version":1,"required_checks":["public-availability","authenticated-mutation","capacity-and-intervals","durable-bookings","cancellation-policy","exact-deployment"],"verifier":verifier_digest,"producer":"trusted:slotbook-verifier","maximum_candidate_bytes":CANDIDATE_BYTES,"validity_ms":3600000});
    let policy_path = write(&root, "policy.json", &policy)?;
    let policy_digest = local(
        &cli,
        &[
            "blueprint".into(),
            "effect-policy".into(),
            argument(&policy_path),
        ],
    )?["digest"]
        .clone();
    let requirement = json!({"cancellation_required":false,"categories":[{"type":"process"}],"exact_capability":"managed.slotbook-build.worker","maximum_side_effect":"non_idempotent_write","operation":"workspace.execute","provider_profile":null,"required_features":[],"streaming":null,"trust_zones":[]});
    let nodes = vec![
        node(
            "repair-window",
            json!({"type":"wait","duration_ms":45000}),
            false,
            true,
        ),
        worker(
            "repair.begin",
            &requirement,
            &["/bin/test", "-r", "/workspace/app"],
            false,
        ),
        worker(
            "repair.end",
            &requirement,
            &["/bin/cat", "/workspace/app"],
            true,
        ),
        effect(
            "verify-candidate",
            &requirement,
            "resource.evaluate_candidate",
            "candidate",
            args.target_version,
        ),
        effect(
            "publish-candidate",
            &requirement,
            "resource.publish_candidate",
            "evaluation",
            args.target_version,
        ),
        node(
            "done",
            json!({"type":"terminal","outcome":"success"}),
            true,
            false,
        ),
    ];
    let mut edges = Vec::new();
    for (i, pair) in nodes.windows(2).enumerate() {
        edges.push(edge(
            &format!("control-{i}"),
            pair[0]["id"].as_str().ok_or("node identity absent")?,
            pair[1]["id"].as_str().ok_or("node identity absent")?,
            "control",
            "out",
            "in",
        ));
    }
    edges.extend([
        edge(
            "candidate-input",
            "repair.end",
            "verify-candidate",
            "data",
            "stdout",
            "candidate",
        ),
        edge(
            "evaluation-input",
            "verify-candidate",
            "publish-candidate",
            "data",
            "resource_result",
            "evaluation",
        ),
    ]);
    let mutations: Vec<_> = nodes
        .into_iter()
        .map(|node| json!({"type":"add_node","node":node}))
        .chain(
            edges
                .into_iter()
                .map(|edge| json!({"type":"add_edge","edge":edge})),
        )
        .collect();
    write(&root, "method-mutations.json", &json!(mutations))?;
    write(
        &root,
        "adaptation-scope.json",
        &json!({"node_prefix":"repair.","maximum_nodes":8,"maximum_revisions":4,"requirements":[requirement]}),
    )?;
    local(
        &cli,
        &[
            "blueprint".into(),
            "create".into(),
            argument(root.join("method-mutations.json")),
            "--workflow".into(),
            "slotbook".into(),
            "--author".into(),
            "human:operator".into(),
            "--output".into(),
            argument(root.join("base.json")),
        ],
    )?;
    let agreement = local(
        &cli,
        &[
            "blueprint".into(),
            "govern".into(),
            argument(root.join("base.json")),
            "--scope".into(),
            argument(root.join("adaptation-scope.json")),
            "--name".into(),
            "slotbook-agreement".into(),
            "--effect-policy".into(),
            policy_digest.as_str().ok_or("policy digest absent")?.into(),
            "--author".into(),
            "human:operator".into(),
            "--output".into(),
            argument(root.join("governed.json")),
        ],
    )?["agreement"]
        .clone();
    let limits =
        json!({"memory_bytes":536870912,"cpu_percent":100,"pids":64,"temporary_bytes":16777216});
    write(
        &root,
        "protected-recipe.json",
        &json!({"schema_version":2,"kind":"protected_service","name":"slotbook-test","image":args.image,"limits":limits,"startup_ms":30000,"shutdown_ms":10000,"port":19848,"application":{"resource":"pottery","capacity":2},"token_file":root.join("service.token"),"token_digest":format!("b3_{}",blake3::hash(&fs::read(root.join("service.token"))?)),"clock_file":root.join("clock"),"agreement":agreement,"policy":policy,"verifier_executable":verifier,"verifier_digest":verifier_digest,"verification_timeout_ms":180000,"data_disposition":"preserve"}),
    )?;
    write(
        &root,
        "worker-recipe.json",
        &json!({"schema_version":2,"name":"slotbook-build","worker_image":args.image,"worker_network":"none","worker_limits":limits,"task_timeout_ms":30000,"output_bytes":CANDIDATE_BYTES,"minimum_free_bytes":16777216,"data_disposition":"preserve","model_service":{"type":"disabled"}}),
    )?;
    let repair = worker(
        "repair.begin",
        &requirement,
        &["/bin/cp", "/fixtures/slotbook", "/workspace/app"],
        false,
    );
    let investigate = worker(
        "repair.investigate",
        &requirement,
        &["/bin/test", "-s", "/workspace/app"],
        false,
    );
    write(
        &root,
        "repair-mutations.json",
        &json!([{"type":"replace_node","node":repair},{"type":"add_node","node":investigate},{"type":"replace_edge","edge":edge("control-1","repair.begin","repair.investigate","control","out","in")},{"type":"add_edge","edge":edge("investigated","repair.investigate","repair.end","control","out","in")}]),
    )?;
    println!(
        "{}",
        json!({"root":root,"agreement":agreement,"policy":policy_digest,"candidate_lane":"compiled seeded fixtures, not model output"})
    );
    Ok(())
}

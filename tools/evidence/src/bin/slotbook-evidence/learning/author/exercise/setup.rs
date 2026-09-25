//! Scratch files become maintained tools only through an exact image and a verified recipe update.
use super::super::{prepare, save_config};
use super::{
    Arguments, Caller, EvidenceResult, Expected, Session, Value, ensure, fs, json, load, number,
    reference_args, text, upload, worker, write,
};
use milkdrift_evidence::application::{run_command, write_private};
use std::{process::Command, time::Duration};

pub(super) fn run(args: &Arguments, s: &mut Session, slots: &[Value]) -> EvidenceResult {
    let source = slots.last().ok_or("source workspace absent")?;
    let workspace = text(&source["worker"])?;
    let dir = s.root.join("learning-05");
    if dir.join("setup-improvement.json").exists() {
        let report = load(dir.join("setup-improvement.json"))?;
        upload(
            s,
            "tools-improvement-result",
            &serde_json::to_vec(&report)?,
            "application/json",
        )?;
        return Ok(());
    }
    worker(
        s,
        "tools-seed-app",
        workspace,
        &["/bin/cp", "/fixtures/slotbook", "/workspace/app"],
        false,
    )?;
    worker(
        s,
        "tools-scratch-install",
        workspace,
        &[
            "/bin/cp",
            "/fixtures/slotbook-workspace",
            "/workspace/scratch-tool",
        ],
        false,
    )?;
    let scratch = worker(
        s,
        "tools-scratch-inspect",
        workspace,
        &["/workspace/scratch-tool", "inspect", "/workspace"],
        true,
    )?;
    let before = worker(
        s,
        "tools-before-update",
        workspace,
        &["/bin/cat", "/workspace/app"],
        true,
    )?;
    let before_artifact = output(&before, "application/octet-stream")?;
    let image_dir = dir.join("tool-image");
    fs::create_dir_all(&image_dir)?;
    let tool_size = fs::metadata(&args.workspace_tool)?.len();
    ensure(
        tool_size > 0 && tool_size <= prepare::CANDIDATE_BYTES,
        "native tool exceeds the managed artifact capture bound",
    )?;
    let tool_bytes = fs::read(&args.workspace_tool)?;
    let tool_digest = json!(milkdrift_workspace::ContentDigest::for_bytes(&tool_bytes));
    let tool_manifest = upload(
        s,
        "tools-native-input-manifest",
        &serde_json::to_vec(&json!({"digest":tool_digest,"size_bytes":tool_size,
            "build_path":"slotbook-workspace"}))?,
        "application/json",
    )?;
    let tool_path = image_dir.join("slotbook-workspace");
    if !tool_path.exists() {
        write_private(&tool_path, &tool_bytes)?;
    }
    ensure(
        fs::read(&tool_path)? == tool_bytes,
        "recorded native tool differs",
    )?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&tool_path, fs::Permissions::from_mode(0o755))?;
    }
    let containerfile = format!(
        "FROM {}\nCOPY slotbook-workspace /usr/local/bin/slotbook-workspace\n",
        args.image
    );
    let build_input = upload(
        s,
        "tools-build-input",
        containerfile.as_bytes(),
        "text/plain",
    )?;
    let file = image_dir.join("Containerfile");
    if !file.exists() {
        write_private(&file, containerfile.as_bytes())?;
    }
    let image_id = image_dir.join("image-id");
    if !image_id.exists() {
        let built = run_command(
            Command::new("/usr/bin/podman")
                .args(["build", "--pull=never", "--iidfile"])
                .arg(&image_id)
                .arg("-f")
                .arg(&file)
                .arg(&image_dir),
            None,
            Duration::from_secs(120),
        )?;
        write_private(&image_dir.join("build.stdout"), built.stdout.as_bytes())?;
        write_private(&image_dir.join("build.stderr"), built.stderr.as_bytes())?;
        ensure(built.status.success(), "recorded tool image build failed")?;
    }
    let image = fs::read_to_string(image_id)?.trim().to_owned();
    let mut recipe = load(dir.join(format!("{workspace}.json")))?;
    recipe["name"] = json!("learn-tools-v2");
    recipe["worker_image"] = json!(image);
    let parsed = milkdrift_managed_linux::LinuxRecipe::from_json(&serde_json::to_vec(&recipe)?)?;
    let reference = serde_json::to_value(parsed.reference()?)?;
    let recipe_path = write(s, "learn-tools-v2.json", &recipe)?;
    // A worker-supplied manager mount is outside the finite recipe contract, even before approval.
    let mut expansion = recipe.clone();
    expansion["manager_mount"] = json!("/private/manager");
    ensure(
        milkdrift_managed_linux::LinuxRecipe::from_json(&serde_json::to_vec(&expansion)?).is_err(),
        "recipe reader accepted manager-access expansion",
    )?;
    write(s, "tools-refused-manager-expansion.json", &expansion)?;
    s.stop()?;
    let mut config: Value = toml::from_str(&fs::read_to_string(s.root.join("host/daemon.toml"))?)?;
    let recipes = config["adapters"]["managed_linux"]["recipes"]
        .as_array_mut()
        .ok_or("recipe catalog absent")?;
    if !recipes.contains(&json!(recipe_path)) {
        recipes.push(json!(recipe_path));
    }
    save_config(&s.root, &config)?;
    s.start()?;
    let stage_receipt = s.resource(
        "tools-stage-apply",
        "apply",
        0,
        reference_args(&reference)?,
        "learn-tools-stage",
        Expected::Success,
    )?;
    let stage = s.resource(
        "tools-stage-state",
        "inspect",
        0,
        vec![],
        "learn-tools-stage",
        Expected::Success,
    )?;
    ensure(
        stage["pending"].is_null() && number(&stage["generation"])? > 0,
        "tool candidate did not verify in its fresh installation",
    )?;
    let staged_tool = worker(
        s,
        "tools-stage-native-bytes",
        "learn-tools-stage",
        &["/bin/cat", "/usr/local/bin/slotbook-workspace"],
        true,
    )?;
    let tool = output(&staged_tool, "application/octet-stream")?;
    ensure(
        tool["digest"] == tool_digest && tool["size_bytes"] == tool_size,
        "staged native bytes differ from the recorded build input",
    )?;
    worker(
        s,
        "tools-stage-knowledge",
        "learn-tools-stage",
        &[
            "/usr/local/bin/slotbook-workspace",
            "knowledge",
            "/workspace",
        ],
        false,
    )?;
    worker(
        s,
        "tools-stage-candidate",
        "learn-tools-stage",
        &["/bin/cp", "/fixtures/slotbook", "/workspace/app"],
        false,
    )?;
    let staged = worker(
        s,
        "tools-stage-inspect",
        "learn-tools-stage",
        &["/usr/local/bin/slotbook-workspace", "inspect", "/workspace"],
        true,
    )?;
    let old_state_path = dir.join("tools-old-state.json");
    let current = if old_state_path.exists() {
        load(old_state_path)?
    } else {
        let current = s.resource(
            "tools-old-state",
            "inspect",
            0,
            vec![],
            workspace,
            Expected::Success,
        )?;
        write(s, "tools-old-state.json", &current)?;
        current
    };
    let mut arguments = args![
        "resource",
        "--installation",
        workspace,
        "--expected-version",
        number(&current["version"])?,
        "update"
    ];
    arguments.extend(reference_args(&reference)?);
    arguments.push("--allow-interruption".into());
    s.call(
        "learner-cannot-update-recipe-with-interruption",
        arguments,
        Expected::Refused,
        Caller::Learner,
    )?;
    s.resource(
        "tools-approved-update",
        "update",
        number(&current["version"])?,
        reference_args(&reference)?,
        workspace,
        Expected::Refused,
    )?;
    let mut replacement = reference_args(&reference)?;
    replacement.push("--allow-interruption".into());
    let update_receipt = s.resource(
        "tools-approved-drained-update",
        "update",
        number(&current["version"])?,
        replacement,
        workspace,
        Expected::Success,
    )?;
    let updated = s.resource(
        "tools-updated-state",
        "inspect",
        0,
        vec![],
        workspace,
        Expected::Success,
    )?;
    ensure(
        updated["pending"].is_null()
            && number(&updated["generation"])? > number(&current["generation"])?,
        "recipe update did not activate a verified new generation",
    )?;
    let maintained = worker(
        s,
        "tools-maintained-inspect",
        workspace,
        &["/usr/local/bin/slotbook-workspace", "inspect", "/workspace"],
        true,
    )?;
    let after = worker(
        s,
        "tools-after-update",
        workspace,
        &["/bin/cat", "/workspace/app"],
        true,
    )?;
    ensure(
        output(&after, "application/octet-stream")?["digest"] == before_artifact["digest"],
        "tool update changed working candidate bytes",
    )?;
    let learning_result = load(dir.join("result.json"))?;
    let note = format!(
        "Maintained tool recipe {} at generation {} was verified in a fresh installation before activation. Candidate bytes were preserved. Selected method: {}. Comparison: agent:slotbook-evaluator/{}; promoted: {}. Future tasks must select this guidance explicitly.",
        text(&reference["digest"])?,
        number(&updated["generation"])?,
        text(&learning_result["selected_method"])?,
        super::key(args, "learning-comparison"),
        learning_result["promoted"]
    );
    worker(
        s,
        "tools-record-guidance",
        workspace,
        &[
            "/usr/local/bin/slotbook-workspace",
            "note",
            "/workspace",
            &note,
        ],
        false,
    )?;
    let updated_guidance = worker(
        s,
        "tools-export-guidance",
        workspace,
        &["/bin/cat", "/workspace/KNOWLEDGE.md"],
        true,
    )?;
    let updated_reference = output(&updated_guidance, "application/octet-stream")?;
    let destination = dir.join("KNOWLEDGE.updated.md");
    if !destination.exists() {
        s.ok(
            "tools-guidance-download",
            args![
                "artifact",
                "get",
                text(&updated_reference["identity"])?,
                "--output",
                destination.display()
            ],
        )?;
    }
    let guidance = upload(
        s,
        "tools-updated-guidance",
        &fs::read(destination)?,
        "text/plain",
    )?;
    let mut selection = load(dir.join("learning-select.json"))?["selection"].clone();
    let original_guidance = selection["guidance"].clone();
    selection["guidance"] = guidance.clone();
    selection["method"] = learning_result["selected_method"].clone();
    selection["supersedes"] = super::receipt("learning-select", super::OPERATOR);
    selection["approval"] = if learning_result["promoted"] == true {
        super::study_receipt(
            args,
            "learning-automatic-promotion",
            "agent:slotbook-evaluator",
        )
    } else {
        Value::Null
    };
    let mut wrong_workspace = selection.clone();
    wrong_workspace["workspace"] = slots[0]["worker"].clone();
    super::command(
        s,
        "learning-refused-cross-workspace-supersession",
        json!({"type":"select","selection":wrong_workspace}),
        Caller::Operator,
        Expected::Refused,
    )?;
    let next = super::command(
        s,
        "learning-next-selection",
        json!({"type":"select","selection":selection}),
        Caller::Operator,
        Expected::Success,
    )?;
    s.stop()?;
    s.start()?;
    let previous = super::command(
        s,
        "learning-original-selection-after-update",
        json!({"type":"inspect","receipt":super::receipt("learning-select",super::OPERATOR)}),
        Caller::Operator,
        Expected::Success,
    )?;
    ensure(
        previous["selection"]["guidance"] == original_guidance
            && guidance["digest"] != original_guidance["digest"],
        "future knowledge selection rewrote the original guidance",
    )?;
    write(
        s,
        "knowledge-update-result.json",
        &json!({"previous":previous,"next":next,"proposal_context":load(dir.join(format!("{}.json",super::key(args,"proposal-observation"))))?["attempt"]["context_manifest"]}),
    )?;
    let report = json!({"scratch":scratch,"native_input":tool,"native_manifest":tool_manifest,"build_input":build_input,"image":image,"recipe":reference,"fresh_stage_receipt":stage_receipt,"fresh_stage":stage,"staged_observation":staged,"old_setup":current,"activation_receipt":update_receipt,"activated_setup":updated,"maintained_observation":maintained,"preserved_candidate":before_artifact,"relationship":"The selected method and its prior invocation receipts are unchanged. The recipe has a separately recorded generation; no active context receives new guidance or a new publication implicitly.","limits":["Base OCI bytes must remain preloaded; no upstream rebuild availability is claimed.","Working bookings and notes require data backup; rebuilding the tool image does not restore mutable data."]});
    write(s, "setup-improvement.json", &report)?;
    upload(
        s,
        "tools-improvement-result",
        &serde_json::to_vec(&report)?,
        "application/json",
    )?;
    Ok(())
}

fn output<'a>(value: &'a Value, media: &str) -> EvidenceResult<&'a Value> {
    value["observations"]
        .as_array()
        .ok_or("worker observations absent")?
        .iter()
        .filter(|o| o["category"] == "artifact")
        .map(|o| &o["event"]["kind"]["reference"])
        .find(|a| a["media_type"] == media)
        .ok_or_else(|| "worker artifact absent".into())
}

//! Bounded live source development; proposal bytes and trusted observations remain distinct.
mod proposal;
use super::{
    InvocationMode, Prepare, Qualify,
    client::{Caller, Expected, Session, load, number, text},
    prepare, qualification,
};
use milkdrift_evidence::{EvidenceResult, application::ensure};
use serde_json::{Value, json};
use std::{fs, path::PathBuf, thread, time::Duration};

const MODEL: &str = "slotbook-development-model";
const AUTHOR: &str = "agent:source-author";

#[derive(clap::Args)]
pub(super) struct Arguments {
    /// New private directory; existing evidence is never replaced.
    #[arg(long)]
    root: PathBuf,
    #[arg(long, default_value = "target/debug/milkdrift")]
    cli: PathBuf,
    #[arg(long, default_value = "target/debug/milkdrift-daemon")]
    daemon: PathBuf,
    #[arg(long, default_value = "target/debug/slotbook-verifier")]
    verifier: PathBuf,
    /// Approved exact image with rustc, sh and ordinary build tools. No network is granted.
    #[arg(long)]
    image: String,
    /// Explicit operator-reviewed local model profile supporting structured output.
    #[arg(long)]
    model_profile: PathBuf,
    #[arg(long, default_value_t = 19768)]
    port: u16,
    /// Separately labelled repair from /fixtures/slotbook-seeded in the approved image.
    #[arg(long)]
    seeded_initial: bool,
    /// Remove only these installations, preserving their source, output, data and evidence.
    #[arg(long)]
    remove_disposable: bool,
    /// Continue the retained attempt sequence without repeating completed model calls.
    #[arg(long)]
    resume: bool,
}

pub(super) fn run(args: Arguments) -> EvidenceResult {
    if args.resume {
        ensure(
            load(args.root.join("development-inputs.json"))? == declared_inputs(&args)?,
            "resume image, model profile bytes, lane or port differ from the declared study",
        )?;
        let mut s = Session::resume(
            &args.root,
            &args.cli,
            &args.daemon,
            args.port,
            &args.root.join("evidence"),
            &args.root.join("host/operator.token"),
        )?;
        s.protected = load(args.root.join("evidence/bootstrap.json"))?["recipe"].clone();
        s.worker = load(args.root.join("evidence/worker-preview.json"))?["recipe"].clone();
        ensure(
            load(args.root.join("worker-recipe.json"))?["worker_image"] == args.image,
            "resume image differs from the retained source worker",
        )?;
        let config: Value =
            toml::from_str(&fs::read_to_string(args.root.join("host/daemon.toml"))?)?;
        ensure(
            config["adapters"]["model_profiles"][0]["profile"]
                == json!(args.model_profile.canonicalize()?),
            "resume model configuration differs",
        )?;
        configure_author(&s)?;
        s.start()?;
        let result = exercise(&args, &mut s);
        let stopped = s.stop();
        result?;
        return stopped;
    }
    prepare::run(Prepare {
        root: args.root.clone(),
        cli: args.cli.clone(),
        image: args.image.clone(),
        verifier: args.verifier.clone(),
        target_version: 6,
    })?;
    prepare::write(
        &args.root,
        "development-inputs.json",
        &declared_inputs(&args)?,
    )?;
    // The source worker compiles actual returned Rust, unlike the fixture-copy qualifier.
    let worker_path = args.root.join("worker-recipe.json");
    let mut worker = load(&worker_path)?;
    worker["worker_limits"] = json!({"memory_bytes":2147483648u64,"cpu_percent":400,"pids":256,"temporary_bytes":268435456});
    worker["task_timeout_ms"] = json!(300000);
    fs::write(worker_path, serde_json::to_vec(&worker)?)?;
    let mut s = Session::prepare(&Qualify {
        root: args.root.clone(),
        cli: args.cli.clone(),
        daemon: args.daemon.clone(),
        port: args.port,
        published: false,
        invocation_mode: InvocationMode::Direct,
        candidate: PathBuf::new(),
        drain_before_renewal: false,
    })?;
    configure(&args, &s)?;
    configure_author(&s)?;
    s.start()?;
    let result = exercise(&args, &mut s);
    let stopped = s.stop();
    result?;
    stopped
}

fn declared_inputs(args: &Arguments) -> EvidenceResult<Value> {
    let profile = args.model_profile.canonicalize()?;
    Ok(
        json!({"image":args.image,"model_profile":profile,"model_profile_digest":format!("b3_{}",blake3::hash(&fs::read(&profile)?)),"seeded_initial":args.seeded_initial,"port":args.port}),
    )
}

fn configure(args: &Arguments, s: &Session) -> EvidenceResult {
    let profile_path = args.model_profile.canonicalize()?;
    let profile = load(&profile_path)?;
    let endpoint = url::Url::parse(text(&profile["base_url"])?)?;
    let destination = format!(
        "{}:{}",
        endpoint.host_str().ok_or("model host absent")?,
        endpoint
            .port_or_known_default()
            .ok_or("model port absent")?
    );
    let path = s.root.join("host/daemon.toml");
    let mut config: Value = toml::from_str(&fs::read_to_string(&path)?)?;
    config["adapters"]["model_profiles"] = json!([{"capability_id":MODEL,"profile":profile_path}]);
    let actor = &mut config["actors"][0];
    actor["authority"]["resources"]["capability"]["identities"]["values"]
        .as_array_mut()
        .ok_or("capability grant absent")?
        .push(json!(MODEL));
    actor["authority"]["resources"]["network"] =
        json!({"profiles":[profile["identity"]],"destinations":[destination]});
    actor["authority"]["budget"]["duration_ms"] = json!(3600000);
    fs::write(path, toml::to_string_pretty(&config)?)?;
    Ok(())
}

fn configure_author(s: &Session) -> EvidenceResult {
    let path = s.root.join("host/daemon.toml");
    let mut config: Value = toml::from_str(&fs::read_to_string(&path)?)?;
    let mut author = config["actors"][0].clone();
    author["actor"] = json!(AUTHOR);
    author["credential_ref"] = json!("credential:source-author");
    author["grant_id"] = json!("grant:source-author");
    author["preset"] = json!("advisor");
    let resources = &mut author["authority"]["resources"];
    resources["workflow_run"] = json!({"type":"workflow","workflow":"slotbook"});
    resources["capability"]["identities"] =
        json!({"type":"only","values":["managed.slotbook-build.worker","milkdrift.resources"]});
    resources["capability"]["operations"] = json!({"type":"only","values":["workspace.execute","resource.evaluate_candidate","resource.publish_candidate"]});
    resources["filesystem"] = json!([]);
    resources["network"] = json!({"profiles":[],"destinations":[]});
    resources["artifacts"] = json!({"type":"deny_all"});
    let actors = config["actors"].as_array_mut().ok_or("actors absent")?;
    if let Some(existing) = actors.iter().find(|a| a["actor"] == AUTHOR) {
        ensure(existing == &author, "retained source author grant differs")?;
        return Ok(());
    }
    actors.push(author);
    let token = s.root.join("source-author.token");
    if !token.exists() {
        prepare::credential(&token)?;
    }
    config["secret_sources"]["credential:source-author"] = json!({"type":"file","path":token});
    fs::write(path, toml::to_string_pretty(&config)?)?;
    Ok(())
}

fn exercise(args: &Arguments, s: &mut Session) -> EvidenceResult {
    if s.root.join("development-result.json").exists() {
        return finish(
            s,
            &load(s.root.join("development-result.json"))?,
            args.remove_disposable,
        );
    }
    let mut installed = std::collections::BTreeMap::new();
    for (name, recipe) in [
        ("slotbook-test", s.protected.clone()),
        ("slotbook-build", s.worker.clone()),
    ] {
        s.resource(
            &format!("{name}-apply"),
            "apply",
            0,
            qualification::reference_args(&recipe)?,
            name,
            Expected::Success,
        )?;
        let ready = s.resource(
            &format!("{name}-ready"),
            "inspect",
            0,
            vec![],
            name,
            Expected::Success,
        )?;
        ensure(
            ready["pending"].is_null()
                && (ready["state"] == "stopped" || ready["state"] == "running"),
            "installation did not finish preparation",
        )?;
        installed.insert(name, ready);
    }
    let setup_path = s.root.join("development-setup.json");
    if !setup_path.exists() {
        // No source run starts before this checkpoint. A resumed preparation may have
        // produced its first successful inspections in the new session's evidence directory.
        s.write("development-setup.json", &json!(installed))?;
    }
    let setup = load(setup_path)?;
    crate::preservation::unchanged(
        installed
            .get("slotbook-build")
            .ok_or("worker observation absent")?,
        &setup["slotbook-build"],
    )?;
    let version = number(&setup["slotbook-test"]["version"])?;
    let mut revision = load(s.root.join("governed.json"))?["revision"].clone();
    ensure(
        revision["semantic"]["nodes"]["verify-candidate"]["data_inputs"]["target"]["binding"]["value"]
            ["expected_version"]
            == version,
        "target version differs from the frozen method",
    )?;
    for file in ["base.json", "governed.json"] {
        s.ok(
            &format!("import-{file}"),
            args!["blueprint", "import", s.root.join(file).display()],
        )?;
    }
    let mut selected = json!({"lane":if args.seeded_initial {"seeded repair; no observed model planning failure"} else {"live source development; no candidate supplied"}});
    if args.seeded_initial {
        let input = s.root.join("seed-inputs.json");
        if !input.exists() {
            s.write("seed-inputs.json", &json!([{"name":"command","value":{"type":"inline","value":{"argv":["/bin/sh","-c","cp /fixtures/slotbook-seeded /workspace/app && cat /workspace/app"],"stdout_artifact":true}}}]))?;
        }
        let execution = s.invoke(
            "development-seed",
            "managed.slotbook-build.worker",
            "workspace.execute",
            &input,
            Caller::Operator,
        )?;
        let observed = s.ok(
            "development-seed-wait",
            args!["invocation", "wait", execution],
        )?;
        let candidate = observed["value"]["observations"]
            .as_array()
            .ok_or("seed observations absent")?
            .iter()
            .find(|o| o["category"] == "artifact" && o["event"]["kind"]["name"] == "stdout")
            .ok_or("seed candidate absent")?["event"]["kind"]["reference"]
            .clone();
        let evaluated = qualification::target(
            s,
            "seed-evaluate",
            "evaluate",
            version,
            qualification::candidate_args(&candidate, "identity")?,
            Expected::Success,
        )?;
        let evidence = qualification::evidence(
            s,
            "seed-evidence",
            text(&evaluated["evaluation"]["identity"])?,
        )?;
        ensure(
            evidence["complete"] == true && !qualification::passed(&evidence),
            "seeded candidate did not fail the fixed checks",
        )?;
        selected["seeded_candidate"] = candidate;
        selected["trusted_evaluation"] = evidence;
    }
    let mut attempts = Vec::new();
    let mut accepted = false;
    for index in 1..=3 {
        let proposed = match proposal::generate(args, s, index, &revision, &selected)? {
            proposal::Outcome::Ready(path) => path,
            proposal::Outcome::Invalid(failure) => {
                selected = failure.clone();
                attempts.push(failure);
                continue;
            }
        };
        let submitted = s.call(
            &format!("source-{index}-proposal-submit"),
            args![
                "--expected-revision",
                text(&revision["id"])?,
                "proposal",
                "submit",
                proposed.display()
            ],
            Expected::Success,
            Caller::SourceAuthor,
        )?;
        // Invalid proposals are retained as negative evidence; do not substitute a mutation.
        ensure(
            submitted["status"] == "success",
            "model proposal was not authorized",
        )?;
        let next = text(&submitted["value"]["value"]["proposed_revision"])?;
        let revision_path = s.root.join(format!("source-{index}-revision.json"));
        if !revision_path.exists() {
            s.ok(
                &format!("source-{index}-revision"),
                args![
                    "blueprint",
                    "show",
                    next,
                    "--output",
                    revision_path.display()
                ],
            )?;
        }
        revision = load(revision_path)?["revision"].clone();
        let run = format!("slotbook-source-{index}");
        s.ok(
            &format!("source-{index}-start"),
            args!["run", "start", run, "slotbook", next],
        )?;
        let result = observe(s, &run, index)?;
        selected = json!({"run":result["run"]["run_id"],"terminal":result["run"]["terminal"],"attempts":result["attempts"],"omission":"Only selected compiler/verifier diagnostics and exact artifact references are supplied; full run inspection is retained separately."});
        accepted = result["run"]["terminal"] == "succeeded";
        attempts.push(result);
        if accepted {
            break;
        }
    }
    let target = qualification::inspect(s, "source-final-target")?;
    let evaluation = if target["accepted_evaluation"].is_string() {
        qualification::evidence(
            s,
            "source-final-evidence",
            text(&target["accepted_evaluation"])?,
        )?
    } else {
        Value::Null
    };
    ensure(
        !accepted || (qualification::passed(&evaluation) && target["observed_running"] == true),
        "successful source run lacks applicable protected publication",
    )?;
    if accepted {
        ensure(
            attempts.iter().any(|a| {
                a["attempts"].as_array().is_some_and(|nodes| {
                    nodes.iter().any(|n| {
                        n["selected_outputs"].as_array().is_some_and(|outputs| {
                            outputs.iter().any(|o| {
                                o["completed_evaluation"]["identity"]
                                    == target["accepted_evaluation"]
                            })
                        })
                    })
                })
            }),
            "published evaluation does not belong to this source run",
        )?;
    } else {
        crate::preservation::unchanged(&target, &setup["slotbook-test"])?;
    }
    let result = json!({"source_input":"slotbook-source-workshop","seeded_initial":args.seeded_initial,"attempts":attempts,"accepted":accepted,"evaluation":evaluation,"target":target,"worker":setup["slotbook-build"],"limit":"Finite real-model source/repair evidence, not held-out learning or power-loss qualification."});
    s.write("development-result.json", &result)?;
    finish(s, &result, args.remove_disposable)
}

fn finish(s: &Session, result: &Value, remove: bool) -> EvidenceResult {
    if remove {
        for name in ["slotbook-test", "slotbook-build"] {
            let state = s.resource(
                &format!("{name}-before-remove"),
                "inspect",
                0,
                vec![],
                name,
                Expected::Success,
            )?;
            let expected = &result[if name == "slotbook-test" {
                "target"
            } else {
                "worker"
            }];
            let guarded = crate::preservation::unchanged(&state, expected)?;
            if state["state"] != "removed" {
                s.resource(
                    &format!("{name}-remove"),
                    "remove",
                    guarded.version,
                    vec![],
                    name,
                    Expected::Success,
                )?;
            }
            let after = s.resource(
                &format!("{name}-removed"),
                "inspect",
                0,
                vec![],
                name,
                Expected::Success,
            )?;
            crate::preservation::unchanged(&after, &state)?;
            ensure(
                after["state"] == "removed" && after["pending"].is_null(),
                "disposable installation removal incomplete",
            )?;
        }
    }
    ensure(
        result["accepted"] == true,
        "bounded live development did not produce an accepted candidate; evidence retained",
    )
}

fn observe(s: &Session, run: &str, index: u8) -> EvidenceResult<Value> {
    let mut state = Value::Null;
    for poll in 0..=240 {
        state = s.ok(
            &format!("source-{index}-observe-{poll}"),
            args!["run", "show", run],
        )?["value"]
            .clone();
        if !state["terminal"].is_null() || state["uncertainty_count"] != 0 {
            break;
        }
        ensure(
            poll < 240,
            "source execution exhausted its 241 bounded observation requests",
        )?;
        thread::sleep(Duration::from_secs(5));
    }
    ensure(
        state["uncertainty_count"] == 0,
        "source execution uncertain; inspect before any further effect",
    )?;
    let mut attempts = Vec::new();
    for node in state["nodes"].as_array().ok_or("source nodes absent")? {
        if !node["latest_attempt_id"].is_string() {
            continue;
        }
        let id = text(&node["latest_attempt_id"])?;
        let attempt = s.ok(
            &format!("source-{index}-attempt-{id}"),
            args!["attempt", "inspect", run, id],
        )?["value"]
            .clone();
        let mut outputs = Vec::new();
        for output in attempt["outputs"]
            .as_array()
            .ok_or("attempt outputs absent")?
        {
            // Worker reports and verifier reports are bounded selected diagnostics. Native
            // executable stdout is retained by its artifact identity, never fed as model text.
            if (output["name"] == "worker_result" && node["node_id"] == "repair.begin")
                || output["name"] == "resource_result"
            {
                let path = s.logs.join(format!(
                    "source-{index}-{id}-{}.json",
                    text(&output["name"])?
                ));
                s.ok(
                    &format!("source-{index}-{id}-download-{}", text(&output["name"])?),
                    args![
                        "artifact",
                        "get",
                        text(&output["artifact"]["artifact_id"])?,
                        "--output",
                        path.display()
                    ],
                )?;
                let bytes = fs::read(path)?;
                ensure(
                    bytes.len() <= 65536,
                    "selected diagnostics exceed model context allocation",
                )?;
                let mut content: Value = serde_json::from_slice(&bytes)?;
                if output["name"] == "worker_result" {
                    // Keep the exact artifact above; compiler output can dwarf the useful
                    // source brief. The selected excerpt discloses every omitted byte.
                    content = json!({"exit_code":content["exit_code"],"stderr":excerpt(text(&content["stderr"])?,4096),"stdout":excerpt(text(&content["stdout"])?,1024)});
                }
                let evaluation = if output["name"] == "resource_result"
                    && content["evaluation"]["identity"].is_string()
                {
                    qualification::evidence(
                        s,
                        &format!("source-{index}-{id}-completed-evaluation"),
                        text(&content["evaluation"]["identity"])?,
                    )?
                } else {
                    Value::Null
                };
                outputs.push(json!({"reference":output["artifact"],"content":content,"completed_evaluation":evaluation}));
            }
        }
        attempts.push(json!({"attempt_id":id,"node":node["node_id"],"state":attempt["state"],"terminal":attempt["terminal"],"terminal_detail":attempt["terminal_detail"],"uncertain":attempt["uncertain"],"selected_outputs":outputs}));
    }
    Ok(json!({"run":state,"attempts":attempts}))
}

fn excerpt(value: &str, limit: usize) -> Value {
    let mut end = value.len().min(limit);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    json!({"text":&value[..end],"omitted_bytes":value.len()-end,"selection":"leading UTF-8 excerpt; exact complete bytes remain in the referenced artifact"})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_excerpt_discloses_omission_at_utf8_boundary() {
        let selected = excerpt("abéerror", 3);
        assert_eq!(selected["text"], "ab");
        assert_eq!(selected["omitted_bytes"], 7);
        assert_eq!(excerpt("short", 4096)["omitted_bytes"], 0);
    }
}

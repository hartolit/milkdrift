//! Bounded live source development; proposal bytes and trusted observations remain distinct.
mod assisted;
mod authoring;
mod observations;
mod proposal;
use super::{
    InvocationMode, Prepare, Qualify,
    client::{Caller, Expected, Session, load, number, text},
    prepare, qualification,
};
use milkdrift_evidence::{EvidenceResult, application::ensure};
use observations::{observe, source_snapshot};
use serde_json::{Value, json};
use std::{fs, path::PathBuf};

const MODEL: &str = "slotbook-development-model";
const REPAIR_MODEL: &str = "slotbook-repair-model";
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
    #[arg(
        long,
        required_unless_present = "assisted_source",
        conflicts_with = "assisted_source"
    )]
    model_profile: Option<PathBuf>,
    /// Explicit operator-assisted Rust source; records direct provenance and makes no model call.
    #[arg(long, conflicts_with_all = ["model_profile", "repair_model_profile", "seeded_initial"])]
    assisted_source: Option<PathBuf>,
    /// Optional second model for alternate repair attempts; the initial model handles odd attempts.
    #[arg(long, requires = "model_profile")]
    repair_model_profile: Option<PathBuf>,
    /// Finite proposal allowance, fixed before the study starts.
    #[arg(long, default_value_t = 6, value_parser = clap::value_parser!(u8).range(1..=8))]
    maximum_attempts: u8,
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
    ensure(
        args.assisted_source.is_none() || args.maximum_attempts == 1,
        "an assisted source submission requires --maximum-attempts 1; further corrections need a new declared submission",
    )?;
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
            config["adapters"]["model_profiles"] == model_profiles(&args)?,
            "resume model configuration differs",
        )?;
        configure_author(&s)?;
        s.start()?;
        let result = exercise(&args, &mut s);
        return s.finish(result);
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
    s.finish(result)
}

fn declared_inputs(args: &Arguments) -> EvidenceResult<Value> {
    let profile = args
        .model_profile
        .as_ref()
        .map(|p| p.canonicalize())
        .transpose()?;
    let profile_digest = profile
        .as_ref()
        .map(|p| fs::read(p).map(|bytes| format!("b3_{}", blake3::hash(&bytes))))
        .transpose()?;
    let repair = args
        .repair_model_profile
        .as_ref()
        .map(|p| p.canonicalize())
        .transpose()?;
    let repair_digest = repair
        .as_ref()
        .map(|p| fs::read(p).map(|bytes| format!("b3_{}", blake3::hash(&bytes))))
        .transpose()?;
    let mut inputs = json!({"authoring_version":authoring::VERSION,"image":args.image,"model_profile":profile,"model_profile_digest":profile_digest,"repair_model_profile":repair,"repair_model_profile_digest":repair_digest,"maximum_attempts":args.maximum_attempts,"seeded_initial":args.seeded_initial,"port":args.port});
    if let Some(path) = &args.assisted_source {
        inputs["assisted_source"] = assisted::input(path)?.0;
    }
    Ok(inputs)
}

fn configure(args: &Arguments, s: &Session) -> EvidenceResult {
    let Some(profile_path) = &args.model_profile else {
        return Ok(());
    };
    let profile_path = profile_path.canonicalize()?;
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
    config["adapters"]["model_profiles"] = model_profiles(args)?;
    let mut profiles = vec![profile["identity"].clone()];
    let mut destinations = vec![destination];
    let mut capabilities = vec![json!(MODEL)];
    if let Some(path) = &args.repair_model_profile {
        let path = path.canonicalize()?;
        let repair = load(&path)?;
        let url = url::Url::parse(text(&repair["base_url"])?)?;
        destinations.push(format!(
            "{}:{}",
            url.host_str().ok_or("repair host absent")?,
            url.port_or_known_default().ok_or("repair port absent")?
        ));
        profiles.push(repair["identity"].clone());
        capabilities.push(json!(REPAIR_MODEL));
    }
    let actor = &mut config["actors"][0];
    actor["authority"]["resources"]["capability"]["identities"]["values"]
        .as_array_mut()
        .ok_or("capability grant absent")?
        .extend(capabilities);
    actor["authority"]["resources"]["network"] =
        json!({"profiles":profiles,"destinations":destinations});
    actor["authority"]["budget"]["duration_ms"] = json!(14400000);
    fs::write(path, toml::to_string_pretty(&config)?)?;
    Ok(())
}

fn model_profiles(args: &Arguments) -> EvidenceResult<Value> {
    let mut profiles = Vec::new();
    if let Some(path) = &args.model_profile {
        profiles.push(json!({"capability_id":MODEL,"profile":path.canonicalize()?}));
    }
    if let Some(path) = &args.repair_model_profile {
        profiles.push(json!({"capability_id":REPAIR_MODEL,"profile":path.canonicalize()?}));
    }
    Ok(json!(profiles))
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
    let lane = if args.assisted_source.is_some() {
        "operator-assisted source; direct proposal, no new model generation"
    } else if args.seeded_initial {
        "seeded repair; no observed model planning failure"
    } else {
        "live source development; no candidate supplied"
    };
    let mut selected = json!({"lane":lane});
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
    for index in 1..=args.maximum_attempts {
        let generated = if let Some(path) = &args.assisted_source {
            proposal::Outcome::Ready(assisted::proposal(s, path, &revision, &selected)?)
        } else {
            proposal::generate(args, s, index, &revision, &selected)?
        };
        let proposed = match generated {
            proposal::Outcome::Ready(path) => path,
            proposal::Outcome::Invalid(failure) => {
                selected["invalid_proposal"] = failure.clone();
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
            "source proposal was not authorized",
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
        selected = json!({"run":result["run"]["run_id"],"terminal":result["run"]["terminal"],"attempts":result["attempts"],"source":source_snapshot(s,index)?,"omission":"Complete source and selected compiler/verifier diagnostics are supplied with exact artifact references; full run inspection is retained separately."});
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
    let result = json!({"source_input":"slotbook-source-workshop","source_origin":lane,"seeded_initial":args.seeded_initial,"attempts":attempts,"accepted":accepted,"evaluation":evaluation,"target":target,"worker":setup["slotbook-build"],"limit":"Finite source/repair workflow evidence with the recorded source origin, not autonomous model competence, held-out learning or power-loss qualification."});
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
        "bounded source development did not produce an accepted candidate; evidence retained",
    )
}

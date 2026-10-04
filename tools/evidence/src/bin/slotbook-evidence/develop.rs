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
        s.protected = load(args.root.join("evidence/bootstrap.json"))?
            .pointer("/recipe")
            .ok_or("missing /recipe")?
            .clone();
        s.worker = load(args.root.join("evidence/worker-preview.json"))?
            .pointer("/recipe")
            .ok_or("missing /recipe")?
            .clone();
        ensure(
            load(args.root.join("worker-recipe.json"))?
                .pointer("/worker_image")
                .ok_or("missing /worker_image")?
                == args.image.as_str(),
            "resume image differs from the retained source worker",
        )?;
        let config: Value =
            toml::from_str(&fs::read_to_string(args.root.join("host/daemon.toml"))?)?;
        ensure(
            config
                .pointer("/adapters/model_profiles")
                .ok_or("missing /adapters/model_profiles")?
                == &model_profiles(&args)?,
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
    let worker_fields = worker.as_object_mut().ok_or("expected JSON object")?;
    worker_fields.insert("worker_limits".into(), json!({"memory_bytes":2147483648u64,"cpu_percent":400,"pids":256,"temporary_bytes":268435456}));
    worker_fields.insert("task_timeout_ms".into(), json!(300000));
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
        inputs
            .as_object_mut()
            .ok_or("expected JSON object")?
            .insert("assisted_source".into(), assisted::input(path)?.0);
    }
    Ok(inputs)
}

fn configure(args: &Arguments, s: &Session) -> EvidenceResult {
    let Some(profile_path) = &args.model_profile else {
        return Ok(());
    };
    let profile_path = profile_path.canonicalize()?;
    let profile = load(&profile_path)?;
    let endpoint = url::Url::parse(text(
        profile.pointer("/base_url").ok_or("missing /base_url")?,
    )?)?;
    let destination = format!(
        "{}:{}",
        endpoint.host_str().ok_or("model host absent")?,
        endpoint
            .port_or_known_default()
            .ok_or("model port absent")?
    );
    let path = s.root.join("host/daemon.toml");
    let mut config: Value = toml::from_str(&fs::read_to_string(&path)?)?;
    config
        .pointer_mut("/adapters")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or("missing object /adapters")?
        .insert("model_profiles".into(), model_profiles(args)?);
    let mut profiles = vec![
        profile
            .pointer("/identity")
            .ok_or("missing /identity")?
            .clone(),
    ];
    let mut destinations = vec![destination];
    let mut capabilities = vec![json!(MODEL)];
    if let Some(path) = &args.repair_model_profile {
        let path = path.canonicalize()?;
        let repair = load(&path)?;
        let url = url::Url::parse(text(
            repair.pointer("/base_url").ok_or("missing /base_url")?,
        )?)?;
        destinations.push(format!(
            "{}:{}",
            url.host_str().ok_or("repair host absent")?,
            url.port_or_known_default().ok_or("repair port absent")?
        ));
        profiles.push(
            repair
                .pointer("/identity")
                .ok_or("missing /identity")?
                .clone(),
        );
        capabilities.push(json!(REPAIR_MODEL));
    }
    let actor = config
        .pointer_mut("/actors/0")
        .ok_or("operator actor absent")?;
    actor
        .pointer_mut("/authority/resources/capability/identities/values")
        .ok_or("missing /authority/resources/capability/identities/values")?
        .as_array_mut()
        .ok_or("capability grant absent")?
        .extend(capabilities);
    actor
        .pointer_mut("/authority/resources")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or("missing object /authority/resources")?
        .insert(
            "network".into(),
            json!({"profiles":profiles,"destinations":destinations}),
        );
    actor
        .pointer_mut("/authority/budget")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or("missing object /authority/budget")?
        .insert("duration_ms".into(), json!(14400000));
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
    let mut author = config
        .pointer("/actors/0")
        .ok_or("missing /actors/0")?
        .clone();
    let author_fields = author.as_object_mut().ok_or("expected JSON object")?;
    author_fields.insert("actor".into(), json!(AUTHOR));
    author_fields.insert("credential_ref".into(), json!("credential:source-author"));
    author_fields.insert("grant_id".into(), json!("grant:source-author"));
    author_fields.insert("preset".into(), json!("advisor"));
    let resources = author
        .pointer_mut("/authority/resources")
        .ok_or("author resources absent")?;
    resources
        .as_object_mut()
        .ok_or("author resources are not an object")?
        .insert(
            "workflow_run".into(),
            json!({"type":"workflow","workflow":"slotbook"}),
        );
    let capability_fields = resources
        .pointer_mut("/capability")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or("missing object /capability")?;
    capability_fields.insert(
        "identities".into(),
        json!({"type":"only","values":["managed.slotbook-build.worker","milkdrift.resources"]}),
    );
    capability_fields.insert("operations".into(), json!({"type":"only","values":["workspace.execute","resource.evaluate_candidate","resource.publish_candidate"]}));
    let resource_fields = resources
        .as_object_mut()
        .ok_or("author resources are not an object")?;
    resource_fields.insert("filesystem".into(), json!([]));
    resource_fields.insert("network".into(), json!({"profiles":[],"destinations":[]}));
    resource_fields.insert("artifacts".into(), json!({"type":"deny_all"}));
    let actors = config
        .pointer_mut("/actors")
        .ok_or("missing /actors")?
        .as_array_mut()
        .ok_or("actors absent")?;
    if let Some(existing) = actors.iter().find(|a| a["actor"] == AUTHOR) {
        ensure(existing == &author, "retained source author grant differs")?;
        return Ok(());
    }
    actors.push(author);
    let token = s.root.join("source-author.token");
    if !token.exists() {
        prepare::credential(&token)?;
    }
    config
        .pointer_mut("/secret_sources")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or("missing object /secret_sources")?
        .insert(
            "credential:source-author".into(),
            json!({"type":"file","path":token}),
        );
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
            ready
                .pointer("/pending")
                .is_some_and(serde_json::Value::is_null)
                && (ready.pointer("/state").and_then(serde_json::Value::as_str) == Some("stopped")
                    || ready.pointer("/state").and_then(serde_json::Value::as_str)
                        == Some("running")),
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
        setup
            .pointer("/slotbook-build")
            .ok_or("missing /slotbook-build")?,
    )?;
    let version = number(
        setup
            .pointer("/slotbook-test/version")
            .ok_or("missing /slotbook-test/version")?,
    )?;
    let mut revision = load(s.root.join("governed.json"))?
        .pointer("/revision")
        .ok_or("missing /revision")?
        .clone();
    ensure(
        revision.pointer("/semantic/nodes/verify-candidate/data_inputs/target/binding/value/expected_version").ok_or("missing /semantic/nodes/verify-candidate/data_inputs/target/binding/value/expected_version")?
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
        let candidate = observed
            .pointer("/value/observations")
            .ok_or("seed observations absent")?
            .as_array()
            .ok_or("seed observations absent")?
            .iter()
            .find(|o| {
                o["category"] == "artifact"
                    && o.pointer("/event/kind/name").and_then(Value::as_str) == Some("stdout")
            })
            .ok_or("seed candidate absent")?
            .pointer("/event/kind/reference")
            .ok_or("missing /event/kind/reference")?
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
            text(
                evaluated
                    .pointer("/evaluation/identity")
                    .ok_or("missing /evaluation/identity")?,
            )?,
        )?;
        ensure(
            evidence
                .pointer("/complete")
                .and_then(serde_json::Value::as_bool)
                == Some(true)
                && !qualification::passed(&evidence),
            "seeded candidate did not fail the fixed checks",
        )?;
        let selected_fields = selected.as_object_mut().ok_or("expected JSON object")?;
        selected_fields.insert("seeded_candidate".into(), candidate);
        selected_fields.insert("trusted_evaluation".into(), evidence);
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
                selected
                    .as_object_mut()
                    .ok_or("expected JSON object")?
                    .insert("invalid_proposal".into(), failure.clone());
                attempts.push(failure);
                continue;
            }
        };
        let submitted = s.call(
            &format!("source-{index}-proposal-submit"),
            args![
                "--expected-revision",
                text(revision.pointer("/id").ok_or("missing /id")?)?,
                "proposal",
                "submit",
                proposed.display()
            ],
            Expected::Success,
            Caller::SourceAuthor,
        )?;
        // Invalid proposals are retained as negative evidence; do not substitute a mutation.
        ensure(
            submitted
                .pointer("/status")
                .and_then(serde_json::Value::as_str)
                == Some("success"),
            "source proposal was not authorized",
        )?;
        let next = text(
            submitted
                .pointer("/value/value/proposed_revision")
                .ok_or("missing /value/value/proposed_revision")?,
        )?;
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
        revision = load(revision_path)?
            .pointer("/revision")
            .ok_or("missing /revision")?
            .clone();
        let run = format!("slotbook-source-{index}");
        s.ok(
            &format!("source-{index}-start"),
            args!["run", "start", run, "slotbook", next],
        )?;
        let result = observe(s, &run, index)?;
        selected = json!({"run":result.pointer("/run/run_id").ok_or("missing /run/run_id")?,"terminal":result.pointer("/run/terminal").ok_or("missing /run/terminal")?,"attempts":result.pointer("/attempts").ok_or("missing /attempts")?,"source":source_snapshot(s,index)?,"omission":"Complete source and selected compiler/verifier diagnostics are supplied with exact artifact references; full run inspection is retained separately."});
        accepted = result
            .pointer("/run/terminal")
            .and_then(serde_json::Value::as_str)
            == Some("succeeded");
        attempts.push(result);
        if accepted {
            break;
        }
    }
    let target = qualification::inspect(s, "source-final-target")?;
    let evaluation = if target
        .pointer("/accepted_evaluation")
        .ok_or("missing /accepted_evaluation")?
        .is_string()
    {
        qualification::evidence(
            s,
            "source-final-evidence",
            text(
                target
                    .pointer("/accepted_evaluation")
                    .ok_or("missing /accepted_evaluation")?,
            )?,
        )?
    } else {
        Value::Null
    };
    ensure(
        !accepted
            || (qualification::passed(&evaluation)
                && target
                    .pointer("/observed_running")
                    .and_then(serde_json::Value::as_bool)
                    == Some(true)),
        "successful source run lacks applicable protected publication",
    )?;
    if accepted {
        ensure(
            attempts.iter().any(|a| {
                a["attempts"].as_array().is_some_and(|nodes| {
                    nodes.iter().any(|n| {
                        n["selected_outputs"].as_array().is_some_and(|outputs| {
                            outputs.iter().any(|o| {
                                o.pointer("/completed_evaluation/identity")
                                    .and_then(Value::as_str)
                                    .is_some_and(|identity| {
                                        target.get("accepted_evaluation").and_then(Value::as_str)
                                            == Some(identity)
                                    })
                            })
                        })
                    })
                })
            }),
            "published evaluation does not belong to this source run",
        )?;
    } else {
        crate::preservation::unchanged(
            &target,
            setup
                .pointer("/slotbook-test")
                .ok_or("missing /slotbook-test")?,
        )?;
    }
    let result = json!({"source_input":"slotbook-source-workshop","source_origin":lane,"seeded_initial":args.seeded_initial,"attempts":attempts,"accepted":accepted,"evaluation":evaluation,"target":target,"worker":setup.pointer("/slotbook-build").ok_or("missing /slotbook-build")?,"limit":"Finite source/repair workflow evidence with the recorded source origin, not autonomous model competence, held-out learning or power-loss qualification."});
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
            if state.pointer("/state").ok_or("missing /state")? != "removed" {
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
                after.pointer("/state").and_then(serde_json::Value::as_str) == Some("removed")
                    && after
                        .pointer("/pending")
                        .is_some_and(serde_json::Value::is_null),
                "disposable installation removal incomplete",
            )?;
        }
    }
    ensure(
        result["accepted"] == true,
        "bounded source development did not produce an accepted candidate; evidence retained",
    )
}

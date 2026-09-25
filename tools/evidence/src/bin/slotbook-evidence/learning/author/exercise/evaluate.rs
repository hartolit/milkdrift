//! Exact public invocations supply both comparison observations and separately tracked products.
use super::super::{ARTIFACT_BYTES, prepare};
use super::{
    Caller, EvidenceResult, Expected, OPERATOR, STUDY_METHOD, Session, Value, budget,
    cap_reference, command, ensure, json, load, number, text, upload, write,
};
use crate::publication;

fn method(
    s: &Session,
    revision: &Value,
    slot: &Value,
    capability: &str,
    generation: u64,
) -> EvidenceResult<Value> {
    let name = text(&slot["name"])?;
    let catalog = s.ok(
        &format!("{name}-catalog-{generation}"),
        args!["invocation", "catalog"],
    )?;
    let worker = format!("managed.{}.worker", text(&slot["worker"])?);
    let descriptor = catalog["value"]["catalog"]["entries"]
        .as_array()
        .ok_or("catalog absent")?
        .iter()
        .find(|e| e["descriptor"]["identity"] == worker)
        .ok_or("worker descriptor absent")?["descriptor"]
        .clone();
    let authority = s.ok(
        &format!("{name}-authority-{generation}"),
        args![
            "--token-file",
            s.root
                .join("learning-05")
                .join(format!("{name}.token"))
                .display(),
            "daemon",
            "authority"
        ],
    )?["value"]
        .clone();
    let mut method = publication::method(descriptor, revision, &authority);
    method["descriptor"]["identity"] = json!(capability);
    method["descriptor"]["descriptor_revision"] = json!(generation);
    method["documentation"] = json!(
        "Finite seeded Slotbook method: construct and inspect three candidate submissions under fixed protected checks. Published inputs and the service grant select one independent prepared product workspace. Passing this invocation is not automatic deployment."
    );
    method["allowance"] = budget();
    method["maximum_duration_ms"] = json!(3600000);
    method["workspace_budget"]["max_total_artifact_bytes"] = json!(ARTIFACT_BYTES);
    method["inputs"]["product"] =
        json!({"type":"artifact","media_type":"application/json","maximum_bytes":16384});
    for i in 1..=3 {
        method["inputs"][format!("target-{i}")] = json!({"type":"choice","values":[{"schema_version":3,"command":format!("{name}-verify-{i}"),"installation":slot["target"],"expected_version":slot["target_state"]["version"]}]});
    }
    method["outputs"] = json!({"candidate":{"field":"candidate","media_type":"application/octet-stream","maximum_bytes":prepare::CANDIDATE_BYTES},"evaluation":{"field":"evaluation","media_type":"application/vnd.milkdrift.managed+json","maximum_bytes":65536}});
    Ok(method)
}
pub(super) fn preauthorize(
    args: &super::Arguments,
    s: &Session,
    baseline: &Value,
    _declaration: &Value,
    slots: &[Value],
) -> EvidenceResult {
    let source = slots.last().ok_or("source workspace absent")?;
    let retained = s.root.join("learning-05/published-baseline.json");
    let initial = if retained.exists() {
        load(retained)?
    } else {
        method(s, baseline, source, STUDY_METHOD, 1)?
    };
    let path = write(s, "published-baseline.json", &initial)?;
    let published = s.ok(
        "learning-baseline-publish",
        args!["method", "publish", path.display()],
    )?["value"]["value"]
        .clone();
    let mut future = initial;
    future["descriptor"]["descriptor_revision"] = json!(2);
    command(
        s,
        &super::key(args, "learning-promotion-policy-v1"),
        json!({"type":"preauthorize","declaration":super::study_receipt(args,"learning-declare",OPERATOR),"executor":"agent:slotbook-evaluator","method":future,"expected_previous_version":published["version"]}),
        Caller::Operator,
        Expected::Success,
    )?;
    Ok(())
}

fn invoke(
    s: &mut Session,
    key: &str,
    revision: &Value,
    slot: &Value,
    input: &Value,
) -> EvidenceResult<Value> {
    let capability = text(&slot["capability"])?;
    let path = s
        .root
        .join("learning-05")
        .join(format!("{key}-method.json"));
    let method = if path.exists() {
        load(path)?
    } else {
        method(s, revision, slot, capability, 1)?
    };
    ensure(
        method["revision"] == revision["id"] && method["descriptor"]["identity"] == capability,
        "retained publication selected another method or slot",
    )?;
    let document = write(s, &format!("{key}-method.json"), &method)?;
    s.ok(
        &format!("{key}-publish"),
        args!["method", "publish", document.display()],
    )?;
    let mut inputs = vec![
        json!({"name":"product","value":{"type":"artifact","reference":cap_reference(input)}}),
    ];
    for i in 1..=3 {
        inputs.push(json!({"name":format!("target-{i}"),"value":{"type":"inline","value":method["inputs"][format!("target-{i}")]["values"][0]}}));
    }
    let inputs = write(s, &format!("{key}-inputs.json"), &json!(inputs))?;
    let execution = s.invoke(key, capability, "method.invoke", &inputs, Caller::Operator)?;
    let completed = s.call(
        &format!("{key}-wait"),
        args!["invocation", "wait", execution],
        Expected::InvocationObservation,
        Caller::Operator,
    )?["value"]
        .clone();
    let lookup = s.ok(
        &format!("{key}-lookup"),
        args!["invocation", "show", execution],
    )?["value"]
        .clone();
    ensure(
        !completed["terminal"].is_null() || completed["history"]["type"] == "archived",
        "variant invocation has no terminal observation",
    )?;
    let mut artifacts = Vec::new();
    for observation in completed["observations"]
        .as_array()
        .ok_or("invocation observations absent")?
    {
        if observation["category"] == "artifact" {
            artifacts.push(observation["event"]["kind"]["reference"].clone());
        }
    }
    Ok(
        json!({"invocation":execution,"lookup":lookup,"observations":completed,"outputs":artifacts,"method":revision["id"],"input":input,"slot":slot}),
    )
}

pub(super) fn run(
    args: &super::Arguments,
    s: &mut Session,
    baseline: &Value,
    candidate: &Value,
    declaration: &Value,
    slots: &[Value],
) -> EvidenceResult {
    let revision_id = text(&candidate["revision"])?;
    let revised = super::revision(s, revision_id)?;
    for (i, pair) in declaration["pairs"]
        .as_array()
        .ok_or("pairs absent")?
        .iter()
        .enumerate()
    {
        for (offset, label, revision) in [(0, "baseline", baseline), (1, "candidate", &revised)] {
            let key = format!("evaluate-{}-{label}", text(&pair["name"])?);
            if s.root
                .join("learning-05")
                .join(format!("{key}-result.json"))
                .exists()
            {
                continue;
            }
            let result = invoke(s, &key, revision, &slots[i * 2 + offset], &pair["input"])?;
            write(s, &format!("{key}-result.json"), &result)?;
            // A restart between pairs preserves each accepted request and its cumulative account.
            if i == 0 && offset == 0 {
                s.stop()?;
                s.start()?;
            }
        }
    }
    let comparison = command(
        s,
        &super::key(args, "learning-comparison"),
        json!({"type":"compare","declaration":super::study_receipt(args,"learning-declare",OPERATOR),"candidate":super::study_receipt(args,"learning-candidate","agent:slotbook-learner")}),
        Caller::Evaluator,
        Expected::Success,
    )?;
    let comparison_ref =
        super::study_receipt(args, "learning-comparison", "agent:slotbook-evaluator");
    let mut unauthorized_publication = load(s.root.join("learning-05/published-baseline.json"))?;
    unauthorized_publication["revision"] = revised["id"].clone();
    unauthorized_publication["descriptor"]["descriptor_revision"] = json!(2);
    let publication_path = write(
        s,
        "evaluator-cannot-publish.json",
        &unauthorized_publication,
    )?;
    s.call(
        "evaluator-cannot-publish",
        args!["method", "publish", publication_path.display()],
        Expected::Refused,
        Caller::Evaluator,
    )?;
    let request = json!({"type":"auto_promote","policy":super::study_receipt(args,"learning-promotion-policy-v1",OPERATOR),"comparison":comparison_ref});
    command(
        s,
        &super::key(args, "learner-cannot-promote"),
        request.clone(),
        Caller::Learner,
        Expected::Refused,
    )?;
    let eligible = comparison["result"]["outcome"] == "eligible";
    let promoted = command(
        s,
        &super::key(args, "learning-automatic-promotion"),
        request.clone(),
        Caller::Evaluator,
        if eligible {
            Expected::Success
        } else {
            Expected::Refused
        },
    )?;
    s.stop()?;
    s.start()?;
    let replay = command(
        s,
        &super::key(args, "learning-automatic-promotion"),
        request,
        Caller::Evaluator,
        if eligible {
            Expected::Success
        } else {
            Expected::Refused
        },
    )?;
    ensure(promoted == replay, "promotion replay changed its result")?;
    let selected = if eligible { &revised } else { baseline };
    let retained_result = s.root.join("learning-05/result.json");
    if retained_result.exists() {
        let report = load(retained_result)?;
        ensure(
            report["comparison"] == comparison
                && report["selected_method"] == selected["id"]
                && report["declaration"]
                    == super::study_receipt(args, "learning-declare", OPERATOR),
            "retained product result belongs to another comparison or method",
        )?;
        upload(
            s,
            "learning-result",
            &serde_json::to_vec(&report)?,
            "application/json",
        )?;
        return Ok(());
    }
    let mut variations = Vec::new();
    for (index, pair_index) in [(8, 0), (9, 2)] {
        let pair = &declaration["pairs"][pair_index];
        let input = upload(
            s,
            &format!("variant-{}", text(&pair["name"])?),
            &serde_json::to_vec(
                &json!({"lineage":pair["input"],"application":slots[index]["application"],"method":selected["id"],"purpose":"independent product variant under the selected method"}),
            )?,
            "application/json",
        )?;
        let key = text(&slots[index]["name"])?.to_owned();
        let retained = s
            .root
            .join("learning-05")
            .join(format!("{key}-result.json"));
        let result = if retained.exists() {
            load(retained)?
        } else {
            let result = invoke(s, &key, selected, &slots[index], &input)?;
            write(s, &format!("{key}-result.json"), &result)?;
            result
        };
        ensure(
            result["method"] == selected["id"] && result["input"] == input,
            "retained variant belongs to another selected method or input",
        )?;
        variations.push(result);
    }
    let mut accepted = Vec::new();
    for (index, variant) in variations.iter().enumerate() {
        let evaluation_output = variant["outputs"]
            .as_array()
            .ok_or("variant outputs absent")?
            .iter()
            .find(|a| a["media_type"] == "application/vnd.milkdrift.managed+json")
            .ok_or("variant verification output absent")?;
        let destination = s
            .root
            .join(format!("learning-05/variant-{index}-evaluation.json"));
        if !destination.exists() {
            s.ok(
                &format!("variant-{index}-output"),
                args![
                    "invocation",
                    "output",
                    text(&variant["invocation"])?,
                    text(&evaluation_output["identity"])?,
                    destination.display()
                ],
            )?;
        }
        let receipt = load(&destination)?["evaluation"].clone();
        let evaluation = s.resource(
            &format!("variant-{index}-private-evidence"),
            "evidence",
            0,
            args!["--evaluation", text(&receipt["identity"])?],
            text(&slots[8 + index]["target"])?,
            Expected::Success,
        )?["evaluation"]
            .clone();
        ensure(
            receipt["subject"] == evaluation["subject"],
            "variant receipt and private evidence differ",
        )?;
        ensure(
            evaluation["complete"] == true
                && evaluation["checks"]
                    .as_array()
                    .is_some_and(|checks| checks.iter().all(|c| c["passed"] == true)),
            "variant has no complete passing evidence",
        )?;
        ensure(
            evaluation["subject"]["target"] == slots[8 + index]["target"],
            "variant evidence belongs to another target",
        )?;
        accepted.push(evaluation);
    }
    ensure(
        variations[0]["invocation"] != variations[1]["invocation"]
            && slots[8]["worker"] != slots[9]["worker"]
            && slots[8]["application"] != slots[9]["application"],
        "variants lack independent execution or different product configuration",
    )?;
    let evaluation = &accepted[0];
    let target = text(&slots[8]["target"])?;
    let before_path = s.root.join("learning-05/selected-target-before.json");
    let before = if before_path.exists() {
        load(before_path)?
    } else {
        let before = s.resource(
            "selected-target-before",
            "inspect",
            0,
            vec![],
            target,
            Expected::Success,
        )?;
        write(s, "selected-target-before.json", &before)?;
        before
    };
    s.resource(
        "wrong-variant-evidence",
        "publish",
        number(&before["version"])?,
        args!["--evaluation", text(&accepted[1]["identity"])?],
        target,
        Expected::Refused,
    )?;
    let deployed = s.resource(
        "selected-variant-deploy",
        "publish",
        number(&before["version"])?,
        args!["--evaluation", text(&evaluation["identity"])?],
        target,
        Expected::Success,
    )?;
    let observed_deployment = s.resource(
        "selected-variant-observed",
        "inspect",
        0,
        vec![],
        target,
        Expected::Success,
    )?;
    ensure(
        observed_deployment["desired_running"] == true
            && observed_deployment["observed_running"] == true
            && observed_deployment["accepted_evaluation"] == evaluation["identity"],
        "selected variant has no observed protected deployment",
    )?;
    let other = s.resource(
        "unselected-variant-retained",
        "inspect",
        0,
        vec![],
        text(&slots[9]["target"])?,
        Expected::Success,
    )?;
    ensure(
        other["desired_running"] == false && other["accepted_evaluation"].is_null(),
        "unselected variant was deployed",
    )?;
    let report = json!({"lane":"seeded_fixture","proposal_profile":load(&args.model_profile)?,"declaration":super::study_receipt(args,"learning-declare",OPERATOR),"comparison":comparison,"promoted":eligible,"selected_method":selected["id"],"variations":variations,"acceptance":accepted,"selection":{"index":0,"reason":"Operator scenario chooses the independently verified camera-loan variant; the class variant remains retained without deployment.","evaluation":evaluation["identity"],"deployment":deployed,"observed_deployment":observed_deployment,"other":other}});
    write(s, "result.json", &report)?;
    upload(
        s,
        "learning-result",
        &serde_json::to_vec(&report)?,
        "application/json",
    )?;
    Ok(())
}

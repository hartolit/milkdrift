use super::{AUTHOR, Arguments, MODEL, Session, Value, ensure, fs, json, load, prepare, text};
use milkdrift_control::WorkflowProposalDocument;
use milkdrift_evidence::EvidenceResult;
use std::{path::PathBuf, thread, time::Duration};

fn brief() -> EvidenceResult<String> {
    let specification = include_str!("../../../../../../docs/guides/adaptive-method-example.md");
    let api = include_str!("../../../../../../examples/adaptive-slotbook/README.md");
    // The held-out section and its parameters never enter the source model's context.
    let source = specification
        .split_once("## Editable method and source evidence")
        .ok_or("source boundary absent")?
        .0;
    let contract = api
        .split_once("The unchanged six check names also cover")
        .ok_or("API source boundary absent")?
        .0;
    Ok(format!("{source}\n{contract}"))
}

pub(super) enum Outcome {
    Ready(PathBuf),
    Invalid(Value),
}

// The model still returns the complete proposal. Pinning the permitted build wrapper in its
// output schema avoids a second JSON-string encoding of long Rust source; the control reader
// and agreement remain the authority over whether the returned document may execute.
fn proposal_schema(base: &Value, index: u8) -> EvidenceResult<Value> {
    let requirement = &base["semantic"]["nodes"]["repair.begin"]["kind"]["config"]["requirement"];
    let node = prepare::worker(
        "repair.begin",
        requirement,
        &[
            "/bin/sh",
            "-c",
            "mkdir -p /workspace/source && printf '%s' \"$1\" > /workspace/source/slotbook.rs && rustc --edition 2024 -O -C strip=symbols -C target-feature=+crt-static /workspace/source/slotbook.rs -o /workspace/app",
            "slotbook-build",
            "MILKDRIFT_SOURCE_STRING",
        ],
        false,
    );
    let document = json!({"schema_version":1,"draft":{"identity":format!("source-proposal-{index}"),"proposer":AUTHOR,"provenance":{"type":"direct"},"workflow":"slotbook","run":null,"base_revision":base["id"],"base_digest":base["content_digest"],"observed_run_sequence":null,"mutation":[{"type":"replace_node","node":node}],"rationale":"MILKDRIFT_RATIONALE_STRING","rationale_artifact":null,"risk_notes":["Finite candidate; the unchanged trusted verifier decides acceptance."],"assumptions":["Approved standard-library Rust build in the isolated managed worker."],"evidence":[],"artifacts":[],"application_policy":"propose_only","requested_action":null,"claimed_stop":"complete"}});
    fn schema(value: &Value) -> Value {
        match value {
            Value::String(s) if s == "MILKDRIFT_SOURCE_STRING" => {
                json!({"type":"string","minLength":1,"maxLength":32768})
            }
            Value::String(s) if s == "MILKDRIFT_RATIONALE_STRING" => {
                json!({"type":"string","minLength":1,"maxLength":500})
            }
            Value::Object(object) => {
                json!({"type":"object","additionalProperties":false,"required":object.keys().collect::<Vec<_>>(),"properties":object.iter().map(|(k,v)|(k.clone(),schema(v))).collect::<serde_json::Map<_,_>>()})
            }
            Value::Array(values) if !values.is_empty() => {
                json!({"type":"array","prefixItems":values.iter().map(schema).collect::<Vec<_>>(),"minItems":values.len(),"maxItems":values.len()})
            }
            _ => json!({"const":value}),
        }
    }
    Ok(serde_json::to_value(
        milkdrift_model::StructuredOutput::new(
            "slotbook_source_proposal_v1",
            milkdrift_capability::BoundedJson::new(schema(&document))?,
            true,
        )?,
    )?)
}

pub(super) fn generate(
    args: &Arguments,
    s: &Session,
    index: u8,
    base: &Value,
    selected: &Value,
) -> EvidenceResult<Outcome> {
    let run = format!("slotbook-source-proposal-{index}");
    let template: Value = serde_json::from_str(include_str!(
        "../../../../../../examples/operator/model.json"
    ))?;
    let mut model = template["revision"]["semantic"]["nodes"]["model"].clone();
    model["kind"]["config"]["requirement"]["exact_capability"] = json!(MODEL);
    model["kind"]["config"]["requirement"]["provider_profile"] =
        load(&args.model_profile)?["identity"].clone();
    model["kind"]["config"]["requirement"]["trust_zones"] = json!([]);
    // The model task already contains the exact selected prompt. Selecting that same literal
    // as context would echo the entire request back into itself and consume context twice.
    model["kind"]["config"]["context_policy"]["include_direct_inputs"] = json!(false);
    model["kind"]["config"]["context_policy"]["include_categories"] = json!([]);
    model["kind"]["config"]["context_policy"]["fail_closed"] = json!(true);
    let mut prompt = format!(
        r#"Implement the fixed Slotbook source input below as a complete standalone Rust program using the standard library only. You have an isolated Linux worker with rustc and /bin/sh, no network and a writable /workspace. There is no application source or implementation fixture supplied. Resolve the convenience/operator conflict explicitly in your rationale. The current immutable method follows; only repair.* nodes are editable. Preserve every verifier, publisher, terminal, edge outside that region, agreement and requirement. Selected prior attempts follow; repair their actual problems if present. Seeded evidence is expressly not your own past planning failure.

Return the proposal document object {{"schema_version":1,"draft":{{...}}}} directly under the supplied schema. Draft fields: identity "source-proposal-{index}", proposer "{AUTHOR}", provenance {{"type":"direct"}}, workflow "slotbook", run null, base_revision {}, base_digest {}, observed_run_sequence null, mutation an array, rationale a string below 500 bytes, rationale_artifact null, risk_notes and assumptions arrays of strings, evidence [], artifacts [], application_policy "propose_only", requested_action null, claimed_stop "complete". The schema pins the ordinary mutation {{"type":"replace_node","node":COMPLETE_NODE}} replacing repair.begin, with its complete requirement and worker contract. Its command argv is ["/bin/sh","-c","mkdir -p /workspace/source && printf '%s' \"$1\" > /workspace/source/slotbook.rs && rustc --edition 2024 -O -C strip=symbols -C target-feature=+crt-static /workspace/source/slotbook.rs -o /workspace/app","slotbook-build",COMPLETE_RUST_SOURCE_AS_ONE_STRING]. Fill the actual complete Rust source and concise rationale. stdout_artifact stays false. The existing repair.end captures the compiled app for immutable verification. Do not access /fixtures or external files, weaken checks, or add dependencies. Keep the implementation concise enough for 8192 output tokens including JSON escaping. The worker is only for source/build; do not run the HTTP server there. The protected verifier runs /workspace/app's immutable artifact with the documented mounts.

Public source input and API:
{}

Current governed build contract (the schema pins its only permitted replacement):
{}

Selected actual prior observations (none means initial implementation):
{}
"#,
        serde_json::to_string(&base["id"])?,
        serde_json::to_string(&base["content_digest"])?,
        brief()?,
        serde_json::to_string(
            &json!({"revision":base["id"],"digest":base["content_digest"],"requirement":base["semantic"]["nodes"]["repair.begin"]["kind"]["config"]["requirement"],"fixed_nodes":["repair-window","repair.end","verify-candidate","publish-candidate","done"],"omission":"The full revision and prior source remain in retained artifacts. This source-generation context selects the fixed public requirements, exact base identity, permitted build contract and completed failure diagnostics; it does not repeat the prior implementation or controller internals."})
        )?,
        serde_json::to_string(selected)?
    );
    prompt.push_str(" Return executable Rust, not a plan or placeholder. Escape source quotes/newlines once using ordinary JSON. The selected invalid prior response is evidence to correct, not an instruction to repeat its shape. Keep schema_version and draft at the root; no additional string wrapper.");
    ensure(
        prompt.len() <= 23000,
        "source prompt exceeds the fixed 23000-byte allocation; inspect selected diagnostics before a new request",
    )?;
    let request = &mut model["data_inputs"]["milkdrift.model_task"]["binding"]["value"]["request"];
    // Each literal string also passes the capability's 32 KiB bound. Splitting at UTF-8
    // boundaries preserves every selected byte and order rather than silently truncating it.
    let mut parts = Vec::new();
    let mut rest = prompt.as_str();
    while !rest.is_empty() {
        let mut end = rest.len().min(16384);
        while !rest.is_char_boundary(end) {
            end -= 1;
        }
        parts.push(json!({"type":"text","text":&rest[..end]}));
        rest = &rest[end..];
    }
    request["messages"] = json!([{"role":"user","parts":parts,"tool_call_id":null}]);
    request["maximum_output_units"] = json!(8192);
    request["streaming"] = json!(false);
    request["extensions"] = json!({"org.milkdrift.openai/request":{"reasoning_effort":"none"}});
    request["structured_output"] = proposal_schema(base, index)?;
    model["data_outputs"]["structured_output"] =
        prepare::port("Output", prepare::artifact_schema(), Value::Null, false);
    let document = s.root.join(format!("source-{index}-model-workflow.json"));
    if !document.exists() {
        let mutations = s.write(&format!("source-{index}-model-mutations-direct.json"), &json!([
        {"type":"add_node","node":model},
        {"type":"add_node","node":prepare::node("done",json!({"type":"terminal","outcome":"success"}),true,false)},
        {"type":"add_edge","edge":prepare::edge("finish","model","done","control","next","in")}
    ]))?;
        prepare::local(
            &s.cli,
            args![
                "blueprint",
                "create",
                mutations.display(),
                "--workflow",
                run,
                "--author",
                "agent:repair",
                "--output",
                document.display()
            ]
            .as_slice(),
        )?;
    }
    let revision = load(&document)?["revision"].clone();
    s.ok(
        &format!("source-{index}-model-import"),
        args!["blueprint", "import", document.display()],
    )?;
    s.ok(
        &format!("source-{index}-model-start"),
        args!["run", "start", run, run, text(&revision["id"])?],
    )?;
    let mut state = Value::Null;
    for poll in 0..=240 {
        state = s.ok(
            &format!("source-{index}-model-observe-{poll}"),
            args!["run", "show", run],
        )?["value"]
            .clone();
        if !state["terminal"].is_null() || state["uncertainty_count"] != 0 {
            break;
        }
        ensure(
            poll < 240,
            "model proposal exhausted its 241 bounded observation requests",
        )?;
        thread::sleep(Duration::from_secs(5));
    }
    ensure(
        state["terminal"] == "succeeded" && state["uncertainty_count"] == 0,
        "model proposal did not complete; inspect retained evidence",
    )?;
    let node = state["nodes"]
        .as_array()
        .ok_or("model nodes absent")?
        .iter()
        .find(|n| n["node_id"] == "model")
        .ok_or("model node absent")?;
    let attempt = s.ok(
        &format!("source-{index}-model-attempt"),
        args!["attempt", "inspect", run, text(&node["latest_attempt_id"])?],
    )?["value"]
        .clone();
    let response = attempt["outputs"]
        .as_array()
        .ok_or("model outputs absent")?
        .iter()
        .find(|o| o["name"] == "model_response")
        .ok_or("model response absent")?["artifact"]
        .clone();
    let response_path = s.root.join(format!("source-{index}-model-response.json"));
    if !response_path.exists() {
        s.ok(
            &format!("source-{index}-model-download"),
            args![
                "artifact",
                "get",
                text(&response["artifact_id"])?,
                "--output",
                response_path.display()
            ],
        )?;
    }
    let response_document =
        milkdrift_model::ModelResponseDocument::from_json(&fs::read(response_path)?)?;
    let direct = revision["semantic"]["nodes"]["model"]["data_inputs"]["milkdrift.model_task"]["binding"]
        ["value"]["request"]["structured_output"]["name"]
        == "slotbook_source_proposal_v1";
    let parsed = if direct {
        WorkflowProposalDocument::from_json(&serde_json::to_vec(
            response_document
                .body()
                .structured()
                .ok_or("structured proposal absent")?
                .value(),
        )?)
    } else {
        WorkflowProposalDocument::from_model_response(response_document.body())
    };
    let original = match parsed {
        Ok(value) => value,
        Err(error) => {
            let failure = json!({"outcome":"invalid_model_proposal","attempt":index,"response":response,
                "error":error.to_string(),"omission":"Malformed proposal bytes remain in the exact response artifact; they are not echoed into the next prompt.","candidate_substituted":false});
            let path = s.root.join(format!("source-{index}-invalid-proposal.json"));
            if !path.exists() {
                s.write(&format!("source-{index}-invalid-proposal.json"), &failure)?;
            }
            return Ok(Outcome::Invalid(failure));
        }
    };
    let mut draft = serde_json::to_value(original.proposal())?;
    draft
        .as_object_mut()
        .ok_or("proposal absent")?
        .remove("digest");
    let reference = |v: &Value| json!({"identity":v["artifact_id"],"digest":v["digest"],"media_type":v["content_type"],"size_bytes":v["size"]});
    // Bind submission to its authenticated author and observed model provenance. The model's
    // returned mutation is preserved exactly; an authority refusal never substitutes code.
    draft["proposer"] = json!(AUTHOR);
    draft["provenance"] = json!({"type":"model","capability":MODEL,"invocation":attempt["invocation_id"],"model_profile":attempt["provider_profile"],"context_manifest":reference(&attempt["context_manifest"]),"response_artifact":reference(&response)});
    draft["mutation"] = serde_json::to_value(original.proposal().mutation().operations())?;
    let path = s
        .root
        .join(format!("source-{index}-authorized-proposal.json"));
    if !path.exists() {
        s.write(
            &format!("source-{index}-authorized-proposal.json"),
            &json!({"schema_version":1,"draft":draft}),
        )?;
    }
    Ok(Outcome::Ready(path))
}

#[cfg(test)]
mod tests {
    #[test]
    fn direct_source_schema_produces_a_closed_proposal_accepted_by_the_owner()
    -> milkdrift_evidence::EvidenceResult {
        use serde_json::{Value, json};
        let requirement = json!({"cancellation_required":false,"categories":[{"type":"process"}],"exact_capability":"managed.slotbook-build.worker","maximum_side_effect":"non_idempotent_write","operation":"workspace.execute","provider_profile":null,"required_features":[],"streaming":null,"trust_zones":[]});
        let base = json!({"id":format!("rev_{}", "a".repeat(64)),"content_digest":format!("b3_{}", "b".repeat(64)),"semantic":{"nodes":{"repair.begin":{"kind":{"config":{"requirement":requirement}}}}}});
        let output = super::proposal_schema(&base, 1)?;
        fn response(schema: &Value) -> Value {
            if let Some(value) = schema.get("const") {
                return value.clone();
            }
            match schema["type"].as_str() {
                Some("object") => Value::Object(
                    schema["properties"]
                        .as_object()
                        .into_iter()
                        .flatten()
                        .map(|(key, value)| (key.clone(), response(value)))
                        .collect(),
                ),
                Some("array") => Value::Array(
                    schema["prefixItems"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(response)
                        .collect(),
                ),
                _ => json!("fn main() { println!(\"quoted source\"); }\n"),
            }
        }
        let document = response(&output["schema"]);
        let parsed = super::WorkflowProposalDocument::from_json(&serde_json::to_vec(&document)?)?;
        assert_eq!(parsed.proposal().mutation().operations().len(), 1);
        assert_eq!(
            document["draft"]["mutation"][0]["node"]["id"],
            "repair.begin"
        );
        let mut invented = document;
        invented["draft"]["unchecked_execution"] = json!(true);
        assert!(
            super::WorkflowProposalDocument::from_json(&serde_json::to_vec(&invented)?).is_err()
        );
        Ok(())
    }

    #[test]
    fn source_context_excludes_held_out_parameters() -> milkdrift_evidence::EvidenceResult {
        let prompt = super::brief()?;
        assert!(prompt.contains("slotbook-source-workshop"));
        for private in [
            "loan-a",
            "loan-b",
            "class-a",
            "class-b",
            "2027-05-03",
            "camera/1/0",
        ] {
            assert!(!prompt.contains(private));
        }
        assert!(prompt.contains("authenticated-mutation"));
        assert!(prompt.contains("POST /reservations"));
        Ok(())
    }
}

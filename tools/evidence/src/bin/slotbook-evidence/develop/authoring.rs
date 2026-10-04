//! Places submitted source into the fixed, ordinarily authorized build node.
use super::{AUTHOR, Value, ensure, json, prepare};
use milkdrift_evidence::EvidenceResult;
use serde::Deserialize;
use std::path::{Path, PathBuf};

pub(super) const VERSION: u32 = 6;
const BUILD: &str = "/usr/local/bin/slotbook-source-write \"$1\" \"$2\" && rustc --edition 2024 -O -C strip=symbols -C target-feature=+crt-static --extern serde_json=/opt/slotbook/lib/libserde_json.rlib -L dependency=/opt/slotbook/lib /workspace/source/slotbook.rs -o /workspace/app";

// Resume reconstructs authoring documents from the declared inputs and verified response.
// A retained file is evidence to compare, never an alternative source of authority.
pub(super) fn retain(root: &Path, name: &str, document: &Value) -> EvidenceResult<PathBuf> {
    let path = root.join(name);
    if path.exists() {
        ensure(
            super::load(&path)? == *document,
            &format!("retained authoring document differs: {name}"),
        )?;
    } else {
        prepare::write(root, name, document)?;
    }
    Ok(path)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    application_source: String,
    rationale: String,
}

pub(super) fn schema() -> Value {
    // Canonical property order puts the complete program before its explanation.
    json!({"type":"object","additionalProperties":false,"required":["application_source","rationale"],"properties":{
        "application_source":{"type":"string","minLength":1,"maxLength":32768,"description":"Complete Rust HTTP application source, JSON-escaped exactly once. No placeholders or omitted code."},
        "rationale":{"type":"string","minLength":1,"maxLength":500}
    }})
}

pub(super) fn mapping(base: &Value, index: u8, source: &Value) -> EvidenceResult<Value> {
    let expected = source["digest"].as_str().unwrap_or("absent");
    let node = prepare::worker(
        "repair.begin",
        base.pointer("/semantic/nodes/repair.begin/kind/config/requirement")
            .ok_or("missing /semantic/nodes/repair.begin/kind/config/requirement")?,
        &[
            "/bin/sh",
            "-c",
            BUILD,
            "slotbook-build",
            expected,
            "MODEL_SOURCE",
        ],
        false,
    );
    Ok(
        json!({"version":VERSION,"source_digest":source["digest"],"document":{"schema_version":1,"draft":{"identity":format!("source-proposal-{index}"),"proposer":AUTHOR,"provenance":{"type":"direct"},"workflow":"slotbook","run":null,"base_revision":base["id"],"base_digest":base["content_digest"],"observed_run_sequence":null,"mutation":[{"type":"replace_node","node":node}],"rationale":"MODEL_RATIONALE","rationale_artifact":null,"risk_notes":["Finite candidate; the unchanged trusted verifier decides acceptance."],"assumptions":["Approved Rust compiler and JSON library in the isolated managed worker."],"evidence":[],"artifacts":[],"application_policy":"propose_only","requested_action":null,"claimed_stop":"complete"}}}),
    )
}

pub(super) fn proposal(
    mapping: &Value,
    response: Value,
) -> EvidenceResult<milkdrift_control::WorkflowProposalDocument> {
    ensure(
        mapping["version"] == VERSION,
        "unknown source proposal mapping",
    )?;
    let input: Source = serde_json::from_value(response)?;
    ensure(
        !input.application_source.is_empty()
            && input.application_source.len() <= 32768
            && !input.rationale.is_empty()
            && input.rationale.len() <= 500,
        "model source or rationale exceeds its UTF-8 byte bound",
    )?;
    let mut document = mapping["document"].clone();
    *document
        .pointer_mut("/draft/mutation/0/node/data_inputs/command/binding/value/argv/5")
        .ok_or("source argument slot absent")? = json!(input.application_source);
    document
        .pointer_mut("/draft")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or("missing object /draft")?
        .insert("rationale".into(), json!(input.rationale));
    Ok(milkdrift_control::WorkflowProposalDocument::from_json(
        &serde_json::to_vec(&document)?,
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retained_authoring_documents_refuse_changes_without_overwriting_evidence() -> EvidenceResult
    {
        let root = tempfile::tempdir()?;
        let document = json!({"source":"original", "provenance":{"type":"model"}});
        let path = retain(root.path(), "proposal.json", &document)?;
        let bytes = std::fs::read(&path)?;
        assert_eq!(retain(root.path(), "proposal.json", &document)?, path);
        assert!(retain(root.path(), "proposal.json", &json!({"source":"changed"})).is_err());
        assert_eq!(std::fs::read(&path)?, bytes);
        std::fs::write(&path, b"{}")?;
        assert!(retain(root.path(), "proposal.json", &document).is_err());
        assert_eq!(std::fs::read(&path)?, b"{}");
        Ok(())
    }

    #[test]
    fn initial_and_repair_map_only_exact_source_bytes_and_refuse_wrong_shapes() -> EvidenceResult {
        let requirement = json!({"cancellation_required":false,"categories":[{"type":"process"}],"exact_capability":"managed.slotbook-build.worker","maximum_side_effect":"non_idempotent_write","operation":"workspace.execute","provider_profile":null,"required_features":[],"streaming":null,"trust_zones":[]});
        let base = json!({"id":format!("rev_{}","a".repeat(64)),"content_digest":format!("b3_{}","b".repeat(64)),"semantic":{"nodes":{"repair.begin":{"kind":{"config":{"requirement":requirement}}}}}});
        let source = "fn main() { println!(\"$1 `literal`\\n\"); }\n";
        for (snapshot, expected) in [
            (Value::Null, "absent".to_owned()),
            (
                json!({"digest":format!("b3_{}","c".repeat(64))}),
                format!("b3_{}", "c".repeat(64)),
            ),
        ] {
            let mapping = mapping(&base, 1, &snapshot)?;
            let parsed = proposal(
                &mapping,
                json!({"application_source":source,"rationale":"Implement or repair the API."}),
            )?;
            let operations = serde_json::to_value(parsed.proposal().mutation().operations())?;
            let argv = operations
                .pointer("/0/node/data_inputs/command/binding/value/argv")
                .ok_or("missing /0/node/data_inputs/command/binding/value/argv")?;
            assert_eq!(argv[4], expected);
            assert_eq!(argv[5], source);
            assert_eq!(operations.as_array().ok_or("operations absent")?.len(), 1);
            assert_eq!(
                operations
                    .pointer("/0/node/id")
                    .ok_or("missing /0/node/id")?,
                "repair.begin"
            );
            for invalid in [
                json!({"application_source":source,"rationale":"ok","extra":true}),
                json!({"application_source":"é".repeat(16385),"rationale":"ok"}),
                json!({"source":source,"rationale":"obsolete"}),
                json!({"edits":[],"rationale":"obsolete"}),
            ] {
                assert!(proposal(&mapping, invalid).is_err());
            }
        }
        Ok(())
    }
}

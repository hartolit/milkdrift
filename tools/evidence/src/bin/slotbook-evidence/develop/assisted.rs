//! Explicit source assistance uses the same proposal/build boundary without model attribution.
use super::{Session, Value, authoring, ensure, json, load, observations};
use milkdrift_evidence::EvidenceResult;
use std::path::{Path, PathBuf};

pub(super) fn input(path: &Path) -> EvidenceResult<(Value, String)> {
    let path = path.canonicalize()?;
    let bytes = observations::read_bounded_file(&path, 32768)?;
    ensure(
        !bytes.is_empty() && bytes.len() <= 32768,
        "assisted source is empty or oversized",
    )?;
    let identity = json!({"path":path,"digest":format!("b3_{}", blake3::hash(&bytes)),"size_bytes":bytes.len(),"provenance":"direct operator-assisted source; not a local-model response"});
    Ok((identity, String::from_utf8(bytes)?))
}

pub(super) fn proposal(
    s: &Session,
    path: &Path,
    base: &Value,
    selected: &Value,
) -> EvidenceResult<PathBuf> {
    let (identity, source) = input(path)?;
    ensure(
        load(s.root.join("development-inputs.json"))?
            .pointer("/assisted_source")
            .ok_or("missing /assisted_source")?
            == &identity,
        "assisted source changed after its declared input was frozen",
    )?;
    let mapped = authoring::proposal(
        &authoring::mapping(base, 1, &selected["source"])?,
        json!({
            "application_source":source,
            "rationale":"Explicit operator-assisted correction. Keep authenticated mutations, private names and durable bookings; the unchanged trusted verifier decides acceptance. No local model authored this correction."
        }),
    )?;
    let mut draft = serde_json::to_value(mapped.proposal())?;
    draft
        .as_object_mut()
        .ok_or("proposal absent")?
        .remove("digest");
    draft.as_object_mut().ok_or("expected JSON object")?.insert(
        "mutation".into(),
        serde_json::to_value(mapped.proposal().mutation().operations())?,
    );
    let document = json!({"schema_version":1,"draft":draft});
    authoring::retain(&s.root, "source-1-assisted-proposal.json", &document)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn assisted_input_preserves_bytes_and_refuses_empty_oversized_or_invalid_utf8() -> EvidenceResult
    {
        let root = tempfile::tempdir()?;
        let path = root.path().join("source.rs");
        let bytes = b"fn main() { println!(\"$1 `literal`\"); }\n";
        fs::write(&path, bytes)?;
        let (identity, source) = input(&path)?;
        assert_eq!(source.as_bytes(), bytes);
        assert_eq!(
            identity
                .get("digest")
                .and_then(Value::as_str)
                .ok_or("digest absent")?,
            format!("b3_{}", blake3::hash(bytes))
        );
        for invalid in [Vec::new(), vec![b'x'; 32769], vec![255]] {
            fs::write(&path, invalid)?;
            assert!(input(&path).is_err());
        }
        assert!(input(root.path()).is_err());
        #[cfg(unix)]
        assert!(input(Path::new("/dev/zero")).is_err());
        Ok(())
    }

    #[test]
    fn assisted_proposal_preserves_direct_provenance_and_refuses_conflicting_replay()
    -> EvidenceResult {
        let root = tempfile::tempdir()?;
        let source = root.path().join("source.rs");
        let bytes = "fn main() { println!(\"$1 `literal`\"); }\n";
        fs::write(&source, bytes)?;
        fs::write(root.path().join("token"), b"fixture-token")?;
        let executable = std::env::current_exe()?;
        let s = Session::resume(
            root.path(),
            &executable,
            &executable,
            19768,
            &root.path().join("evidence"),
            &root.path().join("token"),
        )?;
        s.write(
            "development-inputs.json",
            &json!({"assisted_source":input(&source)?.0}),
        )?;
        let requirement = json!({"cancellation_required":false,"categories":[{"type":"process"}],"exact_capability":"managed.slotbook-build.worker","maximum_side_effect":"non_idempotent_write","operation":"workspace.execute","provider_profile":null,"required_features":[],"streaming":null,"trust_zones":[]});
        let base = json!({"id":format!("rev_{}","a".repeat(64)),"content_digest":format!("b3_{}","b".repeat(64)),"semantic":{"nodes":{"repair.begin":{"kind":{"config":{"requirement":requirement}}}}}});
        fs::write(&source, "changed")?;
        assert!(proposal(&s, &source, &base, &Value::Null).is_err());
        assert!(!root.path().join("source-1-assisted-proposal.json").exists());
        fs::write(&source, bytes)?;
        let path = proposal(&s, &source, &base, &Value::Null)?;
        let document = load(&path)?;
        assert_eq!(
            document
                .pointer("/draft/provenance")
                .ok_or("missing /draft/provenance")?,
            &json!({"type":"direct"})
        );
        assert_eq!(
            document
                .pointer("/draft/mutation/0/node/data_inputs/command/binding/value/argv/5")
                .ok_or("missing /draft/mutation/0/node/data_inputs/command/binding/value/argv/5")?,
            bytes
        );
        assert_eq!(proposal(&s, &source, &base, &Value::Null)?, path);
        assert_eq!(load(&path)?, document);
        fs::write(&path, b"{}")?;
        assert!(proposal(&s, &source, &base, &Value::Null).is_err());
        Ok(())
    }
}

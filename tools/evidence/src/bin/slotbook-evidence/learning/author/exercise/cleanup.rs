//! Remove only the generations observed by this study, retaining their declared data.
use super::{EvidenceResult, Expected, Session, Value, ensure, json, load, text, write};
use crate::preservation::unchanged;
use std::collections::BTreeMap;

pub(super) fn drain_selected(s: &Session, slots: &[Value]) -> EvidenceResult {
    // Deployment was observed after its request completed. Free this study's service capacity
    // before the separate tool-stage experiment, including when resuming a stopped installation.
    let result = load(s.root.join("learning-05/result.json"))?;
    let expected = result
        .pointer("/selection/observed_deployment")
        .ok_or("missing /selection/observed_deployment")?;
    let target = text(&slots.get(8).ok_or("selected variant slot absent")?["target"])?;
    let current = s.resource(
        "cleanup-selected-inspect",
        "inspect",
        0,
        vec![],
        target,
        Expected::Success,
    )?;
    let current = unchanged(&current, expected)?;
    if current.desired_running == Some(true) {
        s.resource(
            "cleanup-selected-stop",
            "stop",
            current.version,
            vec![],
            target,
            Expected::Success,
        )?;
        let stopped = s.resource(
            "cleanup-selected-stopped",
            "inspect",
            0,
            vec![],
            target,
            Expected::Success,
        )?;
        let observed = unchanged(&stopped, expected)?;
        ensure(
            observed.observed_running == Some(false),
            "selected study service did not drain",
        )?;
        write(s, "selected-product-drained.json", &stopped)?;
    }
    Ok(())
}

pub(super) fn remove(s: &Session, slots: &[Value]) -> EvidenceResult {
    let dir = s.root.join("learning-05");
    let result = load(dir.join("result.json"))?;
    let setup = load(dir.join("setup-improvement.json"))?;
    let mut expected = BTreeMap::new();
    for slot in slots {
        for kind in ["worker", "target"] {
            expected.insert(
                text(&slot[kind])?.to_owned(),
                slot[format!("{kind}_state")].clone(),
            );
        }
    }
    expected.insert(
        text(&slots.get(8).ok_or("selected variant absent")?["target"])?.to_owned(),
        result
            .pointer("/selection/observed_deployment")
            .ok_or("missing /selection/observed_deployment")?
            .clone(),
    );
    expected.insert(
        text(&slots.last().ok_or("source workspace absent")?["worker"])?.to_owned(),
        setup
            .pointer("/activated_setup")
            .ok_or("missing /activated_setup")?
            .clone(),
    );
    expected.insert(
        "learn-tools-stage".to_owned(),
        setup
            .pointer("/fresh_stage")
            .ok_or("missing /fresh_stage")?
            .clone(),
    );
    let mut removed = Vec::new();
    for (name, expected) in expected {
        let state = s.resource(
            &format!("cleanup-{name}-inspect"),
            "inspect",
            0,
            vec![],
            &name,
            Expected::Success,
        )?;
        let current = unchanged(&state, &expected)?;
        if current.state != "removed" {
            s.resource(
                &format!("cleanup-{name}-remove"),
                "remove",
                current.version,
                vec![],
                &name,
                Expected::Success,
            )?;
        }
        let after = s.resource(
            &format!("cleanup-{name}-removed"),
            "inspect",
            0,
            vec![],
            &name,
            Expected::Success,
        )?;
        let observed = unchanged(&after, &expected)?;
        ensure(
            observed.state == "removed" && observed.observed_running == Some(false),
            "study installation removal incomplete",
        )?;
        removed.push(after);
    }
    write(
        s,
        "disposable-removal.json",
        &json!({"installations":removed,
        "preservation":"Recipe-owned data volumes, source, artifacts, knowledge and evidence retained; attached and unrelated resources are not removal targets."}),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observed() -> Value {
        json!({"schema_version":3,"installation":"study","version":12,"generation":2,
            "state":"running","desired_running":true,"observed_running":true,
            "recipe":{"name":"study","digest":format!("b3_{}", "a".repeat(64))},
            "accepted_evaluation":"evaluation","evaluation":null,"pending":null,
            "blockers":[],"capabilities":[],"diagnostics":[],
            "resources":[{"name":"data","kind":"data","ownership":"owned","identity":"volume-study","disposition":"preserve"}]})
    }

    #[test]
    fn stopped_or_removed_original_can_resume_but_replacement_cannot() -> EvidenceResult {
        let expected = observed();
        for state in ["stopped", "removed"] {
            let mut current = expected.clone();
            let current_fields = current.as_object_mut().ok_or("expected JSON object")?;
            current_fields.insert("state".into(), json!(state));
            current_fields.insert("desired_running".into(), json!(false));
            current_fields.insert("observed_running".into(), json!(false));
            current_fields.insert("version".into(), json!(18));
            assert!(unchanged(&current, &expected).is_ok());
            current
                .as_object_mut()
                .ok_or("expected JSON object")?
                .insert("generation".into(), json!(3));
            assert!(unchanged(&current, &expected).is_err());
        }
        Ok(())
    }

    #[test]
    fn cleanup_refuses_changed_or_destructive_preservation() -> EvidenceResult {
        let expected = observed();
        let mut current = expected.clone();
        current
            .pointer_mut("/resources/0")
            .and_then(serde_json::Value::as_object_mut)
            .ok_or("missing object /resources/0")?
            .insert("disposition".into(), json!("delete_on_removal"));
        assert!(unchanged(&current, &expected).is_err());
        assert!(unchanged(&current, &current).is_err());
        Ok(())
    }

    #[test]
    fn cleanup_refuses_changed_resource_or_unresolved_transition() -> EvidenceResult {
        let expected = observed();
        let mut current = expected.clone();
        current
            .pointer_mut("/resources/0")
            .and_then(serde_json::Value::as_object_mut)
            .ok_or("missing object /resources/0")?
            .insert("identity".into(), json!("replacement-volume"));
        assert!(unchanged(&current, &expected).is_err());
        current = expected.clone();
        current
            .as_object_mut()
            .ok_or("expected JSON object")?
            .insert("pending".into(), json!("transition"));
        assert!(unchanged(&current, &expected).is_err());
        Ok(())
    }
}

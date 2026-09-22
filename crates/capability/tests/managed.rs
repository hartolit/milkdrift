//! Hand-reviewed managed request and generation-binding wire contracts.
use milkdrift_capability::managed::*;
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

#[test]
fn exact_request_shape_refuses_old_versions_unknown_fields_and_claim_ambiguity() -> Result {
    let golden = r#"{"schema_version":1,"command":"inspect-one","installation":"slotbook","expected_version":7,"action":{"type":"inspect"}}"#;
    let request: ManagedRequest = serde_json::from_str(golden)?;
    request.validate()?;
    assert_eq!(serde_json::to_string(&request)?, golden);
    assert_eq!(request.operation(), "resource.inspect");
    for invalid in [
        golden.replace("\"schema_version\":1", "\"schema_version\":0"),
        golden.replace("\"schema_version\":1", "\"schema_version\":2"),
        golden.replace(
            "\"type\":\"inspect\"",
            "\"type\":\"inspect\",\"engine_args\":[]",
        ),
        golden.replace("slotbook", "../slotbook"),
        golden.replace(
            "\"type\":\"inspect\"",
            "\"type\":\"resolve\",\"use_id\":\"unknown\",\"expected_claim\":0",
        ),
    ] {
        match serde_json::from_str::<ManagedRequest>(&invalid) {
            Err(_) => {}
            Ok(request) => assert!(request.validate().is_err()),
        }
    }
    Ok(())
}

#[test]
fn generation_binding_requires_unique_exact_resources() -> Result {
    let mut binding = ManagedBinding {
        installation: ManagedName::new("slotbook")?,
        generation: 1,
        recipe_digest: format!("b3_{}", "a".repeat(64)),
        resources: vec![ResourceRequirement {
            resource: ManagedName::new("working")?,
            mutation: true,
        }],
    };
    binding.validate()?;
    let expected = format!(
        r#"{{"installation":"slotbook","generation":1,"recipe_digest":"b3_{}","resources":[{{"resource":"working","mutation":true}}]}}"#,
        "a".repeat(64)
    );
    assert_eq!(serde_json::to_string(&binding)?, expected);
    binding.resources.push(binding.resources[0].clone());
    assert!(binding.validate().is_err());
    binding.resources.pop();
    binding.generation = 0;
    assert!(binding.validate().is_err());
    Ok(())
}

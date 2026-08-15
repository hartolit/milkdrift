use super::support::*;

#[test]
fn model_construction_is_handle_only_and_fault_retains_the_owner() -> Result<(), String> {
    let mut missing = test_prepared(Vec::new(), DType::F32)?;
    populate_required_model_tensors(&mut missing)?;
    missing.final_tensors.remove("lm_head.weight");
    required_error(
        missing.construct_model(&mut NoopMaterializationObserver),
        "missing native model tensor must fail construction",
    )?;
    assert!(missing.constructed_model.is_none());
    assert!(!missing.final_tensors.is_empty());

    {
        let checkpoint = MaterializationCheckpoint::ModelOwned;
        let mut prepared = test_prepared(Vec::new(), DType::F32)?;
        populate_required_model_tensors(&mut prepared)?;
        required_error(
            prepared.construct_model(&mut FailAt(checkpoint)),
            "post-construction checkpoint must fail",
        )?;
        assert!(prepared.constructed_model.is_some());
        assert_eq!(prepared.final_tensors.len(), 12);
        let mut failed = prepared.into_failed();
        failed
            .cleanup()
            .map_err(|error| format!("cleanup constructed model: {error:?}"))?;
    }

    let mut prepared = test_prepared(Vec::new(), DType::F32)?;
    populate_required_model_tensors(&mut prepared)?;
    let mut events = Events::default();
    prepared
        .construct_model(&mut events)
        .map_err(|error| format!("construct handle-only model: {error:?}"))?;
    assert_eq!(events.batch_synchronizations, 0);
    assert!(prepared.constructed_model.is_some());
    Ok(())
}

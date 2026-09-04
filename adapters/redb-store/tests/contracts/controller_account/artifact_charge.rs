use super::*;

#[test]
fn preexisting_artifact_charge_is_corruption_not_a_conflict() -> TestResult {
    let directory = TempDir::new()?;
    let run = RunId::new("run-controller-artifact-charge-row")?;
    let first_publication = ArtifactPublicationId::new("publication-controller-charge-first")?;
    let second_publication = ArtifactPublicationId::new("publication-controller-charge-second")?;
    {
        let store = RedbStore::open(directory.path())?;
        let _declaration = establish(&store, &run, "artifact-charge-row")?;
        let bytes = b"x";
        let metadata = ArtifactMetadata::new(
            milkdrift_workspace::ArtifactReference::new(
                ArtifactId::new("artifact-controller-charge-first")?,
                ContentDigest::for_bytes(bytes),
                MediaType::new("application/octet-stream")?,
                1,
            ),
            ArtifactSensitivity::Public,
            ArtifactRetention::WhileReferenced,
            ArtifactProvenance::new(
                CausalReference::External {
                    source: CausalId::new("controller-charge-test")?,
                },
                Vec::new(),
            )?,
        )?;
        let publication = BeginArtifactPublication::new(
            first_publication.clone(),
            run.clone(),
            metadata,
            workspace_budget()?,
            WorkspaceUsage::EMPTY,
        )?;
        let _ = store.begin_publication(&publication)?;
        let _ = store.write_chunk(&first_publication, 0, bytes)?;
        let _ = store.commit_publication(&first_publication)?;
    }
    let database = Database::open(directory.path().join("milkdrift.redb"))?;
    let write = database.begin_write()?;
    {
        let mut charges = write.open_table(ARTIFACT_CHARGES)?;
        let bytes = charges
            .get(first_publication.as_str())?
            .ok_or("first controller artifact charge is absent")?
            .value()
            .to_vec();
        charges.insert(second_publication.as_str(), bytes.as_slice())?;
    }
    write.commit()?;
    drop(database);

    let store = RedbStore::open(directory.path())?;
    let bytes = b"x";
    let metadata = ArtifactMetadata::new(
        milkdrift_workspace::ArtifactReference::new(
            ArtifactId::new("artifact-controller-charge-second")?,
            ContentDigest::for_bytes(bytes),
            MediaType::new("application/octet-stream")?,
            1,
        ),
        ArtifactSensitivity::Public,
        ArtifactRetention::WhileReferenced,
        ArtifactProvenance::new(
            CausalReference::External {
                source: CausalId::new("controller-charge-test")?,
            },
            Vec::new(),
        )?,
    )?;
    let publication = BeginArtifactPublication::new(
        second_publication.clone(),
        run.clone(),
        metadata,
        workspace_budget()?,
        store.workspace_usage(&run)?,
    )?;
    let _ = store.begin_publication(&publication)?;
    let _ = store.write_chunk(&second_publication, 0, bytes)?;
    assert_storage_corruption(store.commit_publication(&second_publication));
    Ok(())
}

#[test]
fn preexisting_mismatched_artifact_charge_is_an_immutable_conflict() -> TestResult {
    for mutation in ["account", "run", "reservation", "bytes"] {
        let directory = TempDir::new()?;
        let run = RunId::new(format!("run-controller-charge-conflict-{mutation}"))?;
        let first = ArtifactPublicationId::new(format!(
            "publication-controller-charge-conflict-source-{mutation}"
        ))?;
        let second = ArtifactPublicationId::new(format!(
            "publication-controller-charge-conflict-target-{mutation}"
        ))?;
        let account = {
            let store = RedbStore::open(directory.path())?;
            let declaration = establish(&store, &run, &format!("charge-conflict-{mutation}"))?;
            let bytes = b"x";
            let metadata = ArtifactMetadata::new(
                milkdrift_workspace::ArtifactReference::new(
                    ArtifactId::new(format!("artifact-controller-charge-source-{mutation}"))?,
                    ContentDigest::for_bytes(bytes),
                    MediaType::new("application/octet-stream")?,
                    1,
                ),
                ArtifactSensitivity::Public,
                ArtifactRetention::WhileReferenced,
                ArtifactProvenance::new(
                    CausalReference::External {
                        source: CausalId::new("controller-charge-conflict-test")?,
                    },
                    Vec::new(),
                )?,
            )?;
            let publication = BeginArtifactPublication::new(
                first.clone(),
                run.clone(),
                metadata,
                workspace_budget()?,
                WorkspaceUsage::EMPTY,
            )?;
            let _ = store.begin_publication(&publication)?;
            let _ = store.write_chunk(&first, 0, bytes)?;
            let _ = store.commit_publication(&first)?;
            declaration.account().clone()
        };
        let database = Database::open(directory.path().join("milkdrift.redb"))?;
        let write = database.begin_write()?;
        {
            let mut charges = write.open_table(ARTIFACT_CHARGES)?;
            let source = charges
                .get(first.as_str())?
                .ok_or("source controller artifact charge is absent")?
                .value()
                .to_vec();
            let (from, to) = match mutation {
                "account" => {
                    let foreign = declaration(
                        &RunId::new("run-controller-charge-conflict-foreign")?,
                        "charge-conflict-foreign",
                    )?;
                    (
                        format!("\"account\":\"{account}\""),
                        format!("\"account\":\"{}\"", foreign.account()),
                    )
                }
                "run" => (
                    format!("\"run\":\"{run}\""),
                    "\"run\":\"run-controller-charge-conflict-foreign\"".to_owned(),
                ),
                "reservation" => (
                    "\"reservation\":null".to_owned(),
                    "\"reservation\":\"controller-reservation:charge-conflict\"".to_owned(),
                ),
                "bytes" => ("\"bytes\":1".to_owned(), "\"bytes\":2".to_owned()),
                _ => return Err("unknown preexisting charge mutation".into()),
            };
            let altered =
                rewrite_internal_payload(&source, "controller artifact charge", &from, &to)?;
            charges.insert(second.as_str(), altered.as_slice())?;
        }
        write.commit()?;
        drop(database);

        let store = RedbStore::open(directory.path())?;
        let bytes = b"x";
        let metadata = ArtifactMetadata::new(
            milkdrift_workspace::ArtifactReference::new(
                ArtifactId::new(format!("artifact-controller-charge-target-{mutation}"))?,
                ContentDigest::for_bytes(bytes),
                MediaType::new("application/octet-stream")?,
                1,
            ),
            ArtifactSensitivity::Public,
            ArtifactRetention::WhileReferenced,
            ArtifactProvenance::new(
                CausalReference::External {
                    source: CausalId::new("controller-charge-conflict-test")?,
                },
                Vec::new(),
            )?,
        )?;
        let publication = BeginArtifactPublication::new(
            second.clone(),
            run.clone(),
            metadata,
            workspace_budget()?,
            store.workspace_usage(&run)?,
        )?;
        let _ = store.begin_publication(&publication)?;
        let _ = store.write_chunk(&second, 0, bytes)?;
        assert!(matches!(
            store.commit_publication(&second),
            Err(PersistenceError::ImmutableConflict {
                entity: "controller artifact charge",
                ..
            })
        ));
    }
    Ok(())
}

#[test]
fn committed_bound_publication_requires_its_reverse_controller_charge_link() -> TestResult {
    let directory = TempDir::new()?;
    let run = RunId::new("run-controller-missing-artifact-charge")?;
    let publication = ArtifactPublicationId::new("publication-controller-missing-charge")?;
    {
        let store = RedbStore::open(directory.path())?;
        let _declaration = establish(&store, &run, "missing-artifact-charge")?;
        let bytes = b"x";
        let metadata = ArtifactMetadata::new(
            milkdrift_workspace::ArtifactReference::new(
                ArtifactId::new("artifact-controller-missing-charge")?,
                ContentDigest::for_bytes(bytes),
                MediaType::new("application/octet-stream")?,
                1,
            ),
            ArtifactSensitivity::Public,
            ArtifactRetention::WhileReferenced,
            ArtifactProvenance::new(
                CausalReference::External {
                    source: CausalId::new("controller-missing-charge-test")?,
                },
                Vec::new(),
            )?,
        )?;
        let request = BeginArtifactPublication::new(
            publication.clone(),
            run,
            metadata,
            workspace_budget()?,
            WorkspaceUsage::EMPTY,
        )?;
        let _ = store.begin_publication(&request)?;
        let _ = store.write_chunk(&publication, 0, bytes)?;
        let _ = store.commit_publication(&publication)?;
        assert!(!has_integrity_failure(&store)?);
    }

    let database = Database::open(directory.path().join("milkdrift.redb"))?;
    let write = database.begin_write()?;
    assert!(
        write
            .open_table(ARTIFACT_CHARGES)?
            .remove(publication.as_str())?
            .is_some()
    );
    write.commit()?;
    drop(database);

    let store = RedbStore::open(directory.path())?;
    assert!(has_integrity_failure(&store)?);
    Ok(())
}

#[test]
fn controller_artifact_charge_linkage_rejects_checksum_correct_field_corruption() -> TestResult {
    for mutation in ["account", "reservation", "bytes", "outcome"] {
        let directory = TempDir::new()?;
        let run = RunId::new(format!("run-controller-charge-link-{mutation}"))?;
        let publication =
            ArtifactPublicationId::new(format!("publication-controller-charge-link-{mutation}"))?;
        let account_declaration = {
            let store = RedbStore::open(directory.path())?;
            let declaration = establish(&store, &run, &format!("charge-link-{mutation}"))?;
            let bytes = b"x";
            let metadata = ArtifactMetadata::new(
                milkdrift_workspace::ArtifactReference::new(
                    ArtifactId::new(format!("artifact-controller-charge-link-{mutation}"))?,
                    ContentDigest::for_bytes(bytes),
                    MediaType::new("application/octet-stream")?,
                    1,
                ),
                ArtifactSensitivity::Public,
                ArtifactRetention::WhileReferenced,
                ArtifactProvenance::new(
                    CausalReference::External {
                        source: CausalId::new("controller-charge-link-test")?,
                    },
                    Vec::new(),
                )?,
            )?;
            let request = BeginArtifactPublication::new(
                publication.clone(),
                run.clone(),
                metadata,
                workspace_budget()?,
                WorkspaceUsage::EMPTY,
            )?;
            let _ = store.begin_publication(&request)?;
            let _ = store.write_chunk(&publication, 0, bytes)?;
            let _ = store.commit_publication(&publication)?;
            assert!(!has_integrity_failure(&store)?);
            declaration
        };

        let (from, to) = match mutation {
            "account" => {
                let foreign = declaration(
                    &RunId::new("run-controller-charge-link-foreign")?,
                    "charge-link-foreign",
                )?;
                (
                    format!("\"account\":\"{}\"", account_declaration.account()),
                    format!("\"account\":\"{}\"", foreign.account()),
                )
            }
            "reservation" => (
                "\"reservation\":null".to_owned(),
                "\"reservation\":\"controller-reservation:charge-link-tampered\"".to_owned(),
            ),
            "bytes" => ("\"bytes\":1".to_owned(), "\"bytes\":2".to_owned()),
            "outcome" => (
                "\"outcome\":\"charged\"".to_owned(),
                "\"outcome\":\"contract_violation\"".to_owned(),
            ),
            _ => return Err("unknown controller artifact charge mutation".into()),
        };
        let database = Database::open(directory.path().join("milkdrift.redb"))?;
        let write = database.begin_write()?;
        {
            let mut charges = write.open_table(ARTIFACT_CHARGES)?;
            let stored = charges
                .get(publication.as_str())?
                .ok_or("controller artifact charge is absent")?
                .value()
                .to_vec();
            let altered =
                rewrite_internal_payload(&stored, "controller artifact charge", &from, &to)?;
            charges.insert(publication.as_str(), altered.as_slice())?;
        }
        write.commit()?;
        drop(database);

        let store = RedbStore::open(directory.path())?;
        assert!(has_integrity_failure_matching(
            &store,
            "artifact_publication_indexes",
            "artifact publication has an impossible controller charge linkage",
        )?);
    }
    Ok(())
}

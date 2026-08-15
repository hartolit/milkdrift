use super::support::*;

#[test]
fn actual_workspace_satisfies_architecture_policy() -> Result<(), Box<dyn Error>> {
    let report = validate_workspace(&workspace_manifest())?;
    assert!(
        report.is_valid(),
        "actual workspace violations: {:#?}",
        report.violations()
    );
    Ok(())
}

#[test]
fn scalable_fixture_accepts_all_roles_and_ordinary_legal_edges() -> Result<(), Box<dyn Error>> {
    let fixture = FixtureWorkspace::new("scalable-policy")?;
    let report = fixture.report()?;
    assert!(
        report.is_valid(),
        "ordinary role-DAG fixture violations: {:#?}",
        report.violations()
    );
    Ok(())
}

#[test]
fn missing_and_unknown_roles_fail_closed() -> Result<(), Box<dyn Error>> {
    let missing = FixtureWorkspace::new("scalable-policy")?;
    missing.replace(
        "crates/domain/f0/Cargo.toml",
        "\n[package.metadata.milkdrift]\nrole = \"domain-foundation\"\n",
        "",
    )?;
    let missing_report = missing.report()?;
    assert!(has_violation(&missing_report, "f0", "ROLE-1"));
    assert!(missing_report.violations().iter().any(|violation| {
        violation.source() == "f0" && violation.reason().contains("missing mandatory")
    }));

    let unknown = FixtureWorkspace::new("scalable-policy")?;
    unknown.replace(
        "crates/domain/f0/Cargo.toml",
        "role = \"domain-foundation\"",
        "role = \"mystery-layer\"",
    )?;
    let unknown_report = unknown.report()?;
    assert!(unknown_report.violations().iter().any(|violation| {
        violation.source() == "f0"
            && violation.rule() == "ROLE-1"
            && violation.reason().contains("unknown role")
    }));
    Ok(())
}

#[test]
fn root_policy_schema_version_is_mandatory_and_exact() -> Result<(), Box<dyn Error>> {
    let missing_namespace = FixtureWorkspace::new("scalable-policy")?;
    missing_namespace.replace(
        "Cargo.toml",
        "[workspace.metadata.milkdrift]",
        "[workspace.metadata.other]",
    )?;
    missing_namespace.replace(
        "Cargo.toml",
        "[[workspace.metadata.milkdrift.exceptions]]",
        "[[workspace.metadata.other.exceptions]]",
    )?;
    let missing_namespace_report = missing_namespace.report()?;
    assert!(
        missing_namespace_report
            .violations()
            .iter()
            .any(|violation| {
                violation.source() == "workspace metadata"
                    && violation.target() == "milkdrift"
                    && violation.rule() == "POLICY-EXCEPTION-1"
                    && violation.reason().contains("missing mandatory")
            })
    );

    let missing_version = FixtureWorkspace::new("scalable-policy")?;
    missing_version.replace("Cargo.toml", "policy-version = 1\n", "")?;
    let missing_version_report = missing_version.report()?;
    assert!(missing_version_report.violations().iter().any(|violation| {
        violation.source() == "workspace metadata"
            && violation.target() == "policy-version"
            && violation.rule() == "POLICY-EXCEPTION-1"
            && violation.reason().contains("missing mandatory")
    }));

    let wrong_version = FixtureWorkspace::new("scalable-policy")?;
    wrong_version.replace("Cargo.toml", "policy-version = 1", "policy-version = 2")?;
    let wrong_version_report = wrong_version.report()?;
    assert!(wrong_version_report.violations().iter().any(|violation| {
        violation.source() == "workspace metadata"
            && violation.target() == "policy-version"
            && violation.rule() == "POLICY-EXCEPTION-1"
            && violation.reason().contains("integer 1")
    }));
    Ok(())
}

#[test]
fn explicit_role_at_an_incompatible_location_fails_without_path_inference()
-> Result<(), Box<dyn Error>> {
    let fixture = FixtureWorkspace::new("scalable-policy")?;
    fixture.replace(
        "crates/domain/f0/Cargo.toml",
        "role = \"domain-foundation\"",
        "role = \"adapter\"",
    )?;
    let report = fixture.report()?;
    assert!(report.violations().iter().any(|violation| {
        violation.source() == "f0"
            && violation.rule() == "LAYOUT-1"
            && violation.reason().contains("never inferred")
    }));
    Ok(())
}

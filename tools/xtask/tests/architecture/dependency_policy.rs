use super::support::*;

#[test]
fn portable_infrastructure_and_runtime_upward_edges_are_denied() -> Result<(), Box<dyn Error>> {
    let cases = [
        (
            "crates/domain/f0/Cargo.toml",
            "adapter = { path = \"../../adapters/adapter\" }",
            "f0",
            "adapter",
        ),
        (
            "crates/platform/platform/Cargo.toml",
            "e0 = { path = \"../../runtime/e0\" }",
            "platform",
            "e0",
        ),
        (
            "crates/adapters/adapter/Cargo.toml",
            "e0 = { path = \"../../runtime/e0\" }",
            "adapter",
            "e0",
        ),
        (
            "crates/runtime/e0/Cargo.toml",
            "e1 = { path = \"../e1\" }",
            "e0",
            "e1",
        ),
    ];

    for (manifest, dependency, source, target) in cases {
        let fixture = FixtureWorkspace::new("scalable-policy")?;
        let mut content = fixture.read(manifest)?;
        write!(content, "\n[dev-dependencies]\n{dependency}\n")?;
        fixture.write(manifest, &content)?;
        let report = fixture.report()?;
        assert!(report.violations().iter().any(|violation| {
            violation.source() == source
                && violation.target() == target
                && violation.rule() == "LAYER-DAG-1"
                && violation.dependency_kind() == Some(DependencyKind::Development)
        }));
    }
    Ok(())
}

#[test]
fn product_dependencies_on_benchmarks_and_tools_are_absolute_denials_for_all_kinds()
-> Result<(), Box<dyn Error>> {
    let kinds = [
        ("dependencies", DependencyKind::Normal),
        ("build-dependencies", DependencyKind::Build),
        ("dev-dependencies", DependencyKind::Development),
    ];
    let targets = [
        (
            "observer",
            "../../../benchmarks/observer",
            "BENCHMARK-OBSERVER-1",
        ),
        (
            "policy-tool",
            "../../../tools/policy-tool",
            "TOOLING-ISOLATION-1",
        ),
    ];

    for (section, expected_kind) in kinds {
        for (target, path, rule) in targets {
            let fixture = FixtureWorkspace::new("scalable-policy")?;
            fixture.write(
                "crates/domain/f1-b/Cargo.toml",
                &format!(
                    "[package]\nname = \"f1-b\"\nversion = \"0.1.0\"\nedition = \"2024\"\npublish = false\n\n[package.metadata.milkdrift]\nrole = \"domain-feature\"\nresponsibility = \"Exercise a fixture dependency policy case\"\n\n[{section}]\n{target} = {{ path = \"{path}\" }}\n"
                ),
            )?;
            let report = fixture.report()?;
            assert!(report.violations().iter().any(|violation| {
                violation.source() == "f1-b"
                    && violation.target() == target
                    && violation.rule() == rule
                    && violation.dependency_kind() == Some(expected_kind)
            }));
        }
    }
    Ok(())
}

#[test]
fn actual_acyclic_domain_peer_edges_need_no_duplicate_registry() -> Result<(), Box<dyn Error>> {
    let fixture = FixtureWorkspace::new("scalable-policy")?;
    let report = fixture.report()?;
    assert!(
        report.is_valid(),
        "ordinary F1 -> F1 -> F0 Cargo graph was rejected: {:#?}",
        report.violations()
    );

    Ok(())
}

#[test]
fn same_layer_runtime_peers_are_not_universally_permitted() -> Result<(), Box<dyn Error>> {
    let fixture = FixtureWorkspace::new("scalable-policy")?;
    fixture.replace(
        "crates/runtime/e1/Cargo.toml",
        "role = \"runtime-application\"",
        "role = \"runtime-foundation\"",
    )?;
    let report = fixture.report()?;
    assert!(report.violations().iter().any(|violation| {
        violation.source() == "e1"
            && violation.target() == "e0"
            && violation.rule() == "LAYER-DAG-1"
    }));
    Ok(())
}

#[test]
fn unreachable_runtime_scope_fails_closed() -> Result<(), Box<dyn Error>> {
    let fixture = FixtureWorkspace::new("scalable-policy")?;
    fixture.replace(
        "crates/apps/app/Cargo.toml",
        "\n[dependencies]\ne1 = { path = \"../../runtime/e1\" }\n",
        "",
    )?;
    let report = fixture.report()?;
    for runtime in ["e0", "e1"] {
        assert!(report.violations().iter().any(|violation| {
            violation.source() == runtime && violation.rule() == "PRODUCT-REACHABILITY-1"
        }));
    }
    Ok(())
}

#[test]
fn build_and_development_edges_follow_explicit_policy_distinctions() -> Result<(), Box<dyn Error>> {
    let legal_build = FixtureWorkspace::new("scalable-policy")?;
    legal_build.replace(
        "crates/runtime/e1/Cargo.toml",
        "[dependencies]",
        "[build-dependencies]",
    )?;
    let legal_report = legal_build.report()?;
    assert!(legal_report.violations().iter().any(|violation| {
        violation.source() == "e0" && violation.rule() == "PRODUCT-REACHABILITY-1"
    }));
    assert!(!legal_report.violations().iter().any(|violation| {
        violation.source() == "e1"
            && violation.target() == "e0"
            && violation.rule() == "LAYER-DAG-1"
    }));

    let observer_build = FixtureWorkspace::new("scalable-policy")?;
    observer_build.replace(
        "benchmarks/observer/Cargo.toml",
        "[dependencies]",
        "[build-dependencies]",
    )?;
    let observer_report = observer_build.report()?;
    assert!(has_violation(
        &observer_report,
        "observer",
        "BENCHMARK-BUILD-1"
    ));

    let reviewed_development = FixtureWorkspace::new("scalable-policy")?;
    let mut e1_manifest = reviewed_development.read("crates/runtime/e1/Cargo.toml")?;
    write!(
        e1_manifest,
        "\n[dev-dependencies]\nf1-b = {{ path = \"../../domain/f1-b\" }}\n"
    )?;
    reviewed_development.write("crates/runtime/e1/Cargo.toml", &e1_manifest)?;
    let unreviewed_report = reviewed_development.report()?;
    assert!(has_violation(
        &unreviewed_report,
        "e1",
        "POLICY-EXCEPTION-1"
    ));
    reviewed_development.append_root(
        "\n[[workspace.metadata.milkdrift.exceptions]]\nid = \"local-e1-f1-b-dev\"\nsource = \"e1\"\ntarget = \"f1-b\"\nscope = \"local\"\nkind = \"development\"\nrationale = \"the fixture proves exact local development-edge review\"\n",
    )?;
    let reviewed_report = reviewed_development.report()?;
    assert!(
        reviewed_report.is_valid(),
        "reviewed downward development edge failed: {:#?}",
        reviewed_report.violations()
    );
    Ok(())
}

#[test]
fn exception_registry_requires_exact_live_edges() -> Result<(), Box<dyn Error>> {
    let missing = FixtureWorkspace::new("scalable-policy")?;
    let root = missing.read("Cargo.toml")?;
    let exception = "\n[[workspace.metadata.milkdrift.exceptions]]\nid = \"external-policy-tool-reviewed-ext\"\nsource = \"policy-tool\"\ntarget = \"reviewed-ext\"\nscope = \"external\"\nkind = \"normal\"\nrationale = \"the fixture tooling dependency exercises exact external exception matching\"\n";
    missing.write("Cargo.toml", &root.replace(exception, ""))?;
    let missing_report = missing.report()?;
    assert!(missing_report.violations().iter().any(|violation| {
        violation.source() == "policy-tool"
            && violation.target() == "reviewed-ext"
            && violation.rule() == "EXTERNAL-DEPENDENCY-1"
    }));

    let wrong_kind = FixtureWorkspace::new("scalable-policy")?;
    wrong_kind.replace("Cargo.toml", "kind = \"normal\"", "kind = \"development\"")?;
    let wrong_kind_report = wrong_kind.report()?;
    assert!(wrong_kind_report.violations().iter().any(|violation| {
        violation.rule() == "POLICY-EXCEPTION-1" && violation.reason().contains("wrong-kind")
    }));

    let stale = FixtureWorkspace::new("scalable-policy")?;
    stale.replace(
        "Cargo.toml",
        "target = \"reviewed-ext\"",
        "target = \"stale-ext\"",
    )?;
    let stale_report = stale.report()?;
    assert!(stale_report.violations().iter().any(|violation| {
        violation.rule() == "POLICY-EXCEPTION-1" && violation.reason().contains("stale exception")
    }));
    Ok(())
}

#[test]
fn exception_registry_rejects_duplicates_missing_packages_and_empty_rationales()
-> Result<(), Box<dyn Error>> {
    let duplicate = FixtureWorkspace::new("scalable-policy")?;
    duplicate.append_root(
        "\n[[workspace.metadata.milkdrift.exceptions]]\nid = \"external-policy-tool-reviewed-ext\"\nsource = \"policy-tool\"\ntarget = \"reviewed-ext\"\nscope = \"external\"\nkind = \"normal\"\nrationale = \"duplicate\"\n",
    )?;
    let duplicate_report = duplicate.report()?;
    assert!(duplicate_report.violations().iter().any(|violation| {
        violation.rule() == "POLICY-EXCEPTION-1"
            && (violation.reason().contains("globally unique")
                || violation.reason().contains("duplicate exception"))
    }));

    let missing_package = FixtureWorkspace::new("scalable-policy")?;
    missing_package.replace(
        "Cargo.toml",
        "source = \"policy-tool\"",
        "source = \"missing-tool\"",
    )?;
    let missing_package_report = missing_package.report()?;
    assert!(missing_package_report.violations().iter().any(|violation| {
        violation.rule() == "POLICY-EXCEPTION-1"
            && violation.reason().contains("not a workspace member")
    }));

    let empty = FixtureWorkspace::new("scalable-policy")?;
    empty.replace(
        "Cargo.toml",
        "rationale = \"the fixture tooling dependency exercises exact external exception matching\"",
        "rationale = \"   \"",
    )?;
    let empty_report = empty.report()?;
    assert!(empty_report.violations().iter().any(|violation| {
        violation.rule() == "POLICY-EXCEPTION-1"
            && violation.reason().contains("nonempty rationale")
    }));
    Ok(())
}

#[test]
fn unnecessary_exceptions_and_attempted_absolute_overrides_fail() -> Result<(), Box<dyn Error>> {
    let unnecessary = FixtureWorkspace::new("scalable-policy")?;
    unnecessary.replace(
        "crates/adapters/adapter/Cargo.toml",
        "platform = { path = \"../../platform/platform\" }",
        "platform = { path = \"../../platform/platform\" }\nreviewed-ext = \"=0.1.0\"",
    )?;
    unnecessary.append_root(
        "\n[[workspace.metadata.milkdrift.exceptions]]\nid = \"external-adapter-reviewed-ext\"\nsource = \"adapter\"\ntarget = \"reviewed-ext\"\nscope = \"external\"\nkind = \"normal\"\nrationale = \"must be rejected as redundant\"\n",
    )?;
    let unnecessary_report = unnecessary.report()?;
    assert!(unnecessary_report.violations().iter().any(|violation| {
        violation.source() == "external-adapter-reviewed-ext"
            && violation.reason().contains("unnecessary exception")
    }));

    let absolute = FixtureWorkspace::new("scalable-policy")?;
    absolute.replace(
        "crates/domain/f1-b/Cargo.toml",
        "f1-a = { path = \"../f1-a\" }",
        "f1-a = { path = \"../f1-a\" }\nadapter = { path = \"../../adapters/adapter\" }",
    )?;
    absolute.append_root(
        "\n[[workspace.metadata.milkdrift.exceptions]]\nid = \"local-f1-b-adapter\"\nsource = \"f1-b\"\ntarget = \"adapter\"\nscope = \"local\"\nkind = \"normal\"\nrationale = \"attempted upward override\"\n",
    )?;
    let absolute_report = absolute.report()?;
    assert!(absolute_report.violations().iter().any(|violation| {
        violation.source() == "local-f1-b-adapter"
            && violation.reason().contains("cannot override absolute")
    }));
    assert!(has_violation(&absolute_report, "f1-b", "LAYER-DAG-1"));
    Ok(())
}

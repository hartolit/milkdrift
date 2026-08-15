use super::support::*;

#[test]
fn benchmark_registry_is_bidirectional_and_rejects_duplicates_and_non_benches()
-> Result<(), Box<dyn Error>> {
    let missing = FixtureWorkspace::new("scalable-policy")?;
    missing.replace(
        "crates/domain/f1-a/Cargo.toml",
        "benchmark-targets = [\"sampling_pipeline\"]",
        "benchmark-targets = [\"missing\"]",
    )?;
    let missing_report = missing.report()?;
    assert!(missing_report.violations().iter().any(|violation| {
        violation.source() == "f1-a"
            && violation.rule() == "BENCHMARK-REGISTRY-1"
            && violation.reason().contains("does not exist")
    }));

    let unregistered = FixtureWorkspace::new("scalable-policy")?;
    unregistered.replace(
        "crates/domain/f1-a/Cargo.toml",
        "benchmark-targets = [\"sampling_pipeline\"]\n",
        "",
    )?;
    let unregistered_report = unregistered.report()?;
    assert!(unregistered_report.violations().iter().any(|violation| {
        violation.source() == "f1-a"
            && violation.rule() == "BENCHMARK-REGISTRY-1"
            && violation.reason().contains("unregistered")
    }));

    let non_bench = FixtureWorkspace::new("scalable-policy")?;
    non_bench.replace(
        "crates/domain/f1-a/Cargo.toml",
        "benchmark-targets = [\"sampling_pipeline\"]",
        "benchmark-targets = [\"sampling_pipeline\", \"ordinary_target\"]",
    )?;
    let non_bench_report = non_bench.report()?;
    assert!(non_bench_report.violations().iter().any(|violation| {
        violation.source() == "f1-a"
            && violation.target() == "ordinary_target"
            && violation.reason().contains("not a Cargo bench target")
    }));

    let duplicate = FixtureWorkspace::new("scalable-policy")?;
    duplicate.replace(
        "crates/domain/f1-a/Cargo.toml",
        "benchmark-targets = [\"sampling_pipeline\"]",
        "benchmark-targets = [\"sampling_pipeline\", \"sampling_pipeline\"]",
    )?;
    let duplicate_report = duplicate.report()?;
    assert!(duplicate_report.violations().iter().any(|violation| {
        violation.source() == "f1-a"
            && violation.rule() == "BENCHMARK-REGISTRY-1"
            && violation.reason().contains("unique")
    }));

    let harnessed = FixtureWorkspace::new("scalable-policy")?;
    harnessed.replace(
        "crates/domain/f1-a/Cargo.toml",
        "harness = false",
        "harness = true",
    )?;
    let harnessed_report = harnessed.report()?;
    assert!(harnessed_report.violations().iter().any(|violation| {
        violation.source() == "f1-a"
            && violation.rule() == "BENCHMARK-REGISTRY-1"
            && violation.reason().contains("harness = false")
    }));

    let implicit = FixtureWorkspace::new("scalable-policy")?;
    implicit.replace(
        "crates/domain/f1-a/Cargo.toml",
        "\n[[bench]]\nname = \"sampling_pipeline\"\nharness = false\n",
        "",
    )?;
    let implicit_report = implicit.report()?;
    assert!(implicit_report.violations().iter().any(|violation| {
        violation.source() == "f1-a"
            && violation.rule() == "BENCHMARK-REGISTRY-1"
            && violation.reason().contains("explicit [[bench]]")
    }));
    Ok(())
}

#[test]
fn generated_benchmark_commands_are_sorted_exact_and_never_workspace_wide()
-> Result<(), Box<dyn Error>> {
    let fixture = FixtureWorkspace::new("scalable-policy")?;
    fixture.refresh_lock()?;
    let commands = benchmark_command_plan(&fixture.manifest())?;
    let arguments = commands
        .iter()
        .map(|command| command.arguments().to_vec())
        .collect::<Vec<_>>();
    assert_eq!(
        arguments,
        vec![
            vec![
                "bench",
                "--locked",
                "-p",
                "f1-a",
                "--bench",
                "sampling_pipeline",
                "--no-run",
            ],
            vec![
                "bench", "--locked", "-p", "observer", "--bench", "runtime", "--no-run",
            ],
        ]
    );
    assert!(
        arguments
            .iter()
            .flatten()
            .all(|argument| argument != "--workspace")
    );

    let actual = benchmark_command_plan(&workspace_manifest())?;
    let actual_pairs = actual
        .iter()
        .map(|command| {
            let arguments = command.arguments();
            (arguments.get(3).cloned(), arguments.get(5).cloned())
        })
        .collect::<Vec<_>>();
    assert_eq!(
        actual_pairs,
        vec![
            (
                Some("runtime-benchmarks".to_owned()),
                Some("runtime".to_owned())
            ),
            (
                Some("sampling".to_owned()),
                Some("sampling_pipeline".to_owned())
            ),
        ]
    );
    Ok(())
}

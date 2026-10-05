//! Exact inward edges, independent of spelling and the current target's active branches.

use std::collections::{BTreeMap, BTreeSet};

use super::{
    TestResult,
    manifests::{self, Dependency},
    read, root,
};

fn internal_edges(owner: &str) -> Option<&'static str> {
    Some(match owner.strip_prefix("milkdrift-")? {
        "contracts" | "slotbook" => "",
        "capability" | "control-protocol" => "contracts",
        "blueprint" | "workspace" => "capability contracts",
        "authority" => "blueprint capability contracts workspace",
        "model" => "blueprint capability contracts persistence workspace",
        "persistence" => "authority blueprint capability contracts peer-protocol workspace",
        "runtime" => "authority blueprint capability contracts model persistence workspace",
        "capability-host" => {
            "authority blueprint capability contracts peer-protocol persistence runtime workspace"
        }
        "control" => {
            "authority blueprint capability capability-host contracts model persistence runtime workspace"
        }
        "prompt-sequence" => {
            "authority blueprint capability contracts control persistence workspace"
        }
        "peer-protocol" => "authority capability contracts workspace",
        "control-client" => "capability control-protocol peer-protocol",
        "cli" => {
            // Workspace contributes validated identity contracts for local upload preflight.
            // Runtime services and concrete stores/adapters remain forbidden to clients.
            "authority blueprint capability control-client control-protocol peer-protocol prompt-sequence workspace"
        }
        "local-process" => "authority capability capability-host contracts workspace",
        "model-provider" => "authority capability capability-host contracts model workspace",
        "local-secret" => "authority capability-host",
        "managed-linux" => {
            "authority capability capability-host contracts model-provider persistence workspace"
        }
        "redb-store" => {
            "authority blueprint capability contracts peer-protocol persistence workspace"
        }
        "peer-http" => {
            "authority blueprint capability capability-host contracts peer-protocol persistence runtime workspace"
        }
        "daemon" => {
            "authority blueprint capability capability-host contracts control control-protocol local-process local-secret managed-linux model model-provider peer-http peer-protocol persistence prompt-sequence redb-store runtime workspace"
        }
        "evidence" => {
            "authority blueprint capability capability-host contracts control control-client control-protocol daemon local-process managed-linux model model-provider peer-protocol persistence prompt-sequence redb-store runtime workspace"
        }
        _ => return None,
    })
}

fn edge_errors(owner: &str, edges: &[Dependency]) -> Vec<String> {
    let Some(allowed) = internal_edges(owner) else {
        return vec![format!("unreviewed package owner: {owner}")];
    };
    let mut errors = Vec::new();
    for edge in edges {
        // Development dependencies do not enter a normal product graph. The resolved product
        // feature check separately detects features forwarded through intermediate packages.
        if edge.kind == "dev" {
            continue;
        }
        if let Some(internal) = edge.package.strip_prefix("milkdrift-")
            && !allowed.split_whitespace().any(|name| name == internal)
        {
            errors.push(format!(
                "{owner}: forbidden {} edge {} ({})",
                edge.kind, edge.alias, edge.package
            ));
        }
        if owner != "milkdrift-evidence"
            && edge
                .features
                .iter()
                .any(|f| super::product_graph::HELPER_FEATURES.contains(&f.as_str()))
        {
            errors.push(format!("{owner}: helper feature on {}", edge.package));
        }
        // Future client packages can have their own UI dependencies after an ownership review.
        // None of the current Rust backend owners is an inference or GUI implementation.
        if [
            "iced",
            "egui",
            "tauri",
            "slint",
            "dioxus",
            "candle",
            "candle-core",
            "burn",
            "llama",
            "llama-cpp-2",
            "tch",
            "onnx",
            "ort",
        ]
        .contains(&edge.package.as_str())
        {
            errors.push(format!("{owner}: UI/inference mechanism {}", edge.package));
        }
        if owner != "milkdrift-evidence"
            && ["syn", "proc-macro2", "cargo_metadata", "divan", "proptest"]
                .contains(&edge.package.as_str())
        {
            errors.push(format!(
                "{owner}: development parser/test tool {} in product",
                edge.package
            ));
        }
        if edge.package == "redb" && owner != "milkdrift-redb-store" {
            errors.push(format!("{owner}: database mechanism bypass"));
        }
        if matches!(
            owner,
            "milkdrift-contracts"
                | "milkdrift-capability"
                | "milkdrift-blueprint"
                | "milkdrift-authority"
                | "milkdrift-workspace"
                | "milkdrift-persistence"
                | "milkdrift-model"
                | "milkdrift-control-protocol"
                | "milkdrift-peer-protocol"
        ) && [
            "axum",
            "tokio",
            "reqwest",
            "hyper",
            "tower",
            "redb",
            "rustix",
            "nix",
            "windows-sys",
        ]
        .contains(&edge.package.as_str())
        {
            errors.push(format!(
                "{owner}: mechanism-neutral contract imports {}",
                edge.package
            ));
        }
    }
    errors
}

#[test]
fn cargo_identities_enforce_dependency_direction_and_product_reachability() -> TestResult {
    let repository = root()?;
    let metadata = manifests::metadata(&repository)?;
    let workspace: toml::Value = toml::from_str(&read(repository.join("Cargo.toml"))?)?;
    let mut graph = BTreeMap::new();
    let mut errors = Vec::new();
    for package in metadata
        .packages
        .iter()
        .filter(|p| metadata.workspace_members.contains(&p.id))
    {
        let manifest = toml::from_str(&read(&package.manifest_path)?)?;
        let edges = manifests::dependencies(&manifest, &workspace)?;
        // Cross-check normalization against Cargo, rather than trusting our parser alone.
        for edge in &edges {
            assert!(
                package.dependencies.iter().any(|d| d.name == edge.package
                    && d.rename.as_deref().unwrap_or(&d.name) == edge.alias
                    && d.kind.as_deref().unwrap_or("normal") == edge.kind
                    && d.target == edge.target),
                "Cargo and declaration disagree in {}: {edge:?}",
                package.name
            );
        }
        errors.extend(edge_errors(&package.name, &edges));
        graph.insert(
            package.name.clone(),
            edges
                .into_iter()
                .filter(|e| e.kind != "dev")
                .map(|e| e.package)
                .collect::<BTreeSet<_>>(),
        );
    }
    for client in [
        "milkdrift-cli",
        "milkdrift-control-client",
        "milkdrift-control-protocol",
    ] {
        let reachable = reachable(&graph, client);
        for forbidden in [
            "milkdrift-daemon",
            "milkdrift-evidence",
            "milkdrift-redb-store",
            "redb",
            "milkdrift-local-process",
            "milkdrift-model-provider",
            "milkdrift-managed-linux",
        ] {
            if reachable.contains(forbidden) {
                errors.push(format!("{client} reaches forbidden product {forbidden}"));
            }
        }
    }
    let daemon = reachable(&graph, "milkdrift-daemon");
    for adapter in [
        "redb-store",
        "local-process",
        "local-secret",
        "model-provider",
        "peer-http",
        "managed-linux",
    ] {
        assert!(
            daemon.contains(&format!("milkdrift-{adapter}")),
            "daemon lost adopted {adapter}; conformance tests still prove actual composition"
        );
    }
    assert!(errors.is_empty(), "{}", errors.join("\n"));
    Ok(())
}

fn reachable(graph: &BTreeMap<String, BTreeSet<String>>, root: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let mut pending = vec![root.to_owned()];
    while let Some(owner) = pending.pop() {
        if !found.insert(owner.clone()) {
            continue;
        }
        if let Some(edges) = graph.get(&owner) {
            pending.extend(edges.iter().cloned());
        }
    }
    found
}

#[test]
fn direction_policy_rejects_alias_optional_target_build_and_unknown_owner() -> TestResult {
    for section in [
        "dependencies",
        "build-dependencies",
        "target.'cfg(windows)'.dependencies",
    ] {
        let manifest = toml::from_str(&format!(
            "[{section}.innocent]\npackage = 'milkdrift-runtime'\nversion = '1'\noptional = true\n"
        ))?;
        let edges = manifests::dependencies(&manifest, &toml::Value::Table(toml::Table::new()))?;
        assert!(
            edge_errors("milkdrift-cli", &edges)
                .iter()
                .any(|e| e.contains("forbidden"))
        );
    }
    let manifest = toml::from_str("[dev-dependencies.storage]\npackage = 'redb'\nversion = '2'\n")?;
    let edges = manifests::dependencies(&manifest, &toml::Value::Table(toml::Table::new()))?;
    assert!(edge_errors("milkdrift-runtime", &edges).is_empty());
    assert!(!edge_errors("milkdrift-unreviewed", &[]).is_empty());
    let graph = BTreeMap::from([
        ("client".to_owned(), BTreeSet::from(["bridge".to_owned()])),
        ("bridge".to_owned(), BTreeSet::from(["database".to_owned()])),
    ]);
    assert!(reachable(&graph, "client").contains("database"));
    Ok(())
}

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use cargo_metadata::{Dependency, Metadata, Package};

use super::cuda::validate_cuda_policy;
use super::exceptions::load_exception_registry;
use super::policy::{
    DependencyKind, ExternalDecision, Layer, PolicyFailure, RULE_BENCHMARK_BUILD,
    RULE_BENCHMARK_PACKAGE, RULE_BENCHMARK_REGISTRY, RULE_DOMAIN_DAG, RULE_EXCEPTION,
    RULE_EXTERNAL, RULE_KNOWN_KIND, RULE_LOCAL_TARGET, RULE_LOCATION, RULE_ROLE, dependency_kind,
    external_dependency_policy, local_dependency_policy,
};
use super::report::{ValidationError, ValidationReport, Violation};
use crate::workspace::{
    benchmark_inventory, load_metadata, package_role, relative_manifest,
    role_location_is_compatible,
};

/// Loads locked typed Cargo metadata and validates the workspace containing `manifest_path`.
///
/// The metadata command always uses `--locked --no-deps`. Direct declarations are sufficient for
/// architecture enforcement and avoid conflating the transitive vendor graph with workspace roles.
///
/// # Errors
///
/// Returns an error if Cargo cannot produce locked metadata.
pub fn validate_workspace(manifest_path: &Path) -> Result<ValidationReport, ValidationError> {
    let metadata = load_metadata(manifest_path, true)?;
    Ok(validate_metadata(&metadata))
}

struct WorkspaceIndex<'a> {
    packages_by_name: BTreeMap<String, &'a Package>,
    packages_by_directory: BTreeMap<PathBuf, &'a Package>,
    roles: BTreeMap<String, Layer>,
}

struct DependencyValidationContext<'a> {
    packages_by_directory: &'a BTreeMap<PathBuf, &'a Package>,
    roles: &'a BTreeMap<String, Layer>,
    exceptions: &'a super::exceptions::ExceptionRegistry,
}

fn validate_metadata(metadata: &Metadata) -> ValidationReport {
    let root = metadata.workspace_root.as_std_path();
    let packages = metadata.workspace_packages();
    let mut report = ValidationReport::default();
    let WorkspaceIndex {
        packages_by_name,
        packages_by_directory,
        roles,
    } = build_workspace_index(root, &packages, &mut report);

    match benchmark_inventory(metadata) {
        Ok(_) => {}
        Err(issues) => {
            for issue in issues {
                report.push(Violation::new(
                    issue.package,
                    issue.target,
                    None,
                    None,
                    None,
                    RULE_BENCHMARK_REGISTRY,
                    issue.reason,
                ));
            }
        }
    }

    let exceptions = load_exception_registry(
        metadata,
        &packages_by_name,
        &packages_by_directory,
        &roles,
        &mut report,
    );
    validate_cuda_policy(
        &packages,
        &packages_by_name,
        &roles,
        &exceptions,
        &mut report,
    );

    let context = DependencyValidationContext {
        packages_by_directory: &packages_by_directory,
        roles: &roles,
        exceptions: &exceptions,
    };
    let mut domain_graph = roles
        .iter()
        .filter_map(|(package, role)| {
            role.is_domain()
                .then_some((package.clone(), BTreeSet::new()))
        })
        .collect::<BTreeMap<_, _>>();

    for package in packages {
        let source_name = package.name.to_string();
        let Some(source_role) = roles.get(&source_name).copied() else {
            continue;
        };
        for dependency in &package.dependencies {
            validate_dependency(
                &mut report,
                &source_name,
                source_role,
                dependency,
                &context,
                &mut domain_graph,
            );
        }
    }

    if let Some(cycle) = find_directed_cycle(&domain_graph) {
        let rendered = cycle.join(" -> ");
        report.push(Violation::new(
            "domain production graph".to_owned(),
            rendered.clone(),
            None,
            None,
            None,
            RULE_DOMAIN_DAG,
            format!(
                "actual normal/build Cargo edges among F0/F1 packages must be acyclic; detected {rendered}"
            ),
        ));
    }

    report
}

fn build_workspace_index<'a>(
    root: &Path,
    packages: &[&'a Package],
    report: &mut ValidationReport,
) -> WorkspaceIndex<'a> {
    let mut packages_by_name = BTreeMap::new();
    let mut packages_by_directory = BTreeMap::new();
    let mut roles = BTreeMap::new();

    for package in packages {
        let name = package.name.to_string();
        if packages_by_name.insert(name.clone(), *package).is_some() {
            report.push(Violation::new(
                name.clone(),
                package.manifest_path.to_string(),
                None,
                None,
                None,
                RULE_ROLE,
                "workspace package names must be unique because roles, exceptions, and exact Cargo -p plans use package identity".to_owned(),
            ));
        }
        if let Some(directory) = package.manifest_path.parent() {
            packages_by_directory.insert(directory.as_std_path().to_path_buf(), *package);
        }

        match package_role(package) {
            Ok(role) => {
                roles.insert(name.clone(), role);
                if !role_location_is_compatible(root, package, role) {
                    report.push(Violation::new(
                        name.clone(),
                        relative_manifest(root, package).display().to_string(),
                        None,
                        Some(role),
                        None,
                        RULE_LOCATION,
                        format!(
                            "explicit role `{role}` is incompatible with this manifest location; roles are never inferred from path prefixes and each role is limited to a direct child of its owned repository root"
                        ),
                    ));
                }
                if role == Layer::BenchmarkObserver {
                    validate_benchmark_observer(report, package, role);
                }
            }
            Err(error) => report.push(Violation::new(
                name,
                relative_manifest(root, package).display().to_string(),
                None,
                None,
                None,
                RULE_ROLE,
                error.reason,
            )),
        }
    }

    WorkspaceIndex {
        packages_by_name,
        packages_by_directory,
        roles,
    }
}

fn validate_benchmark_observer(report: &mut ValidationReport, package: &Package, role: Layer) {
    if package
        .publish
        .as_ref()
        .is_none_or(|registries| !registries.is_empty())
    {
        report.push(Violation::new(
            package.name.to_string(),
            package.manifest_path.to_string(),
            None,
            Some(role),
            None,
            RULE_BENCHMARK_PACKAGE,
            "benchmark observers must declare publish = false".to_owned(),
        ));
    }
    if package
        .targets
        .iter()
        .any(cargo_metadata::Target::is_custom_build)
        || package
            .dependencies
            .iter()
            .any(|dependency| dependency_kind(dependency.kind) == Some(DependencyKind::Build))
    {
        report.push(Violation::new(
            package.name.to_string(),
            package.manifest_path.to_string(),
            None,
            Some(role),
            None,
            RULE_BENCHMARK_BUILD,
            "benchmark observers cannot declare build.rs, custom-build targets, or build dependencies"
                .to_owned(),
        ));
    }
}

fn validate_dependency(
    report: &mut ValidationReport,
    source_name: &str,
    source_role: Layer,
    dependency: &Dependency,
    context: &DependencyValidationContext<'_>,
    domain_graph: &mut BTreeMap<String, BTreeSet<String>>,
) {
    let Some(kind) = dependency_kind(dependency.kind) else {
        report.push(Violation::new(
            source_name.to_owned(),
            dependency.name.clone(),
            None,
            Some(source_role),
            None,
            RULE_KNOWN_KIND,
            format!(
                "Cargo reported unsupported dependency kind {:?}; unknown kinds fail closed",
                dependency.kind
            ),
        ));
        return;
    };

    if let Some(path) = &dependency.path {
        let Some(target) = context
            .packages_by_directory
            .get(path.as_std_path())
            .copied()
        else {
            report.push(Violation::new(
                source_name.to_owned(),
                path.to_string(),
                Some(kind),
                Some(source_role),
                None,
                RULE_LOCAL_TARGET,
                "path dependencies must resolve to an explicitly role-classified member of this workspace; outside, excluded, and unknown paths fail closed".to_owned(),
            ));
            return;
        };
        let target_name = target.name.to_string();
        let Some(target_role) = context.roles.get(&target_name).copied() else {
            report.push(Violation::new(
                source_name.to_owned(),
                target_name,
                Some(kind),
                Some(source_role),
                None,
                RULE_LOCAL_TARGET,
                "path dependency target has no valid explicit Milkdrift role".to_owned(),
            ));
            return;
        };

        if source_role.is_domain() && target_role.is_domain() && kind.is_production() {
            domain_graph
                .entry(source_name.to_owned())
                .or_default()
                .insert(target.name.to_string());
        }
        if let Some(failure) = local_dependency_policy(source_role, target_role, kind) {
            report.push(edge_violation(
                source_name,
                source_role,
                &target.name,
                Some(target_role),
                kind,
                failure,
            ));
        } else if kind == DependencyKind::Development
            && !context
                .exceptions
                .has_local(source_name, target.name.as_ref(), kind)
        {
            report.push(edge_violation(
                source_name,
                source_role,
                &target.name,
                Some(target_role),
                kind,
                PolicyFailure {
                    rule: RULE_EXCEPTION,
                    reason: "workspace-local development dependencies require an exact live local exception with a nonempty rationale; normal/build edges use the generic DAG without duplicate records".to_owned(),
                },
            ));
        }
        return;
    }

    validate_external_dependency(
        report,
        source_name,
        source_role,
        dependency,
        kind,
        context.exceptions,
    );
}

fn validate_external_dependency(
    report: &mut ValidationReport,
    source_name: &str,
    source_role: Layer,
    dependency: &Dependency,
    kind: DependencyKind,
    exceptions: &super::exceptions::ExceptionRegistry,
) {
    match external_dependency_policy(source_role, &dependency.name, kind) {
        ExternalDecision::Allowed => {}
        ExternalDecision::NeedsException => {
            if !exceptions.has_external(source_name, &dependency.name, kind) {
                report.push(edge_violation(
                    source_name,
                    source_role,
                    &dependency.name,
                    None,
                    kind,
                    PolicyFailure {
                        rule: RULE_EXTERNAL,
                        reason: "this role/kind requires an exact external exception record matching source, target, scope, and kind with a nonempty rationale".to_owned(),
                    },
                ));
            }
        }
        ExternalDecision::Denied(mut failure) => {
            if failure.reason.is_empty() {
                "benchmark observers cannot use external build dependencies, and exceptions cannot override this absolute denial"
                    .clone_into(&mut failure.reason);
            }
            report.push(edge_violation(
                source_name,
                source_role,
                &dependency.name,
                None,
                kind,
                failure,
            ));
        }
    }
}

fn edge_violation(
    source: &str,
    source_role: Layer,
    target: &str,
    target_role: Option<Layer>,
    dependency_kind: DependencyKind,
    failure: PolicyFailure,
) -> Violation {
    Violation::new(
        source.to_owned(),
        target.to_owned(),
        Some(dependency_kind),
        Some(source_role),
        target_role,
        failure.rule,
        failure.reason,
    )
}

pub(super) fn find_directed_cycle(
    graph: &BTreeMap<String, BTreeSet<String>>,
) -> Option<Vec<String>> {
    fn visit(
        node: &str,
        graph: &BTreeMap<String, BTreeSet<String>>,
        visited: &mut BTreeSet<String>,
        stack: &mut Vec<String>,
    ) -> Option<Vec<String>> {
        if let Some(position) = stack.iter().position(|candidate| candidate == node) {
            let mut cycle = stack.get(position..)?.to_vec();
            cycle.push(node.to_owned());
            return Some(cycle);
        }
        if visited.contains(node) {
            return None;
        }

        stack.push(node.to_owned());
        if let Some(targets) = graph.get(node) {
            for target in targets {
                if let Some(cycle) = visit(target, graph, visited, stack) {
                    return Some(cycle);
                }
            }
        }
        stack.pop();
        visited.insert(node.to_owned());
        None
    }

    let mut visited = BTreeSet::new();
    let mut stack = Vec::new();
    for node in graph.keys() {
        if let Some(cycle) = visit(node, graph, &mut visited, &mut stack) {
            return Some(cycle);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use super::find_directed_cycle;

    fn graph(edges: &[(&str, &str)]) -> BTreeMap<String, BTreeSet<String>> {
        let mut graph = BTreeMap::new();
        for (source, target) in edges {
            graph
                .entry((*source).to_owned())
                .or_insert_with(BTreeSet::new)
                .insert((*target).to_owned());
            graph
                .entry((*target).to_owned())
                .or_insert_with(BTreeSet::new);
        }
        graph
    }

    #[test]
    fn actual_domain_graph_helper_distinguishes_dags_and_cycles() {
        assert!(find_directed_cycle(&graph(&[("f1-b", "f1-a"), ("f1-a", "f0")])).is_none());
        assert!(find_directed_cycle(&graph(&[("f1-a", "f1-b"), ("f1-b", "f1-a")])).is_some());
    }
}

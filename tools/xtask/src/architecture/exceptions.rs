use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use cargo_metadata::{Dependency, Metadata, Package};
use serde_json::{Map, Value};

use super::policy::{
    DependencyKind, ExternalDecision, Layer, RULE_EXCEPTION, dependency_kind,
    external_dependency_policy, local_dependency_policy,
};
use super::report::{ValidationReport, Violation};
use crate::workspace::METADATA_NAMESPACE;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum ExceptionScope {
    External,
    Local,
    CudaForward,
}

impl ExceptionScope {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "external" => Some(Self::External),
            "local" => Some(Self::Local),
            "cuda-forward" => Some(Self::CudaForward),
            _ => None,
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::External => "external",
            Self::Local => "local",
            Self::CudaForward => "cuda-forward",
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct ExceptionRecord {
    pub(super) id: String,
    pub(super) source: String,
    pub(super) target: String,
    pub(super) scope: ExceptionScope,
    pub(super) kind: DependencyKind,
    rationale: String,
}

#[derive(Debug, Default)]
pub(super) struct ExceptionRegistry {
    records: Vec<ExceptionRecord>,
}

impl ExceptionRegistry {
    pub(super) fn has_external(&self, source: &str, target: &str, kind: DependencyKind) -> bool {
        self.has_edge(ExceptionScope::External, source, target, kind)
    }

    pub(super) fn has_local(&self, source: &str, target: &str, kind: DependencyKind) -> bool {
        self.has_edge(ExceptionScope::Local, source, target, kind)
    }

    fn has_edge(
        &self,
        scope: ExceptionScope,
        source: &str,
        target: &str,
        kind: DependencyKind,
    ) -> bool {
        self.records.iter().any(|record| {
            record.scope == scope
                && record.source == source
                && record.target == target
                && record.kind == kind
                && !record.rationale.trim().is_empty()
        })
    }

    pub(super) fn cuda_forwards_from(
        &self,
        source: &str,
    ) -> impl Iterator<Item = &ExceptionRecord> {
        self.records.iter().filter(move |record| {
            record.scope == ExceptionScope::CudaForward
                && record.source == source
                && !record.rationale.trim().is_empty()
        })
    }

    pub(super) fn has_cuda_forward(
        &self,
        source: &str,
        target: &str,
        kind: DependencyKind,
    ) -> bool {
        self.cuda_forwards_from(source)
            .any(|record| record.target == target && record.kind == kind)
    }
}

pub(super) fn load_exception_registry(
    metadata: &Metadata,
    packages_by_name: &BTreeMap<String, &Package>,
    packages_by_directory: &BTreeMap<PathBuf, &Package>,
    roles: &BTreeMap<String, Layer>,
    report: &mut ValidationReport,
) -> ExceptionRegistry {
    let Some(namespace) = metadata.workspace_metadata.get(METADATA_NAMESPACE) else {
        push_configuration_violation(
            report,
            "workspace metadata",
            METADATA_NAMESPACE,
            format!(
                "missing mandatory [workspace.metadata.{METADATA_NAMESPACE}] policy declaration"
            ),
        );
        return ExceptionRegistry::default();
    };
    let Some(table) = namespace.as_object() else {
        push_configuration_violation(
            report,
            "workspace metadata",
            METADATA_NAMESPACE,
            format!("[workspace.metadata.{METADATA_NAMESPACE}] must be a table"),
        );
        return ExceptionRegistry::default();
    };

    validate_policy_version(table, report);
    let Some(exceptions) = table.get("exceptions") else {
        return ExceptionRegistry::default();
    };
    let Some(exceptions) = exceptions.as_array() else {
        push_configuration_violation(
            report,
            "workspace metadata",
            "exceptions",
            "exceptions must be an array of tables".to_owned(),
        );
        return ExceptionRegistry::default();
    };

    let mut records = Vec::new();
    for (index, value) in exceptions.iter().enumerate() {
        if let Some(record) = parse_record(value, index, report) {
            records.push(record);
        }
    }

    validate_duplicates(&records, report);
    for record in &records {
        validate_record(
            record,
            packages_by_name,
            packages_by_directory,
            roles,
            report,
        );
    }

    ExceptionRegistry { records }
}

fn validate_policy_version(table: &Map<String, Value>, report: &mut ValidationReport) {
    let Some(version) = table.get("policy-version") else {
        push_configuration_violation(
            report,
            "workspace metadata",
            "policy-version",
            "missing mandatory integer policy-version = 1".to_owned(),
        );
        return;
    };
    if version.as_u64() != Some(1) {
        push_configuration_violation(
            report,
            "workspace metadata",
            "policy-version",
            "policy-version must be the integer 1".to_owned(),
        );
    }
}

fn parse_record(
    value: &Value,
    index: usize,
    report: &mut ValidationReport,
) -> Option<ExceptionRecord> {
    let Some(table) = value.as_object() else {
        push_configuration_violation(
            report,
            "workspace exceptions",
            &format!("entry {index}"),
            "every exception must be a table".to_owned(),
        );
        return None;
    };

    let mut malformed = false;
    let mut field = |name: &str| -> String {
        if let Some(value) = table.get(name).and_then(Value::as_str) {
            value.to_owned()
        } else {
            malformed = true;
            push_configuration_violation(
                report,
                "workspace exceptions",
                &format!("entry {index}.{name}"),
                format!("exception field `{name}` must be a string"),
            );
            String::new()
        }
    };

    let id = field("id");
    let source = field("source");
    let target = field("target");
    let scope_name = field("scope");
    let kind_name = field("kind");
    let rationale = field("rationale");
    if malformed {
        return None;
    }

    if id.trim().is_empty()
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        push_configuration_violation(
            report,
            &id,
            "id",
            "exception ids must be nonempty stable lowercase kebab-case identifiers".to_owned(),
        );
    }
    if source.trim().is_empty() || target.trim().is_empty() {
        push_configuration_violation(
            report,
            &id,
            "source/target",
            "exception source and target names must be nonempty".to_owned(),
        );
    }
    if rationale.trim().is_empty() {
        push_configuration_violation(
            report,
            &id,
            "rationale",
            "every exception requires a nonempty rationale".to_owned(),
        );
    }

    let Some(scope) = ExceptionScope::parse(&scope_name) else {
        push_configuration_violation(
            report,
            &id,
            &scope_name,
            "exception scope must be `external`, `local`, or `cuda-forward`".to_owned(),
        );
        return None;
    };
    let Some(kind) = DependencyKind::parse(&kind_name) else {
        push_configuration_violation(
            report,
            &id,
            &kind_name,
            "exception kind must be `normal`, `build`, or `development`".to_owned(),
        );
        return None;
    };

    Some(ExceptionRecord {
        id,
        source,
        target,
        scope,
        kind,
        rationale,
    })
}

fn validate_duplicates(records: &[ExceptionRecord], report: &mut ValidationReport) {
    let mut ids = BTreeSet::new();
    let mut edges = BTreeSet::new();
    for record in records {
        if !ids.insert(record.id.as_str()) {
            push_configuration_violation(
                report,
                &record.id,
                "id",
                "exception ids must be globally unique".to_owned(),
            );
        }
        let key = (
            record.source.as_str(),
            record.target.as_str(),
            record.scope,
            record.kind,
        );
        if !edges.insert(key) {
            push_configuration_violation(
                report,
                &record.id,
                &render_record(record),
                "duplicate exception: source, target, scope, and kind must be unique".to_owned(),
            );
        }
    }
}

fn validate_record(
    record: &ExceptionRecord,
    packages_by_name: &BTreeMap<String, &Package>,
    packages_by_directory: &BTreeMap<PathBuf, &Package>,
    roles: &BTreeMap<String, Layer>,
    report: &mut ValidationReport,
) {
    let Some(source) = packages_by_name.get(&record.source).copied() else {
        push_configuration_violation(
            report,
            &record.id,
            &record.source,
            "exception source package is not a workspace member".to_owned(),
        );
        return;
    };

    match record.scope {
        ExceptionScope::External => {
            validate_external_record(record, source, packages_by_name, roles, report);
        }
        ExceptionScope::Local | ExceptionScope::CudaForward => {
            let Some(target) = packages_by_name.get(&record.target).copied() else {
                push_configuration_violation(
                    report,
                    &record.id,
                    &record.target,
                    "local and CUDA-forward exception targets must be workspace packages"
                        .to_owned(),
                );
                return;
            };
            validate_local_record(record, source, target, packages_by_directory, roles, report);
        }
    }
}

fn validate_external_record(
    record: &ExceptionRecord,
    source: &Package,
    packages_by_name: &BTreeMap<String, &Package>,
    roles: &BTreeMap<String, Layer>,
    report: &mut ValidationReport,
) {
    if packages_by_name.contains_key(&record.target) {
        push_configuration_violation(
            report,
            &record.id,
            &record.target,
            "an external exception cannot target a workspace package; use Cargo path metadata and the local role DAG".to_owned(),
        );
        return;
    }

    let matching = source
        .dependencies
        .iter()
        .filter(|dependency| dependency.path.is_none() && dependency.name == record.target)
        .collect::<Vec<_>>();
    validate_matching_kind(record, &matching, report);

    if let Some(role) = roles.get(&record.source).copied() {
        match external_dependency_policy(role, &record.target, record.kind) {
            ExternalDecision::Allowed => push_configuration_violation(
                report,
                &record.id,
                &render_record(record),
                "unnecessary exception: this external edge is already permitted by the generic role policy".to_owned(),
            ),
            ExternalDecision::NeedsException => {}
            ExternalDecision::Denied(_) => push_configuration_violation(
                report,
                &record.id,
                &render_record(record),
                "exception cannot override an absolute dependency denial".to_owned(),
            ),
        }
    }
}

fn validate_local_record(
    record: &ExceptionRecord,
    source: &Package,
    target: &Package,
    packages_by_directory: &BTreeMap<PathBuf, &Package>,
    roles: &BTreeMap<String, Layer>,
    report: &mut ValidationReport,
) {
    let matching = source
        .dependencies
        .iter()
        .filter(|dependency| {
            dependency_target(dependency, packages_by_directory)
                .is_some_and(|package| package.name == target.name)
        })
        .collect::<Vec<_>>();
    validate_matching_kind(record, &matching, report);

    if record.scope == ExceptionScope::CudaForward {
        let reference = format!("{}/cuda", record.target);
        let exact_dependency = matching.iter().any(|dependency| {
            dependency.rename.is_none()
                && dependency.name == record.target
                && dependency_kind(dependency.kind) == Some(record.kind)
        });
        let exact_feature = source
            .features
            .get("cuda")
            .is_some_and(|values| values.iter().any(|value| value == &reference));
        if !exact_dependency || !exact_feature {
            push_configuration_violation(
                report,
                &record.id,
                &render_record(record),
                "stale CUDA-forward exception: the exact unrenamed dependency kind and source `cuda` feature reference do not both exist".to_owned(),
            );
        }
        return;
    }

    if let (Some(source_role), Some(target_role)) = (
        roles.get(&record.source).copied(),
        roles.get(&record.target).copied(),
    ) {
        if local_dependency_policy(source_role, target_role, record.kind).is_some() {
            push_configuration_violation(
                report,
                &record.id,
                &render_record(record),
                "local exceptions cannot override absolute role-DAG, tooling, or observer denials"
                    .to_owned(),
            );
        } else if record.kind != DependencyKind::Development {
            push_configuration_violation(
                report,
                &record.id,
                &render_record(record),
                "unnecessary exception: ordinary legal normal/build edges are already permitted by the generic role DAG"
                    .to_owned(),
            );
        }
    }
}

fn validate_matching_kind(
    record: &ExceptionRecord,
    matching: &[&Dependency],
    report: &mut ValidationReport,
) {
    if matching
        .iter()
        .any(|dependency| dependency_kind(dependency.kind) == Some(record.kind))
    {
        return;
    }

    let reason = if matching.is_empty() {
        "stale exception: no dependency declaration matches its source, target, and scope"
            .to_owned()
    } else {
        let actual = matching
            .iter()
            .filter_map(|dependency| dependency_kind(dependency.kind))
            .map(|kind| kind.to_string())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "wrong-kind exception: registered `{}` but Cargo declares `{actual}`",
            record.kind
        )
    };
    push_configuration_violation(report, &record.id, &render_record(record), reason);
}

fn dependency_target<'a>(
    dependency: &Dependency,
    packages_by_directory: &'a BTreeMap<PathBuf, &Package>,
) -> Option<&'a Package> {
    dependency
        .path
        .as_ref()
        .and_then(|path| packages_by_directory.get(path.as_std_path()).copied())
}

fn render_record(record: &ExceptionRecord) -> String {
    format!(
        "{} --{}:{}--> {}",
        record.source,
        record.scope.as_str(),
        record.kind,
        record.target
    )
}

fn push_configuration_violation(
    report: &mut ValidationReport,
    source: &str,
    target: &str,
    reason: String,
) {
    report.push(Violation::new(
        source.to_owned(),
        target.to_owned(),
        None,
        None,
        None,
        RULE_EXCEPTION,
        reason,
    ));
}

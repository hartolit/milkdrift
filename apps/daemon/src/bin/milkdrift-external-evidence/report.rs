use std::{collections::BTreeSet, fs, path::Path};

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const REPORT_SCHEMA_VERSION: u32 = 1;
const MAX_REPORT_BYTES: usize = 1_048_576;
const MAX_REPORT_ITEMS: usize = 4_096;

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceReport {
    pub schema_version: u32,
    pub generated_at_unix_ms: u64,
    pub platform: PlatformEvidence,
    pub milkdrift: MilkdriftEvidence,
    pub configuration_digest: Option<String>,
    pub qualifying: bool,
    pub fixture_mode: bool,
    pub process: ScenarioEvidence,
    pub model: ScenarioEvidence,
    pub validation: Vec<ValidationEvidence>,
    pub redactions: Vec<String>,
    pub failure_reason: Option<String>,
}

impl EvidenceReport {
    fn validate(&self) -> Result<(), String> {
        if self.schema_version != REPORT_SCHEMA_VERSION {
            return Err("unsupported evidence report schema version".to_owned());
        }
        for (name, scenario) in [("process", &self.process), ("model", &self.model)] {
            if !scenario.profile.is_object() || !scenario.facts.is_object() {
                return Err(format!("{name} profile/facts must be objects"));
            }
            if !matches!(
                scenario.outcome.as_str(),
                "not_run" | "succeeded" | "failed"
            ) {
                return Err(format!("{name} has an invalid outcome"));
            }
            if scenario.qualifying && scenario.outcome != "succeeded" {
                return Err(format!("{name} qualifies without success"));
            }
            if scenario.outcome == "succeeded" && scenario.failure_reason.is_some()
                || scenario.outcome != "succeeded" && scenario.failure_reason.is_none()
            {
                return Err(format!("{name} outcome contradicts its failure reason"));
            }
        }
        let expected = self.process.qualifying && self.model.qualifying;
        if self.qualifying != expected || self.fixture_mode && self.qualifying {
            return Err("top-level qualification contradicts scenario/fixture status".to_owned());
        }
        if self
            .configuration_digest
            .as_deref()
            .is_some_and(|digest| !is_blake3_digest(digest))
            || self.qualifying && self.configuration_digest.is_none()
        {
            return Err(
                "configuration digest is malformed or absent for qualifying evidence".to_owned(),
            );
        }
        if self.generated_at_unix_ms == 0
            || !is_git_object(&self.milkdrift.starting_commit)
            || !is_git_object(&self.milkdrift.starting_tree)
            || self.validation.len() > MAX_REPORT_ITEMS
            || self.redactions.len() > 64
        {
            return Err("report provenance or top-level bounds are invalid".to_owned());
        }
        if self.qualifying {
            if self.milkdrift.dirty_at_start
                || self.failure_reason.is_some()
                || self.validation.is_empty()
                || self.validation.iter().any(|item| {
                    item.exit_status != 0 || item.command.is_empty() || item.command.len() > 1_024
                })
            {
                return Err(
                    "qualifying report has dirty, failed, or absent validation facts".to_owned(),
                );
            }
            self.process.validate_qualifying("process")?;
            self.model.validate_qualifying("model")?;
        }
        Ok(())
    }
}

fn is_git_object(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn is_blake3_digest(value: &str) -> bool {
    value.len() == 67
        && value.starts_with("b3_")
        && value[3..].bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn is_blake3_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn bounded_string<'a>(object: &'a Value, key: &str) -> Option<&'a str> {
    object
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && value.len() <= 1_024)
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlatformEvidence {
    pub os: String,
    pub architecture: String,
    pub build_target: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MilkdriftEvidence {
    pub starting_commit: String,
    pub starting_tree: String,
    pub workspace_version: String,
    pub dirty_at_start: bool,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioEvidence {
    pub qualifying: bool,
    pub outcome: String,
    pub profile: Value,
    pub commands: Vec<String>,
    pub runs: Vec<String>,
    pub revisions: Vec<String>,
    pub attempts: Vec<String>,
    pub proposals: Vec<String>,
    pub artifacts: Vec<ArtifactEvidence>,
    pub restart_boundaries: Vec<RestartEvidence>,
    pub facts: Value,
    pub failure_reason: Option<String>,
}

impl ScenarioEvidence {
    fn validate_qualifying(&self, name: &str) -> Result<(), String> {
        let vectors = [
            self.commands.len(),
            self.runs.len(),
            self.revisions.len(),
            self.attempts.len(),
            self.proposals.len(),
            self.artifacts.len(),
            self.restart_boundaries.len(),
        ];
        if vectors.into_iter().any(|count| count > MAX_REPORT_ITEMS)
            || self.commands.is_empty()
            || self.runs.is_empty()
            || self.revisions.is_empty()
            || self.attempts.is_empty()
            || self.artifacts.is_empty()
            || self.restart_boundaries.is_empty()
            || (name == "process" && self.proposals.is_empty())
        {
            return Err(format!(
                "qualifying {name} evidence omits a required bounded collection"
            ));
        }
        if self
            .commands
            .iter()
            .chain(&self.runs)
            .chain(&self.revisions)
            .chain(&self.attempts)
            .chain(&self.proposals)
            .any(|value| value.is_empty() || value.len() > 1_024)
            || self.artifacts.iter().any(|artifact| {
                artifact.artifact_id.is_empty()
                    || artifact.artifact_id.len() > 1_024
                    || !is_blake3_hex(&artifact.digest)
                    || artifact.content_type.is_empty()
                    || artifact.content_type.len() > 1_024
                    || artifact.role.is_empty()
                    || artifact.role.len() > 1_024
            })
            || self.restart_boundaries.iter().any(|restart| {
                restart.boundary.is_empty()
                    || restart.recovered_state.is_empty()
                    || restart.duplicate_attempts
                    || restart.sequence_before != restart.sequence_after
            })
        {
            return Err(format!(
                "qualifying {name} evidence contains invalid bounded facts"
            ));
        }
        let required_profile: &[&str] = if name == "process" {
            &[
                "executable_content_digest",
                "executable_size_bytes",
                "version_output",
                "profile_digest",
            ]
        } else {
            &[
                "profile_digest",
                "profile_revision",
                "provider_protocol",
                "model_alias",
            ]
        };
        let required_facts: &[&str] = if name == "process" {
            &[
                "repository_initial_commit",
                "repository_final_commit",
                "distinct_process_invocations",
                "process_invocations",
                "terminal_sequence",
            ]
        } else {
            &[
                "context_manifest_digest",
                "streaming_observations",
                "usage",
                "terminal",
                "uncertain",
                "invocation_id",
            ]
        };
        if required_profile
            .iter()
            .any(|key| self.profile.get(*key).is_none())
            || required_facts
                .iter()
                .any(|key| self.facts.get(*key).is_none())
        {
            return Err(format!(
                "qualifying {name} evidence omits required semantic keys"
            ));
        }
        match name {
            "process" => self.validate_process_semantics()?,
            "model" => self.validate_model_semantics()?,
            _ => return Err("unknown qualifying scenario family".to_owned()),
        }
        Ok(())
    }

    fn validate_process_semantics(&self) -> Result<(), String> {
        let profile = &self.profile;
        let facts = &self.facts;
        if bounded_string(profile, "executable_content_digest")
            .is_none_or(|value| !is_blake3_digest(value))
            || profile
                .get("executable_size_bytes")
                .and_then(Value::as_u64)
                .is_none_or(|value| value == 0)
            || bounded_string(profile, "version_output").is_none()
            || bounded_string(profile, "profile_digest")
                .is_none_or(|value| !is_blake3_digest(value))
        {
            return Err("qualifying process profile facts are malformed".to_owned());
        }
        let initial =
            bounded_string(facts, "repository_initial_commit").filter(|value| is_git_object(value));
        let final_commit =
            bounded_string(facts, "repository_final_commit").filter(|value| is_git_object(value));
        let count = facts
            .get("distinct_process_invocations")
            .and_then(Value::as_u64);
        let invocations = facts.get("process_invocations").and_then(Value::as_array);
        if initial.is_none()
            || final_commit.is_none()
            || initial == final_commit
            || count.is_none_or(|value| value < 5)
            || invocations.is_none_or(|items| u64::try_from(items.len()).ok() != count)
            || facts
                .get("terminal_sequence")
                .and_then(Value::as_u64)
                .is_none_or(|value| value == 0)
        {
            return Err("qualifying process remediation facts are malformed".to_owned());
        }
        let Some(invocations) = invocations else {
            return Err("qualifying process invocation facts are absent".to_owned());
        };
        let mut invocation_ids = BTreeSet::new();
        let mut roles = BTreeSet::new();
        for invocation in invocations {
            let id = bounded_string(invocation, "invocation_id")
                .ok_or_else(|| "qualifying process invocation has no identity".to_owned())?;
            let role = bounded_string(invocation, "role")
                .ok_or_else(|| "qualifying process invocation has no role".to_owned())?;
            if !invocation_ids.insert(id)
                || !roles.insert(role)
                || invocation.get("terminal").and_then(Value::as_str) != Some("succeeded")
                || invocation.get("uncertain").and_then(Value::as_bool) != Some(false)
            {
                return Err(
                    "qualifying process invocations are duplicate or nonterminal".to_owned(),
                );
            }
        }
        if !roles.contains("initial_coding")
            || !roles.contains("independent_review")
            || !roles.contains("remediation_coding")
            || !roles.contains("final_verification")
        {
            return Err("qualifying process evidence omits remediation roles".to_owned());
        }
        Ok(())
    }

    fn validate_model_semantics(&self) -> Result<(), String> {
        let profile = &self.profile;
        let facts = &self.facts;
        let usage = facts.get("usage").and_then(Value::as_object);
        if bounded_string(profile, "profile_digest").is_none_or(|value| !is_blake3_digest(value))
            || profile
                .get("profile_revision")
                .and_then(Value::as_u64)
                .is_none_or(|value| value == 0)
            || bounded_string(profile, "provider_protocol").is_none()
            || bounded_string(profile, "model_alias").is_none()
            || bounded_string(facts, "context_manifest_digest")
                .is_none_or(|value| !is_blake3_digest(value))
            || facts
                .get("streaming_observations")
                .and_then(Value::as_u64)
                .is_none_or(|value| value == 0)
            || facts.get("terminal").and_then(Value::as_str) != Some("succeeded")
            || facts.get("uncertain").and_then(Value::as_bool) != Some(false)
            || bounded_string(facts, "invocation_id").is_none()
            || usage.is_none_or(|value| {
                value.get("input_units").and_then(Value::as_u64).is_none()
                    || value.get("output_units").and_then(Value::as_u64).is_none()
            })
        {
            return Err("qualifying model profile or terminal facts are malformed".to_owned());
        }
        Ok(())
    }

    pub fn pending(reason: impl Into<String>) -> Self {
        Self {
            qualifying: false,
            outcome: "not_run".to_owned(),
            profile: Value::Object(Default::default()),
            commands: Vec::new(),
            runs: Vec::new(),
            revisions: Vec::new(),
            attempts: Vec::new(),
            proposals: Vec::new(),
            artifacts: Vec::new(),
            restart_boundaries: Vec::new(),
            facts: Value::Object(Default::default()),
            failure_reason: Some(reason.into()),
        }
    }

    pub fn failed(reason: impl Into<String>) -> Self {
        Self {
            qualifying: false,
            outcome: "failed".to_owned(),
            profile: Value::Object(Default::default()),
            commands: Vec::new(),
            runs: Vec::new(),
            revisions: Vec::new(),
            attempts: Vec::new(),
            proposals: Vec::new(),
            artifacts: Vec::new(),
            restart_boundaries: Vec::new(),
            facts: Value::Object(Default::default()),
            failure_reason: Some(reason.into()),
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactEvidence {
    pub artifact_id: String,
    pub digest: String,
    pub size: u64,
    pub content_type: String,
    pub role: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RestartEvidence {
    pub boundary: String,
    pub sequence_before: u64,
    pub sequence_after: u64,
    pub recovered_state: String,
    pub duplicate_attempts: bool,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ValidationEvidence {
    pub command: String,
    pub exit_status: i32,
}

pub fn write_report(
    path: &Path,
    report: &EvidenceReport,
    forbidden_values: &[Vec<u8>],
) -> Result<(), String> {
    report.validate()?;
    let bytes = serde_json::to_vec_pretty(report).map_err(|error| error.to_string())?;
    if bytes.len() > MAX_REPORT_BYTES {
        return Err("evidence report exceeds the encoded byte bound".to_owned());
    }
    let decoded: EvidenceReport = serde_json::from_slice(&bytes)
        .map_err(|error| format!("serialized evidence report failed strict validation: {error}"))?;
    decoded.validate()?;
    if forbidden_values
        .iter()
        .any(|value| !value.is_empty() && bytes.windows(value.len()).any(|window| window == value))
    {
        return Err("redaction validation found a secret value in the report".to_owned());
    }
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, bytes).map_err(|error| error.to_string())?;
    fs::rename(&temporary, path).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(process: ScenarioEvidence, model: ScenarioEvidence) -> EvidenceReport {
        EvidenceReport {
            schema_version: REPORT_SCHEMA_VERSION,
            generated_at_unix_ms: 1,
            platform: PlatformEvidence {
                os: "test".to_owned(),
                architecture: "test".to_owned(),
                build_target: "test".to_owned(),
            },
            milkdrift: MilkdriftEvidence {
                starting_commit: "a".repeat(40),
                starting_tree: "b".repeat(40),
                workspace_version: "test".to_owned(),
                dirty_at_start: false,
            },
            configuration_digest: Some(format!("b3_{}", "c".repeat(64))),
            qualifying: process.qualifying && model.qualifying,
            fixture_mode: false,
            process,
            model,
            validation: vec![ValidationEvidence {
                command: "cargo test".to_owned(),
                exit_status: 0,
            }],
            redactions: Vec::new(),
            failure_reason: None,
        }
    }

    #[test]
    fn fabricated_empty_qualification_is_rejected() {
        let mut process = ScenarioEvidence::pending("pending");
        process.qualifying = true;
        process.outcome = "succeeded".to_owned();
        process.failure_reason = None;
        let mut model = ScenarioEvidence::pending("pending");
        model.qualifying = true;
        model.outcome = "succeeded".to_owned();
        model.failure_reason = None;
        assert!(report(process, model).validate().is_err());
    }

    #[test]
    fn write_refuses_exact_forbidden_secret_bytes() -> Result<(), Box<dyn std::error::Error>> {
        let process = ScenarioEvidence::pending("secret-sentinel");
        let model = ScenarioEvidence::pending("pending");
        let mut report = report(process, model);
        report.qualifying = false;
        report.configuration_digest = None;
        report.failure_reason = Some("not run".to_owned());
        let root = tempfile::tempdir()?;
        let path = root.path().join("report.json");
        let Err(error) = write_report(&path, &report, &[b"secret-sentinel".to_vec()]) else {
            return Err("secret-bearing report was written".into());
        };
        assert!(error.contains("secret value"));
        assert!(!path.exists());
        Ok(())
    }

    #[test]
    fn artifact_digest_contract_uses_the_stored_lowercase_hex_form() {
        assert!(is_blake3_hex(&"a".repeat(64)));
        assert!(!is_blake3_hex(&format!("b3_{}", "a".repeat(64))));
        assert!(!is_blake3_hex(&"A".repeat(64)));
    }
}

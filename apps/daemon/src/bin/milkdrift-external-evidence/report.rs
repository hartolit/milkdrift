use std::{fs, path::Path};

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const REPORT_SCHEMA_VERSION: u32 = 1;

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
        Ok(())
    }
}

fn is_blake3_digest(value: &str) -> bool {
    value.len() == 67
        && value.starts_with("b3_")
        && value[3..].bytes().all(|byte| byte.is_ascii_hexdigit())
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

pub fn write_report(path: &Path, report: &EvidenceReport) -> Result<(), String> {
    report.validate()?;
    let bytes = serde_json::to_vec_pretty(report).map_err(|error| error.to_string())?;
    let decoded: EvidenceReport = serde_json::from_slice(&bytes)
        .map_err(|error| format!("serialized evidence report failed strict validation: {error}"))?;
    decoded.validate()?;
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, bytes).map_err(|error| error.to_string())?;
    fs::rename(&temporary, path).map_err(|error| error.to_string())
}

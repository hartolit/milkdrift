use std::{
    collections::BTreeSet,
    fs,
    io::{BufReader, Read as _},
    path::{Path, PathBuf},
    process::Command,
};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;

use milkdrift_local_process::{PlatformSupport, ProcessProfileDocument};
use serde_json::{Value, json};

pub struct AgentProfile {
    pub path: PathBuf,
    pub capability: String,
    pub canonical_executable: PathBuf,
    pub content_digest: String,
    pub size_bytes: u64,
    pub version_output: String,
    pub output_names: Vec<String>,
    pub secret_refs: BTreeSet<String>,
}

pub struct GeneratedProfiles {
    pub weak_verifier: PathBuf,
    pub good_verifier: PathBuf,
    pub reviewer: PathBuf,
    pub evidence_source: PathBuf,
}

pub fn prepare_agent_profile(
    source: Option<&Path>,
    fixture: bool,
    repository: &Path,
    session_root: &Path,
    version_arguments: &[String],
) -> Result<AgentProfile, String> {
    let source_value = if fixture {
        fixture_agent_value(repository, session_root)?
    } else {
        let source = source.ok_or_else(|| "--agent-profile is required".to_owned())?;
        let bytes = fs::read(source).map_err(|error| format!("agent profile read: {error}"))?;
        ProcessProfileDocument::from_json(&bytes)
            .map_err(|error| format!("agent profile contract: {error}"))?;
        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|error| format!("agent profile JSON: {error}"))?;
        reject_fixture_profile(&value)?;
        value
    };
    let mut value = source_value;
    let profile = value
        .get_mut("profile")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| "agent profile has no profile object".to_owned())?;
    profile.insert(
        "working_directory".to_owned(),
        json!({"type":"authorized_host_path","path":repository}),
    );
    let roots = profile
        .get_mut("filesystem_roots")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| "agent profile filesystem_roots is absent".to_owned())?;
    roots.push(json!({"path":session_root,"access":"read_write"}));
    roots.push(json!({"path":repository,"access":"read_write"}));

    let encoded = serde_json::to_vec(&value).map_err(|error| error.to_string())?;
    let document =
        ProcessProfileDocument::from_json(&encoded).map_err(|error| error.to_string())?;
    let canonical = document
        .to_canonical_json()
        .map_err(|error| error.to_string())?;
    let rendered_path = session_root.join("agent-profile.rendered.json");
    fs::write(&rendered_path, canonical).map_err(|error| error.to_string())?;

    let profile = value
        .get("profile")
        .and_then(Value::as_object)
        .ok_or_else(|| "agent profile is absent".to_owned())?;
    let capability = string(profile, "capability")?;
    let executable = PathBuf::from(string(profile, "executable")?);
    let canonical_executable = executable
        .canonicalize()
        .map_err(|error| format!("agent executable canonicalization: {error}"))?;
    let executable_size = fs::metadata(&canonical_executable)
        .map_err(|error| format!("agent executable metadata: {error}"))?
        .len();
    let mut executable = BufReader::new(
        fs::File::open(&canonical_executable)
            .map_err(|error| format!("agent executable read: {error}"))?,
    );
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0_u8; 1_048_576];
    loop {
        let read = executable
            .read(&mut buffer)
            .map_err(|error| format!("agent executable read: {error}"))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let observed_digest = format!("b3_{}", hasher.finalize());
    let declared = profile
        .get("implementation")
        .and_then(Value::as_object)
        .ok_or_else(|| "agent implementation declaration is absent".to_owned())?;
    if declared.get("content_digest").and_then(Value::as_str) != Some(&observed_digest)
        || declared.get("size_bytes").and_then(Value::as_u64) != Some(executable_size)
    {
        return Err("agent executable bytes do not match the declared digest/size".to_owned());
    }
    let version_output = executable_version_output(&canonical_executable, version_arguments)?;
    let output_names = process_output_names(profile)?;
    if output_names.is_empty() {
        return Err(
            "agent profile must publish at least one stdout/stderr/file artifact".to_owned(),
        );
    }
    if !output_names.iter().any(|name| name == "diff") {
        return Err(
            "agent profile must publish the prompt-sequence required output named diff".to_owned(),
        );
    }
    let secret_refs = profile
        .get("environment")
        .and_then(|value| value.get("secrets"))
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|values| values.values())
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();
    Ok(AgentProfile {
        path: rendered_path,
        capability,
        canonical_executable,
        content_digest: observed_digest,
        size_bytes: executable_size,
        version_output,
        output_names,
        secret_refs,
    })
}

fn executable_version_output(
    canonical_executable: &Path,
    version_arguments: &[String],
) -> Result<String, String> {
    let mut command = Command::new(canonical_executable);
    command.env_clear().env("LANG", "C.UTF-8");
    if version_arguments.is_empty() {
        command.arg("--version");
    } else {
        command.args(version_arguments);
    }
    let output = command
        .output()
        .map_err(|error| format!("agent version command: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "agent version command exited {}",
            output.status.code().unwrap_or(-1)
        ));
    }
    let mut version_output = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if version_output.is_empty() {
        version_output = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    }
    let boundary = milkdrift_contracts::truncate_utf8(&version_output, 1_024).len();
    version_output.truncate(boundary);
    if version_output.is_empty() {
        return Err("agent version command produced no bounded output".to_owned());
    }
    Ok(version_output)
}

pub fn generated_profiles(
    repository: &Path,
    session_root: &Path,
) -> Result<GeneratedProfiles, String> {
    let python = find_executable(&["python3", "python"])?;
    let weak_verifier = write_profile(
        session_root,
        repository,
        &python,
        "evidence-verifier-weak",
        "evidence-verifier-weak",
        verifier_code(),
        vec!["weak".to_owned()],
        Vec::new(),
        vec![
            (
                "verification_result",
                "verification-result.json",
                "application/json",
                true,
            ),
            ("verification_logs", "verification.log", "text/plain", true),
        ],
        "read_only",
        true,
    )?;
    let good_verifier = write_profile(
        session_root,
        repository,
        &python,
        "evidence-verifier-good",
        "evidence-verifier-good",
        verifier_code(),
        vec!["good".to_owned()],
        Vec::new(),
        vec![
            (
                "verification_result",
                "verification-result.json",
                "application/json",
                true,
            ),
            ("verification_logs", "verification.log", "text/plain", true),
            (
                "verification_pass",
                "verification-pass.json",
                "application/json",
                true,
            ),
        ],
        "read_only",
        true,
    )?;
    let reviewer = write_profile(
        session_root,
        repository,
        &python,
        "evidence-reviewer",
        "evidence-reviewer",
        reviewer_code(),
        Vec::new(),
        vec![("milkdrift.context_manifest", "context/manifest.json")],
        vec![
            ("review", "review.json", "application/json", true),
            (
                "remediation_proposal",
                "remediation-proposal.json",
                "application/json",
                true,
            ),
        ],
        "read_only",
        true,
    )?;
    let evidence_source = write_profile(
        session_root,
        repository,
        &python,
        "evidence-source",
        "evidence-source",
        evidence_source_code(),
        Vec::new(),
        vec![("payload", "inputs/payload.json")],
        vec![("evidence", "evidence.txt", "text/plain", true)],
        "read_only",
        true,
    )?;
    Ok(GeneratedProfiles {
        weak_verifier,
        good_verifier,
        reviewer,
        evidence_source,
    })
}

fn fixture_agent_value(repository: &Path, session_root: &Path) -> Result<Value, String> {
    let python = Path::new("/usr/bin/python3");
    let bytes = fs::read(python).map_err(|error| error.to_string())?;
    Ok(base_profile(
        repository,
        session_root,
        python,
        &bytes,
        "fixture-coding-agent",
        "fixture-coding-agent",
        vec!["-c".to_owned(), fixture_agent_code().to_owned()],
        json!({}),
        vec![("prompt", "inputs/prompt.json")],
        json!({"type":"input","input":"prompt","max_bytes":65536}),
        Some("diff"),
        Some("agent_stderr"),
        Vec::<(&str, &str, &str, bool)>::new(),
        "non_idempotent_write",
        true,
    ))
}

#[allow(clippy::too_many_arguments)] // Profile fixture generation exposes each wire field for scenario-specific variation.
fn write_profile(
    session_root: &Path,
    repository: &Path,
    executable: &Path,
    profile_id: &str,
    capability: &str,
    code: &str,
    extra_args: Vec<String>,
    inputs: Vec<(&str, &str)>,
    outputs: Vec<(&str, &str, &str, bool)>,
    side_effect: &str,
    fixture: bool,
) -> Result<PathBuf, String> {
    let bytes = fs::read(executable).map_err(|error| error.to_string())?;
    let mut arguments = vec!["-c".to_owned(), code.to_owned()];
    arguments.extend(extra_args);
    arguments.push("{{execution_root}}".to_owned());
    let value = base_profile(
        repository,
        session_root,
        executable,
        &bytes,
        profile_id,
        capability,
        arguments,
        json!({"execution_root":{"type":"execution_root"}}),
        inputs,
        json!({"type":"disabled"}),
        None,
        None,
        outputs,
        side_effect,
        fixture,
    );
    let document = ProcessProfileDocument::from_json(
        &serde_json::to_vec(&value).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let path = session_root.join(format!("{profile_id}.json"));
    fs::write(
        &path,
        document
            .to_canonical_json()
            .map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    Ok(path)
}

#[allow(clippy::too_many_arguments)] // This is the single schema-shaped fixture constructor for independently bounded fields.
fn base_profile<I, O>(
    repository: &Path,
    session_root: &Path,
    executable: &Path,
    bytes: &[u8],
    profile_id: &str,
    capability: &str,
    arguments: Vec<String>,
    substitutions: Value,
    inputs: I,
    stdin: Value,
    stdout: Option<&str>,
    stderr: Option<&str>,
    outputs: O,
    side_effect: &str,
    fixture: bool,
) -> Value
where
    I: IntoIterator,
    I::Item: AsInputTuple,
    O: IntoIterator,
    O::Item: AsOutputTuple,
{
    let inputs = inputs
        .into_iter()
        .map(|item| {
            let (input, relative_path) = item.as_input_tuple();
            json!({"input":input,"relative_path":relative_path})
        })
        .collect::<Vec<_>>();
    let outputs = outputs
        .into_iter()
        .map(|item| {
            let (name, relative_path, media_type, required) = item.as_output_tuple();
            json!({"name":name,"relative_path":relative_path,"media_type":media_type,"required":required})
        })
        .collect::<Vec<_>>();
    json!({
        "schema_version": 2,
        "profile": {
            "profile_id": profile_id,
            "revision": 1,
            "capability": capability,
            "descriptor_revision": 1,
            "provider_profile": null,
            "operation": "process.execute",
            "side_effect": side_effect,
            "idempotency": "unsupported",
            "cancellation": "best_effort",
            "trust_class": "trusted_host_process",
            "executable": executable,
            "implementation": {
                "content_digest": format!("b3_{}", blake3::hash(bytes)),
                "size_bytes": bytes.len(),
                "package_revision": if fixture {"milkdrift-evidence-fixture-v1"} else {"milkdrift-evidence-helper-v1"},
                "documentation_reference": "urn:milkdrift:external-evidence-helper:v1"
            },
            "arguments": arguments,
            "substitutions": substitutions,
            "working_directory": {"type":"authorized_host_path","path":repository},
            "filesystem_roots": [
                {"path":executable.parent().unwrap_or(Path::new("/usr/bin")),"access":"execute"},
                {"path":session_root,"access":"read_write"},
                {"path":repository,"access":"read_write"}
            ],
            "inputs": inputs,
            "environment": {"allowed_non_secret":[],"secrets":{},"max_value_bytes":4096},
            "stdin": stdin,
            "stdout": {"max_capture_bytes":1048576,"stream_progress":false,"max_progress_events":0,"overflow_action":"continue_truncated","artifact_name":stdout},
            "stderr": {"max_capture_bytes":1048576,"stream_progress":false,"max_progress_events":0,"overflow_action":"continue_truncated","artifact_name":stderr},
            "outputs": outputs,
            "limits": {
                "max_argv_entries":32,"max_argv_bytes":65536,"max_children_observed":32,
                "max_files":64,"max_file_bytes":16777216,"max_total_materialized_bytes":33554432,
                "max_path_bytes":4096,"max_directory_depth":32,"artifact_chunk_bytes":65536,
                "max_output_files":16,"max_total_output_bytes":33554432,"wall_timeout_ms":300000,
                "graceful_termination_ms":2000,"forced_termination_ms":2000,"heartbeat_interval_ms":1000
            },
            "restart":"retain_uncertain",
            "platform":PlatformSupport::current(),
            "max_concurrent":1,
            "extensions":{"org.milkdrift/evidence-fixture":{"deterministic":fixture}}
        }
    })
}

trait AsInputTuple {
    fn as_input_tuple(&self) -> (&str, &str);
}

impl<A: AsRef<str>, B: AsRef<str>> AsInputTuple for (A, B) {
    fn as_input_tuple(&self) -> (&str, &str) {
        (self.0.as_ref(), self.1.as_ref())
    }
}

trait AsOutputTuple {
    fn as_output_tuple(&self) -> (&str, &str, &str, bool);
}

impl<A: AsRef<str>, B: AsRef<str>, C: AsRef<str>> AsOutputTuple for (A, B, C, bool) {
    fn as_output_tuple(&self) -> (&str, &str, &str, bool) {
        (self.0.as_ref(), self.1.as_ref(), self.2.as_ref(), self.3)
    }
}

fn process_output_names(profile: &serde_json::Map<String, Value>) -> Result<Vec<String>, String> {
    let mut names = BTreeSet::new();
    for stream in ["stdout", "stderr"] {
        if let Some(name) = profile
            .get(stream)
            .and_then(|value| value.get("artifact_name"))
            .and_then(Value::as_str)
        {
            names.insert(name.to_owned());
        }
    }
    for name in profile
        .get("outputs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|output| output.get("name").and_then(Value::as_str))
    {
        names.insert(name.to_owned());
    }
    Ok(names.into_iter().collect())
}

fn reject_fixture_profile(value: &Value) -> Result<(), String> {
    let profile = value
        .get("profile")
        .and_then(Value::as_object)
        .ok_or_else(|| "agent profile has no profile object".to_owned())?;
    let executable = profile
        .get("executable")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let basename = Path::new(executable)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let rejected = [
        "bash",
        "cp",
        "dash",
        "echo",
        "env",
        "false",
        "milkdrift-process-test-helper",
        "node",
        "perl",
        "python",
        "python3",
        "python3.14",
        "ruby",
        "sh",
        "tee",
        "true",
        "zsh",
    ];
    let deterministic_extension = profile
        .get("extensions")
        .and_then(Value::as_object)
        .is_some_and(|extensions| {
            extensions.iter().any(|(key, value)| {
                (key.contains("fixture") || key.contains("test"))
                    && value
                        .get("deterministic")
                        .and_then(Value::as_bool)
                        .unwrap_or(true)
            })
        });
    if rejected.contains(&basename.as_str())
        || basename.starts_with("python")
        || basename.contains("fixture")
        || basename.contains("mock")
        || basename.contains("test-helper")
        || deterministic_extension
    {
        return Err(
            "deterministic/helper executable profile cannot qualify as the real coding agent"
                .to_owned(),
        );
    }
    Ok(())
}

fn string(profile: &serde_json::Map<String, Value>, key: &str) -> Result<String, String> {
    profile
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("agent profile {key} is absent"))
}

fn fixture_agent_code() -> &'static str {
    r#"from pathlib import Path
import sys
p=Path('calculator.py')
s=p.read_text()
if 'return a - b' in s:
    p.write_text(s.replace('return a - b', 'return a + b'))
print('fixture coding process inspected and repaired calculator.py')
sys.stdin.read()
"#
}

fn verifier_code() -> &'static str {
    r#"import json, pathlib, subprocess, sys
mode=sys.argv[1]
root=pathlib.Path(sys.argv[2])
diff=subprocess.run(['git','diff','--binary','HEAD'],check=False,capture_output=True,text=True)
tests=subprocess.run([sys.executable,'-m','unittest','-v'],check=False,capture_output=True,text=True)
log='ORCHESTRATION_FAULT_INJECTION='+str(mode=='weak')+'\n'+tests.stdout+tests.stderr
(root/'verification.log').write_text(log)
(root/'verification-result.json').write_text(json.dumps({'schema_version':1,'mode':mode,'test_exit':tests.returncode,'diff_digest_pending':True,'diff':diff.stdout}))
if mode=='good' and tests.returncode==0 and diff.returncode==0:
    (root/'verification-pass.json').write_text(json.dumps({'schema_version':1,'passed':True}))
sys.exit(0 if diff.returncode==0 and tests.returncode==0 else 1)
"#
}

fn reviewer_code() -> &'static str {
    r#"import json, pathlib, sys
root=pathlib.Path(sys.argv[1])
manifest=pathlib.Path(root/'context/manifest.json')
observed=manifest.exists()
(root/'review.json').write_text(json.dumps({'schema_version':1,'independent_process':True,'context_manifest_observed':observed,'finding':'controlled verifier gate requires remediation workflow'}))
(root/'remediation-proposal.json').write_text(json.dumps({'schema_version':1,'action':'rerun coding and independent verification'}))
"#
}

fn evidence_source_code() -> &'static str {
    r#"import json, pathlib, sys
root=pathlib.Path(sys.argv[1])
payload=json.loads((root/'inputs/payload.json').read_text())
(root/'evidence.txt').write_text(payload)
"#
}

pub fn secure_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    fs::write(path, bytes).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn find_executable(names: &[&str]) -> Result<PathBuf, String> {
    let paths = std::env::var_os("PATH").ok_or_else(|| "PATH is unavailable".to_owned())?;
    for directory in std::env::split_paths(&paths) {
        for name in names {
            let candidate = directory.join(name);
            if candidate.is_file() {
                return candidate
                    .canonicalize()
                    .map_err(|error| format!("executable canonicalization: {error}"));
            }
            #[cfg(windows)]
            {
                let candidate = directory.join(format!("{name}.exe"));
                if candidate.is_file() {
                    return candidate
                        .canonicalize()
                        .map_err(|error| format!("executable canonicalization: {error}"));
                }
            }
        }
    }
    Err(format!("none of {} was found on PATH", names.join(", ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn committed_external_evidence_templates_are_schema_valid()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let examples = root.join("examples/external-evidence");
        let process = fs::read(examples.join("coding-agent-profile.example.json"))?;
        ProcessProfileDocument::from_json(&process)?;
        for name in [
            "openai-compatible-profile.example.json",
            "anthropic-profile.example.json",
        ] {
            let bytes = fs::read(examples.join(name))?;
            milkdrift_model_provider::EndpointProfile::from_json(&bytes)?;
        }
        Ok(())
    }

    #[test]
    fn qualification_rejects_interpreters_and_fixture_markers() {
        for executable in [
            "/usr/bin/python3",
            "/opt/evidence/mock-agent",
            "/opt/evidence/milkdrift-process-test-helper",
        ] {
            let value = json!({"profile":{"executable":executable,"extensions":{}}});
            assert!(reject_fixture_profile(&value).is_err());
        }
        let fixture = json!({
            "profile":{
                "executable":"/opt/real-agent",
                "extensions":{"org.example/test-fixture":{"deterministic":true}}
            }
        });
        assert!(reject_fixture_profile(&fixture).is_err());
        let real = json!({"profile":{"executable":"/opt/coding-agent","extensions":{}}});
        assert!(reject_fixture_profile(&real).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn provenance_version_command_does_not_inherit_operator_environment()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        let executable = root.path().join("version-helper");
        fs::write(
            &executable,
            b"#!/bin/sh\nprintf '%s' \"${HOME-operator-environment-cleared}\"\n",
        )?;
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700))?;
        assert_eq!(
            executable_version_output(&executable, &[])?,
            "operator-environment-cleared"
        );
        Ok(())
    }
}

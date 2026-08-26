# Local process operator guide

`milkdrift-local-process` profile schema v1 configures a generic executable, not a vendor. The
profile is registered with its generated immutable descriptor, while the runtime pins the exact
descriptor generation before execution.

The essential shape for a coding-agent CLI is:

```json
{
  "schema_version": 1,
  "profile": {
    "profile_id": "coding-agent-local",
    "revision": 1,
    "capability": "coding-agent-local",
    "descriptor_revision": 1,
    "provider_profile": "coding-agent-default",
    "operation": "process.execute",
    "side_effect": "non_idempotent_write",
    "idempotency": "unsupported",
    "cancellation": "best_effort",
    "executable": "/opt/coding-agent/bin/agent",
    "arguments": ["run", "--workspace", "{{workspace}}", "--prompt-file", "{{prompt}}"],
    "substitutions": {
      "workspace": { "type": "execution_root" },
      "prompt": { "type": "input_path", "input": "prompt" }
    },
    "working_directory": { "type": "isolated_root" },
    "filesystem_roots": [
      { "path": "/opt/coding-agent/bin", "access": "execute" },
      { "path": "/var/lib/milkdrift/process-work", "access": "read_write" }
    ],
    "inputs": [
      { "input": "prompt", "relative_path": "inputs/prompt.txt" },
      { "input": "repository", "relative_path": "repository.bundle" }
    ],
    "environment": {
      "allowed_non_secret": ["LANG"],
      "secrets": { "AGENT_TOKEN": "secret:coding-agent-token" },
      "max_value_bytes": 8192
    },
    "stdin": { "type": "disabled" },
    "stdout": {
      "max_capture_bytes": 1048576,
      "stream_progress": false,
      "max_progress_events": 0,
      "overflow_action": "terminate",
      "artifact_name": "stdout"
    },
    "stderr": {
      "max_capture_bytes": 1048576,
      "stream_progress": false,
      "max_progress_events": 0,
      "overflow_action": "terminate",
      "artifact_name": "stderr"
    },
    "outputs": [
      { "name": "patch", "relative_path": "outputs/change.patch", "media_type": "text/x-diff", "required": true }
    ],
    "limits": {
      "max_argv_entries": 32,
      "max_argv_bytes": 65536,
      "max_children_observed": 64,
      "max_files": 32,
      "max_file_bytes": 16777216,
      "max_total_materialized_bytes": 67108864,
      "max_path_bytes": 4096,
      "max_directory_depth": 32,
      "artifact_chunk_bytes": 1048576,
      "max_output_files": 8,
      "max_total_output_bytes": 33554432,
      "wall_timeout_ms": 900000,
      "graceful_termination_ms": 5000,
      "forced_termination_ms": 5000,
      "heartbeat_interval_ms": 5000
    },
    "restart": "retain_uncertain",
    "platform": {
      "owned_process_group": true,
      "descendant_escape_prevention": false,
      "terminal_group_observation": true
    },
    "max_concurrent": 2,
    "extensions": {}
  }
}
```

The repository example is an immutable selected input inside a fresh execution directory; the
agent mutates only that copy and exports an explicit patch. A CLI accepting prompt stdin can use
`{"type":"input","input":"prompt","max_bytes":...}` instead of `--prompt-file`.
Each invocation starts a fresh process; a recorded PID is never treated as restart identity. The
shown platform facts are the Unix profile values and must equal the support facts of the build
that loads the document.

Secret-bearing profiles must keep process-text progress streaming disabled. Captures remain
bounded artifacts and exact secret bytes are redacted before capture publication. The descriptor
advertises best-effort cancellation because a request acknowledgement is not terminal proof.
Configure `retry_with_stable_key` only when the executable really accepts a profile-scoped stable
key through an `idempotency_key` substitution; otherwise restart retains a lost process as
uncertain.

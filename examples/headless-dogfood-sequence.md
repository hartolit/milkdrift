```milkdrift-sequence
{
  "schema_version": 2,
  "sequence": {
    "id": "milkdrift-core-convergence",
    "title": "Milkdrift core convergence",
    "workflow_id": "milkdrift-core-convergence",
    "repository": {
      "id": "repository:milkdrift",
      "root_ref": "workspace:milkdrift-main",
      "starting_revision": "operator-pinned-revision",
      "allowed_paths": ["Cargo.toml", "Cargo.lock", "crates", "adapters", "apps", "docs", "README.md", "ARCHITECTURE.md"],
      "allowed_operations": ["read", "write", "execute", "version_control"],
      "dirty_tree": "allow_recorded",
      "isolation": "shared_sequential",
      "cleanup": "retain_accepted",
      "artifacts": {
        "require_starting_state": true,
        "require_diff": true,
        "require_verification_evidence": true
      },
      "credential_refs": [],
      "remote_access_refs": []
    },
    "stages": [
      {
        "id": "implementation",
        "title": "Implement the bounded change",
        "session": "fresh",
        "coding": {
          "capability": "configured-coding-agent",
          "operation": "process.execute",
          "provider_profile": null,
          "execution_trust": "trusted_host_process",
          "maximum_side_effect": "unknown"
        },
        "verification": {
          "profile": {
            "capability": "configured-verifier",
            "operation": "process.execute",
            "provider_profile": null,
            "execution_trust": "trusted_host_process",
            "maximum_side_effect": "read_only"
          },
          "checks": ["rust.complete_quality_gate"],
          "success_artifact": "verification_pass",
          "result_artifact": "verification_result",
          "log_artifact": "verification_logs"
        },
        "failure": "pause_for_review",
        "reviewer": {
          "capability": "configured-reviewer",
          "operation": "process.execute",
          "provider_profile": null,
          "execution_trust": "trusted_host_process",
          "maximum_side_effect": "read_only"
        },
        "approval": "shared_control_path",
        "context_policy_ref": "context:implementation-v1",
        "outputs": [
          {"name": "diff", "media_type": "text/x-diff", "required": true},
          {"name": "result", "media_type": "application/json", "required": true},
          {"name": "logs", "media_type": "text/plain", "required": false}
        ]
      }
    ],
    "budget": {
      "max_review_loops": 3
    },
    "extensions": {}
  }
}
```

## Prompt: implementation
Implement one bounded Milkdrift change, preserve accepted repository state, and report exact artifacts.

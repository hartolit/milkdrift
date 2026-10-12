# Operate tasks without a browser

The CLI and Svelte client use the same daemon task path. A shell client can submit typed task work,
exit, and later discover the result using only the daemon endpoint and current credential. The
[operator recipe](../../examples/operator/README.md) owns installation and a complete request;
the [task API](../reference/control-api.md) owns its fields.

After configuring the CLI's endpoint and private token source, use these operations:

```sh
milkdrift daemon readiness
milkdrift daemon health
milkdrift daemon authority
milkdrift workflow capabilities
milkdrift workflow upload Cargo.toml --request manifest-upload-1
milkdrift workflow create task-request.json
milkdrift workflow receipt task-request-1
milkdrift workflow list --search report --limit 50
milkdrift workflow show WORKFLOW_ID
milkdrift workflow task WORKFLOW_ID TASK_ID
milkdrift workflow result WORKFLOW_ID TASK_ID
milkdrift artifact metadata ARTIFACT_ID
milkdrift artifact get ARTIFACT_ID --output retained-result.txt
```

The create document contains `request_id`, task aliases, instructions and named input bindings.
Substitute the uploaded identity into that document before submitting it. Preserve the exact document
while recovering a lost acceptance reply. List/search does not require knowing an accepted ID in
advance. Explicit cursors continue bounded workflow and task-summary pages.

A queued task can accept a missing input or prospective definition edit. A started task keeps its
frozen definition. A stop request does not itself establish settlement:

```sh
milkdrift workflow edit WORKFLOW_ID changes.json
milkdrift workflow stop WORKFLOW_ID TASK_ID
milkdrift workflow task WORKFLOW_ID TASK_ID
```

Use `--json` and a bounded `--timeout-secs` in automation. See the [CLI guide](../../apps/cli/README.md)
for independent invocation preparation, exact uploads and safe output publication. The connected-task
Rust fixture and real browser suite are described in the [web client](../../apps/web/README.md).
The former prompt-sequence and graph-run operating procedure was retired under
[ADR 0051](../decisions/0051-daemon-task-workflows.md).

# Operator CLI

`milkdrift` submits ordinary tasks to a running daemon, discovers permitted work and reads retained
results. The daemon keeps working when the CLI exits. The CLI uses the
[control client](../../crates/control-client/README.md) and never opens the daemon database.
Start with the [operator recipe](../../examples/operator/README.md) for a fresh private installation.

## Submit and return to work

Use `project directories` to browse permitted folders on the daemon, then `project attach NAME
FOLDER` or `project create NAME PARENT FOLDER_NAME` to register a project. Creation makes exactly one
new child; attachment preserves existing files. Initial access is read only. `project list`,
`project show ID` and `project environments` expose the current registration, guarded settings and
configured choices. `project settings ID --expected-generation N --environment host --allow-changes`
deliberately enables subsequent native file writes. For a configured process or Podman environment,
also pass its returned `--capability`, `--operation` and `--environment-generation`; incompatible
choices refuse without replacing the project or running another environment.

New workflow JSON includes `project_id`. Each new task chooses `execution.kind: "project"` with the
reviewed `expected_generation`, or a complete explicit override including deliberate empty tools
and access. The daemon freezes the resolved selection. Editing only task text retains that binding;
changing project defaults does not grant new tools to accepted tasks. Use `workflow list --project
ID` for grouping and `--earlier` for unassociated retained workflows. `project associate ID WORKFLOW
--expected-generation N` organizes an earlier workflow for future submissions without changing its
original receipts, definitions or results.

Project mutations use the global `--command-id` as their exact request identity. Preserve that ID
and every argument across a lost reply; `project receipt ID` recovers the accepted result.
`project operations` shows unresolved folder creation. After inspecting the reported directory,
`project reconcile OPERATION` explicitly attaches it or `project abandon OPERATION` abandons the
registration attempt. Neither deletes files. `project rename` changes only the label, and `project
remove` refuses registrations referenced by retained workflows or unresolved operations.

`workflow capabilities` reports the daemon identity, current actor, effective limits and configured
task choices. `workflow upload FILE --request ID` captures at most 16 KiB of UTF-8 file bytes.
The daemon also checks the combined instructions and selected inputs before task admission. Put the returned input
identity into a typed create request, then use `workflow create FILE`. The daemon allocates workflow
and task identities and returns their durable receipt. The [control API](../../docs/reference/control-api.md)
owns the JSON request shapes; the [Svelte client](../web/README.md) provides the same ordinary path.

| Need | Command |
| --- | --- |
| Discover work without a saved ID | `workflow list --search TEXT --limit 50` |
| Inspect a workflow and its task page | `workflow show WORKFLOW`; use `--cursor CURSOR` for another page |
| Read selected instructions and frozen inputs | `workflow task WORKFLOW TASK` |
| Read retained output | `workflow result WORKFLOW TASK` |
| Recover a lost acceptance reply | `workflow receipt REQUEST_ID`; replay the exact create/edit document if necessary |
| Change future work | `workflow edit WORKFLOW FILE` with an exact request ID and observed generation |
| Review a complete future batch without submitting | `workflow edit WORKFLOW FILE --preview` |
| Read or conditionally save unfinished canvas work | `workflow authoring WORKFLOW`; `workflow save-authoring WORKFLOW FILE` |
| Review saved work, then admit its exact scope | `workflow review-authoring WORKFLOW FILE`; `workflow run-authoring WORKFLOW FILE` |
| Read or conditionally save separate card positions | `workflow positions WORKFLOW`; `workflow save-positions WORKFLOW FILE` |
| Request cancellation | `workflow stop WORKFLOW TASK`; inspect the resulting state to establish whether it stopped |
| Inspect the installation | `daemon readiness`, `health`, `authority`, `capability`, `provider` |
| Read the shared daemon name and its generation | `daemon name` |
| Rename the daemon for every client | `daemon rename NAME --expected-generation N --request-id ID` |
| Recover an accepted rename | `daemon rename-receipt REQUEST_ID` |
| Use an independent execution host | `invocation prepare`, `start`, `lookup`, `show`, `observe`, `cancel` |
| Retrieve immutable artifacts | `artifact metadata`, `artifact get ARTIFACT --output NEW_FILE` |
| Operate configured infrastructure | `peer` and `resource` command families |

Use family `--help` for exact arguments. Started definitions are immutable. An edit can supply a
missing input, add work, revise an unstarted task, request a stop, or explicitly adopt a successful
correction. It cannot change what an earlier task executed. Queued work has no start timestamp;
`uncertain` means a possible external effect lacks settlement evidence.

The daemon owns its display name. Read `daemon name`, then use the returned generation in
`daemon rename`; the separate configuration permission is required even when the actor can edit
workflows. The CLI authenticates to read the daemon identity and binds the rename and receipt request
to that identity. Duplicate display names are allowed, so continue selecting the intended endpoint.
A rename affects other clients and persists when the daemon restarts. The operator's configured name
seeds absent metadata and does not overwrite a saved rename.

Keep the request identity, name and expected generation after an interrupted rename. Use
`daemon rename-receipt` or repeat those exact arguments. The CLI never replaces a stale generation
with the current one, and it never automatically retries a rename. A later rename can supersede the
name recorded in an earlier receipt; read `daemon name` for the current value.

Use `workflow edit WORKFLOW changes.json --preview` before submitting a batch. It displays the
exact request together with the current workflow generation and each guarded task's version and state.
The preview is a read: it does not validate authority for the proposed actions, reserve work or
change stale guards. If a human or model changed the workflow, inspect `workflow show` and
`workflow task`, review the affected definitions, then deliberately create a new request with
current guards. The daemon either accepts every change atomically or refuses the batch.

One batch can add parallel tasks with local aliases, set required/optional input policies, consume
a failed task through `settled_outcome`, revise future dependencies, supply missing inputs and
adopt a successful correction with explicit version-guarded consumer redirects. Required output
bindings still require producer success. Correction adoption preserves the failed original and
started consumers' frozen inputs. Use an explicit `finish` change for completion, `wait` for a
decision, or `continue` for prospective resume; an empty runnable queue alone means no completion.
Workflow inspection exposes the shared cumulative limits. Resume never resets prior spending.
Use `set_limits` to change complete finite ceilings prospectively. `dismiss_admission` closes a
response decision and can share a batch with explicitly reviewed replacement changes; it preserves
the model's immutable result and grants no additional capability. `workflow show` reports the
current wait question, cumulative task/model admissions, elapsed execution and source-response
decisions for that task page. Task definitions opt in to model actions with `actions` and select
their exact native/process tool catalog with `tools`; plain model text alone never schedules work.

## Automation and recovery

Use `--json` and an overall `--timeout-secs`. Task create/edit identities are inside their typed
JSON documents; the global command ID identifies the CLI record and other maintained command
families. A successful submission establishes acceptance, not task completion. Safe reads have
bounded transport retries, while mutations are sent once. Ctrl-C or a deadline ends the client's
request and observation; it does not cancel daemon work.

A timeout can follow durable acceptance. Preserve the exact body and identity for replay. Changed
reuse conflicts; a new identity expresses new work. After a complete client restart, authenticate
and use list/search to rediscover work. Ordinary tasks need no local recovery file.

Independent invocation preparation still uses an explicit private request file because its exact
host/caller-bound request is its replay contract. Input preparation checks every local file before
upload, then checks its bytes again before sending. Preserve `input.uploading` and `input.uploaded`
records if preparation fails partway. Repeat with the same host, actor, request identity, order and
original bytes. Already accepted uploads remain retained and consume capacity.

Artifact downloads verify full size and digest before publishing to a new destination. The CLI
uses private staging and refuses to overwrite a file, directory or symlink. Failure cleans staging
without deleting another writer's destination. Unix staging is `0700` and files are `0600`;
publication syncs the parent directory. A sync or cleanup failure remains explicit, including when
the file was published but durability is unconfirmed. Keep the output parent and its ancestors under
operator control. Forced termination can leave staging behind.

## Contribute

`main` owns arguments and the overall deadline; `session` owns the authenticated client.
`command/workflow` is a thin task protocol caller. Input preparation, staged output, error mapping
and observation framing have their own modules. Workflow semantics belong to the daemon.
The production parser checks maintained command examples, CLI child tests exercise framing and
local failures, and daemon integration tests exercise real durable operations. Follow the
[verification policy](../../docs/development/workflow.md#choose-verification-for-the-change).

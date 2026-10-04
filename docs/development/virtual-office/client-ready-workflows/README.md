# Make workflows usable from the CLI

## Goal

Create a workflow, choose a model, write prompts, connect steps, run it with an input, read the
result, and use it again. Ordinary use should not require writing graph JSON, calculating hashes,
or understanding the database.

Finish that path and fix the awkward or duplicated code it exposes. Do not rebuild the working
host, container, publication, or learning systems from the previous sprint.

The source reference is `a04b55c0e0a3b56a1074da8861ab1d581e16a05c`; 00 inspected the newer checkout
`d8873cfe32700522347ea4d01858e4a5ddfa9749`. Never reset to the reference. Phases 00–05 are
complete; [05's handoff](handoffs/05.md) records the tested client boundary. The prepared
[linting insertion](linting-insertion.md) is integrated below. [05a](handoffs/05a.md) has implemented
and validated enforcement, with existing policy violations keeping the strict gate red. 05b clears
those findings next, and only then does 06 accept the combined product.
These temporary prompts now live in the virtual office, as their relative
links and the repository's coordination rules require.

## Svelte first; the daemon owns the behavior

**Svelte is the first GUI.** A desktop wrapper may use Tauri. Iced is only a possible later client.
This sprint builds no GUI and adds no frontend dependencies.

Svelte, the CLI, and future interfaces use the same public daemon operations. The daemon and its
Rust libraries decide permissions, valid workflow changes, execution, recovery, and saved results.
Clients do not duplicate those rules, access the database, or invoke the CLI to use the product.

An interface may keep unsaved edits, arrange a canvas, display progress, and help fill in forms.
Its checks can improve feedback, but the daemon still validates submitted work. No essential
operation may require CLI-private helpers or make Svelte reproduce Rust-only rules or hash calculations.

## Work in order

| Prompt | Work |
| --- | --- |
| [00 — Set up the sprint](00-first-execution-prompt.md) | Confirm the starting point, update work rules, and record the Svelte direction. |
| [01 — Create and edit workflows](01-workflow-authoring.md) | Add steps, choose models, write prompts, connect steps, save, and reopen. |
| [02 — Run with inputs and reconnect](02-inputs-execution-and-recovery.md) | Use different inputs and recover a lost connection without duplicate work. |
| [03 — Read results and fix future steps](03-inspection-and-prospective-repair.md) | Explain failures, retrieve results, and safely change work that has not happened yet. |
| [04 — Save and reuse workflows](04-reusable-methods.md) | Reuse/copy workflows and simplify existing publication and evaluation commands. |
| [05 — Keep the interface thin](05-client-boundary-and-contraction.md) | Remove duplicated rules and test another client without the CLI. |
| [05a — Enable stricter checks](05a-enable-strict-checks.md) | Implement and test strict static enforcement; record existing violations for 05b. |
| [05b — Fix strict-check findings](05b-fix-strict-check-findings.md) | Correct the implementations and finish with a passing static gate. |
| [06 — Test the full system and fix problems](06-integrated-acceptance-and-closeout.md) | Test the combined result, fix failures, and finish the sprint. |

Each phase finishes its implementation and focused checks. Its handoff is not a claim that the
whole system passed. Run sequentially; do not start a GUI automatically after 06.

## Existing route and assigned changes

00 traced the CLI through `control-client`, `control-protocol`, daemon command handlers and the
blueprint/control/runtime owners. The table records the assignments; authoring, input/recovery,
result inspection, prospective repair, reuse and publication/evaluation conveniences are complete,
including the independent client boundary; final integrated acceptance remains unfinished.

| Operation | Existing owner | Needed change | Prompt |
| --- | --- | --- | --- |
| Create/edit/save | Blueprint genesis/revise and mutations; daemon definitions; CLI blueprint commands | Public construction and validation from ordinary edits, with server-derived identities and guarded save/reopen | 01 |
| Choose model/write prompt/connect | Authorized capability reads; model task and blueprint binding types | Usable authoring over those contracts, including available choices and complete-output gates | 01 |
| Supply inputs/start | Public `StartRun`; daemon runs; control/runtime `CreateRun` | Carry bounded named inputs through the existing workspace input path | 02 |
| Recover request/observe | Daemon receipts; control-client submit/reads/SSE; CLI session and wait | Retain exact start requests before submission and reconnect without new identities or changed inputs | 02 |
| Read final result | Daemon run/node/attempt/acceptance projections and artifact reads | Compact authoritative result and permitted-action views; verified output retrieval | 03 |
| Change future work | Control proposals/risk/approval; runtime reconciliation, pause and signal | Public edit/proposal conveniences and a nonterminal review hold; preserve failed/completed history | 03 |
| Reuse/copy/publish/evaluate | Immutable revisions; control publication/learning; daemon and CLI method commands | Public copy/request construction and clearer existing evaluation use without another store or engine | 04 |
| Use another frontend | Control protocol/client and daemon | Remove remaining indispensable CLI rules; exercise the same operations without CLI or private builders, then integrated acceptance | 05–06 |

02 completed input submission in [public StartRun](../../../../crates/control-protocol/src/command.rs)
and [daemon start](../../../../apps/daemon/src/host/commands/runs.rs). The
[runtime admission owner](../../../../crates/runtime/src/engine/command_planning/admission.rs)
accepts initial workspace values and checks declared names, required inputs, committed
artifacts and budgets. [Published calls](../../../../crates/control/src/published.rs) map
their separate choice/artifact contracts into those values. Preserve those shared owners and
distinct permissions in later phases.

For unfinished edits, use an ordinary bounded local file containing the workflow/base reference
and pending existing blueprint mutations. Saving that file may retain an incomplete graph; it
neither creates a revision nor starts a run. Keep credentials, per-run input values and execution
history out of it. Safe replacement must detect changed files and preserve richer imported features
or refuse before writing. A future Svelte editor may keep equivalent local state.

Submitting sends one complete edit batch against genesis or the exact saved base through a public
daemon operation. The daemon invokes blueprint genesis/revise, model request construction and
validation under the caller's authority, returns diagnostics or the validated canonical revision,
and saves accepted definitions through the existing revision store. Clients reuse returned
identities rather than calculating hashes. 01 completed this public route. The
[CLI-local constructor](../../../../apps/cli/src/command/blueprint.rs) remains for governed-method
bootstrap before daemon setup. Preserve the existing mutation meaning; add no draft database,
workflow language or duplicate executor. [01's handoff](handoffs/01.md) records its focused checks.

02 followed each brief/draft binding into the prepared provider request.
[Model preparation](../../../../adapters/model-provider/src/adapter.rs) now materializes selected
direct inputs through the existing context path. Preserve that route rather than concatenating
inputs in clients or introducing another context mechanism. Controlled endpoints assert brief/draft
bytes and absence of unrelated evidence.

## One example throughout

Use a two-step release-notes workflow: one model drafts notes from a change brief; another receives
the draft and original brief, then reviews and revises the notes. Use two different briefs through
the same saved version: a host release and a game's networking update. Keep example data outside
product code. Create a second independent workflow too.

Controlled model responses make failure tests repeatable. An empty or incomplete required result
must fail. Repair eligible future work through normal permissions; an ended run stays ended.
In 06, run both briefs through an authorized real local model as well. Report complete output
separately from editorial usefulness. Existing Slotbook and managed-host tests remain required;
this smaller example does not replace them.

The following fictional briefs are the shared input text, not claims about implemented products:

- **Host release:** "Harbor Host 1.4 now shows queued jobs and lets operators download completed
  job logs. This release fixes duplicate status rows after reconnecting. Running jobs keep their
  current settings during an upgrade. Automatic failover is not included. Write release notes
  for the operators who will upgrade."
- **Game networking update:** "Lantern Rally 0.8 adds private lobby codes and reconnecting to a
  match for up to 30 seconds after a dropped connection. It fixes duplicate lap notifications.
  Hosts and players must use the same version. Cross-region matchmaking is not included. Write
  release notes for players."

The draft step asks for concise notes using only the supplied `brief`, including changes, fixes
and limits. The review/revise step receives that original brief and only the first step's selected
draft, checks unsupported claims and missing caveats, and returns revised notes. Declare `brief`
as a run input and expose the revised text as the final output only after acceptance. Both briefs
use the same saved revision. Create an independently identified meeting-summary workflow for
coexistence/copy tests. Implement executable example data and controlled responses in the maintained
example/evidence owners during 01–03; keep their names, responses and chosen limits out of product
code. 00 adds no executable fixture or claimed successful model result.

Use the existing [result acceptance](../../../guides/result-acceptance.md) operation
`workflow.accept_result` with `ResultRequirement::ModelProse`: canonical model output must report
`stop` and non-whitespace final text. A `length` finish fails even with text. Its successful
invocation means evaluation completed; the branch on `accepted_result` decides whether work may
continue. This measures completeness, not editorial usefulness.

For deterministic repair, send a failed completeness branch to an ordinary `SignalWait` review
hold whose unchanged continuation ends in failure. While the run is still nonterminal, pause it,
submit/approve/apply a prospective repair from its exact current base and sequence, then deliver
the authorized signal and resume. The repair inserts new work and a fresh required check; it
does not erase the failed attempt or remove a requirement. Do not signal the unchanged failure
route first. This reuses the mechanism exercised by
[daemon remediation](../../../../apps/daemon/tests/control_plane/control_workflows.rs) and the
[sequence hold](../../../../crates/prompt-sequence/src/remediation.rs); model workflows use the
same underlying nodes/control operations, not the process-only sequence compiler. Preserve
governing agreements through the existing protected-adaptation checks. Once a run is terminal,
resume is refused; any further execution is an explicitly linked new run.

**06 model access remains to be supplied.** No current endpoint/profile, credential reference or
permission for these calls was supplied in 00. The maintained
[attached local-model route](../../../guides/local-model-endpoint.md) is suitable once the operator
identifies an authorized endpoint and exact model alias. Historical Ornith/Bonsai results do not
establish current access. 06 must record the profile and finite request, output and time budgets
before the two briefs' draft/review calls, with bounded explicitly selected retries. Do not call a
model, alter host permissions or reconfigure services to obtain access during setup.

## Shared working rules

Follow [AGENTS.md](../../../../AGENTS.md), [implementation practice](../../practices/implementation.md),
[documentation practice](../../practices/documentation.md), [work rules](../../workflow.md), and
[office procedure](../README.md). The adopted testing schedule below uses the
[explicit sprint exception](../../workflow.md#explicit-multi-phase-sprint-schedule).

Implement each change where it belongs, including callers, tests, and documentation. Fix an
underlying problem when discovered, not in 06. Remove the replaced implementation. Do not add a
wrapper around a broken rule, leave stubs, or make unrelated architectural changes. Preserve
working direct calls, permissions, input isolation, resource ownership, and recovery.

Follow the required reading order once per agent session, then read the current prompt, relevant
handoff decisions, and affected code. Do not reload every prompt and old log after every commit.
Keep one short `handoffs/NN.md`: changes, commit IDs, checks/log paths, blockers, and next starting
point. Put raw logs under ignored `target/client-ready-workflows/`, not in handoffs or chat.

No new inference engine, scheduler, container backend, cluster management, hardware campaign,
self-improvement engine, or speculative frontend framework. Use existing public transports.
An authorized attached model remains valid; do not require containerized inference.

## Test as you build; test the full system in 06

| Phase | Checks |
| --- | --- |
| 00 | Changed documents, links, examples where applicable, and whitespace. |
| 01–05 | Compile affected packages and callers. Run focused behavior, failure, and affected example tests using controlled services. |
| 05a–05b | Focused checker/regression tests and the full selected static matrix. 05a may hand off confirmed policy violations to 05b; broken checkers and unexplained compiler failures are not an accepted handoff. No combined runtime journey. |
| 06 | Complete operator journey, full workspace gate, required local-model checks, and fixes. |

Write and run a regression with its fix. A small daemon/CLI test for one changed operation belongs
in that phase; the complete journey does not. Choose checks by affected behavior and callers,
verify filters discover the intended tests, and reuse successful results until changes affect them.
**Do not repeat the full workspace gate at every commit or handoff.**

Keep CI and test coverage intact: no skip markers, disabled jobs, or weaker assertions. Local
commits need no push. Push only when separately authorized; existing CI may then run as configured.
The final full check is still required even when earlier focused tests passed.

Intended checks and their existing owners follow; these are assignments, not passed results:

- **01:** blueprint `kernel` and model `contracts` tests own definition/request validity; daemon
  `control_plane` and CLI tests own public construction, permission refusal, save/reopen, stale
  edits and zero model/run entry while authoring. Model-provider `mock_endpoints` and runtime
  `causal_context` tests own the actual selected brief/draft content delivered during 02.
- **02:** runtime `durable_runtime`/`structured_runtime`, daemon `control_plane` durability/direct/
  published cases, protocol tests and CLI session tests own input isolation, exact replay and
  conflicts. Add a focused actual-binary lost-reply test through `tools/evidence` and count external
  fixture entries across create/start interruption points.
- **03:** control acceptance tests already distinguish final prose, empty/tool-only output and
  exhaustion. Extend control-service, runtime reconciliation, daemon read/control and CLI artifact/
  stream tests for truthful results, supported repair, terminal refusal, guards and safe downloads.
- **04–05:** control published/learning and daemon publication tests own invoke-only disclosure,
  retirement/replay and honest comparisons. Add focused copy and non-CLI public-route cases under
  daemon/evidence owners, and run affected API/dependency contracts. Preserve existing Slotbook and
  parent/child resource tests whenever their behavior changes.
- **06:** `tools/evidence` owns the actual daemon/CLI and independent-client journey with controlled
  responses; the full gate in the workflow guide and the explicitly selected authorized local-model
  checks establish final acceptance. Retain existing direct/peer, continuation, accounting,
  protected-publication and managed-resource regressions. No new hardware claim is assigned.

## Commit working changes regularly

Use the [small-commit procedure](../../workflow.md#commit-working-changes-regularly) at each prompt's
natural commit points. This sprint assigns local commits after coherent changes and their focused
checks. Preserve the checkpoints and keep required API callers together. Pushing needs separate
authorization.

## Finish

06 verifies the combined product and fixes known in-scope defects. Guides contain tested commands;
reports distinguish passed checks from missing access or model limitations. Required tests cannot
be silently skipped. Keep a blocked sprint open rather than calling it finished or starting another
to avoid a fix.

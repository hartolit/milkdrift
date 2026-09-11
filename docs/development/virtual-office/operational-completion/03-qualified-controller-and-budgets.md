# Assignment 03 — Qualify and install the existing controller lifecycle

## Outcome

Complete the production integration of the controller that already exists. Demonstrate one bounded,
authorized loop from unsatisfactory work through proposal, approval, prospective remediation, and
verified completion or explicit stop. Make budget scope unambiguous. Do not implement another
controller, ledger, policy framework, agent role system, or unbounded background loop.

This is the next implementation assignment after 01–02. Read the sprint README, `AGENTS.md`,
canonical vision/architecture/status/roadmap, `docs/development/workflow.md`, and the implementation
and documentation practices. Preserve the current architecture and read current code before applying
review-era assumptions. Development/testing is authorized; silently enabling autonomous work in a
live operator installation is not.

## Existing owners and evidence

Inspect:
- `crates/control/src/controller/lifecycle.rs`, `controller/policy.rs`, `service.rs`, and controller tests;
- `crates/runtime/src/controller.rs`, `engine.rs`, `engine/structured/repeat.rs`,
  `engine/effects/entry.rs`, and account-related command planning/reporting;
- `crates/persistence/src/controller_account/`, `adapters/redb-store/src/controller_account.rs`,
  and `tests/contracts/controller_account/`;
- `apps/daemon/src/host/startup.rs`, controller command/read paths, config, CLI/controller commands;
- `crates/authority/src/model/resource.rs` and current budget/read/config contracts;
- `docs/product/roadmap.md`, `docs/product/status.md`, `docs/development/verification-evidence.md`,
  `docs/operations/daemon.md`, and `docs/operations/authority.md`.

The reviewed daemon builds an authority-enabled runtime and `ControlService` but does not install
`control.controller_lifecycle_owner()`. `RuntimeService::install_controller_lifecycle` is one-shot
and must run while admission is closed. Libraries already own account establishment, descendant
linkage, final-entry reservations, artifact charges, and policy assessment. Preserve those owners.

## 1. Close a finite activation checklist

At the beginning, map the **current** documented activation prerequisites to their exact owner
tests and accepted evidence: concurrency, crash/reopen, artifact accounting, compaction, mutation,
longevity, operational/full gate, and qualifying external interoperability. Record the actual missing
cases in this assignment's short handoff. This is a bounded implementation step, not another
planning-only sprint or an expanding qualification framework.

Reuse prior evidence only for behavior it tested and a source/configuration scope still justified.
The accepted Codex/LM Studio session establishes its recorded interoperability and settled-restart
claims; it does not independently qualify controller activation, thinking mode, arbitrary peers,
or power loss. Complete the missing local cases below and assess remaining external requirements
explicitly. Do not silently delete a prerequisite or mark it passed from a test name.

## 2. Install through the real composition root

Assemble the single control-owned lifecycle into the runtime before recovery/admission of controller
work. Use the existing installation API and the same store, authority service, clock, and control
service as ordinary operations. Ensure marked controller revisions cannot enter through an alternate
unaccounted route. Do not leave tests on an installed runtime while the actual daemon stays unwired.

Provide deliberate, validated activation configuration or the smallest explicit activation mechanism
consistent with current policy. Ordinary workflows retain their behavior when activation is disabled.
Requested but unavailable/unqualified activation fails clearly; no silent fallback to unaccounted
repeats. Startup of existing controller work must respect account compatibility, recovery, and the
current activation policy. Registration/configuration must not permit double installation.

Changing source is not deployment authorization. Exercise installed paths in isolated daemon tests.
If required external evidence remains unavailable, finish the integration and deterministic tests,
keep production activation refused/default-disabled as appropriate, and return the exact finite
blocker. Do not pretend the main assignment is accepted or restart a speculative cleanup campaign.

## 3. Make accounting scope visible and enforce the existing account

Keep `AuthorityBudget` as per-command/per-request authorization; do not change its meaning into a
lifetime budget. Make config diagnostics, controller status, API/CLI output, and documentation distinguish:

```text
request permission ceiling | attempt adapter limit | worker capacity
cumulative controller committed + reserved + remaining | storage retention
```

For a controlled run, expose exact account identity/revision, lineage, dimensions/units, reservations,
settled usage, blocked/unknown state, remaining allowance where knowable, and the boundary that stopped
progress. Ordinary runs must say cumulative controller accounting is not active rather than showing
a misleading infinite/zero/spent total. Keep provider estimates separate from observed charges and
hard enforceable limits. Missing cost or usage is not zero.

Prove all descendant runs/revisions/attempts governed by a controller stay attached to its canonical
account. Two workers racing for the last allowance cannot both enter. Retries, reconnect, controller
restarts, new revisions, artifact deduplication, and compaction must not reset or double-charge the
account. Unknown externally incurred usage remains reserved/blocked conservatively until supported
evidence settles it. Reject requests whose hard budget cannot be bounded under the current policy;
do not weaken unknown-usage checks just to admit a model with incomplete metering.

## 4. Close the actual control loop

Build from existing nodes/capabilities, proposal validation, authority, and prospective reconciliation:

```text
work -> independent verification -> Assignment 01 acceptance fails
  -> controller inspects exact causal evidence
  -> proposes remediation using ordinary control capability
  -> authorized approval if required
  -> new immutable revision + prospective reconciliation
  -> remediation -> verification -> accepted output or explicit stop
```

The controller cannot approve its own privilege expansion, enlarge budgets, or replace already entered
work by editing the graph. Model output remains untrusted proposal data. Preserve failed invocation
and stage evidence. No hidden scheduler agent loop or new node primitive.

Respect current bounded-repeat/fail-at-bound semantics. If the chosen policy stops rather than pauses
at a limit, expose that exactly; do not manufacture `Completed` or infinite continuation. Ambiguous
multiple proposer occurrences remain refused unless a separate, fully specified change is necessary
for this one loop. Do not broaden controller semantics for hypothetical workflows.

## Required evidence

Use actual daemon/client binaries with deterministic external process/model fixtures. Test the
installed production composition, not only `ControlService` unit tests:

- feature disabled, enabled with valid policy, and requested without prerequisites;
- one accepted remediation cycle and a second cycle stopped by a cumulative bound;
- concurrent descendant entry at the last capacity/usage allowance;
- grant revocation or narrowing before final entry and during approval wait;
- lost reply/exact command replay, crash after reservation/entry/artifact charge, and restart;
- unknown metering, late terminal evidence, overflow, and multi-currency/unsupported-unit refusal
  according to existing contracts;
- context/artifact/proposal links survive compaction without duplicate effects or charges;
- empty or output-exhausted review does not count as a useful decision or unblock work;
- ordinary non-controller workflows remain unchanged.

Run affected account, authority, runtime, control, daemon, artifact, mutation and longevity evidence,
then the full gate and actual-binary scenarios under the repository verification policy. Target the
changed boundary; do not rerun or claim unrelated platform campaigns as if they qualify it.

For the required real external loop, use only an explicitly authorized isolated repository, pinned
agent/profile and endpoint, recorded output/time/cost allowances, approved actions, and retained
provenance. Record server settings as verified/declaration/unknown, as in Assignment 01. A refusal or
unusable answer is valid negative evidence, not a qualifying success. No real-provider credentials
or raw private transcripts belong in the repository.

## Stop and handoff

Update current activation/budget explanations in their canonical owners, not the vision. Remove the
obsolete unconditional “lifecycle uninstalled” statements only when production composition and its
accepted evidence justify doing so. Preserve external evidence limitations.

Stop when the finite prerequisites are accounted for, the real composition is integrated, the loop
and budget semantics are proven, and activation has an explicit accepted or blocked decision. Report
implementation completion and operational qualification separately. Do not call a feature-flag edit
alone completion and do not auto-enable it on the live virtual-office host.

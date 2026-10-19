# 07 — Final full-system test and repair

**Prepared, not assigned.** Prerequisites: 01–06 and corrective prompts 02a/02b/02c/02d/02e complete, both actual
user checkpoints recorded, focused checks passing, no known deferred in-scope defects. This is the
named final acceptance owner
under the [sprint schedule](README.md) and [verification policy](../../workflow.md).
It is a test-and-repair assignment: fix findings, then verify the resulting code. A report of failures
or a gate run on a superseded commit is not acceptance.

Read [AGENTS](../../../../AGENTS.md), all canonical docs, [implementation](../../practices/implementation.md),
[documentation](../../practices/documentation.md), [cohesive review](../../practices/cohesive-review.md),
[workflow](../../workflow.md), [ADR 0051](../../../decisions/0051-daemon-task-workflows.md),
[code-change map](code-change-map.md), [journey](journey.md) and the tested handoff. Inspect current
source/manifests/callers and implementation evidence; do not assume early phase passes remain valid
across later changes. Inventory only actual acceptance gaps, not another broad investigation.

## Establish final product evidence

Run the connected journey from fresh roots through actual daemon/CLI/Svelte binaries. Start with a
user brief, not a preconstructed task graph. Controlled Rust models supply responses/tool requests;
the daemon admits additions, determines readiness, expands templates and recovers interruptions.
The harness may trigger documented faults, never make the scheduling or recovery decisions.

Cover the animation path and the timetable CLI reuse, including failed research, required-success
versus settled-outcome binding, parallel starts, composition, method save before activation,
accepted expansion interrupted by restart, local-copy repair, review-created correction and explicit
finish. Include concurrency conflicts, rejected authority, no-progress/failure limits, cumulative
allowance through restart, bounded tool loops, live cancellation, honest uncertainty and missing input.

Exercise persistent connection creation/rotation/disable/revoke/removal refusals and one remote tool
operation with lost response after entry. Verify same-operation recovery/no duplicate entry and
returned files. Test independent execution-only host/direct operations and Podman safeguards through
retained suites. Two daemons on loopback prove software behavior, not physical remote deployment;
if a non-loopback/physical claim is required, run its authorized lane and report it separately. Do
not reinterpret existing CPU or idle-reboot evidence as active-reboot/GPU/power-loss proof.

Actual browser acceptance includes independent-origin HTTPS/auth/preflight, denied origin, scoped
SSE reconnect, expired cursor resync, two-daemon/actor separation, fresh-client discovery without IDs,
retained text search, file/result inspection, ordinary future edits, method administration and host
connection settings. Run at least Chromium and the browser engine used for the actual user review;
mobile claims require recorded device/browser evidence beyond viewport dimensions. Review keyboard
navigation, accessible labels and readable parallel/blocked/uncertain states in rendered UI.
Include [02a's](02a-daemon-connection-identity.md) shared-name/reconnect/replay and identity-replacement
regressions, and [02b's](02b-visual-design-and-brand.md) real-logo, responsive inspector and visual
review evidence. Include [02d's](02d-canvas-and-direct-authoring.md) direct source/result wiring,
canvas/left-panel navigation, accessible connection actions, bounded partial graphs, retained-output
validation and exact draft/batch recovery. Later method/host surfaces must use the same design. Passing automation does not
replace the user's recorded response to these corrections.

Include [02e's](02e-projects-and-execution-environments.md) project discovery/adoption and exact
creation recovery, consistent file/process/container working location, guarded settings resolution,
cross-workflow path coordination and user-file preservation. Reopen a current sprint store and replay
pre-project receipts. Carry any unperformed physical Podman attachment qualification into this gate;
deterministic mechanism checks alone do not close it. Method expansion and remote tools must preserve
project context without inheriting unavailable roots or treating local paths as remote paths.

Exercise retention with controlled time, enough turnover to cross configured count/byte/page bounds,
reference changes racing collection, accepted expansion holds, pending results, unresolved remote
operations, absent content, exact archived replay/conflict and capacity refusal. Confirm project
files remain untouched. Reopen stores after fault boundaries. Verify old stores refuse before mutation
and preserved refs/data remain. The new engine's own prompt-01 stores must still open correctly after
later phases; fresh adoption is not permission to discard data produced during this sprint.

Real-model evidence is separate: record configured model/provider/profile, supplied brief, actual
results, failure/quality judgments, authority and spend. Deterministic fixtures establish plumbing,
not intelligence. Preserve negative/inconclusive model outcomes. Missing required access or actual
user review keeps acceptance open unless the user explicitly changes that evidence scope.

## Run the complete current gate

Include the transfer-review regressions: ordinary capacity fills without stranding stop/settlement;
native path overlap and uncertain-writer claims; required-task correction adoption; stop/remove races
during accepted expansion; empty nested-reference traversal overflow; secret-import replay/conflict;
and remote catalog selection without private registry setup. At least one early user-reviewed task
must have produced useful input-dependent output, not only canned controlled-model content.

Use [workflow](../../workflow.md#full-local-gate) as the current command owner. At this planning
checkpoint it requires:

```sh
cargo build -p milkdrift-daemon --bin milkdrift-daemon -p milkdrift-cli --bin milkdrift -p milkdrift-local-process --bin milkdrift-process-test-helper
cargo strict-checks --output target/daemon-workflows/final-strict
cargo test --workspace --all-features --no-fail-fast
cargo test --workspace --all-features -- --list
```

The strict runner includes formatting, defaults/all-feature compiler and warning-denying Clippy,
rustdoc, repository contracts, dependency audits and configured static security checks. Install/use
its pinned prerequisites; missing tools are not passes. Keep configured CI assertions/jobs intact.
Generate/review default/all-feature public API inventories for changed owners and their actual
consumers, including evidence-only features. Port obsolete contract expectations to the adopted
owners with equivalent safeguards; do not silence or skip them.

From `apps/web`, run the maintained locked-install, check, lint, unit, production build and complete
browser suites introduced in 01. Verify Rust-generated client output is current against wire fixtures.
Record dependency audits and retained browser artifacts. Backend test/tool code remains Rust;
frontend-native test runners do not acquire workflow authority.

Run the actual-binary evidence suites and affected manual operational/retention/host lanes required
by the resulting public claims. Select exact current tests through discovery after owner changes;
old graph-only tests may have retired but their relevant safety properties must have replacement
coverage. Document genuinely excluded hardware lanes rather than pretending local software tests
qualify them. A missing required lane is an acceptance gap, not an automatic deferral.

## Repair, review and close

For each failure, find and fix the owning cause with all consumers/docs. Use focused regressions
while repairing; make coherent tested local commits. When executable code changes, rerun the final
full gate and connected journey on the resulting state. Do not repeatedly run the full gate while
an already identified defect is unfixed, and do not weaken assertions, budgets or CI to get green.

Use another agent when available to reconstruct the final user journey, inspect ownership and
retired-path searches, and challenge unsupported claims. Describe any sole-agent review honestly.
Search old ordinary graph/proposal/published-workflow/learning/CLI-private paths and verify they no
longer compete with the task path. Inspect effects/authority/accounting protections independently
of the UI. No second scheduler or template versioning hierarchy should have appeared.

Update canonical vision/architecture/status/roadmap, package introductions, operator/API docs and
verification evidence to reflect what is implemented and actually qualified. Preserve scoped old
history as evidence, not current promises. Follow the office cleanup policy only after acceptance;
retain this sprint and the prior v3 archive while unresolved findings/review dependencies still need
them, then repair links when removing completed coordination files.

Finish with tested commits, exact gate and journey results, real-user/model/environment limits,
retained genuine issues and an ordinary supported startup/use instruction. No automatic squash,
rebase, push, deployment, new sprint or successor work. Stop when this finite sprint is accepted, or
state the precise genuine blocker without claiming completion.

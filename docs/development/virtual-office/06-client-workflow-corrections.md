# 06 — Fix the workflow review findings and verify the finished product

## Outcome

Finish the client-workflow work before starting another sprint. Diagnose and fix the two failing
integration tests, make failures useful to investigate, and allow an authorized copy between named
workflows without requiring access to every workflow. Then verify the combined result.

This is one implementation assignment, not another audit or a request to repeat phases 00–05.
Complete the underlying fixes, their callers, tests, and documentation. Do not leave a temporary
workaround for a later agent. Svelte remains the first GUI, but no frontend work is authorized here.
The daemon and its existing domain libraries continue to own workflow rules and permissions.

## 1. Establish the starting point

The review concerns `15157c0acfe4e7f73dbb0eec2b5a5d9e68321f53`. It reports
[quality run 213, job 111500364635](https://github.com/hartolit/milkdrift/actions/runs/37224200192):
the static checks passed, but the `control_plane` target had 36 passes and two failures.
These are historical findings, not a diagnosis of whatever is currently checked out.

| Finding | What was observed | What this assignment must settle |
| --- | --- | --- |
| F1: published input | `published_inputs::published_brief_uses_uploaded_text_and_declared_result` reached its deadline with the serving operation still running and no observations. | Explain where progress stops and fix the responsible implementation or test setup. |
| F1: independent copy | `reuse::independent_copy_is_editable_and_retains_source_without_run_state` stopped waiting while `author.review.accept` was still running, after the original run's model steps had completed. | Preserve overlapping runs and prove their acceptance work completes correctly. |
| F2: failure evidence | The publication test collects child details for a failed terminal result, but not for the timeout actually observed. | Collect useful, bounded, safe evidence for both paths without preventing cleanup. |
| F3: copy permissions | Copying correctly checks both workflows, but the existing scope can name only one workflow, one run, or every workflow. | Support an explicit, bounded selection containing the source and destination, through the ordinary authority path. |

The two failed cases use a controlled local model fixture. Do not blame live Ornith servers or
model quality. The evidence does not establish a deadlock, a harmless timeout, or one shared cause.
F3 is a documented permission limitation, not an existing authorization bypass. This request
includes the small permission improvement needed to remove that limitation; it does not authorize
a general permissions redesign.

Start with `git status --short` and record the starting commit and any existing changes. Compare
current source with the reviewed behavior. Preserve later fixes and other contributors' work;
do not reset to the review commit. Verify relevant CI results where accessible. If a finding is
already fixed, identify the change and prove its coverage rather than implement another solution.

Read [AGENTS.md](../../../AGENTS.md), [vision](../../product/vision.md),
[architecture](../../architecture.md), [status](../../product/status.md), and
[roadmap](../../product/roadmap.md). Apply the
[implementation practice](../practices/implementation.md),
[documentation practice](../practices/documentation.md),
[verification and commit policy](../workflow.md), and [office procedure](README.md).
For copy permissions, also read [ADR 0007](../../decisions/0007-scoped-authority-and-shared-command-path.md)
and [ADR 0020](../../decisions/0020-one-authorized-control-and-read-plane.md).
For published work, read [ADR 0041](../../decisions/0041-published-method-invocation.md).

Register this as the active correction in the office and roadmap. The previous sprint directory
may already have been removed; do not recreate its old assignments. Keep one short working handoff
at `client-workflow-corrections-handoff.md` beside this prompt. Record owners, checkpoints, evidence
locations, and remaining work there, not in new permanent progress documents. Correct any current
unconditional acceptance claim while preserving the earlier local pass as historical evidence.

## 2. Work in useful commits, not one final dump

Keep one owner for workspace Cargo jobs. Run focused checks while implementing, and reserve the
full workspace/product verification for section 6. Existing automatic CI remains enabled; this
local schedule is not permission to weaken its jobs.

Use these commit checkpoints, splitting further when a separate working change is ready:

1. Shared timeout/failure diagnostics and their focused tests.
2. Each demonstrated progress or lifecycle fix, with its regression and affected callers.
3. Named-workflow copy permissions, fully connected to public operations and refusal tests.
4. Final acceptance corrections, evidence summaries, and office closeout.

Compile affected packages and callers, run the relevant focused checks, review the staged diff,
and commit before moving to the next distinct change. Keep contract changes with the consumers
needed to make them work. A diagnostic checkpoint may retain the known integration failures;
record them explicitly instead of calling that checkpoint full acceptance.

Stage explicit files or hunks. Do not commit credentials, private inputs, generated logs, or
unrelated edits. Preserve these checkpoints: no automatic squashing, amending, rebasing, hard
resets, or force pushes. Local commits do not authorize a push. Correct mistakes in new commits.

## 3. Capture the failure before trying to fix it

Start in:

```text
apps/daemon/tests/control_plane/published_inputs.rs
apps/daemon/tests/control_plane/reuse.rs
apps/daemon/tests/control_plane/support.rs
apps/daemon/tests/control_plane/inputs.rs
apps/daemon/tests/control_plane/binary.rs
```

Reuse the existing test and child-process owners. Replace duplicated failure collection with one
small helper in the appropriate test-support module; do not introduce a new observability system
or a production endpoint solely to satisfy these tests.

### What a failure snapshot needs

Collect the test name, source/binary identity, elapsed time and deadline; the last invocation/run
state and accepted sequence; relevant parent/child and attempt identities; pending work and its
last observed progress; fixture request counts; and available authorized health/queue information.
Include bounded child logs where useful. Record unavailable fields and collection failures rather
than manufacturing empty-success values. Prefer identities, states, counts, and timings over
copies of prompts, HTTP bodies, configuration, or credentials.

Use the same collector for timeout and unexpected terminal failure. Capture it on other early
errors when the available state helps explain the failure. The original failure remains primary;
a diagnostic error or cleanup error must not replace it.

Bound collection by total elapsed time, response/log bytes, and number of inspected items. Each
awaited read must fit the remaining allowance. A limit checked only after a blocked request
returns is insufficient. Diagnostics get a separate small allowance after the test deadline;
they must not retry forever or keep the test's normal polling alive.

Use a separate diagnostic identity only where the fixture already grants it appropriate access.
Do not broaden an invoke-only actor's access just to inspect its child. Where public reads are
unavailable, report that fact and use safe existing fixture instrumentation. A diagnostic helper
must not become an alternate product-control path.

### Retention and cleanup

Keep uniquely named, bounded failure output under ignored `target/` and identify it in the failure
message. Use existing lifecycle owners to stop and join owned children, tasks, and listeners on
success, timeout, assertion failure, and early return. Do not detach them or leak temporary-directory
handles merely to retain evidence. Preserve only resources created by this test.

If a store is needed for diagnosis, use the existing safe stopped-store or administrative-copy
procedure. Do not copy a live database as if it were a consistent snapshot. Keep such stores private
by default. CI should upload only an explicitly selected, redacted diagnostic bundle—not an entire
store, workspace, or `target/` directory. Reuse the existing pinned upload action with failure-safe
conditions and finite retention. A skipped test step need not produce a bundle; missing required
diagnostics for a test that actually failed must be reported without masking the test failure.

Add focused tests that force a timeout and an unexpected terminal result. Prove the collector
finishes when a diagnostic read stalls, limits/truncation are visible, protected fields are absent,
parallel failures do not overwrite one another, and cleanup still settles owned children.

**Commit the working diagnostic improvement before investigating further.**

## 4. Find and fix why progress stops

Use the repository's pinned compiler and freshly built binaries. Record relevant build features,
worker settings, test parallelism, fixture limits, and runner conditions. Stale helper binaries must
not be confused with failures in current source. Confirm filters discover and execute the intended
tests; a successful command that runs zero tests proves nothing.

At the reviewed layout, the focused commands are:

```sh
cargo build --locked --all-features \
  -p milkdrift-daemon --bin milkdrift-daemon \
  -p milkdrift-cli --bin milkdrift \
  -p milkdrift-local-process --bin milkdrift-process-test-helper

cargo test --locked -p milkdrift-daemon --test control_plane --all-features \
  published_inputs::published_brief_uses_uploaded_text_and_declared_result \
  -- --exact --nocapture

cargo test --locked -p milkdrift-daemon --test control_plane --all-features \
  reuse::independent_copy_is_editable_and_retains_source_without_run_state \
  -- --exact --nocapture

cargo test --locked -p milkdrift-daemon --test control_plane --all-features
```

Adjust paths/names only when current source actually changed. Run each case alone, then the ordinary
parallel `control_plane` suite. Use a small, declared repetition budget with fresh fixtures and
retain every result. A serial run may help diagnose contention; it is not a replacement for the
required overlapping work. Do not retry a failing campaign until a lucky pass becomes the report.

Trace the last proven progress through serving acceptance, published child creation, dispatch,
worker/resource claims, `workflow.accept_result`, durable reporting, and result observation.
Follow the actual source to the responsible owner. Likely areas include `crates/control`,
`crates/runtime`, `crates/capability-host`, their persistence consumers, and daemon fixture startup
and shutdown. These are leads, not a predetermined diagnosis or a license to rewrite every crate.

Distinguish a queued operation from an entered one, slow ongoing work from no progress, a committed
result from a delayed read, and a product blockage from fixture lock/listener/process contention.
Inspect lock lifetimes, worker ownership while awaiting children, notification delivery, account and
resource holds, cancellation, and shutdown where the retained evidence points. Do not assume both
failures have the same cause.

If the cause is in production, fix it at its owner and update all affected callers. If it is in
test infrastructure, demonstrate that cause and correct the fixture coordination or observation
budget. A deadline change is acceptable only with evidence explaining the previous failure and the
new bound; a longer wait alone is not a fix. Keep HTTP waits, process waits, and cleanup bounded.

Preserve the behavior the tests were written to challenge:

- A published workflow makes progress with one execution worker and one serving worker. Its waiting
  parent must not consume the only worker or hold editing access needed by its authorized child.
- The source and copied workflows can overlap with independent inputs, state, accounting, and
  outputs. Do not globally serialize the product or the test suite to hide their interaction.
- Exact replay/restart does not enter the model twice or create another internal run. Retained
  uncertainty, cancellation, generation protection, and resource cleanup keep their existing rules.

Add the smallest regression that fails for the identified cause and passes with the correction.
Use controlled coordination or fault injection when it can expose the real race without timing
luck. Prove sensitivity against the old behavior in a disposable worktree or equivalent controlled
check; do not disturb the shared working tree. A test-fixture defect needs a fixture regression,
not an invented production fix. Keep independent request counters and artifact/result assertions.

No ignore annotations, unconditional retries, canned success records, broader worker grants,
blanket lint exceptions, or new scheduler. Do not remove concurrency assertions or merely rename
hard-coded delays. When no cause can be established, retain the best evidence and keep acceptance
open rather than asserting the failures were harmless. Continue completing the other assigned work.

**Commit each complete cause-and-regression fix as it becomes ready.**

## 5. Remove the wildcard requirement for an ordinary workflow copy

A user authorized for workflows A and B should be able to copy an allowed revision from A into B,
without access to unrelated workflow C. Missing source-read or destination-write authority must
still refuse the operation. This is the complete desired feature—not a new general policy engine.

Start with `crates/authority/src/model/resource.rs`, the grant owner and evaluators, daemon
configuration readers, and `apps/daemon/src/host/commands/authoring.rs`. Trace every consumer of
`WorkflowRunScope`, including scope narrowing, frozen execution authority, delegation, persistence,
and public grant documents. Inspect existing mechanisms before adding a representation.

Prefer the existing scoped-grant path. Where it still cannot represent A and B, add a validated,
bounded set of exact workflow identities to the authority-owned scope. Do not add a copy-only
permission registry, silently union all grants held by an actor, switch actor identities halfway
through the command, or bypass one of the two checks. Permissions remain explicit in the configured
grant and normal request. A selected workflow set does not grant extra actions or capability access.

Complete all affected parts together:

- Validate identities and the finite set at construction and decoding. Specify canonical ordering,
  duplicates, empty and oversized input behavior. Reject ambiguous/invalid forms; none may become
  `Any`. Own the bound once and justify it under the existing bounded-document policy.
- Implement matching and every existing narrowing/containment rule consistently. A narrower child
  cannot escape its parent's workflows. A run-only scope must not turn into authority to create a
  new workflow. Do not turn missing workflow facts into a wildcard match.
- Update configuration, public readers/writers, grant identities/digests, fixtures, and affected
  stored or wire versions as required. Make the compatibility decision explicit; old supported
  records must retain their meaning or be refused under the repository's documented version policy,
  not be silently reinterpreted. Do not abandon outstanding accepted work to change the grant
  format; any necessary migration or recovery must be explicit and tested. Update affected
  consumers once instead of adding parallel readers or aliases without a supported need.
- Keep the copy command's exact source revision, source inspection, destination import checks,
  immutable provenance, and absence of copied run state. Preserve agreement-bound copy refusal.
  Denial may retain the normal refusal receipt, but must create no destination revision or work.
  Replay must not create another copy, and existing disclosure/revocation rules still apply.

Prove success through the normal CLI and independent JSON-client path using the scoped grant—not
a hidden administrator fixture. Add focused refusal coverage for source-only permission,
destination-only permission, missing actions, unrelated workflow C, malformed scopes, attempted
authority expansion, and changed requests under the same command identity. Verify that empty or
rejected requests leak no hidden revision data. Cover persistence/reopen and exact replay for the
new accepted shape where those facts are stored.

Keep this separate from concurrency diagnosis: do not broaden the actors in the two failed tests
as a way to make them progress. Prefer updating their existing grants to the proven narrower shape
only after the original cause has been understood.

Update the maintained copy example and permission explanation. `workflow_run: any` may remain an
explicit operator choice, but it must no longer be required for this two-workflow operation. Explain
that an allowed workflow set limits the grant's existing actions to those workflows; do not claim
per-workflow action distinctions the grant model does not actually express.

**Commit the fully adopted permission change with its tests and examples.**

## 6. Verify the combined result once the fixes are ready

Review the final diff against the starting commit. Remove superseded helpers and competing rules
within the changed scope. Keep the strict lint/checker policy and the shared daemon/client boundary
intact. Do not add more lints as a substitute for the behavioral regressions.

Run the current full gate from [the workflow](../workflow.md#full-local-gate), plus the actual-binary
scenarios named by [quality CI](../../../.github/workflows/quality.yml) and the relevant
[evidence guide](../verification-evidence.md). Build every application/helper from the corrected
source. Use the recorded correction-start commit as the strict checker's `--secret-base`, so secret
scanning covers the entire correction rather than only the last documentation commit. Re-prove
scanner installation when the tools or checker changed.

The final evidence must include:

| Check | Required result |
| --- | --- |
| Strict checks | Current required compiler, Clippy, documentation, architecture, dependency, workflow and secret checks pass. Missing tools are not success. |
| Full workspace tests | Ordinary parallel execution passes, including both previously failing cases. Record ignored/manual cases separately and verify test discovery. |
| Public workflow journey | Authoring, supplied inputs, result acceptance, prospective repair, separate copies, exact replay/restart, and cleanup remain covered through actual binaries. Keep the independent JSON client. |
| Published inputs | The one-worker case completes, returns the declared artifact, refuses the wrong input form, and preserves narrow invocation permissions. |
| Copy authorization | Named-source/destination success and the refusals in section 5 work without blanket workflow access. |
| Failure handling | Timeout diagnostics remain bounded and useful; child cleanup runs even when collection fails. Selected CI artifacts contain no secrets or private raw stores. |
| Existing scenarios | The maintained headless, deterministic-model and controller scenarios actually execute; a skipped step is not a pass. |

Do not duplicate the authored-workflow journey if the maintained full suite already executes it
and retains the required evidence. Run affected adapter, resource-lifetime, cancellation and
recovery checks when the actual fix changes those owners. The existing Slotbook/publication
protections must not be weakened by the permission addition.

These reported failures need no paid provider or model-quality campaign. Reuse the established
real-model/native qualification only where the changed behavior makes a new run necessary, with
existing authorization and explicit limits. Historical UM790/model evidence stays historical.
Do not change accounts, SSH, NetBird, firewall rules, production services, or remove unowned data
to force qualification. No new GPU, active-reboot, power-loss or browser claim belongs here.

Use focused reruns while correcting a failure discovered in final verification, then rerun the
full gate on the settled executable/test state. Do not count an earlier pass after changing the
code that it tested. Keep ordinary CI jobs enabled and verify the corrected commit's hosted result
through the already authorized contribution workflow. Do not push or change repository settings
without that authorization. Missing hosted access is a specific acceptance limitation, not a
reason to fake a CI pass or stop the executable work that can be completed.

## 7. Correct the records and close this assignment

Update canonical status and verification evidence with the actual cause of each failure, the
correction, tested source/binary identities, exact commands, final outcomes, and remaining evidence
limits. Keep long logs in ignored output or CI. Preserve the earlier `9ceac87` local pass and
`15157c0` hosted failures with their real scopes; do not rewrite either into a different outcome.

Distinguish the tested implementation commit from a later documentation-only closeout commit.
If only prose changes afterward, check that diff and the relevant documentation checks rather than
pretend the self-referential final commit hash was tested earlier. Any later executable, fixture,
configuration, or test change needs affected verification and a valid final acceptance result.

Before removing the temporary assignment/handoff under the office procedure, put every lasting
explanation and required operator rule in its canonical owner. Update the roadmap, office entry,
copy guide, and affected links. Leave unrelated whiteboard topics and other contributors' work
alone. Commit the closeout separately once its stated acceptance is supported.

The final response should identify the cause and fix for each finding, the checkpoint commits,
final verification locations, and any remaining limitation. Do not substitute a large test log
for that explanation. Do not start the Svelte sprint or invent another implementation assignment.

**Done means the stalls are explained and corrected, failure evidence survives safely, a properly
scoped copy works, and the final integrated checks support acceptance. A lucky retry, a wider grant,
a suppressed check, or an unresolved failure with a new issue link is not completion.**

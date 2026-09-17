# 06 — Integrated operation, defect correction, and sprint closure

## Assignment and dependency

After 05 is accepted, own the integrated result of the entire sprint. Read all current assignment
handoffs, [the sprint README](README.md), [the discussion](discussed-direction.md), and
[AGENTS.md](../../../../AGENTS.md). Apply
[implementation practice](../../practices/implementation.md),
[documentation practice](../../practices/documentation.md), the
[verification policy](../../workflow.md), and the [office closeout procedure](../README.md).

This assignment includes fixing demonstrated integration and deeper ownership defects. It is not a
report-only audit, a visual polish pass, or permission for earlier assignments to hand off stubs.
The result must be a coherent operator experience: an independent host, useful managed work,
legitimate method adaptation, protected effects, callable workflow versions, and evaluated reuse.

Use [Slotbook](../../../guides/adaptive-method-example.md) throughout the integrated scenario.
Its source case, verifier, separate evaluation inputs, comparison threshold and variant identities
are fixed by 00. Rerun 04's one-worker same-working-area parent/child test built on 02's resource
transitions, including interruption and authorized resolution. Neither case was executed in 00.

Use a fresh reviewing agent where available. Record who actually reviewed what; do not claim
independence when the implementing agent reviewed its own work. Resolve concrete findings and stop
at the stated acceptance outcome, not an open-ended quest for cosmetic perfection.

## 1. Establish an evidence and safety plan

Record exact source, binaries, toolchain, configuration, schema versions, hosts, identities, and
permissions before testing. Read the current code, not just handoff claims. Confirm every advertised
feature is built through the production composition root and reachable through product APIs. Search
for duplicate execution/resource/policy owners, stale fake-run provenance, old bypasses, inert modes,
unimplemented branches, disconnected adapters, and explanatory claims unsupported by code/tests.

Separate five evidence classes: deterministic contract/fault tests, actual-binary loopback tests,
real Linux container/service enforcement, physical desktop-to-UM790 operation, and real local model
behavior. Restart or process-kill testing is not physical power-loss proof. A checked-in report is
not a newly run check. State the limits of each result.

Use the operator-authorized UM790 setup and connectivity supplied for execution. Do not infer working
credentials or permission to change a machine from its name in chat. Preflight actual available
Linux/systemd/rootless/container/device/storage resources. Do not change firmware, unrelated firewall
rules, accounts, SSH, NetBird, or live production services to force a pass. Use isolated named test
resources and an explicit preservation/removal plan. No broad `prune`, unqualified `rm -rf`, remote
credential disclosure, or destructive tests against unowned resources.

Select exact local model/server inputs that are actually available and approved. Record request
limits, real usage and unknowns; declare local billing explicitly. Do not require a paid cloud
provider. If available hardware cannot run two models simultaneously, use the implemented drained
replacement path. CPU execution may establish functional model use but not an advertised GPU claim.
Use the prepared API-driven setup, not a secret hand-installed environment that bypasses 02.

## 2. Run the complete operator scenario

Build and run actual product daemon and CLI binaries. Provide a maintained reproducible evidence
command through the repository's existing tooling convention; it must not instantiate a privileged
in-process host as a substitute. Scenario setup may generate bounded test data/configuration through
production readers, but cannot synthesize the successful product states it is supposed to observe.

Execute this sequence, retaining IDs and evidence from the normal public path:

1. Start an execution-only host and a workflow-enabled coordinator. On real machines the UM790
   supplies execution/resources and the desktop can coordinate. Demonstrate workflow publication
   on a workflow-enabled owner without making orchestration mandatory on the serving host.
2. Prepare the managed Linux working area, build tools, documentation, and owned local model service.
   Also attach an independently owned external endpoint or controlled sentinel service. Confirm
   actual isolation/protection facts and retained versus disposable data.
3. Invoke useful process and fresh model operations directly, then invoke the same serving operations
   from a desktop workflow. Inspect their different origins and common serving lifecycle; no local
   workflow is invented on the execution-only host. Upload/download real files through product APIs.
4. Develop the demonstration application from the fixed source input under a governed adaptable
   method and record its actual verifier result. For a failed candidate, obtain selected failure
   evidence and a structured model proposal for legitimate repair or investigation. Apply it through
   ordinary preauthorized control, rebuild, and obtain applicable verifier evidence. If the live
   source candidate passes immediately, retain that result and exercise model-driven repair against
   the separately labelled seeded candidate. That demonstrates repair from a seeded failure, not an
   observed model planning failure or a positive live learning result.
5. Invoke the protected publication operation and the same method as a versioned workflow capability.
   Confirm the exact candidate/configuration is deployed to the allowed test target and remains
   served after the deployment request finishes. Inspect public and internal state using appropriate
   different permissions. Do not grant the caller production internals merely to make the demo work.
6. Learn from prior authorized run evidence through 05's actual workflow. Obtain a real model-produced
   candidate method, evaluate it on distinct declared inputs, and promote or reject on the configured
   criteria. Exercise positive promotion in the deterministic mechanism lane; promote the real-model
   candidate only if it meets the unchanged criterion. A negative or inconclusive live result is
   retained as such, including absence of an observed source planning failure. Do not substitute
   seeded behavior or modify cases to manufacture a successful learning claim.
7. Produce at least two meaningful variations through the selected reusable method, with separate
   mutable workspaces and accepted output lineage. Compare/select through supported operations;
   do not silently deploy every candidate or reuse another variant's evidence.
8. Reapply setup, restart relevant daemons and services, and perform a real host reboot when authorized.
   Recover without duplicate work or accidental resource adoption. Update an owned service/configuration
   through the normal staged/drained path while preserving data and old accepted generation facts.
9. Remove the disposable setup using its recorded ownership/preservation policy. Keep selected
   source/output/knowledge/data, leave the attached external service and unrelated sentinel resources
   untouched, and show no unowned temporary workers or services remain.

The scenario must allow useful adaptation as well as reject bypasses. Do not substitute a canned
proposal for the actual model output in the real-model lane. Deterministic fixtures remain useful
in the separate regression lane. An unhelpful model result is evidence to inspect and address within
the supported workflow, not permission to invent success or widen authority. Keep iteration bounded.

## 3. Attack the intended boundaries with controlled tests

Use the existing fault/mutation facilities and isolated test resources to verify these cases. Correct
failures at their actual owner and migrate affected consumers, rather than patching the evidence
harness to hide them.

| Boundary | Required checks |
| --- | --- |
| Acceptance and identity | Drop replies before/after durable acceptance; exact replay after restart/archival; changed bytes under the same key; cross-caller/peer key collisions; one internal run for a published call. |
| Entry and uncertainty | Fail preparation, revoke during preparation, kill after possible entry, lose terminal/artifact commits, and recover. No forbidden read/entry or blind repeated external effect. |
| Context and data | Forged/missing origin, wrong attempt/direct selection, cross-workspace/private artifact access, omission disclosure, corrupt bytes, oversized uploads, and stale context; protect existing workflow continuation. |
| Accounts and delegation | Exhaust cumulative limits across direct/delegated/published/child work, retry/revision/restart, duplicate settlement, unknown usage, and remote response loss. No reset, double charge, or premature release. |
| Resource lifecycle | Create succeeds but result commit fails; generation replacement/removal races with users; stale callbacks; mismatched resource identity; cold execution detail; restoration of a duplicated installation. |
| Manager separation | Worker tries manager files, configuration, credentials, engine/systemd sockets, unapproved mounts/devices/network and trusted-host shortcuts. Test actual claimed OS protection, not only request parsing. |
| Method obligations | Legitimate scoped repair succeeds; deleting/replacing checks, altering verifiers, changing terminals/data dependencies, or weakening policy cannot produce compliant completion. |
| Effect protection | Direct/peer/raw-resource bypass, forged acceptance, changed artifact/configuration/target, replay at another target, revoked/expired evidence, and writes to served content after verification are prevented. |
| Published service | Narrow invoke-only authority works; edit/read-internal/publish rights stay separate; new version and retirement do not rewrite accepted calls; one worker slot still permits internal work; nesting/cycles are bounded. |
| Nested resource use | With one worker, a child edits the parent's managed working area while the parent waits. Parent/conflicting writes and removal/replacement refuse; unrelated resources progress. After proven child quiescence the parent reacquires editing with no duplicate or leaked hold. Interrupt handoff and active use, restart and cancel; exact lineage/claims survive and uncertainty blocks unsafe reuse until authorized resolution with stop/fencing evidence. |
| Learning and variants | Denied evidence stays denied; criteria cannot be edited by the candidate; failed candidates remain unpromoted; promotion is exact and future-only; variant data/evidence/usage are isolated. |
| Shutdown and bounds | Queue saturation, long streams, panics, inherited pipes, stalled supervisor replies, many retained operations, and cancellation produce bounded ownership and truthful outcomes. |

Use independent external counters, resource identities, content digests, and complete store teardown
where the assertion needs them. A successful HTTP status does not prove the side effect happened
once; a retained row does not prove the referenced files exist; an unchanged graph node does not
prove no bypass exists. Exercise privileged administrative exceptions separately from normal
agreement satisfaction and keep them unavailable to the restricted worker.

Review the caller experience for refusal and recovery: the user must identify what is blocked, who
owns it, what evidence is missing, and which authorized recovery action exists. “Busy forever,” a
silent no-op, or a traceback with no resource identity is not an acceptable supported outcome.

## 4. Fix integration and reduce actual debt

Trace every failure through semantic owner, storage transition, adapter, API, and tests. A broken
shared concept is corrected once at the correct owner with full adoption. Remove parallel legacy
paths and unnecessary translation/factory layers. Do not add compatibility aliases, error-swallowing
fallbacks, sleeps, special evidence-only defaults, or broad privileges to make integration pass.

Review package/dependency boundaries and public surface after the implementation has settled. Each
new crate must enforce a real boundary and be used by a production root; each adapter must be the
sole mechanism implementation in its declared scope. Review oversized/mixed-responsibility modules
and duplicated invariants. Refactor where an actual coherence or correctness issue exists, not to
meet arbitrary file-count aesthetics or start an unrelated cleanup campaign.

Preserve existing supported workflow authoring, contextual continuation, controller accounting,
ordinary direct process/model calls, peer placement, artifact handling, backup, and offline recovery.
Scope expansion does not authorize silently dropping earlier supported behavior. Deliberately changed
compatibility must match reviewed readers, fixtures, operations guidance, and exact refusal behavior.

Add regression tests for every repaired defect and demonstrate key negative tests detect weakening.
If required hardware or an external permission is unavailable, complete all executable/software work
and the runnable qualification recipe, retain an explicit qualification blocker, and do not mark
the whole sprint accepted. Missing access is not a reason to leave a known implementation bug.

## 5. Finish product documentation and verification

Provide a newcomer-usable operator route from fresh installation through setup, direct work,
workflow publication, repair, learning/variants, inspection, restart recovery, update, and removal.
Use exact tested commands and generated configuration inputs. Shell/system changes should be
copy/pasteable and scoped, not a list of manual editor instructions. Explain accepted results versus
completed operations, managed versus attached resources, and permissions at the point of use.

Update vision only for real direction changes, architecture for the final ownership, status for
actual supported/qualified behavior and remaining limits, roadmap for genuine unfinished work,
ADRs for durable decisions/compatibility, package/API docs, references, examples, and evidence guide.
Do not copy sprint chronology into them or call a limited model/service scenario universal
portability, security, reproducibility, or provider qualification. New Linux-only behavior must be
explicit; retain existing platform test coverage and honest unsupported behavior elsewhere.

Run the full gate from the current workflow after integrated fixes, plus the new focused and
actual-binary lanes. Ensure test discovery includes the new non-optional contract cases and expensive
or hardware-dependent lanes are explicit rather than silently skipped under a green headline. In a
selected real-Linux/UM790 qualification job, prerequisite mismatch must fail or be reported as not
qualified, not count as a pass. Check generated public API/default-feature inventories and dependencies
under the current policy. Re-run affected checks after subsequent changes.

Measure idle role footprint, bounded concurrency, relevant waits, resource-hold recovery, and retained
state behavior with environment details. Use measurements to find real defects; do not invent a
performance promise or an arbitrary target that was not requested.

## 6. Accept and close the sprint

Write `handoffs/06.md` while the sprint is active. Record exact source/binary identities, verified
coverage of each assignment, real model/host/protection facts, commands and evidence locations, fixed
integration defects, removed alternatives, and genuine remaining limits. Keep raw output under
ignored `target/adaptive-hosts/06/` or CI; canonical evidence documentation records reproducible
commands and narrowly stated results rather than embedding a giant log.

Complete the office closeout only when acceptance is real. Move every lasting decision and necessary
operator rule out of temporary sprint files, reconcile the relevant whiteboard topic and overview,
update the roadmap/status, fix incoming links, then remove the completed sprint directory and its
entry. Preserve unrelated whiteboard topics and other sprints. Git retains committed history.
Do not remove the sprint while a required physical acceptance remains pending unless the user has
explicitly accepted a different closure scope; retain a precise blocker instead of a false success.

The stop condition is the verified integrated product behavior, accurate canonical documentation,
no known in-scope incomplete implementation or bypass, and clean ownership-aware closeout. Do not
create a new sprint, GUI task, or indefinite polishing cycle as part of finishing this one.

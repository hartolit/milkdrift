# 03 — Useful adaptation with protected obligations

## Assignment and dependency

After 02 is accepted, implement the distinction between an adaptable method and the agreement it
must satisfy. Read the accepted predecessor handoffs, [the sprint README](README.md),
[the discussion](discussed-direction.md), and [AGENTS.md](../../../../AGENTS.md). Apply
[implementation practice](../../practices/implementation.md),
[documentation practice](../../practices/documentation.md), and
[the verification policy](../../workflow.md).

The required product behavior is positive: an agent can add legitimate investigation/repair work,
adopt a prospective revision within preauthorized scope, produce a changed candidate, obtain new
applicable evidence, and complete the protected operation. The same delegated authority cannot
weaken the requirements or bypass them through a direct host call. Rejecting every change, asking
for approval for every internal adjustment, or checking only that a node still exists is not enough.

## Existing owners to inspect

Trace blueprint task/structured/interface/mutation/context owners; runtime command admission,
reconciliation, scheduling, final entry, projection/recovery, and controller accounts; control
proposal classification, approval, acceptance, and workflow-control adapter; artifact provenance;
authority grants; and the managed service/candidate operations created in 02. Read the current
result-acceptance guide and tests, ADR 0005, authority ADRs 0007/0014/0019/0020, context ADR 0031,
and result acceptance ADR 0032.

The baseline `crates/control/src/policy.rs` supplies coarse change classification. Approval there is
not a general preservation theorem. The acceptance guide's mechanical/report checks are not a
general semantic-quality guarantee. Keep their valid uses while correcting the missing agreement
and effect boundary. Use 00's owner decision; refine a mistaken abstraction now rather than
wrapping it in another policy layer.

## Required implementation

### 1. An immutable governing agreement for a scope of work

Represent the agreed process/result obligations, required evidence, relevant effect prerequisites,
permitted adaptation scope, resource limits, and completion/failure/escalation meaning. Bind their
exact identity/version to the run or governed scope when accepted. Separate them from the graph
region the delegated agent may change. Carry them through child work, retry, continuation,
prospective revision, and restart without resetting or silently broadening them.

Use explicit validated contracts, not arbitrary strings interpreted independently by different
owners. Reuse existing blueprint, control, authority, artifact, and persistence meanings where they
fit. A new contract must have one semantic owner, strict readers, versioned evidence, and minimal
public surface. Do not create another scheduler or require a new crate merely because the word
“agreement” exists.

Cover process obligations as well as final output: required verification/review, distinct authorized
verifier where declared, permissible targets, candidate/configuration binding, and mandatory approval
where the agreement requires it. Completing a task, accepting an output, satisfying a governing
agreement, and performing an external effect remain distinct observations.

### 2. Scoped automatic adaptation

Implement an enforceable change policy for designated editable scopes. Permit useful internal
investigation, repair stages, and dependencies within the accepted limits. Apply those changes
through the ordinary proposal, validation, reconciliation, and audit path. A permitted method
change does not require fresh human approval merely because the current coarse classifier labels
all node replacement as approval-required; refine that rule within explicitly authorized scope,
not globally. Preserve stricter unrelated policies.

The adapting actor cannot delete/weaken an obligation, alter its validator, expand authority or
budget, change its governing policy, move the protected effect outside its checks, or edit the
immutable enclosing boundary. Cross-scope edits and unknown changes refuse or require the separately
authorized decision specified by the governing owner. Do not infer safe adaptation from a trusted
actor label, a self-reported risk score, or model prose.

Use explicit protected structure, validated interfaces and policy, and final effect checks rather
than trying to infer semantic equivalence of arbitrary programs. Check indirect changes: data-edge
rewiring, removing a dependency, terminal alteration, subworkflow substitution, mutable verifier
configuration, and a newly introduced alternative effect path. Changing the current run remains
separate from promoting the reusable blueprint or altering what future callers are promised.

Define the treatment of a legitimate requirement change or exception. It needs authority outside
the adapting grant and an explicit new agreement/recorded deviation; it cannot retroactively turn
a failed original requirement into a pass. A permitted repair must retain failure evidence and
prospective lineage. No revision mutates historical execution facts.

### 3. Evidence bound to the actual candidate

Implement trusted evaluation and acceptance for the sprint's deployment example using a real
configured verifier and concrete declared checks. Freeze the candidate actually to be deployed:
immutable artifact/build output, relevant configuration, policy, verifier generation, target, and
validity conditions. Source commit identity alone cannot cover modified files or a later rebuild.
The effect must use the checked bytes or refuse; do not validate one checkout and deploy whatever
is at a mutable path later.

The evaluator verifies exact evidence sources and applicable identities, not simply an `accepted`
boolean or artifact checksum. Respect artifact read/sensitivity authority before inspection and
prevent forged producer claims. Separate evaluator credentials and trusted implementation from
agent-editable workspace files. Avoid trusting an agent-supplied test report as independent evidence.
Changing candidate, material configuration, target, policy, or verifier identity invalidates evidence
unless the supported contract explicitly and verifiably allows the reuse.

Preserve existing generic result-acceptance behavior for tasks outside the new governed scope. Do
not turn all model tasks into deployment checks. Express domain checks as composed capabilities and
contracts; runtime should enforce agreement/entry rules, not implement a universal security scanner.
Bound evidence loading and validation, and retain why acceptance failed or remained unknown.

### 4. Enforce the agreement at the consequential operation

Implement the protected publication/deployment operation over the managed service/candidate boundary
from 02. Its owner requires the applicable authorization and acceptance evidence at final entry for
all callers, including CLI/direct API and remote calls. Bind the exact target/resource generation,
request and permitted evidence use; replay is the same accepted effect, not permission to redeploy
elsewhere. Handle changed policy/revocation between verification, preparation, and entry.

Do not give the adapting worker a parallel unrestricted path to the same effect: raw service
configuration, unrestricted container/socket access, writable served content, production credentials,
arbitrary trusted-host process, or an unguarded lower-level start/update operation. Mark protected
resources and enforce equivalent prerequisites on ways that can change their served state. General
unprotected development resources need not acquire production policy; explain that distinction.
Explicit administrative override, when supported, is a separate operation/permission with a recorded
deviation and must never masquerade as normal agreement satisfaction.

Maintain atomic local entry/reservation/resource-hold rules. Record intended external change before
performing it. Crash or response loss around acceptance and publication retains uncertainty and exact
replay; a successful effect followed by failed reporting cannot cause an automatic second deployment.
Authorization to inspect a prior acceptance result alone is not permission to reuse it for a new effect.

### 5. Usable authoring and inspection

Expose supported agreement/adaptation authoring, proposed-change inspection, exact refusal reasons,
acceptance/evidence inspection, and the protected operation through the existing client/API/CLI and
ordinary workflow-control capability. An agent must be able to make a real structured proposal from
selected failure evidence and execute the allowed repair path; do not require a Rust test harness to
construct the only working example.

Provide one maintained adaptable deployment blueprint and input/configuration examples using real
production readers. Include a permitted repair region and concrete acceptance/target policy. Use the
existing external model and process adapters; no new model family or inference implementation.
Keep normal planning and model output untrusted until parsed and authorized.

## Required behavioral matrix

Test through public owners and actual binaries, including concurrent changes and store reopen:

| Attempt | Required result |
| --- | --- |
| Initial application fails the declared check | Failure evidence retained; protected publication cannot proceed. |
| Agent adds valid investigation/repair inside its scope | Ordinary prospective revision succeeds without unnecessary fresh approval; useful repaired candidate reaches new verification. |
| Same graph edit outside its permitted scope | Refusal or separately required approval, with no authority or budget expansion. |
| Delete check, lower requirement, change verifier, rewire output/dependency, or bypass via another terminal | No compliant success and no protected effect. |
| Fabricate a positive report or reuse a report for another artifact/config/target | Refusal before publication. |
| Candidate changes after checking but before entry | Exact immutable candidate used or refusal; never deploy unverified current bytes. |
| Direct call, peer call, raw managed-resource change, or attempted credential/socket path | Same protected effect conditions; worker has no unguarded alternative. |
| Revoked authority/evidence validity, stale proposal, expired lease, exhausted budget | No unauthorized entry, no reset of inherited limits. |
| Crash after entry, failed terminal commit, exact request replay | One effect or explicit uncertainty; no invented success or unsafe retry. |
| Legitimate explicit requirement change by the proper owner | A distinct agreement/deviation, preserved original history, no false claim of satisfying the old requirements. |

Add targeted tests for the most indirect bypasses, not just obvious node deletion. Show that key
tests fail when the relevant enforcement is deliberately disabled in a disposable verification
worktree or existing mutation harness. Also retain positive ordinary workflow, direct-host, model,
and service-lifecycle cases so the protection does not disable the product.

Run the full gate and relevant blueprint/control/runtime/authority/acceptance/daemon/persistence
suites. Actual external-effect counters and immutable candidate bytes must support no-entry and
correct-candidate assertions. Deterministic model fixtures are appropriate for refusal reproducibility;
live model repair is part of the integrated qualification in 06, using the same authoring path.

## Completion and handoff

This assignment finishes the adaptable deployment method and its protected operation, not only a
policy declaration. Publishing the method as a callable catalog capability is the distinct next
assignment. Do not defer effect enforcement, evidence trust, or legitimate autonomous adaptation
there. Fix deeper shared-control or artifact ownership issues within this responsibility now.

Update canonical docs, ADRs for hard-to-reverse decisions, status, supported authoring examples,
CLI parser checks, and exact schema fixtures. Write `handoffs/03.md` with agreement ownership,
allowed adaptation semantics, effect-bypass closure, candidate/evidence identity, replay/recovery,
public use commands, test results, and remaining physical qualification. State the verifier's finite
claim; never present the example as proof of general application correctness or security.

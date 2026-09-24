# 0040 — Adapt the method within a protected agreement

- Status: accepted direction; adaptation/effect implementation assigned to adaptive-hosts 03, evaluated reuse to 05
- Date: 2026-09-18
- Extends: [0005](0005-prospective-revision-reconciliation.md), [0013](0013-immutable-proposal-revisions.md), [0032](0032-purpose-specific-result-acceptance.md)
- Revises: coarse approval policy as the only automatic-adaptation boundary in [0014](0014-shared-human-ai-control-path.md)

## Context

Requiring approval for every node replacement prevents useful preauthorized repair. Simply relaxing
that classifier would also let an agent remove checks or change the meaning of success. Existing
result acceptance correctly distinguishes completion from acceptable output, but trusts a selected
verifier's report and does not protect a deployment against every direct or indirect effect path.

## Ownership and decision

Blueprint owns an immutable agreement definition and its protected scope/adaptation policy: public
input/output meaning, required responsibilities, process/result obligations, permitted edits,
effect prerequisites, limits and failure/escalation behavior. The agreement is a separately identified
immutable object referenced by the starting method and accepted scope; it is not an editable metadata
field inside the region it governs. Blueprint validates structural edits and interfaces, not arbitrary
program equivalence. Persistence stores the definition and accepted binding; runtime owns binding,
inheritance, reconciliation and completion facts. Control owns proposal classification and the ordinary
acceptance evaluator. Authority owns who may adapt, change agreements, verify, publish or override,
and the trusted-verifier policy used in effect decisions.

The host consumes immutable agreement references and validated evidence through existing semantic
and persistence ports. It must not depend on control's application service to protect an effect.
Keep shared evidence identity/provenance with workspace/artifacts, pure agreement semantics in
blueprint, and effect authority evaluation in authority. Concrete verifier checks run as an external
capability. No universal scanner, agreement scheduler or control-to-host dependency cycle is needed.

A governed run freezes its agreement independently of its editable revision. An allowed proposal
can add investigation, replace future implementation/repair tasks or change their dependencies inside
declared regions, within the retained grant, account and interfaces. Control computes the actual
delta and checks it against that scope before applying the existing prospective reconciliation.
The new policy permits such changes without fresh human approval; no global downgrade of the
existing classifier applies outside a governed scope. Started/completed/uncertain effects keep
their existing reconciliation rules. Unknown or cross-scope changes refuse or require the exact
separately authorized decision named by the governing policy.

Protected responsibilities and their inputs, verifier trust/configuration, consequential targets,
completion obligations and adaptation limits cannot be weakened by a repair or learning grant.
Check indirect rewiring, terminal changes and subworkflow substitution as well as node deletion.
An authorized agreement owner may create a new agreement or explicitly record a deviation; that
action never reports a failed old requirement as satisfied. The adapting actor cannot edit its own
limits or promote a method just because it may repair a run.

## Repair and effect trace

The maintained [Slotbook example](../guides/adaptive-method-example.md) supplies finite requirements.
A candidate fails its trusted verifier. Failure evidence remains selected and inspectable while the
agent proposes a permitted repair. New implementation bytes become a new immutable candidate, and
the verifier checks those bytes under the unchanged agreement and configuration. The proposal must
not prescribe a particular extra stage as the only acceptable repair.

Evidence binds the candidate artifact/build identity, configuration digest, target and resource
generation, agreement and verification-policy identities, verifier implementation/generation and
authenticated producer, checks/results, validity and permitted use. Trust in that producer is a
separate decision from verifying an artifact checksum. A commit ID without modified files, or source
verification followed by an unbound rebuild, is insufficient. The verifier executes outside the
agent-editable workspace and publishes through its own authenticated artifact path.

The protected resource operation is the consequential gate. Capability-host's resource owner marks
the target protected and requires applicable evidence at final prepared entry for direct, workflow
and peer callers. All operations capable of changing its served state apply the same prerequisite:
create/replace configuration, update/start a new candidate, or mutate served content. The repair
worker has no raw engine socket, writable served mount, deployment secret or trusted-host shortcut.
Unprotected scratch resources remain useful without acquiring production obligations.

Prepare the exact immutable bytes and configuration, then recheck authority, policy/revocation,
evidence applicability, expected target generation and resource use in the durable entry transaction.
Commit intended deployment identity before the external action. Replaying that request returns its
accepted work; it cannot authorize a different target or changed candidate. Effect recovery uses
0039's exact identity/evidence rules. Public success additionally needs retained agreement satisfaction
and output evidence, not merely a terminal internal run or an agent-authored `accepted` Boolean.
An administrative override, if implemented, requires a separately scoped operation and recorded
deviation and remains unavailable to the constrained worker.

## Learning and publication

Control accepts an evidence-bearing candidate blueprint proposal under ordinary authoring authority.
Evaluation runs belong to runtime and use isolated resource generations/working areas. The evaluator
owns the fixed separate inputs, unchanged verification and comparison criterion; the proposal agent
cannot read held-out inputs or edit evaluation policy. An authorized publisher selects a qualifying
candidate through [0041](0041-published-method-invocation.md). Selection is future-only. Failed or
inconclusive evaluation retains the current method. Knowledge remains ordinary selected artifacts
and scoped files, not a second memory database. No learning process acquires authority from success.

## Compatibility and evidence

Current generic result acceptance, revision identities, risk policy and schemas remain as implemented
until 03. Do not decode an old revision as if it accepted a new agreement. Introduce versioned core
agreement/binding, evidence and policy forms with hand-reviewed fixtures; retain historical risk
decisions under the exact policy that produced them. Where a changed serialized boundary cannot
preserve exact meaning, use explicit refusal under the pre-release policy, never a permissive fallback.
03 owns blueprint, authority, control/runtime, resource entry and public consumers together. 05 owns
the ordinary learning/evaluation examples and any necessary narrow proposal/evidence extensions.
No numeric version or storage migration is selected by this design document.

Positive repair and negative bypass tests must share the same product path. Remove a check, change
its verifier, rewire its candidate, change the target, replay stale evidence, call the raw effect,
revoke permission during preparation and restart after entry. Assert actual no-entry or exact served
bytes independently of model prose. Learning requires baseline/candidate measurements on the same
separate inputs; a renamed stage or an extra artifact is not improvement.

## Alternatives and reconsideration

Unconditional human approval is too restrictive; unrestricted auto-apply removes the promised
boundary. Graph labels alone cannot protect an effect. Arbitrary semantic equivalence is neither
required nor claimed. Reconsider a protected scope when a concrete useful adaptation cannot be
expressed while preserving its obligations; do not weaken the agreement to make a candidate pass.

## Implemented contracts and compatibility

Blueprint/mutation schema 3 carries an independently digest-bound agreement alongside the semantic
method. Its protected fingerprint covers workflow/package identity, interface, metadata, noneditable
nodes and every edge incident to them. Editable regions admit ordinary tasks under exact declared
capability envelopes, a maximum node count and a cumulative revision limit. Structured work and child
pins remain protected. This deliberately avoids inferring program equivalence. Active adoption cannot
introduce, remove or replace an agreement, even with proposal approval. Legitimate requirement changes
use a distinct agreement, target and run; no administrative deviation operation is implemented.

Runtime records the outer accepted binding with a schema-4 `AgreementAccepted` event before start
and inherits it into child work. Snapshot payload 5 retains binding and adoption count. Existing
v1–v3 event meanings remain readable where nested contracts are supported; they cannot contain
agreement acceptance. Blueprint v1/v2 refuse rather than infer an agreement. Redb internal format 19
and physical schema 14 refuse older stores before writes; there is no migration.

Control records scoped classification under `milkdrift.agreement-adaptation` version 1. Ordinary
`milkdrift.control-risk` version 1 is unchanged. A strict proposal draft reader derives mutation and
proposal identities before the same authorized submission path. Draft notes and provenance remain
untrusted. Control protocol 2.9 exposes accepted agreement and cumulative adoption count.

Authority's version-1 protected effect policy fixes all required checks, producer, verifier digest,
byte ceiling and lifetime. Workspace's version-1 evaluation binds those observations to an immutable
artifact, agreement, effective recipe, target and prior generation. The private redb evaluation table
records an incomplete result before calling the verifier and completes it once. Failed or unknown
results persist; uploaded reports can only select an identity, never confer evaluator trust. A hard
4096-record bound refuses new evaluations instead of discarding failures or replay evidence.

Managed contract 2 adds evaluate, evidence and publish to the existing owner. Every call, including
raw CLI and serving adapter calls, uses the same checks. Publication creates generation N+1 only from
a passing evaluation of N, reauthorizes after candidate preparation and checks policy, expiry and
authority before configuration/start. The accepted change and authorization are durable before any
service action. Exact replay returns its original receipt; inspection reports current completion or
uncertainty. Recovery is a separate authorized command over the exact saved intent. Reporting loss
cannot turn the receipt into a fresh deployment permission.

The Linux protected-service mechanism owns data and a fixed user-systemd service with an immutable
image, read-only candidate/configuration, and declared kernel limits. It exposes no raw worker for
the served target. The separate managed worker has no engine socket, production credential or served
content mount. Operator recipe removal/change and verifier/credential changes revoke new entry.
Protected recipe schema 2 and `linux-protected-service-v2` pin the native verifier executable digest;
the platform runs a verified private executable copy. Candidate bytes execute directly, with no
interpreter selection or compilation after approval. The OS and any dynamically linked libraries
remain trusted prerequisites; the maintained Slotbook candidate is statically linked. Earlier
schema-1 interpreter recipes refuse instead of acquiring new meaning. Preserve their stores with
the matching binary; a new native installation requires new verification. Successful
service restart preserves the previously accepted generation; policy expiry limits new controlled
entry, not retrospective deletion of an already running service.

The finite Slotbook harness tests the fixed HTTP contract, authenticated mutation, capacity and
interval behavior, restart persistence, cancellation, and exact deployment. Unknown engine/network
observations never count as passes. See the [maintained example](../../examples/adaptive-slotbook/README.md)
for the frozen API and ordinary product commands. No generic semantic scanner or mandatory approval
exception is inferred from a passing report; separately required command approvals and grant checks
remain applicable. Generic task result acceptance remains independent.

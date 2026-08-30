# ADR 0014: Human and AI workflow control share one application path

- Status: accepted
- Date: 2026-08-26

## Context

AI-produced workflow changes need stricter input handling, but they do not need a privileged runtime
API. Separate human and AI mutation paths would inevitably diverge in authorization, optimistic
guards, approval, idempotency, and audit behavior. Encoding durable authority in an `AI` node kind
or a role name would also confuse workflow semantics with caller authentication and grants.

## Decision

`milkdrift-control::ControlService` is the one application path for human, service, process, and AI
callers. It accepts the same versioned control commands, ordinary `CommandAuthorityClaim`, exact
optimistic guards, proposal documents, and evidence references, then delegates durable changes to
the existing runtime and persistence owners. It exposes bounded authorization-filtered run,
revision, reconciliation, proposal, and timeline read models without owning durable truth.

The in-process `milkdrift-workflow-control` capability is a normal capability-host adapter over that
service. Model-owned input contains only the control request. The adapter derives the immutable
actor/grant context from the invocation's host-frozen `CommandAuthorityClaim` and rejects a missing
or internally inconsistent claim; there is no caller-supplied authority-context reference or global
resolver. Structured model output is decoded as data; prose and tool calls are ignored. Malformed,
hostile, unauthorized, or stale input is reported as a normal rejected task terminal and cannot
write events directly. Successful results are published through an ordinary artifact port.

Observer, Advisor, Supervisor, Controller, and Autonomous are configuration conveniences that
expand to standard immutable grant revisions. They confer no implicit runtime privilege. Ongoing
control is represented by an ordinary acyclic blueprint containing an explicit pinned `Repeat` with
hard resource, action, failure, rejection, repetition, and child-depth ceilings plus a human
checkpoint. The daemon authenticates callers and hosts the service without adding a parallel
semantic path; a future UI must remain a client of that same boundary.

## Rejected alternatives

- A privileged AI/controller node kind, because authority belongs to actors and exact grants rather
  than blueprint topology.
- An adapter that appends events or edits projections, because only the runtime owns transition
  decisions and journal commits.
- Trusting model-supplied grant identifiers or risk labels, because both are untrusted proposal
  data and must be checked against host-owned context and deterministic policy.
- Hard-coded runtime roles, because preset names cannot express exact resources, budgets, validity,
  or revocation and would become a second authorization system.
- An unbounded background loop, because durable ongoing work must use explicit repeat semantics and
  enforceable limits.

## Consequences

Security and recovery tests exercise the same service and runtime commands regardless of caller
type. Human-facing clients can reuse the command/read-model contract; models gain no bypass around
approval or reconciliation. The adapter remains embeddable while the daemon owns authentication,
freezing the invocation authority claim, artifact publication, lifecycle, and transport. Adding a new control
operation requires an exhaustive mapping to an existing authority/runtime operation or a separate
reviewed ownership change.

## Reconsideration triggers

Split the service only if a new caller class has a genuinely different durable owner or transaction
boundary. Convenience, transport format, UI needs, or model-provider differences are not sufficient.

# ADR 0007: Scoped authority and one human/AI command path

- Status: accepted
- Date: 2026-08-26

## Context

Run commands previously recorded an actor label but had no grant, evaluator, revocation,
or durable decision. Actor identity was persistence-owned even though processes, models,
controllers, peers, and clients all need it. Treating the label as proof would make a future
AI controller either privileged by accident or dependent on a separate unreviewable path.

## Decision

`milkdrift-authority` owns canonical actor identity, immutable schema-v1 grant revisions,
typed scopes and operations, opaque secret references, and pure deterministic decisions.
Authentication remains outside this crate. Every external run command supplies an exact grant
revision and revocation generation to an injected evaluator. The runtime wraps unchanged
command-v1 meaning in authorization-audit schema v1 and atomically stores an exact decision in
command-result schema v2 for both allowance and denial. Exact idempotent replay returns the
original decision without reevaluation.

Human and future AI actors use this same external command path. Internal system transitions
and worker reports use private runtime-owned paths and external submission of those variants is
rejected before evaluation.

## Rejected alternatives

- An `is_admin` bit, because it cannot express resource, side-effect, budget, validity, or
  revocation boundaries.
- Arbitrary permission strings, because core operations would have no closed validation or
  exhaustive mapping.
- Actor labels as authorization, because identity claims are not authentication or grants.
- A separate AI mutation API, because it would bypass the same approvals, idempotency, and
  audit evidence required of humans.

## Consequences

Persistence depends inward on authority only for actor and decision contracts. External command
results have a reviewed v2 fixture; v1 remains readable for closed internal records rather than
being reinterpreted. Daemon and peer authentication can later prove actor identity and select
grant material without changing pure evaluation semantics.

## Reconsideration triggers

Add a new core operation only when it has stable cross-adapter meaning. Add namespaced bounded
extensions for domain-specific facts, not to bypass the closed core. Change the command wrapper
or result schema only with reviewed fixtures and an explicit compatibility decision.

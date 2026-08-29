# ADR 0019: Frozen run authority with exact per-entry decisions

- Status: accepted
- Date: 2026-08-29

## Context

Authenticating and authorizing an external run command did not by itself constrain work scheduled
later. Capability resolution could use host-wide selection policy, so a blueprint requirement,
prospective revision, controller proposal, retry, or peer candidate could acquire access that the
initiating actor's immutable grant did not provide. Reconstructing authority later from a role name,
current daemon configuration, or descriptor metadata would make accepted work mutable and would
lose the evidence needed at restart and revocation boundaries.

## Decision

Each externally started run commits one immutable `ExecutionAuthorityBasis` before `RunStarted`.
It binds the actor, exact grant identity/revision/digest, evaluator policy identity/version,
workflow/run and initiating revision lineage, accepted command decision identity/digest, and
revocation generation. Structured child runs inherit the same basis. The complete grant is not
copied into every event; exact future requests are derived from this basis and the semantic work.

Before start or prospective revision adoption, the runtime recursively evaluates reachable
capability requirement envelopes under the frozen basis. This semantic check does not depend on a
currently healthy provider. At scheduling, the host constructs an exact request for each candidate
from its immutable descriptor plus adapter-declared resource requirements. It evaluates authority
before health and capacity, then the runtime durably records the chosen generation and decision.
The exact candidate is evaluated again at effect claim and immediately before adapter code. These
decisions and requests are durable attempt provenance.

The run basis can be inherited or explicitly narrowed, never implicitly replaced or widened.
Current revocation and grant policy affect future resolution and entry. A denial releases the
lease and does not call adapter code. Authority changes after actual entry do not rewrite accepted
history or the capability's eventual terminal evidence.

Process, model, workflow-control, and peer adapters use this same path. A peer relationship proves
transport identity and supplies a typed placement/resource fact; it never replaces the initiating
actor's authority.

## Rejected alternatives

- Host-wide wildcard selection policy, because it separates later execution from the actor that
  caused the work.
- Copying an entire grant into every attempt event, because exact identities and digests plus the
  frozen basis provide proof without repeated mutable-looking policy documents.
- Authorizing only the blueprint requirement, because runtime-selected provider, profile,
  locality, peer, trust, secrets, network, filesystem, and budgets are known only for an exact
  candidate.
- Authorizing only at resolution, because queue delay and revocation can occur before claim or
  adapter entry.
- Treating no authorized candidate as ordinary unavailability, because authorization denial is a
  different durable safety fact and must not trigger fallback outside scope.

## Consequences

Drafts and proposals may be stored without gaining execution authority, but start/adoption fails
closed when their reachable requirements exceed the run envelope. Availability remains a separate
mutable concern after semantic authority succeeds. Restart can prove the same basis and resolution
decision from the journal. ADR 0020 advances authority grants to schema 2 and daemon configuration
to schema 3 for explicit read/resource scopes; broad or unbounded administration still requires a
conspicuous dangerous acknowledgement and older schemas are not silently migrated.

## Reconsideration triggers

Introduce delegated or per-node grants only with an explicit, durable narrowing relation to the run
basis. Add cached decisions only if cache identity includes every canonical request, grant digest,
policy version, revocation generation, and evaluation boundary needed to preserve the three entry
checks. Read/query authorization uses the same canonical evaluator and grant vocabulary, but remains
a distinct typed request/continuation contract under ADR 0020; it does not alter the frozen
execution basis or per-entry decision rules here.

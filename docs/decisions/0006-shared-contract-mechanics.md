# ADR 0006: Shared contract mechanics without shared domain meaning

- Status: accepted
- Date: 2026-08-25

## Context

Capability, blueprint, persistence, workspace, and runtime documents independently
implemented the same recursive JSON ordering, duplicate-key rejection, structural
bounds traversal, and validated string-newtype boilerplate. The copies already had
multiple production consumers and protect canonical bytes and hostile-input
boundaries. Continuing to copy them risks subtle divergence, while placing domain
identities or schema policy in a generic utility crate would erase semantic ownership.

## Decision

An inward `milkdrift-contracts` crate owns only proven cross-domain mechanics:
recursive canonical JSON ordering, duplicate-key-safe JSON parsing, configurable
structural bounds validation, and the common implementation of validated string
newtypes. Its APIs accept limits and validators from consumers and return structural
violations rather than domain policy.

Each consuming crate continues to define and own its public identities, validation
rules, limits, error mapping, schemas, digest domains, and durable meaning. The shared
crate must not acquire workflow, capability, workspace, persistence, or runtime
constants merely because their implementations look similar.

## Rejected alternatives

- Keeping every copy, because the canonicalization and hostile-input behavior had
  already become one invariant with more than three production consumers.
- Moving semantic identity types into the shared crate, because identical mechanics do
  not make a capability identity interchangeable with a run or revision identity.
- A broad utility crate, because unconstrained convenience dependencies obscure
  ownership and tend to accumulate unrelated policy.

## Consequences

Canonical JSON and validated-newtype behavior have one implementation and focused
tests, while domain APIs and error vocabulary remain unchanged. The workspace gains
one small stable inward dependency. A change to shared mechanics now requires running
every consuming crate's compatibility fixtures, because unchanged canonical bytes are
part of the decision.

## Reconsideration triggers

Move a mechanic back into a domain if it loses multiple real production consumers or
needs domain-specific policy to remain correct. Add a new shared mechanic only after
repository-wide evidence shows repeated production implementations with the same
invariant and dependency direction.

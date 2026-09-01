# 0028 — TOML daemon configuration compiles into narrow plans

## Context

Daemon configuration was a schema-8 JSON document that became the validated runtime object by
normalizing its public fields in place. Authentication, storage, scheduling, adapters, peers,
health, and shutdown then read unrelated fields through the same document. The shape admitted an
`enabled` peer Boolean combined with an optional identity and dependent relationship fields, so
invalid conditional state remained representable until a later validation branch.

Milkdrift has no released configuration-compatibility obligation. Operator configuration benefits
from comments and readable tables, while canonical JSON remains appropriate for durable and wire
contracts. Runtime owners need effective policy, not a mutable deserialization property bag.

## Decision

Daemon configuration schema 9 has one operator format: bounded TOML. It rejects duplicate keys,
unknown fields, invalid current versions, unsafe paths/endpoints, and cross-section policy errors.
There is no JSON fallback reader.

The raw `DaemonConfig` document is compiled once into a private-field `DaemonPlan`. Normalized paths
and defaults are resolved before construction. The composition root consumes the plan into narrow
storage, authentication, runtime, adapter, peer, and shutdown inputs; internal subsystems do not
retain or mutate the raw document. Redacted effective output is normalized TOML, while its digest is
derived from canonical JSON over the normalized effective facts and therefore ignores TOML comments
and formatting.

Peer hosting is an explicit `Disabled` or `Enabled { local_peer_id, relationships, serving }` sum
type. Disabled state owns no dependent peer fields, and enabled-without-identity state cannot decode.

## Rejected alternatives

- Keeping JSON alongside TOML would create two operator formats, ambiguous examples, and another
  permanent reader path.
- Retaining the validated public document would continue global configuration mining and permit
  post-validation mutation.
- Adding accessors for every raw field would hide rather than remove the service-locator shape.
- Keeping `enabled + Option<identity>` would preserve a conditional invalid state already expressed
  more directly by an enum.

## Consequences

Schema-8 JSON configuration must be manually rewritten as schema-9 TOML. Adapter profiles, durable
documents, control/peer protocols, and redb schemas do not change. Tests and evidence code may still
construct the raw boundary document programmatically, but daemon startup accepts only a compiled
plan. Adding a new configuration fact now requires assigning it to one effective subsystem owner.

## Reconsideration triggers

Revisit the single TOML source only if a released deployment interface creates a concrete second
configuration boundary. Revisit plan decomposition if a subsystem demonstrably needs a cohesive
cross-section policy that cannot be compiled at the composition root without duplicating ownership.

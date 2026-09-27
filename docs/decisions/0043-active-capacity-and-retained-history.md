# 0043 — Completed history releases active host capacity

- Status: accepted and implemented
- Date: 2026-09-28
- Extends: [0039](0039-managed-resource-ownership.md), [0040](0040-protected-adaptive-methods.md), [0041](0041-published-method-invocation.md)

## Context

Counting all retained installations, evaluations and method generations against operational limits
eventually refuses new work on an otherwise idle host. Removing those rows would lose command-key
conflicts, historical inspection and verification references. A larger limit only delays that choice.

## Decision

The owning records retain authority. Redb maintains derived active membership and count anchors in
the same transaction: installations remain active until verified removal, evaluations until a final
observation, and publications until retirement. Admission counts that membership. Historical queries
stay paged and exact lookup keeps every prior record. Removed installation names remain reserved.
Count loss or index disagreement refuses access; integrity scans check both directions without repair.

Publication storage capacity and executable registry capacity have distinct lifetimes. Retirement
closes selection and releases its storage slot. The publication service removes an adapter only when
the registry's entry permits and durable pending calls permit it, including after restart. Accepted
calls retain their exact definition independently of newer generations or retired catalog membership.

## Consequences

Normal completed work can turn over beyond a configured active bound in the same supported store.
Incomplete evaluations, uncertain resource changes and unresolved calls retain their obligations.
This does not promise fixed lifetime disk usage: history, receipts and referenced artifacts still
require storage, backup and inspection. No age-based deletion or implicit reuse of identity is added.

Physical format 16 adds the active indexes and count anchors without changing document format 20.
Under the unreleased [API policy](../reference/public-api-policy.md), older stores are explicitly
unsupported; retain format-15 evidence with its matching binary. There is no silent upgrade or
reinterpretation of saved facts. Within the supported format, repeated reopen preserves exact replay.

Reconsider retention only with an explicit expiry/archival contract that preserves historical
references and conflict detection. A requirement for cross-version upgrades needs its own supported
migration boundary and evidence; active-capacity fixes do not imply that promise.

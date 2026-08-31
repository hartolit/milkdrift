# ADR 0025: Explicit capability authority selectors

## Status

Accepted.

## Context

Capability authority previously represented every dimension as a set and interpreted an empty set as unconstrained. That made wildcard permission depend on absence, while separate `any` and `none` constructors obscured the same wire shape. A library caller, daemon configuration, or peer relationship with an accidentally empty allowlist could therefore create broader authority than intended.

Capability scopes are durable security meaning. They are embedded in grants, authorization decisions, execution-authority bases, command results, and daemon-owned state, so changing their interpretation without versioning would corrupt historical audit meaning.

## Decision

`milkdrift-authority` owns a private-invariant generic selector with two explicit forms: `Any`, and `Only` containing 1 through 128 unique, deterministically ordered values. Empty, duplicate, oversized, unknown, or mutable-invalid selector states are rejected. Matching and containment use selector algebra: `Only(A)` is within `Only(B)` exactly when `A` is a subset of `B`; every `Only` is within `Any`; `Any` is within only `Any`.

Capability authority is either whole-scope `DenyAll` or an `Allow` scope whose identity, category, operation, provider-profile, trust-zone, execution-trust-class, locality, and peer dimensions each contain one selector. Dimensions remain conjunctive and the maximum side-effect class remains an independent ceiling. Construction uses validated constructors and a builder; public collections cannot be mutated into an empty `Only` selector.

Runtime and control requirement traversal share one conversion from requirements to authority envelopes. Exact facts become singleton `Only` selectors and unspecified facts become deliberate `Any` selectors. Capability-host filtering, proposal containment, presets, and peer relationship expansion use the same matching and containment operations. An empty peer capability or operation filter expands to `DenyAll`, never advertise-or-invoke-any. Daemon dangerous-authority review detects structural wildcard selectors rather than empty collections.

The authority-grant schema advances from 2 to 3 and the authorization-decision schema and digest domain advance from 1 to 2. Daemon configuration advances from 6 to 7. Legacy forms are rejected rather than reinterpreted because their empty collections do not preserve operator intent. Since decisions and grants are embedded in durable rows, redb physical schema 7/internal document format 10 advance to 8/11; older stores are refused without migration.

## Rejected alternatives

Keeping empty-set wildcard semantics behind helper methods would leave direct construction and deserialization unsafe. Treating every legacy empty set as either `Any` or `DenyAll` would guess security intent. Optional sets or a public parameter bag would recreate absence-based authority and permit contradictory states.

## Consequences

Wildcard capability authority is visible in types, configuration, redacted output, and canonical JSON. Empty allowlists fail validation, except peer relationship filters whose product-level meaning is explicitly converted to deny-all. Existing pre-release grant/config files and stores must be recreated or transformed through an operator-reviewed process; no automatic migration is claimed. Selector additions must remain versioned whenever they change canonical authority meaning.

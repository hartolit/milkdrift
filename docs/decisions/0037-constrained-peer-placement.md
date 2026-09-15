# 0037 — Constrain tasks to exact execution hosts

## Context

Descriptors already distinguish locality and authenticated peer identity. A task could not state
those restrictions, so revision admission correctly refused grants narrowed on either dimension.
Exact capability names alone could not prove where an operation would run. Two repository tasks
need to select distinct approved hosts even when both advertise the same operation.

## Decision

The capability requirement owns an optional `PlacementRequirement` containing optional locality
and peer sets. Omission/null is unrestricted; empty sets deny every candidate; nonempty peers imply
peer locality. Wire arrays are bounded, sorted and unique. Peer IDs are exact, with no wildcard
syntax. Nonempty peers combined with localities excluding peer are invalid. These constraints
intersect all existing capability/profile/operation/trust requirements and grant selectors.

Revision admission derives only what the task proves. The host filters actual descriptors before
deterministic authorized selection. Runtime checks the selected descriptor, retains the requirement
and snapshot, and rechecks placement and current authority at entry. An unavailable or unsatisfied
placement never selects a different host outside that envelope. Entered or uncertain work keeps its
existing exact acceptance/replay and reconciliation rules. Constrained no-match details name the
unsatisfied requirement without describing unrelated catalog entries.

Resolved snapshot schema 3 freezes locality, authenticated peer and trust zones in the snapshot
digest alongside the existing exact generation, operation contract and extensions. Peer catalog
generation/digest/expiry remain in the peer adapter's existing provenance extension. Inspection
projects both the requested requirement and those frozen facts.

The two-host workflow also requires usable remote outputs. Serving publication uses the entered
peer execution's accepted remaining artifact-byte allowance under an execution-specific core
accounting domain. The origin run is not a serving-side workflow. Its invocation, context and input
references remain exact external causal commitments tied to the accepted peer request. The origin
fetches metadata for a durable output observation, negotiates an authorized download, and imports
verified chunks through the existing core artifact owner before reporting the output reference.
The two transfer owners bound staging and clean it up on failure. Missing transfer evidence after
entry remains uncertain; it does not cause replacement execution. Input copying remains explicit.

## Compatibility

Absent placement is omitted from canonical requirements, so existing blueprint schema-2 revisions
and mutation digests remain byte-identical. Prompt-sequence schema 3 accepts the same optional field
on capability profiles; existing compiled revisions retain their identity. There is no implicit
grant widening, schema migration or reinterpretation of saved tasks.

The product is unreleased and components upgrade together under the
[API policy](../reference/public-api-policy.md). Snapshot v3 is the only supported resolution
format: category, locality and trust zones are required, with a peer identity exactly for peer
locality. The v1/v2 readers, digest branches and missing-category runtime fallbacks are removed.
Obsolete snapshots are refused, never assigned inferred placement or migrated into new history.
Peer protocol 1.3 and control protocol 2.7 require the exact current network version. Blueprint,
sequence, journal, configuration and storage-format versions remain unchanged. Current saved peer
requests and tombstones retain exact canonical replay/conflict behavior across restart.

## Consequences and evidence

Operators can pin each repository task to an approved peer using the ordinary authoring/import,
grant, registry and entry path. This introduces no discovery, tag selectors, shared checkout,
credential copying, provider family or scheduler. A remote model URL still describes an endpoint,
not an authenticated Milkdrift execution peer.

Capability/schema tests reject obsolete snapshots and contradictory/malformed sets. Blueprint
tests verify immutable identity and unchanged canonical encoding when placement is absent.
Authority/runtime tests prove narrowed admission and current entry decisions. Host/peer tests
cover deterministic ordering, wrong-host counters, stale catalogs/health, removal, revocation,
catalog changes and acceptance replay. The daemon's two-repository scenario verifies output bytes,
chosen peer/catalog provenance, reconnect and restart. The [peer guide](../operations/peers.md)
owns supported operation; the [verification guide](../development/verification-evidence.md) owns
the executable evidence lanes and their limits.

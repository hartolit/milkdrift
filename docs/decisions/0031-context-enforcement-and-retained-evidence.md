# ADR 0031: Enforce context decisions before future execution

- Status: accepted
- Date: 2026-09-10

## Context

The first selector could stop on optional overflow before checking later required evidence.
Omission-reason precedence could also retain protected identities and sizes. Correcting fresh
selection alone would leave retries and recovered leases able to forward older saved omissions.
Those omissions contain a reason and required flag, but no independent scope or authority proof.

Blueprint session intent and the provider-neutral model request are separate declarations. The
runtime owns the governing revision and exact invocation; an adapter owns its supported protocol.
Checking only an inline request during scheduling would leave artifact and retained request paths
outside that agreement check.

## Decision

Runtime selection keeps required-evidence checks active after stopping. Under `fail_closed`, a
later eligible required candidate refuses preparation even if it could fit alone. Excluded sources
retain the existing required/exact/fail-closed conjunction. Omission disclosure depends on scope
visibility and authority, independently of the reported reason. Optional stopping and exclusion
still explain losses without loading the omitted artifact content.

The existing manifest `policy_version` records **2** for these corrected selection rules. This is
the runtime selector's version, separate from the blueprint policy digest, manifest body schema 2,
and model envelope schema 1. No field or document schema is added. Existing readers already preserve
nonzero policy versions and verify their exact digests. Version 1 fixtures and stored bytes remain
readable without migration; runtime refuses unknown policy versions for future reuse.

Before rebinding a retry, runtime checks the exact run/revision/node/execution/attempt identity,
governing policy digest, and budget. A fail-closed required `SelectionStopped` omission refuses reuse.
For policy version 1, an omission retaining a source or nonzero byte sizes
whose reason preceded the old access checks also refuses reuse: stopping, category exclusion,
not-selected, missing/corrupt, unsupported, superseded, or an improperly unredacted access denial.
The old record cannot distinguish a safe reference from one exposed by the defect. This refusal
is deliberately conservative. Version 1 selections without those ambiguous records remain usable;
new version 2 selections retain ordinary optional omission behavior through retry and restart.

Recovered leased work faces the same retained-manifest check before startup opens admission. Work
already entered keeps its existing recovery classification; any subsequent retry must pass the
reuse check. Refusal leaves accepted history and saved bytes unchanged. Runtime neither rescans
newer history nor removes omission metadata to make the old decision appear safe. An operator must
resolve the retained obligation through the existing control path and use a distinct execution
when different evidence is needed.

When claiming a `model.generate` invocation of a model capability, runtime reads its supplied
provider-neutral request and compares the session variant with the immutable governing task policy.
The same check covers inline and artifact documents, retries, and recovered requests. Artifact
loading verifies exact metadata, obtains read authority even when context selection omitted that
direct input, and applies the model document owner's byte ceiling before allocation. A mismatch
records an invalid-request rejection before `NodeStarted` and before host entry. Missing model
input and capability-specific request schemas remain adapter validation responsibilities.
Historical capability snapshots without a recorded category still apply this check to
`model.generate`; a missing classification cannot exempt a retained model request.

Matching continuation does not implement a continuation protocol. The provider still refuses
unsupported session forms before HTTP and never substitutes `Fresh`. Process capabilities do not
receive model request validation. Prompt-sequence stages remain `process.execute` tasks carrying
session intent in stage data and context policy; configured processes must implement their own
session behavior.

The current host reports a provider negotiation refusal through its existing conservative
uncertainty path once runtime has started the attempt. That durable classification is unchanged;
the local HTTP observer establishes that these unsupported continuation cases send no request.

Dispatch journals exact metadata for the artifacts retained by `NodeScheduled`, including direct
request bindings and the context manifest. This lets later output provenance cite the request
artifact without requiring an extra initial workspace value. The event's existing artifact-reference
reader still owns discovery and run accounting; recording metadata does not republish bytes or
charge a second logical artifact. Unknown or contradictory causal metadata still refuses the commit.

New prompt-sequence revision reasons derive their schema label from the validated import version.
The corrected label changes the new revision ID and descendant revision IDs, while import/profile
digests, semantic content, and remediation mutation identity stay unchanged. Stored reasons and
revision IDs remain exact; decoding does not relabel historical imports.

## Consequences

The runtime remains the selection and governing-policy owner, and adapters retain provider support
checks. Corrected selection can be reused without reconstructing history. Older ambiguous evidence
may require an operator decision even when its reference happened to be safe; granting it a new
policy version would invent proof. No schema migration, provider session support, or process-session
continuation is claimed.

Builder tests, production discovery and persisted-manifest tests, old-writer schedule fixtures,
store-reopen retries, and runtime/host/local-HTTP tests establish these boundaries. Exact sequence
tests isolate the label's identity effect and preserve historical decoding. These are deterministic
software checks, not real-provider interoperability evidence.

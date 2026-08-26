# ADR 0013: Proposals create immutable prospective revisions

- Status: accepted
- Date: 2026-08-26

## Context

A workflow-control producer may be a human, process, service, or model. Its output is not trusted,
and a live run may advance while that output is being created. Applying a patch directly to a run
would mix definition editing with execution history, make partial validation observable, and make
replay depend on mutable state. Persisting a proposal as if it were an accepted runtime fact before
the complete candidate validates would create a second, ambiguous truth owner.

## Decision

A proposal is a bounded schema-v1 document containing one closed mutation batch and exact base
revision, base digest, and optional live-run sequence guards. Canonical bytes are bound by a
domain-separated proposal digest. The control application service treats the document as untrusted,
builds the complete prospective revision privately through the blueprint kernel, validates its
workflow lineage and semantic digest, evaluates exact authority deltas and deterministic risk, and
only then publishes the candidate through the existing immutable `RevisionStore` port.

For a live run, the candidate is proposed, approved or rejected, and applied exclusively through
the runtime's existing prospective-reconciliation commands and append-only events. Completed,
started, effect-dispatched, uncertain, or otherwise committed execution remains attached to its
original revision. Stable identities derived from the proposal digest and candidate revision make
exact command redelivery idempotent. Stale base, sequence, digest, reconciliation plan, or approval
guards fail closed without rewriting history.

The revision reason retains a bounded proposal identity, digest, proposer, and production-source
anchor. The control crate does not introduce a second proposal database. A caller that needs the
complete rationale or model analysis preserves the proposal and large reasoning as immutable
artifacts referenced by the document.

## Rejected alternatives

- Apply mutation operations directly to the current run, because run history would become mutable
  and a failure midway through validation could become externally visible.
- Mutate one workflow revision in place, because prior runs and audit evidence would lose their
  exact semantic definition.
- Store arbitrary model prose as executable intent, because text has no closed contract, exact
  optimistic guards, or safely reviewable mutation meaning.
- Add a proposal-specific event writer, because reconciliation and the journal already own live
  transition truth.

## Consequences

Offline proposals can produce immutable candidate revisions without creating a run. A live race may
leave a valid prospective revision stored while the run rejects or stales its adoption; that is safe
because revision existence is not evidence of application. Approval remains an explicit recorded
runtime decision, and every applied run revision is traceable to both immutable semantics and its
proposal digest. Proposal schema or digest changes require a new version and reviewed hostile-input
and canonical round-trip tests.

## Reconsideration triggers

Add a durable proposal index only when multiple clients need bounded proposal discovery independent
of revision lineage and run reconciliation. Such an index must remain derived or transactionally
anchored and cannot become a second authority or event owner.

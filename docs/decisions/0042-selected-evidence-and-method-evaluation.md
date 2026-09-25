# 0042 — Compare reusable methods against independently declared evidence

Status: Accepted design; executed qualification is recorded in product status.

## Context

The same service can repair a product, propose a reusable method, and publish a callable version.
Those operations have different consequences. A successful repair is insufficient evidence for a
method change, and an agent judging its own result must not gain control of the criterion or
publication grant. Mutable workspace documentation is useful for current work but cannot explain
which instructions an earlier invocation actually received.

## Decision

Use existing immutable artifacts and application command receipts for selected knowledge,
declarations, proposals, comparisons and promotion decisions. `control::learning` owns the strict
request documents and deterministic comparison function. The daemon composes authorized reads
from the existing revision, context, runtime, controller, serving and managed-evaluation owners.
There is no separate learning store, execution queue, scheduler or inference service.

Selection freezes permitted artifact references and bounded public timeline projections. An
explicit successor selection records supersession and any approving promotion. Previously selected
bytes remain unchanged. A proposal's context must materialize the exact selected source artifacts;
direct-input metadata does not prove that a model received the source text. Actual response,
invocation, profile and manifest provenance must match durable output events. Ordinary structured
proposal and agreement validators admit the candidate; no new mutation language is introduced.

Before that proposal run exists, the evaluator declares exact paired inputs, reserved invocation
keys, independent writable resources, recipe generations, verifier/check contracts, budgets,
threshold, exact input and terminal product fields, and future publication generation. Distinct
input identities must also contain distinct bytes. Serving receipts resolve those keys to actual child
runs, including after archival. Comparison derives observations from complete bounded journals,
settled accounts and private host-produced verifier evidence. Missing or unresolved facts remain
inconclusive; an established candidate failure or regression still rejects. A declaration checks
the comparison, while the accepted publication/controller account enforces execution limits.
The returned product must be the final verified artifact. Public completion establishes delivery
and elapsed time; an internal terminal alone cannot establish a successful callable result.

Publication remains with `PublishedWorkflowService`. Manual promotion requires an eligible exact
comparison and current publication authority. A separately authorized policy can instead fix an
exact future publication template and executor before proposal generation. Execution substitutes
only the eligible revision, rechecks the authorizing operator's current grant and uses the same
guarded publication commit and replay path. Old generations and accepted invocations remain exact.
Learning retains a compact exact publication reference, leaving the full method with its owner.
That prevents a successfully committed publication from subsequently exceeding the learning
response bound because its entire definition was copied into the promotion receipt.

Product variants use separate ordinary invocations and writable setups. Recipe improvements use
recorded native inputs, exact images, fresh/staged verification and the existing managed update
owner. Neither recipe activation nor a documentation update silently publishes a new method.

## Consequences and compatibility

The control protocol advances to 2.11 for the `learning` command. Its operation body and bounded
declaration use their owning Rust readers; declaration schema is 1. All workspace consumers upgrade
together. Internal learning records are application receipt results, not new redb tables or an
independently versioned memory service. Every consequential operation retains ordinary exact replay
and conflict behavior; a new design or observation requires a new authorized command identity.
The first reviewed declaration requires explicit input and product fields. Unreviewed study
documents that omitted them cannot qualify under this reader. Their original receipts remain
unchanged; use their matching binary for those studies or declare a fresh study with this contract.

The current comparison reader supports at most 4,096 events per root. Unexamined descendants make
qualification inconclusive even though their usage remains in the enclosing account. This is a
finite limitation, not permission to omit inconvenient failures. Source text and model hypotheses
remain untrusted. Seeded implementation evidence and observed model choices keep explicit lanes.
Managed target/request inputs use the same bounded authorized value/artifact reader as other
workflow inputs. Their acceptance receipts remain immutable initial observations; learning reads
completed checks from the private verifier owner after matching immutable identity, subject and
time bounds. Public output declarations preserve the managed response's actual media type.

Catalog selection binds descriptor, callable operations, draining state and availability. Routine
health timestamps, load observations and diagnostic text do not invalidate otherwise identical
prepared selections. Availability, current authority and capacity remain checked at entry; expired
catalog observations refresh under the existing TTL. This preserves bounded, usable prepare/submit
behavior when a host has several independently refreshed workers.

An unnamed capability requirement can select a current registered implementation inside the service's
exact identity grant. Prospective runtime validation probes the ordinary authorized resolver and
narrows only that unspecified identity before checking the remaining requirement envelope. It
creates no attempt or permit and does not require transient health or free capacity. Scheduling
and entry check availability, capacity and authority again. Explicit identities and
other envelope dimensions retain their existing refusal rules. Publication applies the same
prospective check, so a reusable method can serve isolated workers without granting every worker
to every invocation or accepting a publication that runtime cannot start.

See [learning methods](../guides/learning-methods.md),
[ADR 0041](0041-published-method-invocation.md) and
[ADR 0039](0039-managed-resource-ownership.md) for the supported composition and retained owners.

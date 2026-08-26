# ADR 0011: Exact causal context manifests

- Status: accepted
- Date: 2026-08-26

## Context

Workflow history, current projections, branch workspaces, and artifacts can all contain material a
model might use. Reconstructing a prompt later from a changed revision, selecting by timestamp, or
retaining complete history in active state would make execution non-reproducible and weaken branch,
authority, and budget boundaries.

## Decision

Each blueprint task schema v2 owns an immutable validated context policy. The runtime builds a
schema-v1 manifest from explicit data/control ancestry, selected exact nodes or semantic roles,
visible workspace lineage, authority-filtered artifact metadata, declared inputs, and paged journal
evidence supplied by the owning runtime query path. Sibling branches are excluded until an explicit
edge, join, or reducer exposes them.

Selection order is causal depth, semantic kind, source node, then canonical source-reference bytes.
Items are admitted incrementally against item, reference-byte, artifact-byte, and optional
provider-neutral unit budgets before large bytes are loaded. Every candidate is selected with a
reason, omitted with a stable reason, or causes deterministic pre-dispatch failure when required.

The manifest binds run, revision, node, execution, attempt, policy version/digest, ordered entries,
evidence, sensitivity/authority facts, omissions, totals, budget, and a domain-separated digest.
Its canonical bytes are committed as a restricted immutable artifact before external entry.
Invocation-request schema v2 carries that exact reference once. Same-attempt retries reuse the same
request/reference; revised context requires a new attempt.

## Rejected alternatives

- Whole-history chronological concatenation, because timestamps are not causality and it violates
  branch isolation and bounded active-state ownership.
- Reconstructing at inspection time, because prospective revisions and retention can change what is
  reachable.
- An extension-map context payload, because context is core durable meaning requiring a schema.
- A tokenizer-specific core builder, because providers own final token accounting and rejection.

## Consequences

Context selection is inspectable, bounded, and provider-neutral. Candidate acquisition remains
behind runtime/persistence/workspace ports; the manifest contract contains references rather than
unbounded content or secrets. Complete history stays paged. Blueprint v1 is deliberately refused
because the repository makes no compatibility promise for inventing a policy for old semantics.

## Reconsideration triggers

Add a new policy/manifest schema when a new selector, authorized compaction decision, or
continuation form changes selection meaning. Do not reinterpret existing manifest bytes.

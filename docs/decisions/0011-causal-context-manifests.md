# ADR 0011: Exact causal context manifests

- Status: accepted
- Date: 2026-08-29

## Context

Workflow history, current projections, branch workspaces, and artifacts can all contain material a
model might use. Reconstructing a prompt later from a changed revision, selecting by timestamp, or
retaining complete history in active state would make execution non-reproducible and weaken branch,
authority, and budget boundaries.

## Decision

Each blueprint task schema v2 owns an immutable validated context policy, including exact
execution/workspace/evidence selectors and defensive discovery/materialization bounds. The task
definition also declares output semantic roles; runtime selection never guesses roles from names
or model prose.

One `ContextCandidateSource` implementation pages the journal only through the dispatcher's frozen
projection head and joins those facts with immutable revisions, workspace values, artifact metadata,
scope lineage, and the initiating run's frozen execution-authority basis. Metadata discovery covers
declared inputs, bounded control/data ancestors, exact nodes and executions, tagged outputs,
failure/cancellation/uncertainty and typed decision events, join exposure, imported subworkflow
results, selected workspace values, and explicit bounded evidence references. Historical ancestry is
computed using the revision that governed the source execution. A later revision cannot reinterpret
it. Successful sibling output remains branch-local; only explicitly imported/joined results and
joined failure evidence cross the relevant boundary. Authority and branch omissions are redacted.

Selection order is causal depth, semantic kind, source revision/node/execution, then canonical
source-reference bytes. Candidate scans, ancestor depth, historical event summaries, selected item
and artifact count, aggregate and per-item bytes, artifact bytes, manifest bytes, and optional
provider-neutral units are bounded before selected bytes are loaded. Every candidate is selected
with a reason, omitted with a stable policy/budget/authority/missing/unsupported/superseded/isolation
reason, or causes deterministic pre-dispatch failure when required. The core does not invent a
tokenizer or cost estimate when the selected adapter has supplied none.

The schema-v2 manifest body binds run, revision, node, execution, attempt, policy version/digest,
ordered entries, semantic roles, source digests and historical revisions, execution/attempt/scope/sequence,
producer actor/capability/descriptor/provider/peer/invocation, causal evidence,
sensitivity/authority, omissions, totals, budget, and a domain-separated digest. V1 is refused
because it cannot prove selected bytes or exact producer provenance. Its canonical bytes are
committed as a restricted immutable artifact and `ArtifactPublished` is journaled before
`NodeScheduled`, which carries only the compact invocation-request schema-v2 reference.

After that durable boundary, only manifest-selected non-direct content is read through the existing
authorized workspace/artifact port and checked against exact digest, size, and media type. Model
adapters receive the manifest plus delimited untrusted selected text/JSON and supported selected
images; unsupported generic binary evidence fails before network entry. Process profiles may
explicitly materialize the reserved manifest and selected-input names through their existing
input-file contract, with no implicit global file. Published output provenance includes the exact
manifest artifact.

A retry attempt reuses the prior manifest's frozen selection and omissions, rebinding only the new
attempt identity and manifest digest without rescanning newer history. Deliberately different
context therefore requires a distinct attempt/policy path and remains visibly distinct.

## Rejected alternatives

- Whole-history chronological concatenation, because timestamps are not causality and it violates
  branch isolation and bounded active-state ownership.
- Reconstructing at inspection time, because prospective revisions and retention can change what is
  reachable.
- An extension-map context payload, because context is core durable meaning requiring a schema.
- A tokenizer-specific core builder, because providers own final token accounting and rejection.
- Treating the manifest as an ordinary invocation input, because that changes workflow input and
  retry identity semantics; process exposure is an explicit adapter-profile materialization rule.

## Consequences

Context selection is inspectable, bounded, and provider-neutral. The authorized attempt read model
returns bounded manifest policy, entries, omissions, accounting, schema/digest, and exact
capability/provider/peer provenance; historical attempt identity is recovered by bounded-memory
journal paging. Candidate acquisition remains behind runtime/persistence/workspace ports; the
manifest contains references and bounded metadata rather than unbounded content or secrets.
Blueprint v1 remains deliberately refused because the repository makes no compatibility promise
for inventing a policy for old semantics.

## Reconsideration triggers

Add a new policy/manifest schema when a new selector, authorized compaction decision, or
continuation form changes selection meaning. Do not reinterpret existing manifest bytes.

# 0049 — Discover revisions within the authorized workflow collection

Status: accepted.

## Context and decision

A caller with a bounded set of named workflows needs to discover its definitions without already
knowing their identities. The revision listing remains a live page of immutable revisions, ordered
by revision identity. The daemon checks inspection authority for each selected workflow before
querying. Unfiltered discovery selects the current grant's collection; a supplied filter selects
one permitted workflow. Run-only and operation-denied grants cannot use this discovery boundary.

Persistence accepts the authority owner's bounded `WorkflowSet`. Redb maintains a private
workflow/revision index in the same transaction as each immutable revision and digest index.
Scoped queries merge only those ranges, retaining one index head per selected workflow and at
most a page of results. They verify returned summaries against the existing revision owners.
Primary and reverse index checks participate in the bounded integrity scan.

## Alternatives and consequences

Scanning all definitions and then filtering would read hidden data and allow unrelated workflows
to consume a caller's page budget. A per-caller cache would add state and stale authority. A new
workflow registry would duplicate ownership. None is needed for this collection query.

The public cursor remains compact and bound to actor, exact grant, revocation state and requested
filter. Each page checks current authority. Pages do not freeze a snapshot; later insertions behind
the cursor are not revisited, and a full final page may require an empty continuation.

Control protocol 2.19 requires coordinated client/daemon upgrade. Redb physical schema 17 refuses
older stores rather than presenting an empty new index as complete. Internal document format 20
is unchanged. Administrative cursor version 4 includes the new reverse-index scan phase and
refuses earlier cursors. There is no automatic migration or change to execution history.

Reconsider the merge only if the existing bounded workflow grant no longer expresses the product's
authorized collection or measured query costs cannot meet the page budget. Preserve restriction
before reads, current authorization, bounded work and exact immutable revision ownership.

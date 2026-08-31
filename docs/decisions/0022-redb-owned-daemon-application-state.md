# ADR 0022: Redb owns durable daemon application state

- Status: accepted
- Date: 2026-08-29
- Supersedes: the control-sidecar storage details in ADRs 0015, 0017, and 0020

## Context

The daemon was already the single host for a workflow domain, but non-runtime application facts lived in one whole-file JSON sidecar. Layouts, external-command idempotency, proposal discovery, and bounded security audit therefore had a second persistence authority and unrelated growth shared one rewrite/fsync boundary. Proposal listing scanned external receipts, optimistic layout updates replaced the shared document, and a crash between an already-idempotent runtime command and sidecar flush required application-specific guesswork.

These facts are not blueprint or run-event semantics, but neither are they disposable caches. External receipts are idempotency truth. Layout is durable presentation state whose identity must stay outside a semantic revision. Proposal discovery is derived, but its source and validation must be explicit.

## Decision

`milkdrift-persistence` owns four narrow application contracts rather than a generic app-state map:

- `ApplicationCommandStore` for exact actor-scoped command lookup, bounded pages, and atomic receipt/effect commit;
- `ApplicationLayoutStore` for exact and bounded layout reads with optimistic generations;
- `ProposalIndexStore` for bounded per-run discovery plus explicit validation/rebuild;
- `SecurityAuditStore` for an independently bounded decision sequence.

An application receipt schema-1 document binds actor, command identity, command schema and canonical digest, exact grant identity/revision/digest, optional application decision digest, accepted or intentionally durable rejected result bytes, an effect reference, and creation/completion timestamps. ADR 0023 replaces this ADR's original finite non-evicting capacity rule with bounded hot and transparent cold ownership. Security-audit retention is independent.

Redb physical schema 4/internal document format 7 adds separately keyed checked tables for receipts, layouts, proposals, and audit. A new receipt and same-store layout/proposal effect commit in one write transaction. Layout updates preserve creation time and require generation 1 for creation or exactly current plus one for changed content. Proposal discovery is a derived projection that identifies its authoritative accepted receipt; reads and integrity scans validate that link, and rebuilding scans only authoritative application receipts.

Runtime/control commands retain their existing runtime transaction as the sole semantic authority. The daemon derives stable internal runtime command identities from the external command envelope. If runtime acceptance commits before the application receipt, redelivery observes exact runtime replay and commits the missing receipt; it does not create a competing semantic transaction. A fault before runtime acceptance creates neither effect nor successful application receipt.

Daemon startup refuses legacy `control-state-v1.json` instead of silently importing or ignoring idempotency truth. No old redb format migration is claimed: older and future physical/internal formats are refused. Immutable artifact bytes remain in their dedicated content-addressed filesystem store.

## Rejected alternatives

- Keep or shard JSON sidecars, because they retain a second persistence authority and whole-document rewrite boundaries.
- Add a generic application key/value table, because unrelated state would accumulate without typed bounds, ownership, or transaction rules.
- Put layout in blueprint revisions or run events, because presentation changes must not alter semantic identity or execution history.
- Treat application receipts as an evicting cache, because forgetting a client identity permits duplicate effects.
- Combine the runtime and application transactions by moving runtime truth into daemon code, because that would create a second runtime authority and widen the persistence change cone.
- Make proposals themselves mutable application rows, because immutable proposal/revision and reconciliation truth already belongs to `milkdrift-control` and runtime history.

## Consequences

Application growth inserts or updates bounded rows instead of rewriting one file. Exact accepted and deterministic rejected command results survive restart. Same-ID/different-digest reuse is a typed conflict. Layout conflicts are optimistic and restart durable without touching blueprint digests. Proposal list cursors page a first-class projection rather than unrelated receipts. Corrupt application documents and proposal links are typed storage failures and participate in bounded integrity scanning.

The schema transition is intentionally incompatible for pre-release users. A data root with legacy sidecar state or an older redb schema needs a fresh store or a separately reviewed offline migration. Daemon configuration schema 5 removes the misleading `command_ledger_bound` spelling and names hot receipt, archival-batch, and security-audit bounds independently.

## Reconsideration triggers

Add another application-state port only for a durable fact with a named owner, explicit bounds, typed versioned document, transaction/recovery rule, and query need. Add migration only with hand-reviewed old-format fixtures and a restartable protocol. Layout must remain outside semantic identity and runtime facts must remain owned by runtime transactions.

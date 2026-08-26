# ADR 0017: Layout is outside semantic revision identity

- Status: accepted
- Date: 2026-08-26

## Context

Canvas positions, dimensions, annotations, collapsed groups, and viewport preferences change often
and may differ between clients. If they were part of a blueprint revision, harmless presentation
edits would create semantic revisions, alter content digests, complicate reconciliation, and imply
that layout could change executable meaning.

## Decision

Layout document schema 1 is an independent pure control-protocol contract. It binds one exact
workflow/revision association and contains only bounded node presentation geometry, collapsed
presentation groups, non-executable annotations, an optional viewport, server-authenticated author,
positive optimistic generation, and a domain-separated digest.

The daemon stores layout in its atomically synced schema-1 control sidecar rather than in immutable
revision bytes. The first generation is 1 and each changed update advances exactly once. On write,
the daemon replaces the untrusted author with the authenticated actor, recomputes the digest,
checks the durable workflow/revision association, and applies ordinary configured authority.

Layout cannot contain edges, task/node configuration, capability requirements, prompts, secrets,
or semantic mutation instructions. A layout update never creates a revision and cannot affect the
blueprint semantic digest.

## Rejected alternatives

- Include coordinates in blueprint semantics, because presentation preference is not executable
  workflow meaning.
- Store arbitrary UI state blobs, because they would have no enforceable security or compatibility
  boundary.
- Give layout its own crate or database service, because protocol and daemon are its only current
  contract/persistence consumers.

## Consequences

Future Iced clients can share or fork an exact layout association without changing run provenance.
Layout compatibility, digest, and optimistic update rules evolve independently from blueprint
schemas. The current sidecar has no migration or multi-user merge policy.

## Reconsideration triggers

Extract a separate layout persistence port only when multiple production storage owners consume it
or when transactional layout history becomes a reviewed product requirement. Presentation data
must remain excluded from semantic identity regardless of storage choice.

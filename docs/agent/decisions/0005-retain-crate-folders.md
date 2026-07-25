# ADR-0005: Retain the current crate folders

- **Status:** Accepted
- **Date:** 2026-07-22

## Context

The workspace groups crates under `features`, `adapters`, `engines`, and `apps`. The names are project-specific—especially “features,” which can be confused with Cargo features—but contributors can understand them and the manifests define the actual graph. Generation integration will already change important dependencies and ownership paths.

## Decision

Retain the current folder taxonomy during the first product slice. Change crate locations only when an ownership, reuse, build, or dependency change provides concrete evidence for the move. Prefer internal module splits over new crates when code shares one lifecycle and has no independent consumer.

Crate count is an outcome of cohesion and reuse; there is no numerical crate quota.

## Rejected alternatives

- **Rename all folders to conventional names immediately:** rejected as path churn without a dependency or ownership improvement.
- **Extract every responsibility into a crate:** rejected because it fragments APIs and compile units without proven consumers.
- **Consolidate the workspace into a single core crate:** rejected because vendor boundaries and independent portable algorithms have meaningful isolation.

## Consequences

- Existing paths and package names remain stable while generation is integrated.
- Documentation must explain the taxonomy and cannot rely on folder names as enforcement.
- Later restructuring must include a migration rationale beyond aesthetics.

## Review trigger

Review after the first vertical slice, when a crate gains an independent consumer/lifecycle, when folder breadth materially harms discovery, or when the approved dependency graph can be simplified by a real ownership move.

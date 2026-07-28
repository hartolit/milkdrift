# ADR-0005: Retain the current crate folders

- **Status:** Superseded by [ADR-0009](0009-workspace-physical-taxonomy.md)
- **Date:** 2026-07-22
- **Superseded:** 2026-07-29

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

The first vertical slice and the corrective-workflow extraction satisfied this review trigger. ADR-0009 records the resulting physical taxonomy while preserving the dependency roles established here.

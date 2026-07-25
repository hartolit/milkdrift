# ADR-0001: Keep `application-runtime` as the frontend-neutral façade

- **Status:** Accepted
- **Date:** 2026-07-22

## Context

Slint currently needs model resolution, tokenizer validation, persistence, lifecycle commands, normalized state, and shutdown behavior. Native Tauri, CLI, or server hosts would need the same use cases. Reimplementing them in every frontend would duplicate correctness-sensitive orchestration. At the same time, `application-runtime` currently contains concrete Candle, Hugging Face, redb, and host-runtime composition and is growing corrective-workflow responsibilities.

## Decision

Keep `application-runtime` as the frontend-neutral application façade. Frontends submit coarse application use cases and consume application-owned state/events. E1 may remain the native composition root for the first product slice.

Do not make its public type generic over every storage, resolver, tokenizer, backend, clock, or transport service. Introduce coarse cold-path ports or a closed backend enum only when replacement is required. Keep token-sensitive execution statically dispatched below the façade.

## Rejected alternatives

- **Compose E0 and adapters separately in every frontend:** rejected because it duplicates lifecycle, persistence, resolution, and error normalization.
- **Make the public façade generic over all services immediately:** rejected because it spreads implementation type parameters through callers before a second composition proves the seams.
- **Split application API/core/native composition into multiple crates now:** rejected as premature without a second deployment or transport consumer.

## Consequences

- Native frontends can reuse one application boundary.
- Browser-only clients still require a transport to a native or remote host.
- The façade must be kept narrow and should not absorb unrelated domains without evidence.
- Concrete Candle/Hugging Face/redb coupling remains an acknowledged temporary composition constraint.

## Review trigger

Review when a second backend must be selected through E1, a remote/browser transport is implemented, a second storage/resolver composition is needed, or corrective workflow growth gives it an independent lifecycle or consumer.

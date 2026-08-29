# Roadmap

This document owns ordered unfinished product slices. Each slice is an architecture-complete ownership boundary.

Execution and control/read authorization are complete in the current core: one exact immutable grant basis governs run entry, local commands, information-bearing queries, pages, streams, artifacts, layouts, capability/provider views, and peer operations. The remaining slices build on that single authority boundary rather than introducing role or transport privilege.

1. **Daemon persistence ownership** — Converge the remaining bounded control sidecar facts into one durable ownership and recovery model without changing command/read authority semantics.
2. **Peer worker recovery** — Harden remote execution ownership, restart, cancellation, storage, and transfer recovery under the same relationship grants and truthful uncertainty rules.
3. **Public-surface contraction** — Review exported APIs and semantic change cones, remove bypass-capable helpers, and preserve typed route/authority registration guards.
4. **Iced control center** — A thin native canvas over `milkdrift-control-client` with revision history, a virtualized timeline, peer/catalog provenance, inspector, schema-1 layout documents, live controls, and authenticated-cursor reconnect/resume behavior.
5. **Dogfood and continuous remediation** — Imported prompt sequences, fresh coding-agent sessions, verification gates, live reviewer/remediation insertion, daemon-owned bounded controller lifecycles, authorized AI supervision, and historical provenance inspection.

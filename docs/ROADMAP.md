# Roadmap

This document owns ordered unfinished product slices. Each slice is an architecture-complete ownership boundary.

Execution-authority propagation is complete in the current core: runs freeze an exact grant/policy basis, revisions are validated before adoption, and every capability resolution and entry is authorized with durable provenance. Read/query authorization remains deliberately separate and is the next security boundary.

1. **Control/read-plane authorization** — Apply exact actor/grant scopes to daemon queries, pages, streams, artifact reads, capability catalogs, layouts, and health views without weakening the execution basis or leaking hidden objects through counts/cursors.
2. **Iced control center** — A thin native canvas over `milkdrift-control-client` with revision history, a virtualized timeline, peer/catalog provenance, inspector, schema-1 layout documents, live controls, and exact-cursor reconnect/resume behavior.
3. **Dogfood and continuous remediation** — Imported prompt sequences, fresh coding-agent sessions, verification gates, live reviewer/remediation insertion, daemon-owned bounded controller lifecycles, authorized AI supervision, and historical provenance inspection.

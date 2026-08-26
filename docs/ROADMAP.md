# Roadmap

This document owns ordered unfinished product slices. Each slice is an architecture-complete ownership boundary.

1. **Peer capabilities** — A transport-neutral authenticated protocol for advertisement, remote invocation, artifacts, leases, cancellation, reconnect, and uncertainty, plus one reference transport over user-provided connectivity. The peer is a remote capability boundary and never a second semantic truth owner.
2. **Iced control center** — A thin native canvas over `milkdrift-control-client` with revision history, a virtualized timeline, inspector, schema-1 layout documents, live controls, and exact-cursor reconnect/resume behavior.
3. **Dogfood and continuous remediation** — Imported prompt sequences, fresh coding-agent sessions, verification gates, live reviewer/remediation insertion, daemon-owned bounded controller lifecycles, authorized AI supervision, and historical provenance inspection.

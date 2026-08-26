# Roadmap

This document owns ordered unfinished product slices. Each slice is an architecture-complete ownership boundary.

1. **Daemon, control API, and CLI** — One authoritative host for the implemented control service and capability adapter, a bounded blocking owner for redb, versioned commands, queries, and event streams, separately versioned wire status/timeline/inspector models, authenticated authority-context resolution, and a thin automation client.
2. **Peer capabilities** — A transport-neutral authenticated protocol for advertisement, remote invocation, artifacts, leases, cancellation, reconnect, and uncertainty, plus one reference transport over user-provided connectivity.
3. **Iced control center** — A thin native canvas over the shared control service with revision history, a virtualized timeline, inspector, layout documents, live controls, and reconnect/resume behavior.
4. **Dogfood and continuous remediation** — Imported prompt sequences, fresh coding-agent sessions, verification gates, live reviewer/remediation insertion, daemon-owned bounded controller lifecycles, authorized AI supervision, and historical provenance inspection.

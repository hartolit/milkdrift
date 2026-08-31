# Roadmap

This document owns ordered unfinished product slices. Each slice is an architecture-complete ownership boundary.

Execution and control/read authorization are complete in the current core: one exact immutable grant basis governs run entry, local commands, information-bearing queries, pages, streams, artifacts, layouts, capability/provider views, and peer operations. The remaining slices build on that single authority boundary rather than introducing role or transport privilege.

Daemon application persistence ownership is complete: external receipts use bounded transactional hot ownership plus transparent exact-replay cold storage, layouts and proposal discovery remain correct across receipt movement, security audit is independently bounded, health reports redacted archival facts, and legacy sidecar/older physical state is refused. Physical cold history still requires explicit offline store-generation rotation when disk policy demands it; automatic destructive rotation is deliberately not planned.

Peer worker recovery is complete: serving acceptance/admission/dispatch/observations are transactional redb state, fixed workers have truthful entry/restart/shutdown boundaries, artifact transfer uses core publication/read authority, and loopback process execution survives disconnect/reconnect generation replacement.

Public-surface and semantic-change-cone contraction is complete: current consumers define the exported adapter/application surface, internal storage records are not re-exported by transport adapters, large lifecycle/read-model and test-builder ownership is separated, and every retained lint exception records its reviewed semantic boundary.

The independently audited headless dogfood vertical is complete: bounded Markdown prompt sequences
compile into ordinary workflow revisions; fresh coding processes share an explicitly authorized
persistent repository; verification, review, approval, prospective remediation, reconciliation,
restart recovery, and historical provenance all use the existing daemon/control/CLI path. The
headless command/read model is now the stable prerequisite on which the first graphical client can
be designed.

1. **Iced control center** — A thin native canvas over `milkdrift-control-client` with revision
   history, a virtualized timeline, peer/catalog provenance, inspector, schema-1 layout documents,
   live controls, and authenticated-cursor reconnect/resume behavior. It must remain a client of the
   existing protocol-2.1 command/read plane and introduce no workflow semantics.
2. **Optional distributed dogfood and continuous supervision** — An operator-configured two-daemon
   prompt stage, explicit checkpoint capability, and daemon-owned bounded controller lifecycle for
   authorized AI supervision; these must reuse current peer, proposal, authority, and repeat
   boundaries rather than introduce autonomous privilege.

# Roadmap

This document owns ordered unfinished product slices. Each slice is an architecture-complete ownership boundary.

Execution and control/read authorization are complete in the current core: one exact immutable grant basis governs run entry, local commands, information-bearing queries, pages, streams, artifacts, layouts, capability/provider views, and peer operations. The remaining slices build on that single authority boundary rather than introducing role or transport privilege.

Daemon application persistence ownership is complete: external receipts use bounded transactional hot ownership plus transparent exact-replay cold storage, layouts and proposal discovery remain correct across receipt movement, security audit is independently bounded, health reports redacted archival facts, and legacy sidecar/older physical state is refused. Physical cold history still requires explicit offline store-generation rotation when disk policy demands it; automatic destructive rotation is deliberately not planned.

Peer worker recovery and sustainable execution retention are complete: serving acceptance/admission/dispatch/hot observations are transactional redb state, fixed workers have truthful entry/restart/shutdown boundaries, terminal detail compacts atomically into permanent replay/conflict tombstones, retention health/configuration are independent from application receipts, artifact transfer uses core publication/read authority, and repeated loopback process turnover still admits work after both daemons restart. Physical tombstone deletion remains an explicit store-generation rotation concern rather than an online semantic expiry policy.

Capability authority selector closure is complete: wildcard access is an explicit `Any` value, exact allowlists are bounded nonempty `Only` values, whole-scope denial is explicit, matching and containment share one selector algebra, and runtime, control, capability-host, peer, preset, and daemon configuration paths no longer infer privilege from an empty collection. Legacy ambiguous grants/configuration and stores are deliberately refused rather than widened.

Public-surface and semantic-change-cone contraction is complete: current consumers define the exported adapter/application surface, internal storage records are not re-exported by transport adapters, large lifecycle/read-model and test-builder ownership is separated, and every retained lint exception records its reviewed semantic boundary.

The independently audited headless dogfood vertical is complete: bounded Markdown prompt sequences
compile into ordinary workflow revisions; fresh coding processes share an explicitly authorized
persistent repository; verification, review, approval, prospective remediation, reconciliation,
restart recovery, and historical provenance all use the existing daemon/control/CLI path. The
headless command/read model is now the stable prerequisite on which the first graphical client can
be designed.

Before graphical-client implementation begins, the remaining pre-UI closure sequence must prove
real process/model interoperability, collect the required mutation/performance/operational
evidence, finish public API and agent documentation contraction, and pass an independent closure
review. Those passes must reuse the explicit authority boundary and may not introduce an autonomous
privilege path. The daemon-owned bounded controller lifecycle is complete.

1. **Iced control center** — A thin native canvas over `milkdrift-control-client` with revision
   history, a virtualized timeline, peer/catalog provenance, inspector, schema-1 layout documents,
   live controls, and authenticated-cursor reconnect/resume behavior. It must remain a client of the
   existing protocol-2.2 command/read plane and introduce no workflow semantics.
2. **Optional distributed dogfood and continuous supervision** — An operator-configured two-daemon
   prompt stage and explicit checkpoint capability for authorized AI supervision; these must reuse
   the current controller, peer, proposal, authority, and repeat boundaries rather than introduce
   autonomous privilege.

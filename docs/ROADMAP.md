# Roadmap

This document owns ordered unfinished product slices. Each slice is an architecture-complete ownership boundary.

1. **Local process execution** — A production process adapter and bounded effect host with argument-vector safety, environment and secret mediation, workspace materialization, output and artifact bounds, cancellation and process-tree cleanup, restart uncertainty, and complete tests.
2. **Causal context and model endpoints** — Graph-ancestry context policy, artifact selection and budgets, a provider-neutral model-task contract, an OpenAI-compatible/local-server adapter, at least one independently mapped provider family, and honest streaming, tool, schema, and cancellation contracts.
3. **AI workflow control** — Observer, Advisor, Supervisor, Controller, and Autonomous authority presets; structured untrusted proposals; approvals; prospective mutations; a continuous bounded controller pattern; and the same command path used by humans.
4. **Daemon, control API, and CLI** — One authoritative host, a bounded blocking owner for redb, versioned commands, queries, and event streams, separately versioned status, timeline, and inspector read models, and a thin automation client.
5. **Peer capabilities** — A transport-neutral authenticated protocol for advertisement, remote invocation, artifacts, leases, cancellation, reconnect, and uncertainty, plus one reference transport over user-provided connectivity.
6. **Iced control center** — A thin native canvas with revision history, a virtualized timeline, inspector, layout documents, live controls, and reconnect/resume behavior.
7. **Dogfood and continuous remediation** — Imported prompt sequences, fresh coding-agent sessions, verification gates, live reviewer/remediation insertion, authorized AI supervision, and historical provenance inspection.

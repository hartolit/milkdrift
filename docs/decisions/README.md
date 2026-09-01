# Architecture decision records

An ADR records a durable choice that changes a product boundary, compatibility promise, ownership rule, security model, or hard-to-reverse dependency direction. It should state context, the decision, rejected alternatives, consequences, and objective triggers for reconsideration.

Do not write an ADR for a local refactor, a choice obvious from a small piece of code, a status update, or a decision that merely restates the implementation. Current facts belong in `docs/STATUS.md`; intended ordering belongs in `docs/ROADMAP.md`.

- [0001 — Rebirth as a durable workflow system](0001-rebirth-as-durable-workflow-system.md)
- [0002 — Append-only run events and deterministic projections](0002-append-only-run-events-and-deterministic-projections.md)
- [0003 — Redb transactions and content-addressed artifact ownership](0003-redb-transactions-and-content-addressed-artifacts.md)
- [0004 — Truthful side effects, retries, and uncertain outcomes](0004-side-effects-retries-and-uncertain-outcomes.md)
- [0005 — Prospective immutable revision reconciliation](0005-prospective-revision-reconciliation.md)
- [0006 — Shared contract mechanics without shared domain meaning](0006-shared-contract-mechanics.md)
- [0007 — Scoped authority and one human/AI command path](0007-scoped-authority-and-shared-command-path.md)
- [0008 — Generation-safe live capability hosting](0008-generation-safe-capability-hosting.md)
- [0009 — Direct argv and owned process groups](0009-direct-argv-and-owned-process-groups.md)
- [0010 — Host-owned materialization and artifact publication](0010-host-owned-materialization.md)
- [0011 — Exact causal context manifests](0011-causal-context-manifests.md)
- [0012 — Provider-neutral model contracts with explicit endpoint mappings](0012-provider-neutral-model-endpoints.md)
- [0013 — Proposals create immutable prospective revisions](0013-immutable-proposal-revisions.md)
- [0014 — Human and AI workflow control share one application path](0014-shared-human-ai-control-path.md)
- [0015 — One daemon and one bounded runtime owner](0015-single-daemon-runtime-owner.md)
- [0016 — External clients consume projected read models](0016-external-control-read-models.md)
- [0017 — Layout is outside semantic revision identity](0017-layout-outside-semantic-identity.md)
- [0018 — Peer execution uses durable idempotency and truthful uncertainty](0018-peer-idempotency-and-uncertainty.md)
- [0019 — Runs freeze authority and record exact per-entry decisions](0019-frozen-execution-authority.md)
- [0020 — One authorized command and read plane](0020-one-authorized-control-and-read-plane.md)
- [0021 — Byte-pinned identity for trusted host processes](0021-byte-pinned-trusted-host-processes.md)
- [0022 — Redb owns durable daemon application state](0022-redb-owned-daemon-application-state.md)
- [0023 — Exact application replay with bounded hot receipts](0023-exact-application-replay-with-bounded-hot-receipts.md)
- [0024 — Bounded hot peer history and permanent compact tombstones](0024-peer-execution-hot-retention-and-tombstones.md)
- [0025 — Explicit capability authority selectors](0025-explicit-capability-authority-selectors.md)
- [0026 — Controllers use one durable bounded lifecycle](0026-durable-bounded-controller-lifecycle.md)
- [0027 — Production continuous controllers require final-entry resource reservations](0027-controller-final-entry-reservations.md)
- [0028 — TOML daemon configuration compiles into narrow plans](0028-toml-compiled-daemon-configuration.md)

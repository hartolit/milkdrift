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

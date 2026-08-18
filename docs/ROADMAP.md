# Roadmap

This document owns ordered unfinished product slices.

1. Build the durable runtime/persistence/reconciliation boundary: an append-only event journal, recoverable projections, scheduler state machine, effect outbox, structured concurrency, and prospective revision reconciliation.
2. Add the live capability registry, constraint resolution, leases, executor lifecycle, and the first process/model adapters without weakening capability contracts.
3. Add branch-local workspaces, causal context selection, artifact retention, secrets policy integration, and audited side-effect mediation.
4. Add peer capability advertisement and invocation over pluggable user-provided connectivity.
5. Add a daemon that owns durable state and thin CLI and Iced clients for authoring, operation, and inspection.
6. Add bounded workflow-control capabilities for authorized AI controllers, using the same commands, proposals, and approvals as humans.

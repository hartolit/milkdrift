# Pass 5 — Complete the thin CLI as Milkdrift’s headless product surface

Use this prompt with `00-shared-execution-contract.md`. Run it after Pass 4 so the CLI already has coherent private command-family structure.

## Objective

Make the existing `milkdrift` CLI a comprehensive, automation-safe operator and dogfood client for the current daemon/control protocol without giving it semantic or storage ownership.

The CLI should be broad in functionality and thin in meaning. It must let a human or coding agent validate/import definitions, operate runs, inspect evidence, manage proposals, resolve uncertain work, and verify restart behavior entirely through the same external control path a future GUI would consume.

Do not build a GUI, TUI, interactive canvas, REPL, workflow-specific storage, or second API.

## 1. Establish a protocol-to-CLI parity inventory

At the current head, enumerate:

- every `milkdrift-control-protocol::Command` variant;
- every intentional read route/read model exposed by the daemon;
- every `ControlClient` operation;
- every current CLI command.

Classify each protocol/client operation as:

- intentionally exposed by CLI;
- deliberately not an operator operation, with an existing documented reason;
- missing and required by this pass.

Do not expose internal daemon administration merely for numeric parity. `status.md` currently says public configuration, audit, shutdown, and local artifact upload routes are absent; do not invent those routes here.

Known missing operator paths in the 2026-09-01 checkout include:

- `ValidateBlueprint` despite protocol support;
- `ResolveWork` despite protocol support for query, retry, compensation, retention, and evidence-based resolution.

Search again after preceding passes and close every legitimate current gap.

## 2. Complete canonical document handling

The CLI must make the canonical document workflow practical without inventing a second graph language.

Required behavior:

- `blueprint validate FILE|-` validates through the daemon command path without storing;
- `blueprint import FILE|-` imports the exact versioned document;
- blueprint inspection can emit or write the exact stored document without presentation wrapping when explicitly requested, using create-new output semantics and canonical bytes from the owning document contract;
- prompt-sequence validate/import retain JSON/Markdown support through `milkdrift-prompt-sequence`;
- proposal submit and layout put accept the canonical versioned document;
- all file/stdin reads are bounded before allocation according to the owning protocol/document ceiling;
- duplicate JSON keys and hostile structure remain rejected by the canonical reader;
- `-` means one bounded stdin document and cannot be combined with another stdin-consuming option;
- no raw shell command, inline secret, unbounded JSON argument, or hidden path discovery is introduced.

Do not implement node-by-node editing commands, a private CLI graph model, or an alternative workflow DSL. A coding agent can generate a canonical document, validate it, inspect diagnostics, and import it. Future graphical authoring remains a consumer concern.

## 3. Expose retained/uncertain work resolution

Add one clear command family—preferably under the exact attempt/external-work identity—that maps directly to `ResolveWork`.

It must require:

- run identity;
- attempt identity;
- decision identity;
- explicit action;
- optional remediation node only where the selected action permits it;
- exact optimistic guards supported by the protocol;
- explicit confirmation for actions that may retry, compensate, or force evidence-based terminal resolution.

The CLI must not decide whether retry, compensation, query, or forced resolution is legal. It constructs the typed command and renders the daemon’s authorized result.

Names and help text must preserve the distinction between:

- query external truth;
- retry under runtime idempotency policy;
- compensate;
- retain uncertainty;
- resolve succeeded from evidence;
- resolve failed from evidence.

Never call cancellation acknowledgement “termination,” and never describe retry as safe before the daemon accepts it.

## 4. Expose complete command-envelope controls

The CLI currently constructs command envelopes but leaves some fields inaccessible. Add bounded, validated operator inputs for current envelope facts that have legitimate use:

- exact `--command-id`;
- `--expected-sequence`;
- `--expected-revision`;
- repeatable external evidence references using one unambiguous syntax or a bounded versioned file;
- bounded reason.

Generated command IDs may remain a human convenience, but noninteractive examples and dogfood tests must use explicit stable identities so replay/conflict behavior is reproducible.

Do not permit actor, grant, decision outcome, timestamp, or authority facts to be supplied by the CLI when the daemon owns them.

## 5. Make JSON mode genuinely automation-safe

The CLI is intended for coding-agent use. `--json` must form a stable machine contract for both success and failure.

Required properties:

1. Every success is one compact JSON document; streamed observations are JSON Lines with one complete document per line.
2. Every failure in JSON mode emits one bounded structured error document, not an English-only stderr line. Include stable CLI classification, public daemon error code when available, retryability, and safe bounded detail. Never serialize credentials, authorization headers, raw provider bodies, prompts, or internal errors containing sensitive values.
3. Parse/configuration failures, public API failures, conflicts, overload/unavailability, not-found, failed-run terminal, and internal client failures remain distinguishable through stable exit codes.
4. JSON mode never prompts. High-risk commands require `--yes` or fail before submission.
5. Human mode may remain readable, but it must use the same underlying result/error classification.
6. No ANSI/control sequences appear in JSON output.
7. Help/version behavior remains conventional and does not require daemon credentials.
8. Safe query retries remain owned by `ControlClient`; commands are never implicitly replayed by presentation code.
9. Pagination stays explicit. Do not silently auto-drain feeds.
10. Stream reconnect continues from authenticated cursors and emits a typed status event rather than corrupting JSON output.

Version the CLI output schema only if its serialized contract changes. Preserve readers/tests for the supported prior schema only when explicitly promised; do not create indefinite compatibility aliases.

## 6. Keep the CLI semantically thin

The CLI may use canonical constructors and document builders from their owning libraries. It must not reimplement:

- blueprint validation or fingerprints;
- proposal risk/reconciliation policy;
- authority evaluation;
- run state transitions;
- retry/idempotency policy;
- uncertainty classification;
- context selection;
- capability resolution;
- persistence.

Review existing client-side conveniences such as prompt-sequence stage filtering and remediation construction. Move reusable semantic derivation into the owning library when it is currently duplicated or inferred from naming conventions, then keep the CLI as a caller. Do not preserve brittle parsing such as “node ID starts with this prefix” when the owning read model can expose a typed association.

Do not make `apps/cli` a general-purpose library merely to test private presentation code. Extract a small internal library only if both the binary and a real external test/evidence consumer require the same behavior and the public API policy is satisfied.

## 7. Add black-box CLI dogfood evidence

Add a shell-free, deterministic black-box scenario using the actual daemon and CLI executables. Place development-only orchestration in the existing evidence/test ownership rather than a new product crate.

The scenario must use temporary storage/artifact roots, a private temporary bearer file, an ephemeral loopback daemon, explicit command IDs, and deterministic current adapters. It must execute through HTTP/control protocol rather than calling daemon internals.

At minimum prove:

1. readiness and authority inspection;
2. invalid blueprint validation fails without storage;
3. valid blueprint validation then import;
4. exact replay of import/start command IDs and conflict on changed payload;
5. run start and bounded timeline/read inspection;
6. node and attempt inspection;
7. pause/signal/resume or the appropriate current wait path;
8. proposal submit/approve/apply with exact revision guards;
9. one uncertain/retained-work resolution path through the new CLI command;
10. daemon shutdown/restart through the test owner, followed by the same durable reads;
11. artifact metadata/download with create-new and digest/size verification;
12. stable JSON documents and exit codes for success, invalid input, conflict, not-found, unauthorized, overload/unavailable, and failed terminal outcome;
13. no CLI process receives or opens the database path.

Do not add a public daemon shutdown route solely for this test. The harness owns the child process lifecycle.

Provide a concise maintained guide for running an equivalent headless workflow manually or from a coding agent. Reuse canonical examples rather than duplicating large schemas.

## 8. Evidence

Run:

- all CLI unit/integration tests;
- control-protocol and control-client suites;
- daemon control-plane and headless-dogfood suites;
- the new actual-binary scenario;
- repository-contract/public-API checks;
- the full local gate.

Use the CLI itself to perform at least one local headless workflow against the built daemon and include the exact commands in the final report. Do not claim real model interoperability in this pass.

## Scope exclusions

No GUI/TUI, direct storage access, new daemon admin routes, provider family, model manager, context redesign, graph-editing DSL, shell-execution shortcut, or bypass of authority/proposals/reconciliation.

## Acceptance criteria

The pass is complete only when:

- every legitimate current control command/read has an intentional CLI disposition;
- blueprint validation and retained-work resolution are exposed;
- canonical file/stdin handling is bounded;
- command envelope guards/evidence are available;
- JSON success, stream, and failure output are stable and automation-safe;
- the actual CLI and daemon binaries pass one restart-durable black-box scenario;
- the CLI remains storage-free and semantically thin;
- focused and full local gates pass.

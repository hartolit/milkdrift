# ADR 0044: Public workflow authoring over existing mutations

Status: accepted.

## Problem

Ordinary model workflow authors previously needed a complete graph and locally derived revision
identities. A future client needs the same construction and refusal behavior as the CLI, while
unfinished local edits must never become runnable definitions by accident.

## Decision

Control protocol 2.12 adds `author_blueprint` and `construct_blueprint` to the existing command
family. A client retains a workflow identity, exact optional base revision, and pending blueprint
mutations. The daemon loads an authorized base and invokes blueprint genesis/revise using the
authenticated actor as author. The envelope revision guard must equal the draft base. Existing
revision storage and command receipts remain the durable owners; there is no mutable workflow
head, draft database, or new execution language.

`author_blueprint` implements ordinary model-step, prompt, input, connection, order and output
edits. Construction uses the existing model request, blueprint binding, and result acceptance
owners. Every model selection is explicit and rechecked against the caller's filtered catalogue.
Each model step has a `ModelProse` check; rejection enters a signal hold whose unchanged continuation
fails. An output is exposed only on the successful route. Task context requests only direct inputs.
Model and acceptance requirements retain the selected catalogue's category, trust, placement and
side-effect constraints; pinning identity alone would leave those authority dimensions unrestricted.
Per-run value submission and their provider materialization remain separately owned execution work.

The editor recognizes its supported shape by reconstructing the entire semantic definition and
requiring exact equality. Richer imported definitions refuse before editing or storage. Advanced
clients can submit existing mutations through `construct_blueprint`; it validates the complete
candidate without restricting it to the model editor's shape. Offline `blueprint create` and
`govern` remain for the existing governed-method bootstrap before a daemon is configured.

[ADR 0050](0050-editor-edge-identities.md) defines framed identities for new connections and exact
recognition of retained IDs. Unchanged connections preserve their IDs; this keeps old workflows
editable and no-op saves unchanged without weakening the full semantic comparison.

The CLI draft format is version 1. It stores only the base and mutations, with strict bounded
reading. Edits hold an OS file lock, check unchanged bytes, and publish a synced temporary file
by atomic replacement. Create/open refuses an existing destination. An optional edit token guards
an operator's previously inspected state. Separate files based on an older immutable revision may
form explicit branches; no implicit latest-version pointer exists. Saving an unchanged draft
returns its base exactly. Saving changes creates a child and never changes a run.

## Compatibility and evidence

Control protocol 2.17 extends ordinary edits with atomic declared-input rename and unused-input
removal. It retains the same draft, mutation, storage and receipt owners; earlier control versions
remain explicitly refused. Rename updates only workflow-input bindings. Removal never implicitly
disconnects a consumer or deletes an uploaded artifact.

Protocol 2.18 adds output clearing and existing-step output-limit editing through the same owner.
An incomplete editor graph is a retained client draft; the existing complete-save check stays in
place. Model request construction owns the unit range, and existing adapter preparation and
accounting derive admission from that exact request. No separate editable reservation is stored.

Clients and daemons upgrade together; earlier control minor versions are refused. Blueprint,
model, result-acceptance and storage formats do not change. Public authoring tests cover independent
workflows, exact reopen/replay, parent preservation, invalid connections/order, missing or hidden
model choices, stale guards and unsupported edits. CLI file tests cover changed bytes, locks,
symlinks and interrupted temporary writes. Focused phase checks do not establish the sprint's
integrated supplied-input or real-model acceptance.

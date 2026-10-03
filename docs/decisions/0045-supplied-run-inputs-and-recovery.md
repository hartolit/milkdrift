# ADR 0045: Supplied run inputs and exact client recovery

Status: accepted.

## Problem

A reusable definition needs different data on each run. A client that exits before receiving the
start reply must recover accepted work without rereading changed files, inventing another run, or
accidentally sending its request to another installation or caller.

## Decision

Control protocol 2.13 adds named immutable artifact references to `StartRun`. Ordinary file inputs
use the existing authenticated upload owner; the start adapter verifies artifact content access,
integrity and bounds before adapting references into initial workspace values. Runtime `CreateRun`
remains the shared owner of declared/required names, initial-value identity and workspace budgets.
Ordinary supplied fields use the artifact-reference v1 contract. Published choice and artifact
contracts retain their existing mapping and their distinct caller/service permissions.

The model adapter materializes manifest-selected direct inputs through the existing invocation
data-access port. The frozen manifest supplies source labels and digest/byte facts; the adapter
maps supported text/JSON/image content through the same provider context path as selected history.
The model task document is interpreted once as instructions, not repeated as evidence. Unsupported
media fail before provider entry. No client assembles a prompt from files or implements context
selection.

The shared client prepares a schema-1 `SavedRunRequest` containing the complete start envelope and
authenticated host, actor and grant observation. The CLI requires an explicit new destination,
atomically retains it before submission, and can reconnect using that file. Records contain artifact
identities, not credentials or source file bytes. There is no automatic client archive: the operator
keeps each record while recovery is needed and deliberately removes it afterward. Unix files are
created with mode 0600, and unsafe or corrupt records are refused on read.

Replay checks the current installation and caller/grant before resending the exact request. Existing
application and runtime receipts recover the separate create/start commits and the gap before the
outer reply. A changed canonical request conflicts. Reconnect does not select a model: the existing
runtime dispatch boundary still freezes that selection. Observation is optional and separately
bounded; stopping observation does not request cancellation or resolve uncertain effects.

## Compatibility and evidence

The control protocol requires a coordinated client/daemon upgrade. Empty-input commands keep their
existing command-body encoding, while older protocol envelopes remain unsupported. Direct invocation
documents and their supported replay behavior are unchanged. Blueprint, workspace, model and durable
journal versions do not change. The client recovery document is not execution history.

Focused daemon/client tests exercise two briefs on one revision, provider-visible direct inputs and
draft isolation, authority/refusal paths, immutable request files, replay/conflict and the three
create/start interruption boundaries. Actual CLI/daemon processes establish recovery across client
and daemon exit. The sprint's final acceptance remains separately owned by phase 06.

# ADR 0016: External clients consume projected read models

- Status: accepted
- Date: 2026-08-26

## Context

Internal run events, redb keys, snapshot payloads, provider responses, adapter failures, and process
receipts evolve for their owning storage/domain needs. Serializing them directly would make every
CLI/UI client a persistence reader, leak sensitive or high-volume payloads, and prevent independent
wire compatibility and authority filtering.

## Decision

`milkdrift-control-protocol` owns pure protocol 1.0 commands, errors, pages, feed cursors,
observations, and read models. The daemon projects immutable revision summaries/diffs, compact
runs/nodes/attempts, proposal status, bounded timeline categories, capabilities, current authority,
artifact metadata, and health. Timeline entries retain exact durable sequence/provenance anchors
but never expose an internal event discriminant or complete event body.

Queries are page- and byte-bounded. Opaque cursor schema 1 binds a continuation to one exact feed.
Run SSE interleaves projected timeline and compact status positions; capability health uses a
bounded retained observation window; daemon health is coarse and in-process. Heartbeats are
transport comments. Establishment and polling recheck authentication, and an invalid/old/wrong-feed
cursor produces an explicit error or resync observation.

`milkdrift-control-client` is the only HTTP/SSE mapping used by the CLI and future Iced client.
Safe queries may follow a bounded retry policy. Mutation retry is caller-explicit and must reuse the
exact durable idempotency identity and body.

## Rejected alternatives

- Publicly serialize `RunEventKind` or redb rows, because internal compatibility, authority, and
  wire compatibility have different owners.
- Auto-load complete timelines, because run lifetime is unbounded.
- A global prompt/output firehose, because it defeats authority filtering and bounded consumption.
- WebSockets for this slice, because control remains request/response and observations are
  server-to-client; SSE has sufficient resume semantics.

## Consequences

Internal event schemas may evolve without forcing clients to understand every durable checkpoint.
External DTO changes require protocol review and fixtures. Artifact bytes remain a separate
authority-checked range contract. Streams may ask a client to resynchronize instead of retaining
unbounded buffers.

## Reconsideration triggers

Add bidirectional streaming only when a reviewed operation cannot be represented as an idempotent
command plus resumable observations. Add optimized historical indexes only as derived or
transactionally anchored owners, never by exposing storage rows.

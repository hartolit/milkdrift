# 0048 — Prepare independent calls in the serving owner

Status: accepted.

## Decision

Control protocol 2.16 exposes `POST /v1/invocations/prepare` for explicit host, request identity,
capability, operation, inputs and optional limits. The existing capability-host serving owner
returns a complete direct invocation request after current discovery and authority checks. It
derives the exact snapshot, provider profile, required idempotency key and deadline. The CLI
retains local file/upload handling and saves the returned document before submission; it no
longer constructs these product facts. The same route supports direct and published calls while
preserving their separate input contracts, execution owners and disclosure rules.

The server clock supplies the deadline. Absent limits use the discovered serving ceilings;
explicit limits are validated without clamping. Existing admission checks enforce caller authority,
input scope and adapter duration. Adapter preparation still validates capability-specific content
before entry. No generation permit, execution record or worker is reserved by this operation.

## Consequences

A browser client can prepare a call with ordinary JSON without implementing Rust snapshot/hash
rules or trusting its own wall clock. Preparation is not acceptance and cannot recover an earlier
submission. The client must retain and replay the exact returned document or look up its request;
preparing again may return a new catalog or deadline and must not silently replace accepted facts.

The control API requires coordinated client/daemon upgrade. Peer transport and accepted invocation,
blueprint, journal and receipt formats are unchanged. Public refusal tests cover host, capability,
operation, limits and inputs; existing direct and published execution tests retain replay and
disclosure coverage. This exposes an independent client boundary, not browser authentication or CORS.

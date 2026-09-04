# ADR 0012: Provider-neutral model contracts with explicit endpoint mappings

- Status: accepted
- Date: 2026-08-26

## Context

Model endpoints share useful task concepts but differ in roles, content blocks, tools, structured
output, streaming events, usage, finish reasons, auth, and sessions. Making workflow semantics
OpenAI-shaped or allowing an adapter to silently discard unsupported features would make durable
intent depend on one vendor.

## Decision

`milkdrift-model` is a pure inward schema-v1 contract for ordered messages, bounded content
references, tools, structured output, session selection, reasoning controls, responses, tool-call
data, finish reasons, usage, metadata, and exact context manifests. It has no HTTP, runtime, redb,
secret, provider SDK, or model-identity dependency.

`milkdrift-model-provider` is one outer capability adapter. Separate
`openai_compatible` and `anthropic` modules map each protocol independently; `http` and `stream`
share only bounded transport mechanics. An exact non-secret endpoint profile selects protocol,
model alias, base URL, secret reference/no-auth, bounds, redirect/TLS/proxy policy, concurrency,
trust/locality, advertised features, and bounded namespaced options. Feature negotiation happens
before external entry over the complete materialized wire request, including the adapter-injected
system manifest and selected context images. Any unmapped role, part, option, session, or control is
rejected before HTTP.

The HTTP implementation uses reqwest with rustls roots, disables decompression by feature choice,
defaults to no ambient proxy and no redirects, rejects cross-origin redirects even when same-origin
redirects are enabled, requires HTTPS remotely, and allows plaintext only on explicit loopback
development profiles. Secrets exist only during header construction. Streaming fragments are
bounded observations; canonical response/text/structured/tool-call/provider-metadata outputs are
committed artifacts before success. Tool calls remain data.

The direct dependency is reqwest 0.12 with only blocking, JSON, rustls TLS, and stream features.
Its rustls chain introduces the permissive ISC, BSD-3-Clause, and CDLA-Permissive-2.0 licenses;
these are explicitly admitted by dependency policy. No provider SDK is used.

## Rejected alternatives

- Provider identity in blueprints, because selection belongs to capability resolution/profile.
- One universal provider wire shape, because Anthropic's native event and content model is
  materially different.
- Hidden adapter conversations, because retries and inspection would depend on mutable state.
- Provider SDKs as orchestration/authority layers, because endpoint and feature policy must remain
  under Milkdrift ownership.

## Consequences

Adding a protocol means adding an explicit mapping and feature matrix, not changing runtime
semantics. Fresh requests are stateless and reproducible: after exact-attempt verification, both
protocol mappings receive the same canonical context manifest as their first system block. Exact
Milkdrift continuation remains a
provider-neutral contract but both current endpoint mappings reject it, along with provider-managed
sessions, until a protocol/profile maps it explicitly. Blocking HTTP cancellation
is cooperative: acknowledgement never claims remote termination, and the configured conservative
timeout closes a stalled read. Because the current endpoint protocols provide neither a stable
provider idempotency key nor a result-query contract, `model.generate` advertises unknown side
effects, unsupported idempotency, and best-effort cancellation. A malformed/truncated or bounded
response failure after entry, response loss, timeout, or cancellation therefore reports retained
uncertainty and never commits a partial response as success. A complete response settles the local
attempt but does not retroactively claim the provider had no other effect.

Endpoint-profile schema 1 is unchanged: these facts are derived adapter operation semantics, not
operator claims. Exact resolved snapshots already persisted before this correction retain their
original bytes/digest. New registration against a conflicting current descriptor fails exact
snapshot validation instead of silently reinterpreting historical work.

## Reconsideration triggers

Adopt a maintained multi-provider library only if custom endpoints, secret mediation, strict
feature negotiation, bounded raw diagnostics, cancellation truthfulness, and independent provider
mappings remain observable and controlled here.

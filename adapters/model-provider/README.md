# Model endpoint adapter

Provider completion does not establish a usable workflow result. A length-limited response and a
tool-only response remain valid generation observations. Workflows requiring a complete review
must use [result acceptance](../../docs/guides/result-acceptance.md) before continuing. It preserves
the raw response, finish reason, usage, and selected context while evaluating the declared output
shape separately. Generic model tasks acquire no review-specific policy.

This adapter sends a provider-neutral model task to an explicitly configured endpoint and publishes
the response as workflow artifacts. It supports OpenAI-compatible chat completions and native
Anthropic Messages mappings. Milkdrift does not load a model or discover providers here.

## Set up a model task

Use the [local model endpoint guide](../../docs/guides/local-model-endpoint.md) for a supported
operator scenario. A task supplies a `ModelTaskRequestDocument` under `MODEL_TASK_INPUT_NAME` and
requires `MODEL_GENERATE_OPERATION`; the [model crate](../../crates/model/README.md) owns those
request types. Runtime records a frozen context manifest before the task reaches this adapter.

`EndpointProfile::from_json` reads the non-secret endpoint/model choice, protocol mapping, features,
transport policy, and limits. The daemon creates `ModelEndpointAdapter` and registers the matching
`descriptor_for_profile` result in its [capability construction](../../apps/daemon/src/host/capabilities.rs).
Keep the same capability identity and profile in both calls. Profile changes require a new generation;
health describes local adapter readiness and active load without probing endpoint availability.

The adapter checks manifest provenance against the exact attempt, reads selected content through
the host data port, and verifies its size/digest. Both mappings inject the manifest as system context
and label additional evidence as untrusted data. Direct inputs are verified against the manifest;
they are not all appended a second time as evidence messages. `SystemRole` must therefore be
advertised even when the task's own messages contain only a user role.

## Choose features the endpoint actually supports

Negotiation checks the task and injected context against the profile before HTTP entry. The mappings
then reject features they cannot encode rather than silently dropping them.

| Request feature | OpenAI-compatible chat | Native Anthropic mapping |
| --- | --- | --- |
| Text, system context, tools, images, streaming | Available when advertised | Available when advertised |
| Developer role | Available when advertised | Refused |
| Structured JSON output | Sends the requested schema; parses returned JSON | Refused |
| Reasoning | Maps effort when advertised; reasoning-unit budget refused | Refused |
| Generic files or managed sessions | Refused | Refused |

These are Milkdrift mapping choices, not claims about every endpoint implementing either API.
Both send the requested output allowance as `max_tokens`; the model contract's upper limit does not
prove that a particular server accepts that allowance. Provider options use the mapping's explicit
extension namespace and cannot overwrite already emitted request fields.

Only `ModelTaskRequest::session() == Fresh` is accepted. Runtime first compares the request with the
governing blueprint declaration when claiming the invocation, including inline/artifact requests,
retries, and recovered leases. Matching continuation still fails this adapter's protocol check;
neither boundary replaces continuation with a fresh session. Standalone adapter callers supply
their own governing-policy enforcement because this adapter has no workflow revision store.

## Observe a result or a lost response

HTTP enforces request/response, header, SSE line/event, and reported-fragment bounds. The current
blocking client uses the smaller of request and idle timeouts as a whole-request deadline; activity
does not reset an independent idle timer. Remote endpoints require HTTPS, plaintext is limited to
explicit loopback development, redirects default to denied, and proxy discovery is opt-in.

Streaming fragments become bounded progress observations. Provider parsers accumulate text and tool
arguments and require their completion markers before publishing response artifacts. On success,
`model_response` contains the canonical response; available final text, structured JSON, tool calls,
and provider metadata get separate artifacts. Tool calls are data, not automatically executed work.
Missing usage stays unknown, and a successful workflow response can still contain no useful final text.

Cancellation signals a local flag and SSE reading checks it between reads. It cannot prove that
remote computation stopped; non-streaming reads remain bounded by the HTTP timeout. A lost reply,
truncated/malformed stream, or post-entry timeout retains uncertainty without publishing successful
partial output. The descriptor advertises unsupported idempotency and unknown effects, so an HTTP
retry hint alone does not authorize another model request.

For implementation work, [`adapter.rs`](src/adapter.rs) owns negotiation, context consumption, and
publication; [`http.rs`](src/http.rs) and [`stream.rs`](src/stream.rs) own transport/framing bounds.
The two mapping modules own provider-specific request and completion meaning. The
[mock endpoint suite](tests/mock_endpoints.rs) checks both mappings, context, cancellation, and
uncertainty without contacting a provider. The `operational-evidence` feature also exercises fixed
parser fixtures without a network. Neither qualifies real-provider interoperability; that evidence
has its own [status](../../docs/product/status.md#current-validationevidence-snapshot).

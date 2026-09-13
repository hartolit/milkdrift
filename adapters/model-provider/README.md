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
Anthropic and unqualified OpenAI-compatible profiles send `max_tokens`. A qualified text profile
explicitly selects `max_tokens` or `max_completion_tokens`; its server contract must cover all
generated tokens, including reasoning. Provider options use the mapping's explicit
extension namespace and cannot overwrite already emitted request fields.

## Reserve and settle supported model usage

Endpoint schema 2 requires explicit `billing` and `token_limits`. `unknown` preserves ordinary
execution but cannot satisfy a controlled request's applicable unknown allowance. No reader
turns a loopback URL, missing field, or old profile into free service. An operator's `unbilled`
declaration makes provider charge non-applicable; tokens, artifacts and call counts still apply.

The `byte_bpe` contract supports fresh system/user/developer text on OpenAI-compatible chat.
After context injection, preparation counts the entire encoded UTF-8 body plus bounded template
overhead. Byte BPE can only merge its initial byte tokens. The operator must establish that
normalization does not expand input, the exact template stays within its declared overhead,
the server reports logical prompt tokens once, and it caps one complete generation even if disconnected.
The configured input ceiling is a local refusal threshold, not proof from a context-window size.
Tools, images, assistant history, extra choices, reasoning controls, structured output and other
provider extensions are refused for this accounting path. Other ordinary mappings keep their
existing feature rules.

Prepared envelopes explicitly name `model_tokens`. The shared account refuses unspecified units
instead of adding bytes or other provider quantities to tokens. Input means the complete submitted
prompt after templating; output includes every generated token, including reasoning. Re-evaluating
a cache during context shifting is computation, not another submitted prompt. This allowance does
not measure GPU work. A billed contract is unsupported if that work adds charges outside its tariff.

`text_tariff` declares exact currency and rates in currency millionths per million input, cached
input and output tokens. These must cover every charge. Reservation uses the larger input rate,
checked arithmetic and rounds the total upward to a currency millionth. Settlement uses observed
token categories and the same frozen tariff. Different cached rates require cached-token evidence.
Missing applicable usage, overflow, contradictory totals or charge evidence remain unresolved.
Nullable usage details remain unknown. Both response modes retain the raw usage breakdown;
streamed usage is a final aggregate, so a later packet cannot replace an earlier non-null report.
An unbilled response reporting a nonzero charge is uncertain, not silently free.
Conflicts retain the raw provider amount and mark the accounting basis `unresolved`; they never
label that amount as a calculated charge or release the reservation.

For a billed profile, the adapter also declares the maximum permitted call's charge to the
ordinary authority evaluator, rounding upward to its existing hundredth-currency permission unit.
A grant below that profile-wide ceiling cannot select the generation, even for a smaller request.
The cumulative account still reserves the exact prepared request in millionths; permission does
not reserve or spend allowance. Unbilled calls need no monetary permission amount.

For example, a fixture tariff of EUR 1 per million input tokens, EUR 0.50 per million cached input
tokens and EUR 2 per million output tokens is encoded as follows. These are illustrative rates;
an operator must replace them and the source with the approved service's complete tariff.

```json
{
  "type": "text_tariff",
  "currency": "EUR",
  "input_micros_per_million": 1000000,
  "cached_input_micros_per_million": 500000,
  "output_micros_per_million": 2000000,
  "source": "Illustrative text-only fixture tariff v1; no additional billable categories"
}
```

The frozen descriptor records the profile generation and contracts. The `model_response` artifact
keeps raw provider usage separately from `org.milkdrift/model-accounting`, which records the
prepared request digest, envelope, accounting basis and calculated charge. Terminal account usage
uses that basis. A tariff calculation is never presented as a provider invoice. A failed report or
lost response retains the existing reservation and uncertainty behavior; timeout and byte cutoff
do not establish that remote generation stopped. See the
[local setup guide](../../docs/guides/local-model-endpoint.md#controlled-local-text-requests).

Only `ModelTaskRequest::session() == Fresh` is accepted. Runtime first compares the request with the
governing blueprint declaration when claiming the invocation, including inline/artifact requests,
retries, and recovered leases. Matching continuation still fails this adapter's protocol check;
neither boundary replaces continuation with a fresh session. Standalone adapter callers supply
their own governing-policy enforcement because this adapter has no workflow revision store.

## Prepare once before external entry

The host acquires the exact generation permit, then calls the adapter's local `prepare` hook.
It verifies the task and frozen manifest, materializes bounded selected inputs, negotiates features,
encodes the complete HTTP body, and resolves authentication headers. This step never contacts the
provider. The returned one-shot handle retains those exact bytes and the profile's endpoint; entry
does not reload inputs or build a second request. Authentication values remain ephemeral and headers
are marked sensitive. No header or secret bytes enter stored proof or diagnostics.

Runtime checks authority before those reads and again after preparation. Its final transaction
binds the checked run head, entry intent, and any controller reservation. Revocation, a changed run,
an expired lease, or a missing generation cannot turn preparation into permission to send. Dropping
the handle releases its permit. Request equality binds all invocation inputs and the context reference,
generation, execution coordinates, and frozen authority; only a fresh decision for the same authority
request may replace the earlier entry decision.

| Observed boundary | Result |
| --- | --- |
| Local preparation refuses before entry intent | Durable rejected attempt, no provider request or account reservation, no automatic retry. |
| Entry intent exists; send or response completion is unproven | Uncertain until durable terminal evidence or authorized reconciliation resolves it. |
| Complete parsed response and durable terminal | Preserve provider success/failure, usage, and output semantics; result acceptance remains separate. |
| Complete response, followed by local artifact/publication failure | Failed terminal if reporting succeeds; `ResponseObservedFailure` otherwise preserves the stage while runtime retains uncertainty. |

A crash can occur between durable intent and network transmission. Missing send flags cannot prove
that nothing happened. Even a locally observed refusal cannot survive restart as negative proof if
its terminal commit failed. These effect stages preserve historical events. Controlled admission
also requires the supported accounting contract described above; unknown model usage bounds still
deny entry.

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
The HTTP client has automatic retries disabled; configured redirect policy remains independent.

For implementation work, [`adapter.rs`](src/adapter.rs) owns negotiation, context consumption, and
publication; [`http.rs`](src/http.rs) and [`stream.rs`](src/stream.rs) own transport/framing bounds.
The two mapping modules own provider-specific request and completion meaning. The
[mock endpoint suite](tests/mock_endpoints.rs) checks both mappings, context, cancellation, and
uncertainty without contacting a provider. The `operational-evidence` feature also exercises fixed
parser fixtures without a network. Neither qualifies real-provider interoperability; that evidence
has its own [status](../../docs/product/status.md#current-validationevidence-snapshot).

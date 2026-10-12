# Model endpoint adapter

Provider completion does not establish a usable workflow result. A length-limited response and a
tool-only response remain valid generation observations. Workflows requiring a complete review
must use [result acceptance](../../docs/guides/result-acceptance.md) before continuing. It preserves
the raw response, finish reason, usage, and selected context while evaluating the declared output
shape separately. Generic model tasks acquire no review-specific policy.

This adapter sends a provider-neutral model task to an explicitly configured endpoint and publishes
the response as ordinary artifacts. It supports OpenAI-compatible chat completions and native
Anthropic Messages mappings. Milkdrift does not load a model or discover providers here.

Task invocations carry their exact originating coordinates and frozen selected inputs, including
when served through a peer. Task and independent requests use `DirectInputSelection`: only the supplied
inputs and explicitly authorized artifact references, with no workflow-history lookup. Explicit
continuation and provider-managed sessions refuse before provider entry. The serving owner binds
output to its durable host invocation. Use the
[direct recipe](../../examples/operator/README.md#independent-execution-hosts) to upload and call through the CLI.

## Set up a model task

Use the [local model endpoint guide](../../docs/guides/local-model-endpoint.md) for a supported
operator scenario. A task supplies a `ModelTaskRequestDocument` under `MODEL_TASK_INPUT_NAME` and
requires `MODEL_GENERATE_OPERATION`; the [model crate](../../crates/model/README.md) owns those
request types. `TaskRuntime` builds ordinary fresh, nonstream requests from accepted instructions
and named inputs, then freezes the request and selected capability before adapter preparation.

`EndpointProfile::from_json` reads the non-secret endpoint/model choice, protocol mapping, features,
transport policy, and limits. The daemon creates `ModelEndpointAdapter` and registers the matching
`descriptor_for_profile` result in its [capability construction](../../apps/daemon/src/host/capabilities.rs).
Keep the same capability identity and profile in both calls. Profile changes require a new generation;
health describes local adapter readiness and active load without probing endpoint availability.

The adapter verifies that the frozen selection matches the exact invocation request, reads selected
content through the host data port, and verifies artifact size and digest. Both mappings inject the
selection's provenance as system context and label additional named evidence as untrusted data.
The model request itself is not appended again as an evidence message. Nested artifact reads must
belong to the same selection. `SystemRole` must therefore be advertised even when the task's own
messages contain only a user role.

## Choose features the endpoint actually supports

Negotiation checks the task and injected context against the profile before HTTP entry. The mappings
then reject features they cannot encode rather than silently dropping them.

| Request feature | OpenAI-compatible chat | Native Anthropic mapping |
| --- | --- | --- |
| Text, system context, tools, images, streaming | Available when advertised | Available when advertised |
| Developer role | Available when advertised | Refused |
| Structured JSON output | Sends the requested schema; parses returned JSON | Refused |
| Reasoning | Maps effort when advertised; reasoning-unit budget refused | Refused |
| Sessions | Fresh only; explicit continuation refused | Fresh only; explicit continuation refused |
| Generic files or managed sessions | Refused | Refused |

These are Milkdrift mapping choices, not claims about every endpoint implementing either API.
Anthropic and unqualified OpenAI-compatible profiles send `max_tokens`. A qualified text profile
explicitly selects `max_tokens` or `max_completion_tokens`; its server contract must cover all
generated tokens, including reasoning. Provider options use the mapping's explicit
extension namespace and cannot overwrite already emitted request fields.

## Reserve and settle supported model usage

Endpoint schema 2 requires explicit `billing` and `token_limits`. `unknown` remains a readable
profile declaration but cannot establish the enforceable bounds required by ordinary task entry. No reader
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

Prepared envelopes explicitly name `model_tokens`. Task entry charges the prepared input and output
maxima against the workflow's cumulative unit allowance. Input means the complete submitted
prompt after templating; output includes every generated token, including reasoning. Re-evaluating
a cache during context shifting is computation, not another submitted prompt. This allowance does
not measure GPU work. A billed contract is unsupported if that work adds charges outside its tariff.

`text_tariff` declares exact currency and rates in currency millionths per million input, cached
input and output tokens. These must cover every charge. The prepared bound uses the larger input
rate, checked arithmetic and rounds the total upward to a currency millionth. The observed charge
uses reported token categories and the same frozen tariff. Different cached rates require cached-token evidence.
Missing applicable usage, overflow, contradictory totals or charge evidence remain unresolved.
Nullable usage details remain unknown. Both response modes retain the raw usage breakdown;
streamed usage is a final aggregate, so a later packet cannot replace an earlier non-null report.
An unbilled response reporting a nonzero charge is uncertain, not silently free.
Conflicts retain the raw provider amount and mark the accounting basis `unresolved`; they never
label that amount as a calculated charge or refund the entry allowance.

For a billed profile, the adapter also declares the maximum permitted call's charge to the
ordinary authority evaluator, rounding upward to its existing hundredth-currency permission unit.
A grant below that profile-wide ceiling cannot select the generation, even for a smaller request.
The adapter retains the exact prepared charge in millionths. Task entry rounds that bound upward
to hundredths for its cumulative account and binds the account to one currency; mixed currencies
refuse. Selection permission alone does not spend allowance. Unbilled calls need no monetary amount.

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
prepared request digest, envelope, accounting basis and calculated charge. Observed usage remains
separate from the cumulative allowance charged at entry; a terminal report does not replenish that
allowance. A tariff calculation is never presented as a provider invoice. A failed report or
lost response retains the charged allowance and uncertainty; timeout and byte cutoff
do not establish that remote generation stopped. See the
[local setup guide](../../docs/guides/local-model-endpoint.md#exercise-the-ordinary-path).

`Fresh` uses current selected inputs without discovering a conversation. A task can explicitly
bind a prior producer's named output as evidence for a new fresh request. The daemon freezes that
artifact identity when the consumer starts; subsequent correction does not rewrite it. Neither
`ExplicitContinuation` nor `ProviderManaged` has a maintained execution path through this adapter.
The [continuation guide](../../docs/guides/model-continuation.md) explains those bindings.

## Prepare once before external entry

The host acquires the exact generation permit, then calls the adapter's local `prepare` hook.
It verifies the request and frozen selection, materializes bounded selected inputs, negotiates features,
encodes the complete HTTP body, and resolves authentication headers. This step never contacts the
provider. The returned one-shot handle retains those exact bytes and the profile's endpoint; entry
does not reload inputs or build a second request. Authentication values remain ephemeral and headers
are marked sensitive. No header or secret bytes enter stored proof or diagnostics.

Runtime checks authority before those reads and again after preparation. Its final task transaction
compares the frozen dispatch, checks stop state and the execution deadline, charges the cumulative
allowance, and records entry intent. A revoked grant, stopped task or expired allowance refuses entry.
Dropping the prepared handle releases its generation permit. Request equality binds all invocation
inputs, capability generation, execution coordinates and frozen authority; only a fresh decision
for the same authority request may replace the earlier entry decision.

| Observed boundary | Result |
| --- | --- |
| Local preparation refuses before entry intent | Failed task, no provider request or spent entry allowance. Temporary exact-generation capacity exhaustion queues the same unentered dispatch. |
| Entry intent exists; send or response completion is unproven | Uncertain until durable terminal evidence or authorized reconciliation resolves it. |
| Complete parsed response and durable terminal | Preserve provider success/failure, usage, and output semantics; result acceptance remains separate. |
| Complete response, followed by local artifact/publication failure | Failed terminal if reporting succeeds; `ResponseObservedFailure` otherwise preserves the stage while runtime retains uncertainty. |

A crash can occur between durable intent and network transmission. Missing send flags cannot prove
that nothing happened. Even a locally observed refusal cannot survive restart as negative proof if
its terminal commit failed. These effect stages preserve retained task evidence. Ordinary task entry
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

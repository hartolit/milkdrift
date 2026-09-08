# milkdrift-model

A workflow review task needs both a request to send and evidence to review. This library describes
the model messages and generation choices, the response returned by the endpoint, and the manifest
of context selected by runtime. These are distinct documents: the request expresses what to ask,
the manifest records the selected evidence, and the response preserves what the endpoint reported.

## Construct a request

The [crate introduction](src/lib.rs) has a runnable example that builds a `Message`, creates a
`ModelTaskRequest` with a fresh session and a 512-unit output allowance, and round-trips it through
`ModelTaskRequestDocument`. Supply the encoded document as inline JSON or an immutable artifact
under `MODEL_TASK_INPUT_NAME` in the invocation inputs. A workspace-value reference is not accepted
for this reserved input by the current adapter. The resolved endpoint profile supplies the model
identity. The [model-provider adapter](../../adapters/model-provider/src/adapter.rs) reads the
request and maps it through its OpenAI-compatible or native Anthropic implementation.

Choose an output allowance for the work. Current mappings send it as `max_tokens`; it limits
generated tokens according to the endpoint's accounting.
[`ModelTaskRequest::new`](src/task.rs) owns the shared count/text bounds. Endpoint request, response,
and stream byte limits are separate [profile choices](../../adapters/model-provider/src/profile.rs).

Shared validation does not establish endpoint support. The adapter checks configured features
and protocol mappings before HTTP, including roles, images, tools, structured output, reasoning,
streaming, and sessions. Current mappings accept only `SessionSelection::Fresh`. There is no
endpoint-specific numeric output-token limit discovery; an endpoint can refuse an allowance
that passes shared validation. Follow the [model endpoint guide](../../docs/guides/local-model-endpoint.md)
for setup and a complete operator scenario rather than treating the library example as a live call.

[`ModelResponse`](src/task.rs) retains final text, structured data, tool calls, finish reason,
usage, and provider metadata. Returned tool calls remain data. A valid response need not contain
useful text, and missing usage remains unknown. Provider failures after external entry may leave
the outcome uncertain; [ADR 0012](../../docs/decisions/0012-provider-neutral-model-endpoints.md)
explains why response loss is not permission for an automatic retry.

## Follow the selected context

The blueprint's [`TaskContextPolicy`](../blueprint/src/context.rs) requests inputs and earlier
evidence. [Runtime discovery and selection](../runtime/src/context.rs) apply it with authority,
branch visibility, and budgets for model/process tasks, then save a
[`ContextManifest`](src/context.rs) before dispatch. This contract also serves process tasks;
it is not a model conversation store.

The manifest records selected sources and omission reasons, exact byte digests and sizes,
known producer identities, the consuming attempt, policy digest, totals, and budget. Producer
facts distinguish results from different executions or capability generations. The
[host](../capability-host/src/materialization.rs) and adapter use the byte facts to reject
content that contradicts the saved selection. A digest does not grant read access or prove
the content's claims. Retries retain the selection and omissions while recording a new attempt.

Read saved bytes with `ContextManifestDocument::from_json`, then inspect `body()`. Entries explain
included evidence; omissions explain losses. The omission API documents a current gap where
reason precedence can bypass metadata redaction, so retained references are not proof of read
permission. The [manifest API](src/context.rs) owns version, digest, ordinal, and total checks. The
[golden fixture](tests/fixtures/context-manifest-v2.json) shows the envelope and body, and
[ADR 0011](../../docs/decisions/0011-causal-context-manifests.md) explains why selection is saved
before dispatch. Blueprint's [policy API](../blueprint/src/context.rs) discloses the current session
and required-evidence limitations; reading a manifest does not rerun policy selection.

## API detail and verification

This package has no feature flags. Pure construction and document checks need no model endpoint
or credentials. Generate API documentation with `cargo doc -p milkdrift-model --no-deps --open`.
The [contract tests](tests/contracts.rs) cover canonical documents and invalid input; the runtime
suite covers context budgets, omissions, isolation, and persistence.

From the repository root:

```sh
cargo test -p milkdrift-model --test contracts --all-features
cargo test -p milkdrift-model --doc --all-features
cargo test -p milkdrift-runtime --test causal_context --all-features
```

Use the [verification policy](../../docs/development/workflow.md#choose-verification-for-the-change)
for additional checks required by a change.

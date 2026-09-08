# milkdrift-model

This library defines the messages and generation choices sent to an external model, the result
reported back, and the context selected for a task attempt. Workflow authors and adapters use the
same typed documents without making workflow definitions depend on a provider's HTTP shape.
The package contains no model runner, network client, credentials, or durable store.

## Construct a request

The [crate introduction](src/lib.rs) has a runnable example that builds a `Message`, creates a
`ModelTaskRequest` with a fresh session and a 512-unit output allowance, and round-trips it through
`ModelTaskRequestDocument`. Supply that document under `MODEL_TASK_INPUT_NAME` in a model task's
invocation inputs. The resolved endpoint profile supplies the model identity. The
[model-provider adapter](../../adapters/model-provider/src/adapter.rs) reads the request and maps
it through its OpenAI-compatible or native Anthropic implementation.

[`ModelTaskRequest::new`](src/task.rs) rejects zero output units or more than
`MAX_MODEL_OUTPUT_UNITS` (4,000,000) with `ModelContractError::Invalid`. Current mappings send the
allowance as `max_tokens`: it limits requested generated tokens according to the endpoint's
accounting, not input tokens or bytes. It is an allowance, not a prediction or actual usage count.
Input text and [endpoint request/response/stream bytes](../../adapters/model-provider/src/profile.rs)
have separate limits.

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

Read saved bytes with `ContextManifestDocument::from_json`, then inspect `body()`. The outer
document envelope uses schema version 1; the manifest body uses version 2. The body reader refuses
v1 and unknown versions and verifies the digest, entries, order, and totals. It does not infer
the materialization or producer facts absent from v1. The
[golden fixture](tests/fixtures/context-manifest-v2.json) shows the two version fields, and
[ADR 0011](../../docs/decisions/0011-causal-context-manifests.md) explains the durable selection boundary.

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

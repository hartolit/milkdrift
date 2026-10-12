# milkdrift-model

This crate defines provider-neutral model requests, messages, responses and tool exchange records.
It does not run inference or choose workflow context. Runtime constructs ordinary task requests from
frozen instructions and explicitly named inputs; the configured model-provider adapter owns endpoint
transport and feature negotiation.

`ModelTaskRequestDocument` wraps a bounded request with a verified canonical digest. Hosted invocations
supply that document as inline JSON or an immutable artifact under `MODEL_TASK_INPUT_NAME`. Explicit
text and artifact inputs are materialized by the host with content integrity and source provenance;
there is no implicit traversal of prior workflow history.

The request's output allowance and the configured endpoint's supported token contract are distinct.
The adapter must establish an enforceable bound before a task can enter. Unsupported roles, tools,
images, reasoning, streaming or session modes refuse at the adapter boundary. Ordinary task execution
uses fresh sessions. Typed continuation records remain available as data contracts, but they do not
authorize an implicit workflow conversation or provider-managed session.

`ModelResponse` retains text, structured content, returned tool calls, finish reason, reported usage
and provider metadata. Tool calls are data until separately admitted through an authorized execution
path. Missing usage remains unknown. Loss of a response after external entry does not permit an
automatic retry.

The crate introduction contains a pure request construction example. [Contract tests](tests/contracts.rs)
cover canonical request/response envelopes and invalid content without an endpoint or credentials;
model-provider tests cover explicit materialization and real local HTTP failure boundaries.

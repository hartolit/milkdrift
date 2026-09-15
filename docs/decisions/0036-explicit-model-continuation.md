# ADR 0036: Freeze explicit model continuation in context artifacts

Status: Accepted for implementation; operational evidence is recorded in
[verification evidence](../development/verification-evidence.md).

## Context

The existing model request names a predecessor manifest and response, and task policy separately
declares continuation intent. Neither field previously had an endpoint mapping. Implementing a
mutable session record would introduce another authority over history and make retries sensitive
to later publications. Copying raw responses into a system prompt would erase their source roles.

## Decision

Runtime's context owner resolves both exact artifacts before scheduling. It verifies their media
families, digests, producing invocation, manifest linkage, completed outcome, scoped causality,
actor/grant and current read authority. Historical selection goes through the existing retained-policy
validator. The control adapter remains the owner of result acceptance; runtime follows its durable
accepted-output marker for evaluations of the source invocation, including exact workspace-value
bindings. The operation and subject/accepted-output names are shared capability protocol constants,
not a second acceptance evaluator.

The smallest added durable document is `ContinuationHistoryDocument`, schema 1, selected as a
required `Continuation` entry in the ordinary manifest. It contains a flat bounded history, exact
source references and journal anchors, message boundaries and initiating authority. There is no new
run/session ledger, mutable latest pointer or workflow primitive. Selection and retries keep the
same context publication and retention path. Reuse reconstructs and compares the exact saved sources
before entry rather than discovering a newer predecessor. Current authority and new acceptance
evidence may refuse reuse without changing history.

The [operator guide](../guides/model-continuation.md) owns message construction, bounds and supported
use. Prior instructions are omitted, prior data keeps user/assistant roles, and complete tool pairs
are required. Both adapters consume this same semantic history and preserve their native wire
forms. Unsupported combinations fail before HTTP. Fresh remains independent; provider-managed and
process sessions remain refused, including sequence imports and retained process work.

## Compatibility and ownership

Manifest schema 2 and selection policy 2 remain unchanged: the existing continuation inclusion reason
now selects a separately versioned artifact. Model request/response envelopes stay at schema 1.
`Message.tool_calls` is an optional assistant-only field omitted when empty, so existing canonical
request fixtures remain byte-identical and new readers accept existing requests. Earlier readers
refuse nonempty tool-call messages as unknown fields and earlier adapters already refuse explicit
continuation. This is not a promise that old binaries can execute new requests. Golden fixtures cover
the companion and complete continuation request, and hostile readers refuse unknown/future shapes.

No storage format, control protocol, daemon configuration or inspector DTO version changes. The
inspector projects the existing manifest entry and authorized artifact. New model exports are durable
schema and workspace adapter contracts consumed by runtime and provider mappings. Runtime selection
helpers and provider encoders remain private. Control's existing constant exports retain their names
while referencing the shared protocol definitions. Prompt-sequence schema 3 narrows import support
to the only process behavior implemented; older immutable revisions remain inspectable but cannot
use unsupported non-Fresh process execution.

## Consequences

An operator can continue selected work without hidden provider memory, but must explicitly expose
causality and authorize each source. Sibling branch conversations, multimodal history and provider-
specific extensions need an explicit fresh evidence boundary. Source and journal bounds can refuse
very old or large conversations even when their references are exact. A future wider mapping must
define its own evidence, role and lifecycle semantics; accepting an enum variant is insufficient.

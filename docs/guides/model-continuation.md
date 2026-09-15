# Continue an exact model answer

Use `ExplicitContinuation` when a new model invocation should receive a particular earlier question
and answer. Milkdrift builds the conversation from immutable evidence. It does not ask the provider
to recover an opaque session, and `Fresh` never discovers an earlier conversation automatically.
Both existing mappings support bounded text history and complete tool exchanges. Their profile
feature checks and request-byte limits still apply.

## Select the predecessor

1. Inspect the successful producing attempt with `milkdrift attempt inspect RUN ATTEMPT`. Obtain its
   `context_manifest` metadata and the `model_response` entry in `outputs`. Use the canonical response,
   not `final_text`, a progress observation, or the provider's response ID.
2. Put those exact references in the new model request's `session`, with type
   `explicit_continuation`. Map inspection's `artifact_id`, `digest`, `content_type`, and `size` to
   the request reference's `identity`, `digest`, `media_type`, and `size_bytes`. Supply the new question
   in the request's messages and supply any current system/developer instructions there.
3. Set the new task's context policy to `explicit_continuation` too. Remove `prior_prompt` from its
   excluded categories; remove `tool_trace` only when continuing a tool exchange. Keep other category
   restrictions appropriate to the task. The default policy excludes both categories.
4. Make the predecessor causal through the workflow's control/data path. Add or replace pending work
   using the ordinary [proposal controls](headless-dogfood.md). The current actor/grant must be able
   to read every source, and the predecessor must be in this run's visible scope lineage. A sibling
   branch's private conversation cannot be imported by knowing its artifact ID. A join's exported
   result can instead be selected as evidence for a fresh task.

The [request example](../../examples/local-model/continuation-request.example.json) shows the
complete request document. Its placeholder references are reader examples, not artifacts in an
operator's store. Use inspection to obtain real values. The
[daemon evidence scenario](../../tools/evidence/src/bin/local-model-evidence/continuation.rs) performs
inspection, prospective proposal submission/adoption, release, and restart through the actual CLI.

## What the next request contains

Runtime builds history from oldest to newest. For each producing invocation it contributes its
selected non-direct text/JSON evidence as labelled, delimited user data, its non-instruction task
messages in order, and its canonical response as an assistant message. Other direct inputs were not
sent by the model mappings and are not invented as conversation messages. The prior request's system
and developer instructions are omitted. Current instructions precede the saved history; current
non-instruction messages follow it. Current policy-selected evidence keeps the mapping's existing
untrusted user-evidence position. Text resembling role tags remains text in its original role.

A structured response contributes its exact text only when parsing that text yields the same
structured value. Usage and provider metadata are provenance, not conversational instructions.
Only `stop` and `tool_calls` responses with text or calls can be predecessors. When a workflow
acceptance task has evaluated any output of that invocation, its durable accepted marker and
successful evaluation are required. A rejected or unfinished evaluation refuses continuation;
remediation can select the failed output explicitly as evidence for a fresh request.

An assistant's tool calls retain names, arguments and IDs. The current request must provide exactly
one `tool_result` for every outstanding call before another user/assistant message. Calls must have
been declared by the producing request. Missing, duplicate, repeated or unknown call IDs are refused.
The current policy must allow `tool_trace` for calls and results in earlier request messages as
well as calls returned in a response.
OpenAI uses assistant `tool_calls` followed by `tool` messages; Anthropic uses `tool_use` and
`tool_result` blocks. Neither mapping executes the calls. Non-text predecessor parts, provider-specific
request extensions, unsupported current profile features and unknown response message semantics are
explicit refusals.

## Frozen evidence and limits

The ordinary manifest selects one required artifact with reason `continuation`. Download it with
`milkdrift artifact get ARTIFACT --output history.json`. Its
[version-1 contract](../../crates/model/src/continuation.rs) records ordered messages,
exact manifest/response references, journal anchors, per-turn message boundaries and initiating
authority. The predecessor manifests retain the original selections and omissions. Inspection exposes
the companion reference through the existing authorized context view.

There are at most 32 predecessor invocations, 256 combined messages and 1 MiB of aggregate message
text. The document reader caps each document at 2 MiB. Current context item, non-artifact byte,
artifact-count, artifact-byte and per-item limits also bound source reads and the companion; the
manifest and endpoint impose their own encoded-byte limits. Predecessor discovery and acceptance
checks must fit `max_candidate_records`. A configured model-input-unit estimate is refused for
continuation because this assembly has no estimator; endpoint accounting still measures the actual
prepared request where configured. Exhaustion refuses preparation without silently shortening history.
Choose a new policy/revision or explicitly summarize evidence into a fresh task.

Preparation pins the exact selection. Lease recovery and permitted retries retain it and recheck
source integrity, the governing policy, current authority and acceptance evidence before entry.
Runtime repeats these checks after local preparation, so a worker waiting since claim cannot use
a source that has since become corrupt or unauthorized. Such refusal creates no external entry
intent or controller reservation.
Later output publication cannot replace the chosen predecessor. New denial or missing evidence can
prevent entry; it never rewrites the saved attempt. Existing uncertainty rules still prohibit retry
after possible provider entry without the required durable evidence. No provider session or automatic
exactly-once claim follows from continuing a conversation.

`ProviderManaged` remains unsupported. Process tasks and prompt-sequence imports accept only Fresh
execution with explicitly selected durable prior evidence.

# Assignment 05 — Support exact-reference Milkdrift continuation

## Outcome

Implement the already-declared `ExplicitContinuation` mode through existing model mappings, using
exact prior Milkdrift artifacts and manifests. Preserve `Fresh` and reject unsupported provider-
managed or process sessions. This is not authorization to build a provider session service, hidden
agent memory, new model protocol, or general semantic knowledge database.

Read the sprint README, `AGENTS.md`, canonical docs, workflow and implementation/documentation
practices. Work on the current result of Assignments 01–04. Recheck source before changing a contract
that may already be completed; avoid duplicate session identifiers or new mutable session ledgers.

## Current source and purpose

The review found honest Fresh-only production mappings. The source already distinguishes
`ContextSessionPolicy::{Fresh, ExplicitContinuation, ProviderManaged}` and
`SessionSelection::ExplicitContinuation { manifest, response }`. The adapter refuses the latter
because no protocol mapping is installed. Exact-reference continuation is the narrow useful
completion: the operator can continue selected work without hidden provider memory, while the fresh
agent stages motivating Milkdrift remain fresh.

Inspect `crates/blueprint/src/context.rs`, `crates/model/src/task.rs`, model/context contracts,
`crates/runtime/src/context/`, `crates/runtime/src/engine/effects/session.rs`, invocation/task freezing,
`adapters/model-provider/src/adapter.rs` session negotiation, both wire mappings and
`tests/mock_endpoints/runtime_session.rs`. Inspect prompt-sequence session declarations and current
API/inspector read models as consumers; do not assume process-only sequence stages now support model
or persistent CLI sessions.

## Implement

1. **Make the existing distinction real.** `Fresh` supplies only the current policy-selected context;
   it does not implicitly discover a conversation. `ExplicitContinuation` requests exact prior
   manifest and response artifacts. The current policy and request session declaration must still
   agree before claim/entry. Never implement continuation by copying every event since run start or
   following an unversioned “latest session” pointer.

2. **Resolve provenance, authority and causality first.** Validate the artifact family/version,
   content digest, producing invocation and manifest linkage, scope/branch visibility, permitted
   sensitivity, and current actor/grant. A caller cannot attach another branch's or user's response
   to obtain its privileged context. Pin the selected predecessor at attempt preparation; a concurrent
   later result does not change it. Preserve unsafe-retained-policy refusal. Enforce cycle/depth,
   item and byte bounds before recursive materialization.

3. **Build one deterministic message history.** Define exactly what prior selected inputs, user/task
   messages, assistant response, structured content, and tool exchanges contribute. Avoid duplicating
   the previous system instructions or promoting external response text to instructions. Preserve
   role/source labels. Incomplete tool-call/result pairs, unknown wire features, incompatible profile
   policies or missing artifacts are explicit refusals, not silently dropped messages. A complete
   provider response that failed Assignment 01's quality gate is not a successful prior answer;
   remediation may include it explicitly as failed evidence under current policy.

4. **Use the existing context owner.** The runtime/context assembly owns selection and frozen
   provenance. Provider adapters translate a bounded prepared request; they do not discover history
   or query a hidden session database. Both OpenAI-compatible and Anthropic mappings must receive the
   same semantic selection while preserving their distinct valid wire shapes. Reject combinations
   a mapping cannot represent. Do not widen provider support by merely accepting the enum variant.

5. **Persist what this attempt consumed.** Bind predecessor references, resolved ordered message/
   artifact selection, omissions, limits and current authority to the existing frozen manifest or
   smallest appropriate versioned companion. Reuse exact selection on retry/restart. No unbounded
   chain in hot projections; history remains in artifacts/journal. Context exhaustion requests an
   explicit new policy/revision or fails according to the existing policy—not silent truncation.

6. **Keep unsupported modes truthful.** `ProviderManaged` stays refused until an actual endpoint
   protocol and opaque-session lifecycle are implemented separately. A fresh process with a string
   named `session` is not persistent process continuation. Do not let a sequence import advertise
   continuity that its configured process capability cannot provide. Document the supported
   alternative: fresh execution plus explicitly selected durable prior evidence.

## Acceptance

Use two invocations and local mock endpoints through actual daemon composition. The second receives
exact authorized prior context/response and new input under ExplicitContinuation; a sibling Fresh
invocation receives no implicit prior conversation. Show the same manifest/provenance in inspection.

Test retry/restart after selection, concurrent predecessor publication, cross-branch/cross-actor
references, wrong manifest-response pairing, missing/corrupt/unsupported artifacts, unsafe historical
selection, budget exhaustion, revoked grant, response-role injection, and supported/unsupported tool
pairs. Test both model mappings plus honest process/provider-managed refusal. Assertions inspect the
prepared messages and server request, not merely a successful constructor.

Run focused model/blueprint/runtime/context/session, mock-endpoint, sequence validation and daemon
suites; run the full gate and affected actual-binary model evidence lane. Update the model feature
matrix, session/context docs, examples, fixtures and status accurately. No external provider is
required for deterministic acceptance; do not claim real provider sessions were tested.

Stop when the existing explicit-reference mode is usable and bounded end to end, Fresh isolation is
unchanged, unsupported modes remain explicit, and restart/authority checks pass. No new session
framework or unrelated memory feature belongs in the handoff.

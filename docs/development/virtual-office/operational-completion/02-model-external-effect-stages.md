# Assignment 02 — Prove no-request failure without weakening uncertainty

## Outcome

Refine the existing model execution boundary so a proven local refusal before any provider request
is not misclassified as an uncertain remote effect. Retain conservative uncertainty whenever the
request may have left the host or the durable record cannot prove otherwise. This assignment changes
failure-stage evidence, not provider capabilities or retry policy broadly.

Read the sprint README, `AGENTS.md`, canonical docs, `docs/development/workflow.md`, and both
`docs/development/practices/implementation.md` and `documentation.md`. Work on the current checkout
after Assignment 01. No reset, new provider, networking stack, or alternate executor is authorized.

## Source and observed limitation

The review found model feature negotiation inside adapter execution: some unsupported/local errors
arise after generic adapter entry but before HTTP. The present conservative result is safer than an
unsafe retry, but unnecessary uncertainty creates operator work.

Trace:
- `adapters/model-provider/src/adapter.rs` from task loading, session checks, negotiation and request
  construction through send, stream, output publication, and reporting;
- `crates/capability-host/src/adapter.rs` admission/pre-entry hooks and error contracts;
- `crates/runtime/src/executor.rs` prepared one-shot entry;
- `crates/runtime/src/engine/effects/entry.rs`, effect/report handling and recovery;
- model mock endpoints and shared adapter conformance tests.

Do not assume any helper named `rejected` implies the remote server saw nothing. Establish where
requests can actually occur and what the existing prepared-entry contract can freeze.

## Implement one complete boundary

Move deterministic task/profile/session/feature validation and request preparation before external
entry where the existing architecture permits it. Bind prepared data to the exact request digest,
frozen context manifest, profile/capability generation, endpoint identity, and required authority.
Reuse it for the one authorized entry rather than preparing a different request after validation.
Keep sensitive headers/secret bytes ephemeral; do not add them to stored proof or diagnostics.

Preparation must not contact a provider, create a provider session, invoke a tool, or perform a
billable action. Local reads are authorized and bounded. If an adapter's preparation genuinely
requires remote work, that work is not “pre-effect” and needs truthful existing entry semantics.

Use the smallest typed stage/outcome distinction needed. Prefer current prepared-entry/refusal types
if sufficient. Distinguish at least proven not submitted, request possibly submitted, complete
provider response observed, and failure of local persistence/reporting after response. Do not add a
caller-set `safe_to_retry` Boolean or decide safety from error text/status alone.

Preserve the final authority/account reservation transaction before remote entry. A prepared request
does not bypass revocation, grant narrowing, endpoint generation changes, or budget admission. A
preparation refusal must not leak permits or strand the attempt/account. Do not erase an existing
durable entry intent after a crash simply because no local send-completion flag was persisted.
Network transmission and local journaling are not an atomic transaction. After intent but before a
provable terminal result, recovery may legitimately remain uncertain.

No-request proof permits classification as a deterministic pre-effect failure, not automatic retry
of all failures. Retry still follows task policy and current authority; unsupported features should
not spin. Timeouts after send, malformed/truncated streams, response loss, process interruption, and
reporter/persistence failure after possible entry retain their appropriate conservative treatment.

## Tests

Use a local counting mock transport/server and fault hooks. Cover unsupported feature/session and
invalid request with zero sends, stale prepared request/generation refusal, authority revocation after
preparation, valid exact one-shot send, and duplicate entry rejection. Cover failures just before send,
during write, after acceptance, mid-stream, after complete response, during artifact publication, and
during durable terminal reporting. Test crash/reopen at ambiguous windows without inventing negative
proof. Assert request counts, terminal/uncertain classifications, reservation settlement, and no
hidden fallback or duplicate non-idempotent call.

Keep Assignment 01's result-acceptance semantics: a complete but useless response is not “no effect.”
A provider response with a tool request is data, not authorization to run that tool.

## Verification and completion

Run model mock endpoints, shared adapter conformance, runtime effect/recovery/controller-account,
and daemon integration suites. Run the full gate and affected actual-binary model evidence lane.
Update current endpoint/error-stage docs and compatibility fixtures only where semantics changed.
No blanket migration, event rewrite, or broad adapter refactor belongs here.

Finish when a traceable typed proof distinguishes genuine zero-request refusal from ambiguous
external work, existing unknown-effect protection remains intact, production paths use the proof,
and tests demonstrate both sides. Hand off any account reservation/settlement change to Assignment 03.

# Work package: pristine generic load ownership and E0 accounting

## Mission

Audit and harden the portable prepared-load contract and the backend-independent inference runtime after the artifact/loading subsystem has been corrected. Make every ownership and accounting transition exact or explicitly uncertain, remove false precision after backend contract violations, simplify admission/rollback structure without weakening auditability, and ensure the runtime remains a durable execution kernel for future local backends.

Do not merely adapt compilation to the previous commit. Independently verify the generic semantics and fix any design that would let a contract-violating backend hide, lose, or misreport ownership.

## Read before editing

Read:

- the repository agent context and persona;
- `README.md` and `docs/vision.md`;
- `docs/project/architecture.md`;
- `docs/project/inference-runtime.md`;
- `docs/project/candle-backend.md`;
- `docs/project/lifecycle.md` if present;
- `docs/project/implementation-status.md`;
- ADRs 0006, 0010, 0013, 0019, and 0020;
- the original and executed Phase 12 plans;
- the preceding artifact-loading commit and its completion report.

Inspect all current load, cleanup, snapshot, memory, shutdown, and fault-injection code rather than assuming the prior agent's report is accurate.

## Owned area

Primary ownership:

- `crates/domain/domain-contracts/**` for generic model/load/ownership vocabulary;
- `crates/runtime/inference-runtime/**`;
- `crates/platform/host-runtime/**` only where generic bounded hosting or observation must change;
- backend-independent deterministic fixtures and fault injection.

You may make compile-preserving adaptations in Candle or E1, but defer application semantics to the next work package. Do not introduce workflow/workspace concepts into E0.

## Required architectural outcomes

### 1. Revalidate the prepared-load contract

The generic boundary must make these facts unambiguous:

- preparation owns one exact accepted source/configuration/device plan;
- an unmaterialized preparation rejected before materialization is ordinary-drop-safe;
- once materialization starts, failure returns the sole cleanup owner;
- cleanup success is the only ordinary transition authorizing owner drop;
- cleanup failure leaves the owner valid and retryable;
- the plan's final footprint and loading peak describe named ownership phases rather than sampled memory;
- E0 does not call a second plan function or reconstruct backend policy;
- a preparation cannot be consumed twice.

Review whether the current `PreparedLoad`, `FailedLoad`, `ModelLoader`, `LoadPlan`, and `LoadedModel` shapes communicate this cleanly to a third-party backend implementer. Improve names, docs, type states, and tests where ambiguity remains. Avoid exposing filesystem, Safetensors, Candle, or vendor types through the portable boundary.

Do not add dynamic dispatch to the token/tensor hot path.

### 2. Eliminate false exactness after a complete-model contract violation

The current runtime can materialize a complete model, observe that its descriptor/device/execution scalar/accounted footprint violates the accepted plan, fail explicit unload, and then retain the model while publishing only the planned loading peak as though it were exact ownership.

That is not sufficient for a backend-neutral runtime. A backend that has already violated its footprint contract may own more or differently classified resources than the accepted plan.

Implement a durable retained-ownership representation that distinguishes at least:

- exact known ownership;
- a conservative known lower bound or otherwise unverified ownership after a backend contract violation;
- fully released ownership.

The design may use an enum or an equivalent state model, but it must satisfy these properties:

- ordinary accepted models and failed partial preparations retain exact accounting;
- a complete model whose post-load contract is violated never becomes falsely “exact” merely because a plan existed;
- the runtime incorporates every trustworthy reported component into the conservative retained state using checked arithmetic;
- if exact upper-bound accounting cannot be established, further model/resource admission fails closed while that owner remains;
- snapshots and cleanup state expose the uncertainty explicitly without leaking backend types;
- successful cleanup removes the retained state exactly once;
- exhausted cleanup remains observable and admission-blocking until the process reclamation boundary;
- shutdown never turns handle absence into proof of release.

Do not solve this by assigning `u64::MAX`, saturating silently, or claiming process RSS/device samples are ownership accounting. Do not simply choose `max(plan_peak, reported_actual)` and call it exact if the backend has already broken the contract.

Add fault-injection cases where the loaded backend reports:

- an actual footprint larger than final and larger than loading peak;
- reclassified host/device ownership;
- overflowing aggregate totals;
- a smaller footprint;
- a correct footprint paired with wrong device/scalar/descriptor;
- cleanup failure and later success for each class;
- permanent cleanup exhaustion.

### 3. Make reservation transitions transactional and provable

Audit the complete load path:

```text
preflight
→ create exact preparation
→ validate plan
→ admit loading peak and final state
→ reserve loading peak
→ materialize
→ clean failed preparation or retain it
→ validate complete model
→ clean incompatible complete model or retain uncertainty
→ publish model
→ replace peak with final reservation
```

For every return path, prove through code structure and tests:

- which owner exists;
- which identity indexes exist;
- what reservation state is published;
- whether model generation advances;
- whether admission is locked;
- what cleanup resource is observable;
- whether a later retry can release exactly once.

No transient state may be published as a normal loaded model. No error conversion may discard the only cleanup owner or replace the primary failure with cleanup failure.

Use checked component-wise arithmetic. Add property-style/table-driven tests for reservation addition/subtraction and phase transitions, including pre-existing resident ownership.

### 4. Refactor admission and cleanup by transaction responsibility

The current admission and cleanup files use large functions justified by “one auditable transaction.” Preserve contiguous transaction semantics, but do not use that as a reason to keep unrelated validation, ownership construction, index mutation, and error translation in one oversized function.

Refactor toward explicit internal operations such as:

- load preflight;
- preparation validation;
- reservation decision;
- failed-preparation retention;
- complete-model verification;
- incompatible-model retention;
- final commit;
- cleanup release transaction.

Use small private state/decision types where they make illegal transitions harder to express. Do not create a generic transaction framework or macro abstraction that obscures ownership.

Remove broad `clippy::too_many_lines` expectations where better structure now exists. Keep hot scheduling code separate from cold load machinery.

### 5. Make cleanup scheduling fair and scalable

Review cleanup polling across pending sequences, failed preparations, and complete models. The runtime should not permanently starve one cleanup class because ordered maps always return another retryable owner first.

Required behavior:

- one bounded cleanup opportunity per poll remains acceptable;
- retry budgets remain per owner;
- exhausted owners are not retried automatically;
- runnable/healthy model work remains possible where ownership certainty and policy allow it;
- cleanup selection rotates fairly across eligible owners/classes;
- admission remains fail-closed when unverified complete-model ownership exists;
- shutdown consumes the finite remaining cleanup budget deterministically.

Add deterministic fairness tests instead of relying on map order.

### 6. Preserve failure identity and observability

Review `CleanupFailureReport`, `CleanupRetryState`, `CleanupResource`, runtime snapshots, model snapshots, load receipts, and terminal shutdown reports.

Ensure they distinguish:

- preparation/materialization failure;
- failed-load cleanup failure;
- post-load backend contract violation;
- complete-model unload failure;
- exact retained ownership versus unverified retained ownership;
- retryable versus exhausted cleanup;
- terminal process-lifetime retention.

Keep the public structures bounded and allocation-conscious. Avoid recursive boxed error chains. Do not make debug strings the API.

### 7. Audit backend substitution assumptions

Use the deterministic backend to test malicious or malformed implementations, including:

- plan mutation between calls where the type system permits it;
- mismatched accepted configuration;
- empty or contradictory descriptor facts;
- a preparation that claims a peak below final;
- failure owners that change reported state across retries;
- loaded models that contradict the preparation;
- cleanup hooks that fail without invalidating the owner.

Where E0 cannot verify a claim generically, document the exact trust boundary and fail closed rather than pretending it was verified.

### 8. Keep the engine publication path clean

Review the public surface of `domain-contracts` and `inference-runtime` as the future basis of publishable Milkdrift engine crates.

Requirements:

- portable contracts continue to compile on the documented `no_std` targets;
- no adapter-only source identity or tensor inventory leaks into F0;
- public types have coherent names and rustdoc examples/invariants where useful;
- normal production dependencies remain backend-neutral;
- Candle remains a dev/test dependency of `inference-runtime`, not a production dependency;
- no UI, Hugging Face, persistence, conversation, or workflow assumptions enter E0.

Do not rename packages solely for branding in this work package; focus on semantic readiness.

## Testing requirements

Extend backend-independent tests for:

- exact preparation consumption;
- every plan validation failure before materialization;
- all failed-preparation cleanup outcomes;
- all complete-model mismatch/cleanup outcomes;
- exact versus unverified retained ownership;
- admission blocking under uncertainty;
- aggregate accounting with existing models/sequences/workspaces;
- cleanup fairness across several owners/classes;
- unload and shutdown with retryable and exhausted owners;
- no double release, stale identity reuse, or generation regression;
- snapshots and receipts never claiming false release or exactness.

Keep native Candle integration tests for at least one homogeneous and one mixed layout to prove the corrected adapter still satisfies the generic contract.

## Validation

Run:

- formatting for the workspace;
- targeted checks/tests/Clippy/rustdoc for `domain-contracts`, `host-runtime`, and `inference-runtime`;
- portable WASM and embedded checks for changed portable crates using isolated target directories;
- default CPU tests;
- CUDA compilation and relevant native E0 hardware tests when available.

Do not claim whole-workspace closure; later prompts own E1, CI, and final verification.

## Completion

Update the inference-runtime documentation and ADR-0020 or create a narrowly scoped ADR if retained ownership certainty is a new durable decision. Keep history factual.

Create one coherent commit and do not push. Report:

- commit SHA and tree SHA;
- the final prepared-load and retained-ownership state model;
- how admission behaves under unverified ownership;
- cleanup selection/fairness behavior;
- exact validation executed;
- downstream E1 adaptations the next work package must complete.

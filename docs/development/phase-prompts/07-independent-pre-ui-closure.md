# Pass 7 — Independent pre-UI closure and repair

Use this prompt with `00-shared-execution-contract.md`.

Give this pass to a fresh agent that did not implement the earlier passes. Provide the resulting repository, not earlier agents’ summaries. The agent must independently inspect, reproduce, repair, and qualify the current source.

## Objective

Determine whether the repository now has one coherent pre-UI system: stable semantic owners, truthful cross-platform paths and recovery, narrow package/public boundaries, conformant adapters, cohesive private modules, a comprehensive thin CLI, and proven external model behavior. Fix every defect found within those areas. Do not merely publish an audit.

Controller activation remains governed by ADR 0027 and the repository’s existing controller-admission closure prompt. This pass may not activate or preserve activation unless the current source and evidence satisfy that contract exactly.

## 1. Establish the actual current state

Read all canonical documents and relevant ADRs. Inspect:

- every commit and diff produced by the preceding passes;
- current required/hosted workflow conclusions and logs;
- current package dependency graph and public API inventories;
- all durable/wire schema constants, fixtures, readers, and writers;
- repository-contract exceptions and lint allowances;
- status/roadmap claims against actual evidence artifacts.

Run the full local gate before assuming the code is sound. A failure is work to repair, not a reason to stop at a report.

## 2. Audit semantic ownership

Prove from source that:

- peer adapter/service/worker/persistence layers cannot race to assign different meanings to one post-entry outcome;
- exact recovery obligations survive clock/store failure, worker shutdown, and restart without duplicate external entry;
- filesystem authority has one platform-aware representation and component containment across every producer/consumer;
- runtime alone owns workflow state and external-entry interpretation;
- adapters only report observations and own their mechanism resources;
- definition, execution, and control truth remain separate;
- controller resource accounting, when present, has one final-entry-adjacent durable owner and no projection-based competing hard limit;
- uncertain work remains visible and can be resolved only through the authorized command path;
- status/read models do not turn restricted/missing/unknown facts into false success or absence.

Search for duplicate reason strings, transition helpers, retry loops, direct store mutations, local authority checks, direct system clocks, and bypass constructors that compete with the intended owners.

## 3. Audit package and public boundaries

Verify:

- removed runtime/query compatibility paths have not returned;
- deterministic fakes are absent from default product APIs;
- `milkdrift-contracts` owns only proven cross-domain mechanics;
- semantic digest/identity/schema policies remain with their owners;
- blueprint and capability responsibilities remain distinct;
- no duplicate canonical identity or convenience re-export obscures ownership;
- no product crate depends on evidence/test packages;
- no new common/core/framework/plugin crate has appeared;
- every public trait/type has a real production, adapter, durable-schema, or wire consumer;
- default/all-feature API inventories and repository checks agree.

When a boundary fails, implement the atomic contraction and remove the obsolete path. Do not paper over it with architecture prose.

## 4. Audit adapter conformance and lifecycle

Run the reusable conformance suite against every current production adapter. Independently inspect that the suite actually reaches each implementation and does not merely test a wrapper fake.

Verify exact selection, no fallback, reporter failure, terminal uniqueness, cancellation correlation, supplied health time, drain, shutdown, panic containment, resource cleanup, host-lock release, and any current prepared-entry/admission-envelope behavior.

Add missing hostile assertions and repair interface/implementation divergence. One implementation-specific exception must be explicit and tested rather than silently skipped.

## 5. Audit cohesion and complexity

Re-run file/function diagnostics and repository contracts.

- Inspect every production file near/above the review threshold.
- Confirm exceptions are exact, justified, bounded, and still needed.
- Confirm the five targeted owners have meaningful private structure rather than forwarding modules.
- Search child modules for copied dispatch, validation, transition, and error-mapping logic.
- Remove stale exception entries and split newly mixed responsibilities.
- Do not fragment cohesive exhaustive reducers to improve a metric.

## 6. Audit the CLI as an external consumer

Build the actual daemon and CLI binaries and operate them against temporary state.

Verify:

- protocol/CLI parity is intentional;
- blueprint validation/import/export and prompt-sequence operations use canonical document owners;
- proposals and live revisions use exact optimistic guards;
- retained-work resolution exposes every current action without client-side policy;
- JSON successes, errors, and stream events are bounded stable documents;
- exit codes and retryability are correct;
- high-risk JSON commands cannot prompt or proceed without confirmation;
- stdin/file handling is bounded and duplicate-safe;
- explicit command replay/conflict works across daemon restart;
- the CLI has no database/storage access and does not infer semantic state from names or display strings;
- the black-box actual-binary scenario exercises the real HTTP/control path.

Repair any gap. Do not add graphical or interactive presentation.

## 7. Audit local model and external-effect truth

Run deterministic model dogfood. Run the real local endpoint lane when operator resources are present. Inspect the resulting timeline, attempts, context manifest, fragments, terminal/uncertainty state, and artifacts rather than trusting the harness summary alone.

Verify:

- one exact profile/model generation and one adapter entry;
- no fallback or duplicate request;
- clean success produces exact provenance and no uncertainty;
- pre-entry failure consumes no external-entry truth;
- post-entry disconnect/truncation/timeout/cancellation remains uncertain where termination is unproved;
- no successful partial artifact from malformed streams;
- retry/retain/query/compensate/evidence resolution uses the same authorized path;
- late evidence is idempotent and history remains append-only;
- secrets/raw provider content are absent from logs, public errors, reports, and debug output.

If strict external-evidence resources exist, run the qualifying process+model harness from a clean checkout. Otherwise preserve the explicit blocker.

## 8. Full evidence campaign

After all repairs, run and retain exact results for:

### Required local gate

```sh
cargo fmt --all -- --check
cargo check --workspace --all-targets --all-features
cargo test --workspace --all-features --no-fail-fast
cargo clippy --workspace --all-targets --all-features -- -D warnings
RUSTDOCFLAGS='-D warnings' cargo doc --workspace --all-features --no-deps
cargo deny check
cargo machete
cargo tree --workspace --duplicates
cargo test --workspace --all-features -- --list
```

### Repository and API

- `milkdrift-evidence` repository contracts;
- default/all-feature public API inventories for all touched packages;
- duplicate dependency inspection and unused dependency audit.

### Mutation

Run every current-source focused shard, not an old mutant count:

```sh
cargo mutation-evidence authority
cargo mutation-evidence retention
cargo mutation-evidence runtime
cargo mutation-evidence uncertainty
cargo mutation-evidence controller
cargo mutation-evidence context
cargo mutation-evidence peer
```

Fix unclassified survivors or record only exact classifications permitted by repository policy. Timeouts are failures.

### Operational and longevity

Run the operational evidence lane, effect-worker forced-shutdown proof, ordinary bounded-frontier tests, all current manual release longevity/storage lanes, peer restart/retention, controller checkpoint/admission longevity where present, and the actual-binary CLI dogfood lane.

### Hosted

Inspect or trigger the pinned Ubuntu, Windows, and macOS contract workflows when access permits. A local target check is not hosted evidence. Repair current failures and rerun until the final commit is evidenced, or leave the limitation exact.

### External

Run deterministic external-evidence fixture mode, local model real mode when supplied, and strict real process+model evidence only when operator resources satisfy its contract.

## 9. Documentation truth

Only after final evidence:

- update `status.md` with implemented facts and the exact dated evidence snapshot;
- update `roadmap.md` only when an ordered blocker genuinely closes or changes;
- update verification guides and command references to match current commands;
- remove stale pass histories or duplicated implementation prose;
- preserve limitations for absent hosted, mutation, longevity, or real external evidence;
- do not authorize UI merely because the CLI works.

Do not commit generated reports, API inventories, mutation output, credentials, profiles, model artifacts, or local evidence directories.

## 10. Final acceptance

This pass may report closure only when:

- all required local gates pass at the final tree;
- every current-source mutation shard has no unclassified survivor or timeout;
- required operational/longevity lanes pass;
- package/API and cohesion checks pass;
- actual daemon/CLI black-box dogfood passes across restart;
- deterministic model failure semantics pass;
- real/hosted evidence is either successful at the final commit or explicitly still open;
- no GUI/UI package or presentation-owned semantic path exists;
- documentation says exactly what the evidence proves.

If any condition is not met, leave the system fail-closed where required, implement every repair possible in the current environment, and report the precise remaining external blocker without weakening the contract.

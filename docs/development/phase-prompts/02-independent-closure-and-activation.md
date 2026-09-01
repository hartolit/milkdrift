# Independently close and qualify controller admission

Independently audit, repair, qualify, and—only when justified—activate the controller final-entry admission implementation produced by the previous pass.

Do not trust its completion report or assume its abstractions are correct because tests pass. Read the governing documents and attached `00-controller-admission-contract.md`, inspect the complete resulting source and diff, reproduce the invariants from externally observable behavior, and modify the repository to fix every defect you find. This is not a report-only audit.

## Governing material

Read:

1. `AGENTS.md`
2. `docs/product/vision.md`
3. `docs/architecture.md`
4. `docs/development/engineering-rules.md`
5. `docs/product/status.md`
6. `docs/product/roadmap.md`
7. ADRs 0026 and 0027
8. verification workflow/evidence guides
9. the attached task contract

Inspect Git status/history and preserve unrelated work. Do not reset/rebase, create prompt-history docs, or paper over failures with disabled tests, broad allowances, retries without bounds, or unsupported mutation classifications.

## 1. Reconstruct and audit the design

Before relying on tests, trace the complete source path for:

```text
controller activation/account establishment
  -> child-run account inheritance
  -> exact-generation preparation
  -> final authority + resource admission commit
  -> adapter entry
  -> artifact publication
  -> terminal/cancellation/uncertainty/late evidence
  -> controller assessment
  -> restart/integrity verification
```

Search the whole workspace for old and new equivalents. Confirm there is exactly one owner and one normal path for admission, account mutation, run binding, artifact charging, and lifecycle progress.

Repair any of the following rather than merely documenting them:

- projection/subworkflow totals still enforcing controller resources;
- a second final-entry decision path;
- envelope values sourced from observations/provider claims;
- envelope lookup followed by a racy second generation acquisition;
- default/`None` paths that bypass accounting for a bound run;
- mutable or actor/proposer-derived account attribution;
- controller bytes charged at both publication and terminal aggregation;
- release on cancellation/absence without authoritative proof;
- retry reuse of an uncertain reservation;
- ordinary execution accidentally subjected to controller-only metering;
- redb rows not covered by exact schema/integrity validation;
- public abstractions that do not reduce duplicated policy or lifecycle;
- modules/crates that exist only as named boxes;
- stale docs, fixtures, tests, constructors, or exports.

Review production files near the cohesion/backstop limits and split only real responsibilities into named child modules. Remove obsolete code rather than wrapping it.

## 2. Adversarial functional proof

Build independent tests from public/runtime/store behavior. Do not only reuse implementation helpers. Ensure all of these are demonstrated:

### Atomic boundary

- With a controller process-admission limit `N`, concurrent attempts from independent runtime/store handles produce exactly `N` newly accepted final-entry commits and exactly `N` adapter executions; candidate `N+1` is durably rejected without execution.
- Exact equality succeeds for cost, units, artifact bytes, and category counts; one positive unit beyond each limit fails.
- A stale account/event plan cannot commit after another transition changes the account.
- Dropping a prepared handle on denial/conflict releases the capability permit and does not poison later ordinary work.
- Final dispatch substitution of request, generation, attempt, lease, run, or authority is rejected.

### Truthful envelopes

- Every production adapter has explicit coverage for each dimension.
- `ResourceObservations` cannot affect controller admission.
- A model/peer request with unknown tokenizer/pricing facts is denied for a controlled run before execution but continues to work ordinarily under existing authority.
- `NotApplicable` is accepted only where the capability cannot consume the dimension; missing information is `Unknown`.
- Currency mismatch fails before entry.

### Retry, cancellation, and uncertainty

- A crash/uncertain return after reservation retains the original obligation through reopen.
- A retry has another identity and cannot fit by reusing the original reservation.
- A late authoritative terminal settles only its original reservation once and does not erase the uncertainty event.
- Pre-final-entry cancellation creates no charge.
- Post-entry acknowledged cancellation without complete authoritative usage does not release unknown cost/units.
- Missing usage blocks later cycles even if the numeric reservation alone would leave apparent space.
- Usage above the envelope records visible durable violation evidence and blocks all further admission.

### Artifact publication

- Invocation publication consumes only its exact reservation and direct runtime/context publication consumes the run-bound account.
- Exact remaining logical bytes commit; one extra byte leaves no logical metadata/accounting commit.
- Concurrent descendant runs cannot overspend one account.
- Same-publication replay charges once.
- Same digest under a distinct logical artifact/publication charges logical bytes again.
- Aborted and orphan-cleaned temporary streams charge nothing.
- Fault injection around temp write, content rename/fsync, metadata commit, and account commit reopens to one conservative valid state.
- Terminal/subworkflow aggregation cannot charge the same artifact again.

### Attribution and recovery

- Repeat bodies, nested children, supported detached children, retries, and restart retain one exact account.
- Changing actor, proposer, or current revision cannot change attribution.
- Conflicting binding and a nested marked controller fail closed.
- Current-schema reopen, projection snapshot/compaction, and integrity scan retain exact totals/reservations.
- Legacy v1/v2 marked event histories without account facts fail closed.
- Checksum-correct corrupt account, reservation, binding, and artifact-link rows are rejected.

### Lifecycle and ordinary regressions

- Controller assessment uses settled plus outstanding committed account totals.
- Lifecycle-only bounds—cycles, revisions, proposal shape, elapsed time, failures, rejections, depth, checkpoints—retain their prior exact behavior.
- Humans, services, ordinary processes/models, peer execution, authority, idempotency, and uncertainty remain on their existing shared paths.

Any weak or absent case must be implemented and tested now.

## 3. Schema and API review

Verify every persisted or wire-semantic change has one intentional current version, strict reader behavior, canonical re-encoding fixture, and accurate reference text. Confirm old physical redb roots are refused rather than partly opened and that no implicit migration/reset was introduced.

Run the repository’s public-API inventory procedure for changed library crates. Reduce accidental public surface, but do not hide real cross-owner contracts. Ensure all open interfaces with multiple implementations use one reusable conformance suite.

## 4. Required evidence

Run the full local gate:

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
cargo test -p milkdrift-evidence --test repository_contracts --all-features
```

Run all seven current-source mutation shards, not only the files changed most recently:

```sh
cargo mutation-evidence authority
cargo mutation-evidence retention
cargo mutation-evidence runtime
cargo mutation-evidence uncertainty
cargo mutation-evidence controller
cargo mutation-evidence context
cargo mutation-evidence peer
```

Do not accept unclassified survivors or timeouts. Add independent assertions for real misses. Use only the exact accepted classification categories with reviewed mutant identities.

Run the operational evidence commands from `docs/development/workflow.md`, the effect-worker shutdown proof, all four existing manual release-mode longevity lanes, and the new controller-admission longevity lane. In particular run the controller checkpoint/restart lane with `--ignored --exact --nocapture` and ensure it now exercises the durable account path rather than a fake/projection-only path.

Run the hermetic external-evidence fixture. Then run the strict real external-evidence workflow exactly as documented with operator-supplied, byte-pinned coding-agent and supported model-endpoint resources. Preserve only the redacted qualifying report outside source control. A fixture, mock endpoint, unavailable credential, dirty source tree, or partially completed scenario does not satisfy ADR 0027.

## 5. Production activation gate

Install `ControllerLifecycleOwner` in `apps/daemon` only when all of the following are true in the current source state:

- every shared-contract invariant and hostile case passes;
- full-store integrity/restart evidence passes;
- all required mutation shards pass;
- current controller-admission and existing longevity lanes pass;
- the full local gate passes;
- a current qualifying real external-evidence run passes;
- there is no known bypass, unknown accounting path, or unreviewed survivor.

When qualified:

1. Install the existing lifecycle through the normal daemon composition root before runtime recovery and before admission opens.
2. Ensure controller-account integrity is verified before marked histories are recovered.
3. Add daemon integration tests proving a marked controller can activate through the production composition, exact resource denial prevents adapter execution, restart preserves the account, and CLI/control commands cannot bypass it.
4. Remove the deliberate production refusal and any obsolete branch used solely to keep the owner uninstalled.
5. Update only canonical architecture, status, roadmap, daemon-operation, evidence, and reference facts. Do not rewrite ADR history; add a new ADR only if the final design materially differs from ADR 0027.
6. Make the support statement narrow: process/controller combinations with complete enforceable envelopes are supported; model/peer generations with `Unknown` dimensions remain refused until separate metering work closes them.

If any qualification prerequisite cannot be executed or fails, **do not install the lifecycle**. Repair code/test defects that are within the repository. For unavailable operator credentials/runners, leave the production refusal intact and report the exact unsatisfied command/resource and the evidence already completed. Do not add a bypass, downgrade a hard limit to advisory, or claim the roadmap item closed.

## Completion report

Return a concise, evidence-based report containing:

- defects found in the previous pass and exact repairs;
- the final canonical ownership/entry/settlement path;
- hostile, restart, artifact, mutation, longevity, operational, and external-evidence results;
- schema/public-API changes and compatibility behavior;
- whether daemon activation was performed and the precise proof allowing it;
- if activation remains closed, the single exact remaining blocker rather than a speculative next-phase list;
- the final commit/patch identity and clean-tree status.

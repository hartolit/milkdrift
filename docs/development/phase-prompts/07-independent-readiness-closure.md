# Pass 7 — Independently audit, repair, qualify, and freeze

Independently review the complete repository produced by passes 1–6. Do not trust prior completion reports, metric claims, architectural choices, or test updates. Reconstruct the intended system from governing documents and current externally observable behavior, then modify the repository to repair every defect within scope.

Follow `00-pristine-readiness-contract.md` in full. This is an implementation-and-qualification pass, not a report-only audit.

## Primary outcome

Reach one of two honest conclusions:

1. **Maintainership-ready pre-UI kernel:** all in-repository readiness conditions pass; the architecture is frozen; unavailable hosted/credential evidence remains one explicit external qualification limitation.
2. **Not ready:** repair everything possible, leave the architecture unfrozen, and report one exact remaining in-repository blocker with reproducible evidence.

Do not use “more cleanup may be useful” as a blocker. Do not declare readiness while a known semantic, composition, test, documentation, or operator-path defect remains.

## 1. Reconstruct the implementation from source

Without relying on prior summaries, trace these complete paths:

```text
A. CLI mutation
   -> protocol/client
   -> daemon authentication/authority
   -> runtime acceptance/idempotency
   -> durable result

B. task execution
   -> eligibility/lease
   -> capability resolution/generation
   -> final authority/resource entry
   -> adapter report
   -> terminal or uncertainty

C. artifact
   -> publication intent/chunks/commit
   -> accounting/provenance
   -> authorized range read

D. restart
   -> schema/open checks
   -> closed recovery
   -> projection/index reconciliation
   -> adapter/peer/worker start
   -> readiness

E. live edit
   -> proposal/revision
   -> risk/approval
   -> reconciliation plan
   -> prospective adoption
```

For each path, identify one owner for every decision and one independent test through public or narrow owner behavior. Search the whole workspace for alternate implementations, bypasses, compatibility paths, duplicate validators, and stale terminology. Delete or repair what you find.

## 2. Verify package and public-surface contraction

Re-run the package-boundary and public-API analysis from pass 1.

Confirm:

- no ownerless core/common/types crate appeared;
- blueprint, capability, and contract mechanics remain separated or were merged only with a demonstrably simpler dependency graph and preserved semantics;
- every package still earns its boundary;
- public test helpers and obsolete compatibility items are absent from default features;
- adapters, protocols, durable schemas, and semantic owners remain narrow and correctly directed;
- the CLI has only its intended Milkdrift dependencies and performs no semantic-owner work;
- evidence tooling remains a development-only leaf.

Repair accidental exports, re-exports, package cycles, forwarding layers, and unused dependencies now.

## 3. Verify structural contraction rather than metric gaming

Compare the final tree with the recorded pass-1 base commit using meaningful diffs and before/after metrics.

Inspect every production file above 1,000 lines and every test file above 1,000 lines. Reject:

- arbitrary file splitting;
- `include!` or wildcard façade tricks;
- minified formatting;
- giant passive context bags;
- macros that hide ordinary control flow;
- moved duplicate implementations;
- public abstractions with one implementation and no independent consumer;
- reduced tests that merely stopped checking difficult behavior.

The following must be true:

- no production Rust file is at or above 1,500 lines unless one independently defensible exhaustive owner remains and its local rationale names the exact invariant;
- the cohesion-exception list is materially smaller than the supplied checkout’s 24 entries and contains no generic “complete contract” rationales;
- `too_many_arguments` and similar allowances are materially lower and every remaining use is locally justified;
- production, test/evidence, public-API, and dependency metrics show real contraction in the scopes changed;
- no pass increased conceptual layers merely to decrease a line count.

Repair violations instead of documenting them.

## 4. Independently challenge tests and evidence

Select representative high-risk invariants and construct or strengthen tests from outside their implementation:

- authority deny/hidden-resource behavior;
- exact command replay/conflict;
- concurrent final entry and zero adapter calls on denial;
- post-entry adapter failure versus worker/clock/panic failure precedence;
- uncertain non-idempotent work across retry, cancellation, late evidence, and restart;
- event replay equals incremental projection;
- controller reservations and artifact settlement at exact bounds;
- redb checksum-valid cross-reference corruption;
- peer acceptance/reconnect/archival without duplicate execution;
- context branch isolation and frozen retry selection;
- owner-queue overload and shutdown dependency ordering;
- CLI JSON error/timeout/stream termination behavior.

Ensure shared helpers do not encode expected semantics. Run all seven mutation shards and fix real survivors. Do not accept unclassified survivors or timeouts.

## 5. Prove routine product operation

From a new temporary directory and built release/debug binaries as documented:

1. generate or copy only the supported operator example—not a test fixture;
2. validate configuration;
3. start the daemon;
4. use the actual CLI for readiness and authority;
5. validate/import a workflow or prompt sequence;
6. run a real byte-pinned local process capability;
7. wait with a hard bound;
8. inspect run, node, attempt, timeline, context/provenance, and artifact output;
9. replay an exact command identity and prove no duplicate work;
10. stop/restart the daemon and re-inspect state;
11. exercise one failure, uncertainty/resolution, or prospective remediation path;
12. terminate every process cleanly.

Run the deterministic loopback model endpoint lane. When a separately managed real llama.cpp/OpenAI-compatible endpoint is available, run the ordinary documented local-model path and verify structural evidence. Do not substitute exact generated text or an evidence-only direct library path.

## 6. Run complete qualification evidence

Run the complete local gate:

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

Run:

- all seven current-source mutation shards;
- benchmark smoke and operational evidence;
- effect-worker shutdown proof;
- every current manual release-mode longevity/stress lane;
- hermetic external-evidence and local-model evidence;
- actual-binary headless CLI evidence;
- public-API inventories for all libraries;
- hosted Windows/macOS workflows and current CI lanes when available;
- strict real external process-plus-model evidence when operator-supplied credentials and byte-pinned resources are available.

A local substitute cannot qualify a hosted or credentialed claim. An unavailable external resource must be recorded precisely, while every code-local gate still passes.

## 7. Decide controller activation separately

Do not infer that general codebase readiness permits continuous controller activation.

Install or retain production controller lifecycle according to the exact current ADR/status/evidence criteria. If real qualifying external evidence, hostile reservation tests, longevity, or another existing prerequisite is absent, leave activation closed. Ensure the CLI, configuration, and grants cannot bypass that refusal.

## 8. Establish the architecture freeze

Only after all in-repository readiness conditions pass:

- update status with exact current evidence;
- reduce the roadmap to genuine external qualification or bounded next product work;
- state in the roadmap or development policy that broad architectural cleanup is frozen;
- require future architectural changes to begin from a measured defect, public operator need, or violated invariant;
- do not add a “final audit” document, readiness report, or prompt history.

The owner should now be able to begin source study using the maintained implementation map and public CLI scenario. Further AI work should explain and challenge the stable implementation before expanding it.

## Final completion report

Return, outside the repository:

- final commit/tree and clean status;
- defects discovered in prior passes and exact repairs;
- the final package/owner/dependency result;
- before/after production, test/evidence, documentation, public-API, large-file, exception, and lint-allowance metrics;
- one-page source trace for paths A–E;
- complete gate, mutation, operational, longevity, CLI, model, hosted, and external-evidence outcomes;
- controller activation state and exact justification;
- readiness decision;
- one exact remaining external or in-repository blocker, if any.

Do not claim readiness from line counts alone. Claim it only when the product is routinely operable, its normal path is singular and independently proven, and the remaining architecture is small enough to learn by responsibility rather than archaeology.

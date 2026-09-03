# Pass 1 — Restore current-head semantic integrity and cross-platform portability

Use this prompt with `00-shared-execution-contract.md`.

## Objective

Restore one deterministic, truthful baseline before any further architectural or product work. Correct the known peer post-entry recovery race, the Unix-only filesystem-authority representation exposed by Windows, and every directly related stale evidence claim.

This is not a request to patch two expected strings. It is a request to remove the ownership inconsistencies that made the current behavior race-dependent and platform-dependent.

## 1. Reproduce and trace the current failures

Start from the current checkout and run the focused tests before editing.

Known regression targets from the 2026-09-01 main checkout include:

- `lifecycle::post_entry_clock_failure_retries_recovery_until_uncertainty_is_durable` in the peer service suite;
- the Windows `milkdrift-local-process` process-execution tests failing because a canonical Windows path is rejected as an invalid `FilesystemScope` root;
- the Linux quality and cross-platform workflow conclusions disagreeing with the success claims in `status.md` and `verification-evidence.md`.

Inspect at least:

- `adapters/peer-http/src/dispatch.rs`;
- `adapters/peer-http/src/service/worker.rs`;
- peer persistence transitions and fault tests;
- `crates/runtime/src/executor.rs` and capability-host post-entry error classification;
- `crates/authority/src/model/resource.rs` and `crates/authority/src/evaluator.rs`;
- local-process path binding, profile parsing, authority requirements, platform code, tests, and daemon configuration;
- peer and external-evidence filesystem scopes;
- the current GitHub workflow logs and source documentation that claims validation results.

Map every producer and consumer before choosing a representation.

## 2. Establish one owner for peer recovery meaning

The worker loop currently observes a returned error or panic and can retain a generic textual post-entry reason, while `PeerService::run_claimed` and the capability-host/runtime execution path can independently derive a more specific post-entry result. A clock or persistence failure can make whichever path wins the race determine the durable reason.

Replace that ambiguity with one typed recovery contract.

Required semantics:

1. The service/execution boundary owns classification of normal adapter completion, adapter-returned failure, missing terminal evidence, and whether durable entry was reached.
2. The dispatch worker owns only worker lifecycle: claiming, invoking the service operation, retaining an exact recovery obligation, retrying that exact obligation, handling panic, waiting, and shutdown.
3. A returned application/adapter outcome must not be flattened into the same `Err` channel as inability to persist the already-determined outcome.
4. When persistence or the daemon clock is temporarily unavailable after the outcome is known, retain a typed recovery operation containing the exact transition and bounded semantic evidence that must be committed. Retry that operation unchanged after recovery.
5. The worker must not replace a service-derived classification or reason with a generic fallback merely because the first durable write failed.
6. A panic may create its own explicit recovery classification, but it must be derived once from durable phase/entry evidence and retried unchanged.
7. Pre-entry claim release and post-entry uncertainty must remain separate operations.
8. A terminal or uncertain record already committed by another path makes recovery an exact idempotent no-op or replay, never a conflicting second outcome.
9. No recovery path may invoke the adapter a second time for the same entered execution.
10. Shutdown may leave the exact durable claim for startup recovery, but it must not discard in-memory evidence while the worker is still alive and able to retry.

Prefer a closed enum/value representing the recovery transition over a string-bearing `PendingRecovery`. Stable classification should not depend on prose equality. Preserve bounded redacted detail for diagnosis only where the owning durable contract supports it.

If persisted peer records, protocol messages, or public read models change, apply the exact schema/version/fixture rules from the shared contract. Do not change a schema merely to alter a test string.

### Required peer tests

Add or strengthen deterministic tests proving:

- adapter returns a post-entry error while the clock becomes unavailable, then the exact derived uncertainty commits after clock recovery;
- adapter returns without terminal evidence under the same clock failure;
- adapter panic after entry has a distinct stable classification;
- failure before entry releases the claim and remains retryable without uncertainty;
- a terminal observation committed before recovery is not replaced;
- repeated recovery calls are exact replays/no-ops;
- clock failure, store failure, worker shutdown, and restart never cause duplicate adapter entry;
- the result is independent of thread scheduling and no test relies on racing a clock toggle against an arbitrary sleep.

## 3. Replace the Unix-only filesystem authority assumption

`FilesystemScope` is a durable, pure authority contract. Its current validation and lexical containment assume that every absolute root begins with `/`, while the local-process adapter supplies canonical native paths. Weakening the constructor to accept backslashes is not a complete solution.

Establish one platform-aware absolute-root representation and one component-aware containment rule.

Required properties:

1. Authority evaluation remains pure and deterministic. It must not call the filesystem or depend on ambient host state.
2. Host adapters canonicalize/inspect native paths at their trust boundary, then convert them into the canonical durable authority representation.
3. The representation explicitly distinguishes the path families it supports. At minimum, current Unix absolute roots and ordinary Windows drive-absolute roots used by hosted tests must be representable. Support or explicitly reject UNC roots, device prefixes, drive-relative forms, alternate data-stream syntax, mixed separators, and other Windows forms according to a documented fail-closed rule.
4. `.` and `..`, empty components, NUL, ambiguous roots, trailing-separator ambiguity, and relative paths are rejected.
5. Containment compares normalized components and root identity. Raw string prefix checks are forbidden: `/work/a` must not contain `/work/ab`, and `C:/work/a` must not contain `C:/work/ab`.
6. Different drives, UNC shares, or path families never contain one another accidentally.
7. Case behavior on Windows must be explicit and security-conscious. Do not assume that lowercasing arbitrary Unicode path text reproduces filesystem semantics. Use canonical host evidence or fail closed where exact comparison cannot be represented safely.
8. Broad-root authority must be explicit per supported path family. Do not treat Unix `/` as an undocumented wildcard for every host.
9. The local-process adapter must continue checking that the executable is under an executable root and an authorized host working directory is under a read-write root using native canonical path semantics before converting to authority facts.
10. Daemon configuration, peer execution scopes, external-evidence rendering, grants, requests, digests, read models, fixtures, and documentation must use the same canonical representation.

Do not move OS path behavior into `milkdrift-contracts`; this is authority/path meaning, not generic JSON mechanics.

### Required path tests

Cover at least:

- Unix root, exact match, child, sibling-prefix attack, traversal, repeated separator, and relative rejection;
- Windows drive root, exact match, child, sibling-prefix attack, drive mismatch, drive-relative rejection, slash/backslash normalization at the host adapter, and canonical executable/workspace roots;
- UNC or explicit UNC refusal, according to the selected contract;
- serialization/canonical digest stability and hostile deserialization;
- local-process profile construction and execution on every supported hosted platform;
- daemon configuration and peer/external-evidence scopes using the same constructor path.

The Windows test fixture must derive real native paths on Windows rather than fabricating Unix paths to bypass the contract.

## 4. Restore truthful repository evidence

After the code is corrected:

- run the focused peer and local-process suites;
- run the full local gate;
- run any cross-target checks available locally;
- inspect the hosted workflow definitions and, when repository access permits, the actual hosted results.

Update `status.md`, `verification-evidence.md`, and related evidence prose only to match results that actually exist at the final tested commit. Preserve unresolved hosted limitations until a hosted runner has passed. Do not keep a dated “all pass” statement when the cited checkout has a failing required gate.

Add or strengthen repository checks where practical so current status/evidence claims cannot silently contradict required workflow conclusions or source schema constants.

## 5. Scope exclusions

Do not perform the controller-admission implementation, broad package consolidation, CLI feature expansion, context redesign, provider expansion, or UI work in this pass.

Small private refactors required to establish the two canonical owners are in scope. Unrelated cleanup is not.

## Acceptance criteria

The pass is complete only when:

- peer post-entry outcome classification is deterministic and has one owner;
- a transient clock/store failure retries the exact semantic transition rather than inventing a second reason;
- no entered peer execution can be invoked twice by this recovery path;
- `FilesystemScope` and its containment semantics support the declared hosted platforms without Unix-only assumptions or raw string prefixes;
- every producer and consumer uses the canonical path contract;
- focused and full local gates pass;
- status and evidence documents make no claim stronger than the available results.

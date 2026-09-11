# Assignment 04 — Inspect blocked stores and preserve recoverable generations

## Outcome

Provide an explicit offline/read-only operating path for a store that cannot complete normal startup,
and a verifiable stopped-store backup/restore procedure. The operator must be able to investigate
unsafe retained context without running it, rewriting it, or losing command/peer replay history.

Read the sprint README, `AGENTS.md`, canonical docs, `docs/development/workflow.md`, and the
implementation/documentation practices. This is a scoped storage-operations change. It does not
authorize universal migrations, online destructive rotation, cold-history deletion, or changes to
live stores merely to make startup succeed.

## Source and evidence

The review reports that retained selection-policy-v1 evidence can remain readable but unsafe for
reuse. An affected active lease may prevent HTTP startup. Keep that fail-closed execution rule.
Hot state is bounded; cold command receipts and peer tombstones preserve replay identity for the
store generation. Their growth is an explicit cost, not proof of a leak.

Read:
- `docs/operations/daemon.md` startup and backup/compatibility sections;
- `docs/decisions/0031-context-enforcement-and-retained-evidence.md`;
- `crates/runtime/src/context/retained.rs` and context-enforcement tests;
- daemon startup, owner, read-only query/auth composition and CLI/client commands;
- `crates/persistence` administrative/read interfaces and redb store open, schema, integrity,
  application receipts, controller accounts, peer tombstones, artifact and clock ownership.

## 1. Explicit maintenance/read-only mode

Implement one application/administrator entry point that does not require successful runtime
recovery. Prefer the existing daemon application's composition boundary; keep ordinary CLI clients
thin rather than adding redb access behind normal network commands. An explicitly named offline
storage-admin mode has OS/file-owner authority, not a forged workflow `ActorRef`. Document that trust
boundary. If a network read-only mode is used, retain authentication and resource read authorization.
Do not quietly fall back to weaker access after ordinary startup fails.

Opening this mode must not construct effect workers, contact providers/peers, dispatch tasks, refresh
execution leases, settle accounts, rewrite manifests, advance semantic clocks, initialize missing
schemas, run migrations, clean artifacts, or rebuild indexes automatically. Use a genuinely read-only
supported store path, or inspect a verified isolated copy if the storage library's ordinary open
can write housekeeping metadata. Prove and describe the actual source-modification guarantee.

Acquire appropriate exclusive/offline ownership before physical copying or any operation that cannot
safely coexist with a writer. Refuse ambiguous concurrent-writer situations. Do not reuse execution
startup just to obtain read access.

## 2. Useful bounded diagnostics

Expose supported bounded/paged inspection of schema compatibility, startup blockers, affected
run/attempt/lease identities, frozen context policy/manifests, account reservation/unknown state,
uncertain work, receipts, peer execution/tombstone links, and artifact metadata/digest health. Read
retained data as historical evidence, never as permission to dispatch it. Protect sensitive omission
metadata and content; a diagnostic must not revive disclosure fixed by policy v2.

Distinguish unsupported schema, corrupt record, unsafe-but-readable evidence, and unavailable content.
Return precise limitations instead of silently skipping failed records and calling the store healthy.
Support existing resumable integrity scans as an explicit opt-in operation with artifact hashing
selected deliberately. No mandatory whole-history scan for an ordinary small inspection.

Provide a bounded diagnostic export if needed, with provenance and redaction. Label it an inspection
report, not a runnable backup or migration result. Do not provide raw secrets/grants/working files
indiscriminately.

## 3. Consistent stopped-store backup and restore verification

Implement or finish a usable offline command/procedure that copies a closed **complete** generation:
database, content-addressed artifacts, schema/store identity, durable clock, account/receipt/peer
facts, and every other data-root component required by current readers. Record producer binary/
source and schema versions plus a bounded manifest of integrity information. Do not copy live redb
and changing artifact files independently and call the result consistent.

The destination is explicit and create-new; never overwrite a store or backup. Handle incomplete
copies with detectable partial state and a final completion marker/manifest. Verify the backup using
compatible readers before recommending it. Restoring into an empty isolated location must preserve
run/attempt identities, command replay/conflicts, peer tombstones, accounts, clock high-water state,
and artifacts. Test both normal and interrupted-copy cases. Backups contain sensitive material:
check intended destination ownership/permissions and never include resolved credential values or
secret-source files outside the stated data-root policy.

A restored clone must not automatically reconnect to providers, remote workers, or the original live
control clients. Keep it inspection-only until explicitly authorized; two runnable copies of the same
generation can duplicate effects. Never lower clock facts to make a restored instance pass checks.

## 4. Honest compatibility and disposition

Keep unsupported physical schemas refused. Do not patch version constants or manufacture a general
migration. For an unsafe active attempt, show the existing authorized reconciliation path if usable;
otherwise explain the preserved evidence and supported offline/new-generation procedure. Starting
an empty generation does not settle old effects. Retain old generation identity and require fresh
client/request namespaces as documented; do not replay old commands as new intent.

Do not delete cold receipts/tombstones to achieve a storage-size target. Report their footprint and
preserved idempotency purpose. General export/delete, online rotation, historical migration and
forensic tamper protection remain outside this assignment unless already implemented and testable.

## Tests and verification

Use temporary current-format stores containing unsafe active retained evidence and unrelated healthy
history. Assert normal startup refuses, explicit inspection succeeds without external entry, and
source state remains unchanged according to the implemented read-only guarantee. Include protected
metadata, corrupt/missing artifacts, unsupported schema, live-writer refusal, pagination, interrupted
backup, restore into existing destination refusal, exact receipt/peer replay after isolated restore,
and prevention of accidental dual execution.

Run redb admin/contracts, retained-context/runtime, daemon/CLI parsing and actual-binary maintenance
scenarios, documentation contracts, then the full executable gate. Use deterministic failure hooks,
not manual row edits against an operator's store.

Update `docs/operations/daemon.md`, compatible binary/schema guidance, current status and actual CLI
examples. Finish when an operator can diagnose and preserve a blocked generation without weakening
execution validation. State precisely what can be inspected/recovered and what remains unsafe to run.

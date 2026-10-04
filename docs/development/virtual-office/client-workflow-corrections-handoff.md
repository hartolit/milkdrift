# Client workflow correction handoff

Owner: this assignment's primary agent, including all workspace Cargo jobs. Starting source:
`15157c0acfe4e7f73dbb0eec2b5a5d9e68321f53`, clean working tree. The hosted quality failure is
confirmed through GitHub; raw metadata/logs live under ignored `target/client-workflow-corrections/`.

Checkpoints: bounded diagnostic collection and fixture cleanup; each demonstrated progress cause
with sensitive regression; complete named-workflow scope adoption; final acceptance and separate
office closeout. Full verification is reserved for the assignment's final section; intermediate
focused passes do not settle the known hosted failures. No push or live server changes authorized.

Diagnostic checkpoint: shared failure-only authorized reads, redacted unique 64 KiB JSON bundles,
three-second total/250 ms per-read bounds, at most sixteen protocol-bounded responses and eight
items per run. Timeout/terminal/early-error paths share collection; the two affected tests catch
unwinding and join owned daemons/model listeners. CI retains only selected bundles. Four focused
tests pass; eight documentation contracts pass; focused Clippy and fresh application/helper build
pass. Logs use the `diagnostics-*` and `registration-docs.log` names under the evidence root.
No progress cause or final acceptance is yet claimed.

The declared reproduction campaign completed: both isolated cases passed (31.68/34.03 seconds);
three ordinary parallel suites returned 41/42, 42/42, 42/42. The additional failure was a binary
daemon exit before readiness; its discarded logs cannot establish a cause. Startup failures now
retain bounded private logs plus a selected redacted CI bundle (`39d9677`); its two focused tests
and Clippy pass. No retry result replaces the failed campaign.

The owner ran full maintenance after every request, including worker clock requests. A debugger
snapshot found effect entry waiting on that queue while its owner replayed history during effect
claim maintenance. The production regression brackets one request on the owner thread and proves
six unwanted clock samples on old behavior; it passes after interval-based maintenance. Both
original cases then pass in 6.62/10.91 seconds, with unchanged deadlines, capacity and overlap.
Eight owner tests, all-target daemon Clippy, fresh binaries and the ordinary 43-test suite pass
(51.15 seconds). Evidence: `maintenance-*` and `reuse-debugger.log` in the evidence root.

Maintenance fix is committed at `d4ecdf4`. Named-workflow implementation is ready for its checkpoint:
bounded authority-owned sets, strict wildcard decoding, normal matching and configured public copy,
explicit workflow filters on collection reads, protected individual reads and unchanged old grant
bytes. Authority/refusal/frozen-basis tests, scoped CLI copy, independent actual-daemon JSON
replay/reopen, all-target Clippy and eight documentation contracts pass. New public surface is
limited to the durable scope variant and validated set; inventories live under
`target/public-api/client-workflow-corrections/`.

Scoped copy is committed at `a08038e`; its 33-check static gate and sixteen scanner probes pass.
The additional one-CPU comparison ran each original case once on `39d9677`: both pass at
40.46/40.40 seconds. It does not reproduce the hosted deadlines; the deterministic maintenance
regression is the cause-sensitive evidence. The disposable worktree is removed. Shared-target
reuse then produced a zero-test filter; that result is discarded and the affected package caches
were invalidated before rebuilding. No zero-discovery command counts as coverage.

Binary fixture startup has a demonstrated released-port race. A child-selected-port regression
fails before the fixture can discover the bound address, then passes after using its structured
startup output. Initial configuration now requests port zero and saves the actual bound port for
reopen. The earlier campaign startup failure has no retained logs, so its historical cause cannot
be proved. Focused startup/CLI/JSON cleanup and recovery checks precede this fixture checkpoint.

Fixture fix is committed at `479f255`. The corrected one-CPU runs pass at 8.93/13.88 seconds.
At that source the strict gate passes 33 checks, probes record 13 passes/3 expected refusals,
and the ordinary full workspace passes 1,117 tests (24 doctests, 30 repository contracts),
with eight ignored cases. Discovery, headless, model and controller scenarios pass. A second
controller run on preserved application binaries fails; that failure supersedes local acceptance.
All results remain under `settled-*` and `accepted-*`, including exact binary checksums.

The controller failure is retained in `accepted-controller/`. Read-only stopped-store inspection
(`controller-stopped-*`) and authenticated recovery on a private stopped copy
(`controller-inspection/`) identify an uncertain process report: another branch held an artifact
publication, causing `OwnerBusy`. Report ingestion treated that temporary commit conflict as a
lost effect, so its parent correctly waited. Inspection is stopped and joined. The original store
is preserved; raw stores and logs are private and are not CI uploads.

Runtime now retries publication-owner and workspace-usage conflicts within the existing sixteen
commit attempts, rebuilding the same report without re-entering its adapter. Final-entry retries
use the same classification and fresh time/authority checks. Production redb fault injection
proves one entry/terminal/reopen across temporary conflicts, finite exhaustion and refusal of
unrelated storage failures. The regression fails with the prior conflict classification and passes
with the correction (`report-contention-old.log`, `report-contention-fixed.log`). This is fault
injection at the real precommit boundary, not another reproduction of the original hosted stalls.

Focused verification passes: runtime all-target/all-feature Clippy, 206 runtime tests (one existing
ignored case), fresh applications, and the complete controller qualification. Its competing
admissions record two entries and one denial, followed by successful crash/reopen checks.
The report-contention fix is committed at `199d1a5fc86c001972cf205a2b37bb749907c37e`.
Final `corrected-*` verification passes all 33 strict checks and 1,119 workspace tests (including
24 doctests and 30 repository contracts), with eight existing opt-in cases ignored. Discovery
confirms every affected test. All 45 control-plane cases and the maintained authored journey pass.
The six preserved application binaries pass headless, deterministic-model and complete controller
qualification; their SHA-256 checks still match afterward. All owned test children are settled.
Historical live/native evidence remains historical. Hosted query returns no corrected run; no push made.

Next: check and commit canonical acceptance documentation, then separately remove this completed
assignment/handoff and update the office/roadmap links with documentation-only verification.

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

Next: commit scoped copy, then run final strict/workspace/discovery gates and fresh binary scenarios.
Historical live/native evidence remains historical unless an affected owner requires a new
finite qualification.

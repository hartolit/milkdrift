# 03 — Read results and fix future steps

After 02 is ready, follow [the shared rules](README.md),
[implementation practice](../../practices/implementation.md), and
[documentation practice](../../practices/documentation.md).

Start with public run/result reads, `crates/control-client`, CLI inspection/run/artifact/proposal
commands, and the existing control and result-acceptance code. The daemon supplies facts and
permitted actions; the CLI formats them rather than calculating its own execution state.

## Build this

Give a compact answer to: what is running, what finished, what failed, and what can I do next?
Show the workflow name, version, run, current step, outcome, and final output. Offer more detail
for model choice, inputs, limits, usage, and failure evidence. Do not require digging through raw
JSON to find the file containing the result.

Keep important differences visible: the daemon accepted a request; the model returned; the result
passed its required checks; the workflow finished. These are not interchangeable. Empty, truncated,
or uncertain output must not be displayed as accepted success. Unknown usage stays unknown.
Use safe terminal rendering, bounded output, and verified downloads. Do not expose secrets or
private internal results to callers who only have permission to invoke a published workflow.

Reconnect observation using existing reads and event feeds. Handle duplicate events and expired
cursors with an authorized fresh view, not guessed state or an unlimited history download.
Closing the view does not stop the run. Keep errors actionable without assuming the user has
permission to take every suggested action.

Distinguish editing a saved workflow for later runs from changing future steps of an active run.
The latter uses existing proposals, permissions, approval rules, and the exact current version.
Show the affected work before applying a consequential change. Preserve completed steps and their
history. Reject stale or unauthorized edits. Never remove a failed requirement to make a run pass.

Use the example's existing pause/review point for a legitimate repair. The repair may use the
selected failed result and original brief, not unrelated private history. An ended run remains
ended; offer an explicitly linked new run instead of pretending that resume can undo a terminal
failure. Do not add an automatic repair engine or weaken protected checks.

## Check just this work

Test progress/result presentation for complete, empty, truncated, refused, and uncertain responses.
Test safe downloads, reconnect, duplicate events, and fresh views after a stale cursor.
Test an allowed future-step repair, stale/unauthorized refusal, protected-check refusal, and the
ended-run case. Verify old completed records are unchanged. Use existing owners and controlled
model responses, not a fixture that fabricates the desired final state.

Compile the affected packages/callers and run these focused tests. The complete scenario is for 06.

## Commit points

Commit useful status/result reads and CLI display; then reconnect handling; then permitted repair
commands and their tests. Keep each change working, and update the existing recovery guide alongside it.

Stop when the user can understand a result and take a permitted next step without database knowledge.
Leave the commands, commit IDs, checks, and supported repair point in `handoffs/03.md`.
Do not run the full workspace gate.

# 06 — Test the full system and fix problems

After 05b has cleared the strict checks and its handoff is accepted, follow [the shared rules](README.md),
[implementation practice](../../practices/implementation.md),
[documentation practice](../../practices/documentation.md), and
[office closeout](../README.md).

**This prompt owns the full-system test and all fixes needed to pass it.** Do not merely report
failures or open another sprint for known problems. Read the short handoffs, inspect the finished
code, and test the actual clients and daemon. No new feature expansion or GUI work.

## Set up safely

Record the source commit, remaining edits, toolchain, binaries, configuration, and available
permissions. Use fresh private test directories and owned temporary resources. Know what cleanup
may remove. Do not modify live services, reboot machines, change privileges, or use paid providers
without explicit authorization. An old hostname is not proof of access or permission.

Use controlled model responses for repeatable failure tests. For real-model checks, use an actually
authorized local endpoint and declare finite request, output, and time budgets first. Keep native
or attached inference supported; do not add container setup simply to run this example.
Preserve failed outputs and clearly distinguish fixture responses from real model responses.

## Test the complete user journey

Build actual daemon and CLI binaries. Use the maintained example and public operations, not a
privileged in-process test runtime or helpers that create successful product records behind the API.

1. Follow the documented one-time setup. List permitted models and show a useful refusal for an
   unavailable choice. Create the release-notes workflow from scratch, add its prompts and two steps,
   connect its brief and draft, save, close, and reopen it. Keep a second workflow independent.
2. Run the first brief. Inspect progress, the required result checks, and the final text. Retrieve
   the result without finding internal IDs in raw JSON. Repeat through the non-CLI client route.
3. Lose a start reply, close the client, restart the daemon where supported, and reconnect. Recover
   the same request and inputs. Confirm work was not started twice using an independent fixture
   counter. Resend changed bytes under the same request key and confirm rejection. A local timeout
   must not claim the work was cancelled.
4. Return an empty or incomplete required response at the supported review point. Inspect the
   failure, make an allowed change to future work, and continue. Refuse stale/unauthorized changes
   and attempts to weaken checks. An ended run stays ended; completed history stays unchanged.
5. Save and reuse the workflow with the second brief. Confirm separate inputs, writable state, and
   outputs. A later edit to the saved workflow must not alter already accepted runs.
6. Publish a reviewed version, call it with invoke-only permission, and retrieve only public output.
   Test retirement/replacement and replay. Inspect the existing eligible, rejected, and inconclusive
   evaluation cases; use real comparison/promotion code, not fabricated success records.
7. Clean up only disposable test-owned resources. Keep selected definitions, requests, and results.
   Leave attached model servers and unrelated files or services untouched.

Also run the two briefs through the authorized real model, using the same authored workflow and
public commands. Check for a complete result and separately describe its usefulness. Do not swap
in assistant-written output and attribute it to the model, relax required checks, or run unlimited
retries. A learning candidate does not have to win. Missing access or poor model output must be
reported accurately, not counted as a passed live test.

## Run the full checks

Run the current full gate in `docs/development/workflow.md`: workspace build/check/tests, formatting,
Clippy, documentation, dependency checks, and test discovery. Include the complete CLI/daemon journey,
the independent-client tests, and changed example/documentation contracts.

Retain existing regression coverage for direct/peer execution, input privacy, model continuation,
accounting, protected publication, Slotbook, and nested parent/child resource use. In the latter,
a waiting parent must not block its authorized child's editing or allow unrelated conflicting
writes/removal. Unknown stop state must still prevent unsafe reuse. A simple release-notes test
cannot replace these checks. Hardware-specific tests retain their explicit scope; do not claim a
new UM790 or power-loss qualification from an ordinary software pass.

Put new deterministic regressions into normal discovery. Keep CI intact and real-model/hardware
checks explicitly selected. Do not hide required failures with ignored tests, weaker assertions,
blanket timeout increases, or broader permissions.

## Fix failures in small commits

For each real failure, locate the responsible code, add or improve the regression, fix the cause,
run the focused test and affected callers, then commit the working fix. Keep API changes and their
required consumers together. Preserve one commit per coherent fix where possible, rather than one
large final repair commit. A failing test is not automatically wrong; investigate before changing it.

Collect fixes using focused reruns. Do not restart the entire long test sequence after every small
fix. When executable fixes have settled, run the full gate and combined journey on that final code.
If they reveal another defect, repeat the focused-fix cycle and then verify the final code again.
Report exact tested commits; an earlier pass is not evidence for later executable changes.
For documentation-only closeout edits, rerun the affected document/example checks rather than the
entire unchanged runtime suite.

## Finish only when the evidence supports it

Update the existing guide, status, roadmap, and evidence record with tested behavior and actual
limits. Keep Svelte as the first future GUI and the daemon as the owner of product behavior.
Do not claim browser readiness, autonomous coding, or useful live learning from this sprint.

Record commits, checks, log locations, defects fixed, and any genuine blocker in `handoffs/06.md`.
Keep logs under ignored `target/client-ready-workflows/` or CI. If required access is unavailable,
finish all executable work and keep acceptance explicitly blocked; do not silently skip it or
remove the active sprint unless the user accepts a narrower scope.

Once all required checks pass, move lasting facts into existing documentation and follow office
closeout. Remove only this completed sprint's files and entry. Commit that documentation/closeout
separately from code fixes. Preserve the earlier small commits; do not squash them or push without
separate authorization. The result is a tested usable CLI product, not an automatic start on Svelte.

# Canvas experience implementation handoff

## Current assignment

The user explicitly assigned S01 on 2026-10-24. Its complete lifecycle boundary, focused checks and
public-client/browser evidence are finished. S02–S05 and the remaining main sprint are
unassigned. The completed planning is preserved at `851d216d`. No full-system pass or actual user
acceptance is claimed by this record.

S01 repairs explicit waits, frozen starts paused before entry, restart recovery and temporary
native project contention. Runtime owns scheduling and semantic control guards; the host releases
prepared resources and known-unentered claims; existing public clients expose the same facts.
Singleton Pause/Continue/Stop controls may tolerate progress-only generations while preserving
decision guards. Ordinary authoring retains its exact guard until S02.

## Delivered boundary

The runtime now owns one scheduling path for first dispatch and safe redispatch after pause,
contention or restart. Temporary native overlap has its own typed result; corruption and changed
identity retain refusal. Prepared handles drop before safe cleanup. Storage errors retain bounded
delivery/reconciliation ownership, while uncertain entry or cleanup forbids automatic re-entry.

Control 3.5 adds summary entry intent and adopts it in Rust clients and generated TypeScript.
Physical/internal formats stay 21/22. Exact receipt replay and accepted frozen records are preserved;
the implementation did not reset an existing store or replace a running user service.

The existing canvas exposes direct Pause, Resume and Stop, preserves unrelated drawings and selected
tasks, and distinguishes Preparing/Waiting before entry from entry intent. Recorded lifetime bars
include preparation and waiting, so they no longer imply actual execution overlap. This does not
implement S02's saved sketches, blank task placement or right inspector.

## Verification and reproduction

The [canonical S01 evidence](../../verification-evidence.md#side-s01-pause-recovery-and-project-contention)
records exact targets, case groups, counts and limits. It includes 60 durable runtime cases, the
complete affected host/control/daemon targets, storage and peer checks, CLI/protocol/client tests,
all 30 repository contracts, scoped Clippy/rustdoc and default/all-feature public API review.
The actual-binary CLI/public-client journey also passes. Raw logs use ignored `target/s01-*`.
Frontend check/lint, 115 unit tests and production build pass. The complete browser run passed 39
cases; one two-client case exceeded its overall 60-second budget after an accepted response. Its
individual waits remained within bounds. With only that test's overall budget raised to 120 seconds,
the case passed on a fresh fixture against the same production build. This is passing coverage of
all 40 cases, not a claim of one green 40-case invocation. All three new S01 browser journeys pass.
Desktop pause/busy and 390-pixel recovered-task renders were inspected and are retained under
`target/canvas-experience/s01-browser-final-2/test-results/`. The focused conflict rerun is under
`target/canvas-experience/s01-browser-conflict-final/`.

For focused reproduction, run the maintained cases against production redb/runtime and the actual
daemon/CLI; the test fixtures control external preparation, clocks and storage faults:

```sh
cargo test -p milkdrift-runtime --test durable_runtime --all-features tasks::lifecycle
cargo test -p milkdrift-capability-host --test effect_worker --all-features deferral::
cargo test -p milkdrift-daemon --test control_plane --all-features explicit_wait_survives_input_restart_and_replayed_earlier_continue
```

Use the [evidence harness guide](../../../../tools/evidence/README.md) for a fresh `serve --canvas-cases`
fixture and the [web guide](../../../../apps/web/README.md) for the production build and maintained
browser command. Browser fixture roots contain credentials; publish only explicit redacted reports,
never `browser.json`. Preserve consumed roots and use a new one for a complete suite.

For ordinary use, run matching 3.5 daemon/client builds with the existing operator configuration.
Open a workflow and use **Pause workflow**; accepted work that has already entered may continue.
Supplying input leaves the human pause visible. **Resume workflow** permits eligible work to continue
without applying unrelated drawing edits. **Project is in use** needs no invented dependency: the
daemon keeps the accepted start and retries when access becomes available. Recover an unconfirmed
control reply through its exact receipt before making another change.

## Remaining work

The prior diagnostic harness is historical evidence; maintained regressions now own these cases.
S05 retains the full side-sprint gate, and main 07 retains its later integrated gate. Actual user
review and physical Podman qualification remain open. Controlled model responses and phone-sized
Chromium renders establish neither real-model quality nor physical-phone/HTTPS qualification.

S01 is complete with no known in-scope defect deferred. Stop ready for
[S02](S02-saved-canvas-authoring.md); it is not authorized by S01 completion. The main sprint stays
unfinished, and no full-system or hosted CI pass is claimed for this source.

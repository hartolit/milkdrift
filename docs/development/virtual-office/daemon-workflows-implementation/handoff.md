# Prompt 03 handoff

Prompt 03 is complete locally. The daemon accepts model-authored future tasks through the same
guarded batch transaction as humans, runs bounded model/file/process tools, schedules parallel work,
preserves failed history and adopts corrections for future consumers. CLI and Svelte expose the
complete batch, limits, wait decisions, frozen inputs and cause of follow-up work.

The user authorized proceeding with actual prompt-02 review still pending. Agent browser checks do
not complete that review. Prompt 04 is ready for a separate assignment; prompts 04–07 were not started.
Prompt 07 owns the integrated full-system gate. No push, history rewrite, external service change or
paid model use occurred.

## Tested checkpoints and evidence

Backend checkpoint `34564c9` contains shared admission/recovery, input/correction policy, cumulative
limits, model/tool execution, native path protection and public daemon projections. Checkpoint
`566fe72` adds CLI batch preview and the controlled automatic journey; `ede0147` presents final model
answers before retained provider metadata. Svelte checkpoint `2c94cad` contains the complete batch
editor, model/tool choices, provenance and verified repair/navigation fixes.
Checkpoint `0d9a774` additionally refuses model action policies on ordinary process capabilities.
Earlier checkpoints, including `37aa8c4`, `95a4dc5` and `c9e9809`, remain preserved. The prior
review handoff is available at `179e189`.

[Verification evidence](../../verification-evidence.md#prompt-03-evolving-task-acceptance) owns the
commands, logs and qualified claims. Focused owner suites, 46 runtime cases, eight production model
admission cases, 31 daemon control-plane cases, 30 repository contracts, API review, Clippy/rustdoc
and dependency audits pass. Frontend check/lint, 43 unit tests, build and all 11 Chromium cases pass.
Desktop and 390-pixel phone-sized views were inspected. These are local focused results, not hosted
CI or full-system acceptance.

The actual-binary report `target/daemon-workflows/prompt03-check-d/check.json` observes one initial
planner, three accepted model batches, eight retained tasks, native tools, actual research overlap,
an explicit failed outcome, correction/rebinding and restart with no duplicate work. The fixture
creates no descendants and sends no continuation batches. Transaction fault tests cover before/after
commit separately from the completed-work binary restart.

Controlled endpoints establish integration, not model quality or implementation of a real animation
system. No configured authorized real model was exercised. Native file tools require Linux and
bounded UTF-8 input; other platforms refuse them. Existing process profiles remain ordinary
capabilities. Methods, remote administration and rolling retention remain later assignments.

## Local review fixture

The latest bounded fixture root is:

`/tmp/milkdrift-prompt03-browser-20261017-g/`

It runs immutable daemon/fixture copies with a four-hour lifetime. The daemon executable passed the
final 31 control-plane cases. This root uses temporary storage after repository filesystem linking
stalled and a fresh-store startup timed out; it may disappear at host reboot. Its `ready.json` records
current endpoints and process paths; `browser.json` is private. Start the independent production
static preview from the repository root when needed:

```sh
npm --prefix apps/web run preview -- --port 4173 --strictPort
```

Open `http://127.0.0.1:4173/` and add a named connection using the matching row:

| Connection | Daemon URL | Expected installation | Private credential file under the fixture root |
| --- | --- | --- | --- |
| Studio A | `http://127.0.0.1:35793` | `host:browser-a` | `daemon-a/operator.token` |
| Studio B | `http://127.0.0.1:38757` | `host:browser-b` | `daemon-b/operator.token` |

Paste the matching credential from its local file; never put it in a URL, screenshot or committed
document. The browser retains it only in memory. The two daemons have separate actors and stores.

For input-dependent process work, choose **cargo-report · process.execute**, supply a brief and upload
the root or runtime `Cargo.toml`. Results describe the supplied manifest. Close the client while work
runs, reconnect and discover the retained result.

For the controlled automatic journey, choose **controlled-planner · model.generate**, enable
**Allow model to propose future work**, supply an animation-research brief and use instructions
starting with `Animation fixture role: planner`. Leave **Finish when required work settles** unchecked;
the model review requests finish. Observe parallel research, failed render research and its correction.
Select tasks to inspect frozen inputs, source admission and results. Use **Edit future batch** to stage
and preview a guarded batch. Conflicts retain drafts for explicit review; lost replies retain the exact
request identity.

If the fixture expires, preserve its root and choose an unused new root:

```sh
cargo build -p milkdrift-daemon --bin milkdrift-daemon \
  -p milkdrift-cli --bin milkdrift -p milkdrift-evidence --bin connected-tasks
target/debug/connected-tasks serve \
  --root target/daemon-workflows/prompt03-review-new \
  --daemon target/debug/milkdrift-daemon --lifetime-secs 14400
```

Read that root's `ready.json` for newly allocated URLs. The command refuses an existing root. Current
task documents use schema 3 and redb internal format 22 (physical schema 19); old stores are refused,
not migrated or reset. Earlier failed roots, prior evidence stores and preservation Git refs remain
intact. Open old roots only with their matching retained executable/configuration. The handoff leaves
prompt 02 pending and is ready for explicit prompt-04 authorization.

# Daemon workflows implementation handoff

Prompt 03 is complete locally. The daemon accepts model-authored future tasks through the same
guarded batch transaction as humans, runs bounded model/file/process tools, schedules parallel work,
preserves failed history and adopts corrections for future consumers. CLI and Svelte expose the
complete batch, limits, wait decisions, frozen inputs and cause of follow-up work.

The user authorized proceeding with actual prompt-02 review still pending. That review has now
begun and produced the corrections below. Agent browser checks do not complete it. Prepared prompts
02a/02b now precede completion of the early review and a separate prompt-04 assignment; none of
02a/02b or 04–07 has been implemented.
Prompt 07 owns the integrated full-system gate. No push, history rewrite, external service change or
paid model use occurred.

## Actual user review and corrections

On October 17, the requesting user reported, “I added Studio A,” using the production Svelte preview
at `http://127.0.0.1:4173/`. The local source checkpoint was `9fe2915` with the completed prompt-03
build. This is the user's report of a successful connection, not an agent-observed click trace.
The desktop browser/client environment is known; its browser engine and physical-phone use were
not established. No task brief, uploaded input or useful result has yet been reported in this review.

The user found connection setup too complicated, questioned the local name and installation identity
fields, and requested a daemon-owned name whose changes persist across client GUIs. They strongly
disliked the current UI, supplied an earlier mockup as a rough visual guide while explicitly noting
its outdated features, and pointed out that the real logo was missing. They asked for corrective
prompts to keep the sprint aligned, rather than immediate implementation.

Source inspection confirms browser-local profile labels, manual identity entry and a literal `m`
brand mark. The installation identity is the daemon's stable `host_id`, not a device-registration
field. The authenticated session already returns it. Later phases previously included broad review
and repair but no explicit assignment for this visual direction or browser naming change. Phase 05
owns A-to-B execution-host relationships, a separate boundary.

| Finding | Prepared owner and acceptance |
| --- | --- |
| Setup requires users to invent a local name and supply a technical identity. | [02a](02a-daemon-connection-identity.md): address/credential setup, learned identity pin, durable shared name, authorized rename across clients and restart; repeat connection with the user. |
| Current appearance is rejected and the supplied brand is absent. | [02b](02b-visual-design-and-brand.md): dark workspace, compact navigation, real logo and desktop inspector using current task semantics; user reviews the working result. |
| User review has only reached connection. | Resume [02](02-early-user-review.md) after the corrections: useful input-dependent work, results, client closure/reconnect, daemon switching, missing-input repair, failure and stop understanding remain open. |

The [retained mockup](assets/milkdrift-ui-sample.png) is byte-identical to the user's
`/home/hartolit/Desktop/milkdrift-ui-sample.png`; SHA-256 is
`6e4eaaf841727207fc1e3f92d3c11506ae1693d1ee73f76c9cfb969c0505f575`.
It is source design material, not a screenshot of the implemented app or an acceptance result.
The supplied [logo](../milkdrift-remedial-planning-v3/assets/milkdrift-logo.svg) remains the brand
source; 02b establishes its maintained production location. The mockup's old controls do not
authorize features. Current planning changes no executable code, store, credential or running UI.

Next sequence: assign 02a, then 02b, finish the actual 02 review, then separately assign 04.
Preserve the completed 03 work and 04–07 numbering. Prompt 07 remains the sole integrated gate;
this partial feedback is not user acceptance of the journey or the proposed corrections.

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

`/tmp/milkdrift-user-review-20261017-a/`

It was started for this actual review with immutable daemon/fixture copies and a four-hour lifetime.
The previous prompt-03 browser fixture had stopped and its temporary root was absent. This new root
may disappear at host reboot; verify readiness rather than assuming it still runs. Its `ready.json`
records endpoints and process paths; `browser.json` is private. Start the independent production
static preview from the repository root when needed:

```sh
npm --prefix apps/web run preview -- --port 4173 --strictPort
```

Open `http://127.0.0.1:4173/` and add a named connection using the matching row:

| Connection | Daemon URL | Expected installation | Private credential file under the fixture root |
| --- | --- | --- | --- |
| Studio A | `http://127.0.0.1:34049` | `host:browser-a` | `daemon-a/operator.token` |
| Studio B | `http://127.0.0.1:45853` | `host:browser-b` | `daemon-b/operator.token` |

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
prompt 02 open and is ready for explicit prompt-02a authorization.

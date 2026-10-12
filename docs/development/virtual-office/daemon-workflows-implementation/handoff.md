# Prompt 01 handoff

The assigned review corrections F01–F03 are complete locally. Current task consumers replace stale
CI targets, explicit absence keeps required inputs queued, and cumulative execution time reserves
live work before entry and retains uncertainty through cancellation/restart. Clean source `c9e9809`
passes all 33 strict static checks with zero findings and the focused production journeys below.
Hosted quality run 227 failed on the earlier `e720bb9`; publication and a corrected hosted run remain
pending. No new hosted pass is claimed.

The production daemon, CLI, independent JSON consumer and standalone Svelte client use the same
durable task owner. Stop here: prompt 02 is the
user's actual review and remains unperformed; no later sprint prompt has been executed or assigned.
The named full-system gate remains in 07.

## Tested local checkpoints

- `37aa8c4` — useful public task and first standalone Svelte round trip, reported before broad
  retirement. A trusted Rust process analyzed the uploaded real Cargo manifest; after the browser
  closed, a fresh client discovered the retained result. A different manifest changed the report.
- `95a4dc5` — complete ordinary task cutover, obsolete graph/controller/publication retirement,
  durable authority/refusal behavior, bounded storage and input/result projections, and adoption
  by maintained consumers and current documentation.
- `e720bb9` — initial handoff, subsequently reviewed against failed hosted quality run 227.
- `c01a19f` — required explicit absence stays queued; public process-entry and edit/restart regressions.
- `45d3bb6` — cumulative duration admission/accounting, adapter bounds and old-format refusal.
- `c9e9809` — current CI/evidence consumers and strict workspace integration; the complete static gate
  passed with this clean source. Corrections are local; no push or history rewrite occurred.

The first and final useful-work evidence are separate from controlled model plumbing. Final browser
checks pass all eight cases; frontend check/lint and 26 unit tests pass against the unchanged production
Svelte build. Focused Rust owner checks, 30 repository contracts, public API review, current native
CLI/client evidence and five external-evidence fixture checks pass.
[Verification evidence](../../verification-evidence.md#prompt-01-review-corrections)
records the commands' logs and exact coverage. Earlier failed runs remain preserved. Actual desktop
and 390-pixel phone-sized screenshots were inspected by agents; this does not replace user review.

## Open the current review build

The static production preview is running at **http://127.0.0.1:4173/**. It is a thin standalone
client; closing it does not stop accepted daemon work. The two review daemons run immutable copies
of the tested executable from this fresh root:

`/home/hartolit/Projects/dev/milkdrift/target/prompt01-review/review-ui-b/`

| Connection | Daemon URL | Expected installation | Account | Private credential file under the root |
| --- | --- | --- | --- | --- |
| Studio A | `http://127.0.0.1:43899` | `host:browser-a` | `human:browser-a` | `daemon-a/operator.token` |
| Studio B | `http://127.0.0.1:42999` | `host:browser-b` | `human:browser-b` | `daemon-b/operator.token` |

Use **Add daemon connection**, enter the name, URL and installation identity, and paste the matching
credential from its local file. Both files have mode 0600. No credential value belongs in this handoff,
a URL, a screenshot or a committed file. The browser keeps credentials only in memory. The Rust
fixture already provisioned independent actor grants, exact process/model capabilities and finite
budgets. It did not configure or call a paid service.

The corrected fixture owner has a four-hour lifetime starting around 18:15 UTC on October 12
(20:15 Copenhagen), expiring around 00:15 Copenhagen on October 13. Its `ready.json`
contains nonsecret endpoints and process paths; `browser.json` is private. The static preview has
its own lifetime. If only the preview has stopped, run from the repository root:

```sh
npm --prefix apps/web run preview -- --port 4173 --strictPort
```

If the fixture owner has exited, retain `review-ui-b` and choose an unused new root to restart the two
configured development daemons and controlled model together. This example refuses an existing
root and must not be used to overwrite prior evidence:

```sh
cargo build -p milkdrift-daemon --bin milkdrift-daemon \
  -p milkdrift-cli --bin milkdrift -p milkdrift-evidence --bin connected-tasks
target/debug/connected-tasks serve \
  --root target/daemon-workflows/02-user-review-new \
  --daemon target/debug/milkdrift-daemon --lifetime-secs 14400
```

Read that new root's `ready.json` for its allocated URLs and use its matching local credential files.
This launches the same reviewed development fixture; it does not execute prompt 02 or erase the old
review results. Retained roots can be inspected with their matching saved binary and configuration;
do not point a newer incompatible executable at an earlier store.

## Useful task to review

Connect to A and select **New task**. Use a title such as **Review Cargo project**, a brief describing
what you want to inspect, and instructions to report package/workspace structure, targets and
dependencies. Choose **cargo-report · process.execute** and upload the repository's `Cargo.toml`
through **Text file input**. Submit, observe actual Running/Completed state and inspect the result.
The configured trusted Rust tool parses those supplied bytes; it does not infer unprovided files,
resolve the lockfile or claim to perform an animation implementation.

Repeat using `crates/runtime/Cargo.toml` to see the input change the report. Close the complete browser
while a task runs, reopen with only the connection information above, and find the work through
search/list. Switch to B to inspect its separate account and results. The same form can leave
**Supply brief now** unchecked, allowing a queued missing input to be repaired in the task inspector.
**controlled-model · model.generate** exercises the supplied brief/file through the configured
Rust-controlled endpoint and is explicitly plumbing evidence.

The CLI reads the same retained work without a browser cache or recovery file:

```sh
target/debug/milkdrift --url http://127.0.0.1:43899 \
  --token-file target/prompt01-review/review-ui-b/daemon-a/operator.token \
  --json workflow list --search Cargo
```

Prompt 02 owns the actual user observations and any resulting repairs. No agent has claimed that
review passed. This handoff intentionally leaves later automatic task authoring, saved methods,
connection administration and retention expansion unexecuted.

## Bounds, evidence and preservation

Captured text files and individual text inputs currently have a 16 KiB limit; combined escaped
instruction/input content has a 24 KiB bound. Selected task previews are labelled at 16 KiB, while
complete text downloads allow 2 MiB of output content plus bounded separators. Discovery and search
have explicit finite pages. SSE messages request snapshot resynchronization; they are not a lossless
public event journal. Restricted content and non-UTF-8 output remain explicit inspector states.

Corrected screenshots and logs are under `target/prompt01-review/frontend/`; prior final and
first-checkpoint artifacts remain under `target/daemon-workflows/frontend/`. Backend logs and API inventories are indexed
by verification evidence. Manual peer longevity, real Podman qualification, real models/coding agents,
physical phones, deployed HTTPS and other operating systems remain unqualified by this assignment.

Initial known stores, unrelated Podman/Ollama services and their roots were not changed. Earlier
owned development daemons `b`, `c`, `d` and `f` were stopped through their fixture owners after evidence
collection; their stores and logs remain. Root `e` remains as failed-startup evidence. Root `g` and the
static preview were left running during these corrections. The new `review-ui-b` fixture uses
internal document format 21; do not open the earlier stores with this binary. Attempted root
`review-ui-a` contains only an interrupted fixture binary copy and is preserved. The preservation branch
`codex/milkdrift-pre-product-reset-2026-10-12` and tag `milkdrift-pre-product-reset-2026-10-12`
still resolve to `f821ce093742246390f17edc77252ceb75672f97`. Old stores are refused before writes;
no migration, reset or cleanup of those stores was performed.

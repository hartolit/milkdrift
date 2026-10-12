# Prompt 01 handoff

Prompt 01 is complete through its focused acceptance. The production daemon, CLI, independent JSON
consumer and standalone Svelte client use the same durable task owner. Stop here: prompt 02 is the
user's actual review and remains unperformed; no later sprint prompt has been executed or assigned.
The named full-system gate remains in 07.

## Tested local checkpoints

- `37aa8c4` — useful public task and first standalone Svelte round trip, reported before broad
  retirement. A trusted Rust process analyzed the uploaded real Cargo manifest; after the browser
  closed, a fresh client discovered the retained result. A different manifest changed the report.
- `95a4dc5` — complete ordinary task cutover, obsolete graph/controller/publication retirement,
  durable authority/refusal behavior, bounded storage and input/result projections, and adoption
  by maintained consumers and current documentation.
- The following handoff commit records completion and these review instructions. All commits are
  local; no push, amend, rebase or preservation-ref rewrite occurred.

The first and final useful-work evidence are separate from controlled model plumbing. Final browser
checks pass all eight cases; frontend check/lint, 26 unit tests and production build pass. Focused
Rust owner checks, 30 repository contracts, public API review and the five external-evidence fixture
checks pass. [Verification evidence](../../verification-evidence.md#prompt-01-focused-task-acceptance)
records the commands' logs and exact coverage. Earlier failed runs remain preserved. Actual desktop
and 390-pixel phone-sized screenshots were inspected by agents; this does not replace user review.

## Open the current review build

The static production preview is running at **http://127.0.0.1:4173/**. It is a thin standalone
client; closing it does not stop accepted daemon work. The two review daemons run immutable copies
of the tested executable from this fresh root:

`/home/hartolit/Projects/dev/milkdrift/target/daemon-workflows/01-connected-20261012-g/`

| Connection | Daemon URL | Expected installation | Account | Private credential file under the root |
| --- | --- | --- | --- | --- |
| Studio A | `http://127.0.0.1:41977` | `host:browser-a` | `human:browser-a` | `daemon-a/operator.token` |
| Studio B | `http://127.0.0.1:41997` | `host:browser-b` | `human:browser-b` | `daemon-b/operator.token` |

Use **Add daemon connection**, enter the name, URL and installation identity, and paste the matching
credential from its local file. Both files have mode 0600. No credential value belongs in this handoff,
a URL, a screenshot or a committed file. The browser keeps credentials only in memory. The Rust
fixture already provisioned independent actor grants, exact process/model capabilities and finite
budgets. It did not configure or call a paid service.

The fixture owner has a four-hour lifetime starting around 18:31 UTC on October 12. Its `ready.json`
contains nonsecret endpoints and process paths; `browser.json` is private. The static preview has
its own lifetime. If only the preview has stopped, run from the repository root:

```sh
npm --prefix apps/web run preview -- --port 4173 --strictPort
```

If the fixture owner has exited, retain root `g` and choose an unused new root to restart the two
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
target/debug/milkdrift --url http://127.0.0.1:41977 \
  --token-file target/daemon-workflows/01-connected-20261012-g/daemon-a/operator.token \
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

Final screenshots and logs are under `target/daemon-workflows/frontend/final-g/`; first-checkpoint
artifacts are under `frontend/first-checkpoint/`. Final backend logs and API inventories are indexed
by verification evidence. Manual peer longevity, real Podman qualification, real models/coding agents,
physical phones, deployed HTTPS and other operating systems remain unqualified by this assignment.

Initial known stores, unrelated Podman/Ollama services and their roots were not changed. Earlier
owned development daemons `b`, `c`, `d` and `f` were stopped through their fixture owners after evidence
collection; their stores and logs remain. Root `e` remains as failed-startup evidence. Root `g` and the
static preview remain available for review. The preservation branch
`codex/milkdrift-pre-product-reset-2026-10-12` and tag `milkdrift-pre-product-reset-2026-10-12`
still resolve to `f821ce093742246390f17edc77252ceb75672f97`. Old stores are refused before writes;
no migration, reset or cleanup of those stores was performed.

# 01 — A real task through standalone Svelte

**Prepared, not assigned.** Execute only on the user's implementation instruction. Own the first
production journey across semantic owners, daemon, public clients and Svelte. Do not stop at an API,
a UI shell or a plan to integrate them. Stop before prompt 02 after the focused acceptance below.

## Read and establish the boundary

Read [AGENTS](../../../../AGENTS.md), vision, architecture, status and roadmap in their required order;
then [implementation](../../practices/implementation.md), [documentation](../../practices/documentation.md),
[workflow](../../workflow.md), [sprint README](README.md), [ADR 0051](../../../decisions/0051-daemon-task-workflows.md),
[code-change map](code-change-map.md) and [journey](journey.md). Inspect current source/consumers,
manifests, tests, worktree and preservation refs. Do not restart the earlier broad investigation.

Register this explicit assignment in the roadmap/office handoff while retaining the named final gate
in 07. Recheck bounded adoption facts from the ADR: actual active daemon processes, known configured
store paths and old-store refusals. Do not open old stores for writes, reset them, change running
services or rewrite preservation refs. Use an explicit new empty development root. Unexpected real
active data requires a narrow preservation decision before dependent deployment, not a new migration
project. Unaffected source and temporary-store work continues.

## Deliver the connected slice first

Within this assignment, make the first working checkpoint a real standalone Svelte submit → daemon
execution → output → reconnect path, before completing broad deletion cleanup. Build in the existing
owners, not an alternate executable or demo engine. Keep and extend that production client and API.
Any transient source overlap during edits ends at the cutover; no competing workflow engine or old
ordinary route may survive this prompt's handoff.

1. Introduce the canonical workflow/task/input/dependency data shape in blueprint and replace the
   ordinary runtime scheduling path in place. A submitted meaningful single task can consume supplied
   text and an uploaded/captured text file, call a configured model or trusted process, and expose its
   immutable result. Implement the basic dependency and missing-input rules now so subsequent phases
   extend this representation. Future changes use generation guards; start freezes definition,
   content, selection and authority. Defaults/limits come from the ADR, not the example.
2. Adapt persistence/redb atomic commands, journal/projection, accounts, artifacts and clock to a
   fresh format. Freeze first-entry intent before external effects, replay exact requests, reject
   conflicting reuse, and keep starts/results discoverable across restart. Refuse incompatible old
   stores before any write. Preserve current host final-entry, lifetime, cancellation and uncertainty
   mechanisms while changing workflow provenance. Wire bounded storage accounting and protected refs
   now; the full configurable expiry surface is completed in 05.
3. Deliver public create/list/search/read-task/read-result/upload/download/edit-unstarted/stop and
   request-receipt operations through control-protocol, daemon owner queue and control-client. A
   request body contains client-local aliases for new tasks; the daemon assigns IDs and returns an
   alias map atomically. List across every permitted workflow scope without requiring a known ID.
   Unknown submit outcome is discoverable and replayable; a local recovery file is optional, never
   required. Port CLI and one independent raw-JSON consumer to exactly these operations.
4. Create production `apps/web` using Svelte/TypeScript/Vite. Provide named daemon connections,
   authenticate/disconnect, workflow list/search, task form, input upload, result inspector and a
   left-to-right timeline plus accessible list. Show actual titles/descriptions/start/end states,
   queued reasons and stop-requested/uncertain separately from stopped. Two connections have disjoint
   credentials, caches, selections and settings. Empty/error/expired/forbidden states are real server
   responses; no fake successful task or browser-owned continuation.
5. Implement exact-origin CORS, preflight and bearer-fetch/SSE access in daemon HTTP/auth/config,
   plus snapshot resync after expired cursor. Tokens stay in memory and out of logs/URLs. Document
   independent static hosting and browser-trusted HTTPS termination for phone/LAN use; loopback is
   the development exception. Reject origin/identity/scope mismatch and mixed-content arrangements
   with useful diagnostics. Do not depend on native EventSource custom headers or a CLI subprocess.
6. Retire old ordinary graph author/run/revision/proposal/controller paths, structured child runs,
   prompt-sequence compiler and workflow-backed publication/evaluated-learning consumers described
   in the map. Remove their dead records/readers, commands, registrations, fixtures and dependencies.
   Preserve generic independent process/model/peer serving and managed-resource guards; do not turn
   protected old installations into unprotected new ones. Update all changed callers and public APIs.

Introduce Rust-generated TypeScript DTO/client-codec output with golden protocol fixtures, not a
hand-maintained second semantic validator. Add locked frontend dependencies and the README's check
scripts. Browser tests may use Playwright; backend fixture servers and process/fault orchestration
remain Rust in the existing evidence tooling. Use finite temporary roots and ports.

Native project paths need the ADR's narrow durable read/write claim owner in capability-host;
managed-resource claims alone are not sufficient. Freeze/capture under those claims and retain
uncertain writers across restart. Implement the default requiredness and adopt-correction command
shape now, using the same scoped task editor; phase 03 exercises automatic correction. Reserve the
space needed to settle/stop accepted work before ordinary admission can fill storage.

## Failure and verification contract

Build affected packages and consumers and run focused owner suites. Port graph-specific test setup
without deleting the behavior it proves. Existing starting commands (adapt removed target names only
when their replacement tests exist and discovery proves coverage):

```sh
cargo build -p milkdrift-daemon --bin milkdrift-daemon -p milkdrift-cli --bin milkdrift -p milkdrift-local-process --bin milkdrift-process-test-helper
cargo test -p milkdrift-blueprint -p milkdrift-authority -p milkdrift-persistence -p milkdrift-redb-store --all-features
cargo test -p milkdrift-control-protocol -p milkdrift-control-client -p milkdrift-control -p milkdrift-cli --all-features
cargo test -p milkdrift-runtime --test durable_runtime --all-features
cargo test -p milkdrift-daemon --test control_plane --all-features
cargo test -p milkdrift-capability-host --all-features
cargo test -p milkdrift-managed-linux --all-features
cargo test -p milkdrift-evidence --test repository_contracts --all-features
```

Select and run relevant local-process/peer serving lifecycle regressions from the map as well. The
wide first cut warrants these owner checks; it does not schedule a workspace full gate here. Review
changed public APIs with default/all-feature inventories under the existing API policy. Run the
new frontend check/lint/unit/build and focused browser suite, and inspect actual rendered desktop
and phone-sized layouts for keyboard access, readable failures and truthful timeline states.

Tests must include exact duplicate/conflicting submission; stale edit versus start; missing input;
text-file byte capture; revoked grant before entry; rejected origins; auth/cursor isolation;
response loss after acceptance; daemon restart; entered process uncertainty; cancellation request
without stop evidence; protected storage capacity refusal; and old-store open refusal without byte
changes. Test independent-host and managed-resource protections after the deletion boundary.

Use a Rust controlled model that returns a meaningful structured response to a supplied brief/file,
and an ordinary temporary-file process task. The harness may return content, never create the task
or fake completion. In an actual browser close/dispose the entire client while the task runs, then
use a fresh session with endpoint and credential only to find the workflow and inspect its result.
Run two daemons with disjoint actor grants to prove separation. Browser behavior is required evidence;
a successful native HTTP request is insufficient. If an already authorized model is available, run
the same brief and retain its observed result separately from deterministic plumbing evidence.

Controlled content alone cannot satisfy useful-work acceptance. Also execute one genuinely
input-dependent task: an authorized real model/coding agent, or a trusted Rust process inspecting
the supplied project/data and producing a substantive report (for example targets/dependencies from
a real Cargo project). Show its actual output in Svelte and vary the input to establish causality.
Do not substitute an echo, canned fixture response or prewritten report for that early usable task.

## Commits, documents and handoff

Commit tested vertical milestones: useful public task plus first Svelte round trip; durable reconnect/
authority/refusal completeness; complete old-path retirement and consumer/docs adoption. Keep shared
changes with their callers so each checkpoint builds and has meaningful focused evidence. Never
leave retirement for 07 or rewrite the UI as a disposable prototype.

Update affected package READMEs, current architecture/status, public protocol/config/operator docs,
examples, version tables and repository contracts. Add only actually executed evidence with source/
environment limits. Update this sprint's handoff with tested commit and exact local commands/URLs
for a user to perform prompt 02, including account setup without exposing a token. Known defects
must be fixed now. Stop ready for real user review; no advanced automation/method implementation yet.

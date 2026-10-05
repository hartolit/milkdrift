# Finish workflow editing and verify the fixes

## Assignment

Implement the remaining corrections from the client-ready workflow review. Finish the public
behavior, its callers, tests, and documentation—not just the code branches mentioned below.
Continue through this entire assignment without requesting a new instruction after each section.

The review examined snapshot `32a5ba6f543c20374cb6dbe49d811d860f38c582`. It identified three
implementation gaps and one evidence gap:

| Finding | Required result |
| --- | --- |
| R1 — Incomplete revision comparison | A client can reliably see what changed between two saved definitions. |
| R2 — Missing ordinary edits | Users can correct input names, remove unused inputs, clear the chosen output, and change a model step's output limit. |
| R3 — Restricted workflow discovery | A caller can list its permitted workflows without already knowing every identity. |
| E1 — Unavailable final verification records | Reviewable results identify the exact corrected source that was tested. |

These were source-review findings, not executed failing tests. Check them against current `HEAD`.
Preserve newer fixes and other contributors' work; do not restore the reviewed snapshot. Where a
finding is already fixed, verify its behavior and coverage rather than reimplementing it.

This user assignment authorizes these corrections and the underlying changes necessary to finish
them. It does not authorize Svelte implementation, a new engine, another persistence layer, a
broad rewrite, or another lint-expansion project. Svelte remains the first planned GUI. The daemon
owns workflow rules; the CLI, Svelte, and future clients use the same public operations.

## 0. Read the owners and establish the working baseline

Follow `AGENTS.md` and any applicable nested instructions. Read the canonical vision, architecture,
status, and roadmap in their required order. Apply these specific development guides:

- [Implementation practice](../../practices/implementation.md).
- [Documentation practice](../../practices/documentation.md).
- [Work and verification procedure](../../workflow.md).
- [Public API policy](../../../reference/public-api-policy.md).
- [Verification evidence](../../verification-evidence.md).
- [Virtual-office procedure](../README.md).

Inspect current Git status, relevant source, callers, and tests. Record the starting commit and
any pre-existing changes. Register this finite assignment in the existing office overview and
roadmap without replacing unrelated work. Use this prompt and its [README](README.md) as the plan;
do not produce another design essay or duplicate the project rules.

The reviewed starting points are:

| Area | Start here, then follow actual callers and owners |
| --- | --- |
| Comparison and listing | `apps/daemon/src/host/definitions.rs`, `apps/daemon/src/host/read_model.rs`, `crates/control-protocol/src/read.rs` |
| Editor commands | `crates/control-protocol/src/authoring.rs`, `apps/daemon/src/host/commands/authoring.rs`, its `edits.rs` and `graph.rs` submodules |
| CLI | `apps/cli/src/workflow_args.rs` and the corresponding command handlers |
| Permissions and pages | `apps/daemon/src/auth/`, the authority evaluator, control-protocol cursors, persistence revision queries and their storage implementation |
| Regression coverage | `apps/daemon/tests/control_plane/{authoring,authoring_cli,copy_authority,independent_client,inputs,reuse}.rs` and relevant protocol/storage tests |
| Final checks | `docs/development/workflow.md`, `docs/development/verification-evidence.md`, `tools/evidence/src/bin/strict-checks/`, `.github/workflows/quality.yml` |

Resolve necessary contract or ownership problems where they belong. Do not layer a CLI workaround
on a daemon defect, export private internals solely for tests, or leave old and new implementations
as competing production paths. Keep modules cohesive and comply with the existing strict checks.

### Testing and commits throughout this assignment

Sections 1–3 use focused verification. Compile affected packages and consumers; run the regression
cases, relevant refusal/recovery cases, and affected document/fixture checks. Confirm that test
filters actually select the intended tests. Do not repeat the full product run for every section.
Section 4 owns full integrated verification. This schedule does not disable or weaken CI.

Make small, coherent, tested commits while working. Suggested checkpoints are the comparison fix;
input removal/rename; output selection/limit editing; authorized listing; and final verification
support/documentation. Split further when a change has an independently useful, tested boundary.
Do not save the whole assignment for one commit or commit broken intermediate APIs as finished work.
Before each commit inspect the diff, stage only owned changes, and run `git diff --cached --check`.
Preserve checkpoints: no automatic squash, amend, rebase, reset, push, or force push. Correct earlier
mistakes in new commits. Keep one short `handoff.md` with the last tested commit and unfinished work
so an interrupted agent can resume rather than reconstruct the assignment.

## 1. Make revision comparison complete — R1

The reviewed `revision_diff` compares only nodes and edges. It can return an empty change list
with `truncated: false` after a workflow rename, an interface change, or an agreement-only change.
The public response already promises a structured comparison broader than that implementation.

Implement the comparison in the shared owner:

1. Inventory the saved semantic definition's fields. Cover metadata, declared inputs and outputs,
   nodes, edges, the governing agreement, and other semantic fields supported by that type. A
   rename is not the only metadata change: descriptions, labels, and extensions must not disappear
   from the comparison when they are part of the saved definition.
2. Return stable, useful change categories and identities through the existing public response.
   Agreement addition, removal, and modification must be visible. Keep summaries bounded and useful
   without dumping complete prompts, artifacts, credentials, or unrelated execution state.
3. Preserve deterministic ordering, authorization of both revisions, and the same-workflow check.
   Distinguish semantic differences from authorship, timestamps, or revision ancestry; document
   that distinction instead of treating every different revision ID as a content change.
4. Enforce the response limit through the appropriate existing policy owner. Report truncation
   exactly when changes were omitted; do not add another scattered literal or construct an
   unrestricted intermediate diff merely to truncate it afterward. Keep supported-input costs bounded.
5. Update every affected reader, CLI presentation, protocol fixture, and contract description.
   Do not implement a second comparator in a client or turn an incomplete result into "no changes."

Add focused regressions for metadata-only, input-only, output-only, agreement-only, node/edge,
unchanged-content, and limit-boundary comparisons. Include additions/removals and an unauthorized
or cross-workflow comparison. Exercise the normal public read, not just a new private helper.

**Done:** a real semantic change cannot produce an unqualified empty comparison. Unchanged
semantics produce no invented change. Responses remain authorized, bounded, and deterministic.
Commit the fix with its tests and documentation.

## 2. Complete the ordinary editing operations — R2

The reviewed editor can add an input but cannot remove or rename its declaration. Disconnecting
an input leaves it required by the workflow. It also cannot clear the selected final output or
change `maximum_output_units` on an existing model step.

Implement these operations through `BlueprintEdit` and the existing daemon editor, then expose
them through the normal CLI. Keep the client's role to collecting edits, retaining returned draft
information, and displaying results; it must not reconstruct graph rules or derive semantic hashes.

### Remove and rename declared inputs

- Remove a declared input when no bindings reference it. If it is still used, refuse clearly and
  identify the connections the user must change. Do not silently disconnect consumers or delete
  retained input artifacts. After removal and save, future runs must no longer require that input.
- Rename an existing declaration and all references to that workflow input in one validated edit.
  Update only those references—not prompts, similarly named step ports, output names, or unrelated
  strings. Refuse invalid names, absent sources, and collisions. Define same-name handling clearly.
- A refused edit must leave the caller's draft intact. A successful rename must not leave partial
  bindings or require a hidden sequence of low-level mutations.

### Clear the selected output and change a step's limit

- Provide an ordinary action to clear the final-output selection. The user must then be able to
  remove the only selected step and choose or create its replacement. Keep refusing deletion of
  a step that still has consumers unless the user explicitly disconnects them.
- Allow incomplete drafts to be returned, retained locally, and reopened for continued editing.
  This is not permission to save or execute an incomplete workflow. Preserve complete validation
  at the existing save/start boundaries; do not add a second daemon draft database.
- Let the user change an existing model step's `maximum_output_units` without replacing the step.
  Reuse the model request's validation, selected-capability checks, and applicable budget checks at
  their existing owners. Preserve its prompt, bindings, identity, and other unrelated properties.
  Update any derived reservations consistently rather than changing only a displayed number.

### Complete the public behavior

Update command serialization/readers, matching branches, client handling, CLI help, maintained
examples, and tests together. Follow the public API policy: development formats do not justify
unnecessary compatibility shims, but unsupported formats must never be silently reinterpreted.
Preserve exact request replay/conflict behavior, save guards, refusal of unsupported advanced
workflow shapes, saved revisions, and already accepted runs.

Cover these actual user paths with focused tests:

- Correct a misspelled input used by multiple steps; save and run with the corrected name.
- Refuse removal while connected, then disconnect and remove it; show that a new run no longer
  asks for the removed field. Collision/invalid rename refusals leave the draft unchanged.
- Clear the only output, remove its step, retain/reopen the incomplete draft, and reject an
  incomplete save; then add a replacement, select its output, and save successfully.
- Change a step's limit through the public edit; verify the constructed request and applicable
  accounting, preservation of unrelated fields, and rejection of invalid values or permissions.
- Reopen the saved revision and retain the changes. A prior revision and an already accepted run
  still use their original definitions and inputs. Retrying an exact save does not create another
  saved result; changing a request under the same identity still conflicts.

Use the existing CLI and separate HTTP/JSON-client tests where they provide the right coverage.
Do not duplicate every permutation in every layer; prove shared behavior at its owner and verify
that both consumers reach it. Commit the input changes and the output/limit changes as separate
working checkpoints.

## 3. Let narrowly authorized callers discover their workflows — R3

The reviewed listing path rejects an unfiltered request under `WorkflowRunScope::Workflows` with
"a named-workflow grant requires an explicit workflow filter." That is a deliberate restriction,
not an established security flaw. Complete discovery without broadening the caller's permissions.

Prefer completing the existing revision-listing operation so `workflow list` works with a grant
naming several workflows. Preserve its documented revision-list semantics; do not silently replace
revision pages with a different kind of workflow summary. No new registry is needed.

Required behavior:

1. Without an explicit workflow filter, return only entries from workflows the current caller may
   inspect. With a filter, enforce the requested workflow against the same authority evaluator.
   Preserve the supported single-workflow and unrestricted cases. A run-only or operation-denied
   grant must not acquire general definition access.
2. Restrict the collection before fetching/projecting hidden workflow data. Do not fetch every
   workflow, expose hidden names or counts, or authorize the request as unrestricted and filter
   the answer afterward. If multi-workflow querying needs a storage-contract correction, finish
   it in the existing query/storage owners and update their consumers.
3. Keep page size, work, and cursor size bounded, using the existing ordering and live-page contract.
   Pagination must make progress and must not lose permitted entries behind hidden ones. An empty
   intermediate page, if the documented scan bound permits one, is not the end while work remains;
   its continuation must advance safely. No unbounded scan or per-caller result cache.
4. Keep cursors opaque and bound to their intended actor/grant, feed, and filters. Recheck current
   authorization for every page. Reject tampering, incompatible filters, a different grant, and
   revoked access according to existing policy. Do not let a cursor preserve authority that was
   removed after the first page.
5. Update CLI behavior and the independent client's coverage. Users must not supply raw IDs just
   to discover the workflows their grant already names.

Test an account permitted to inspect two workflows while others remain hidden; multiple revisions;
a page size that requires several pages; an empty allowed collection; an explicit unauthorized
filter; invalid page sizes; and cursor tampering, reuse under another grant, and revocation between
pages. Include enough hidden entries to catch false end-of-list behavior. Assert ordering and lack
of duplicates/omissions under the chosen existing page semantics—not an invented snapshot guarantee.

**Done:** the normal listing call discovers all permitted entries across pages without disclosing
unrelated workflows or requiring wider authority. Commit the complete query-to-client change.

## 4. Verify the corrected product and retain the evidence — E1

The reviewed archive describes successful local checks at executable source `199d1a5`, but the
archive itself identifies `32a5ba6`. Raw results were under ignored `target/` directories and absent
from the review package. This establishes neither current failure nor current success. Replace the
uncertainty with fresh, accessible evidence; do not reuse the old counts as a new pass.

First extend the existing operator journey and independent-client coverage with the corrected
behavior. A narrowly authorized user should discover a workflow, make the new edits, save and
compare revisions, run with the revised inputs, inspect results, reconnect after interruption,
and reuse the method with the other maintained brief. Preserve assertions that lost replies or
retries do not cause duplicate work. Use the existing example and fixtures, not another demo app.
The normal regression and full gate must continue protecting managed hosting, publication,
acceptance, and evaluation behavior.

Once sections 1–3 are integrated and focused tests pass:

1. Commit the complete executable changes. Record the commit SHA, Git tree, working-tree status,
   pinned toolchain and actual tool versions, environment, and relevant configuration. Do not
   include credentials, private prompts, or model endpoint secrets in logs or metadata.
2. Run the current full local gate from `docs/development/workflow.md`, its required checker probes,
   and the required actual-binary/operator scenarios in `docs/development/verification-evidence.md`.
   Build their documented prerequisites. Read the actual current commands rather than guessing
   test names, tool versions, or reusing stale counts from this prompt.
3. Ensure secret-check coverage includes the entire correction range, not only `HEAD^` after the
   latest small commit. Use the existing runner's explicit base/current-content options and record
   coverage. Keep all other configured strict checks; do not start a new linting project.
4. Fix failures in their owning implementations. Use focused reruns while investigating, then run
   the full required gate and integrated journey again on the final executable state. "At the end"
   means avoiding repeated full runs during development—not reusing a pass after changing code.
   Do not loosen deadlines without evidence, skip failing tests, weaken assertions or lint policy,
   hide missing tools, disable jobs, or claim historical timeouts were fixed without a relevant test.
5. Retain a compact evidence manifest and relevant redacted raw results using the existing evidence
   tooling. Include source identity, exact commands, return codes, outcomes, discovered test counts,
   ignored/not-run cases, limits, and hashes of packaged records. Preserve relevant failed attempts
   alongside the final result so the package does not erase what required correction.
6. Deliver the evidence as an accessible artifact, not merely a path under an agent's ignored
   `target/` directory. Use the existing CI artifact route where an authorized run is available,
   or provide a downloadable local archive. Do not commit all generated logs or `target/`, upload
   private data, or add a separate evidence framework. Inspect that the delivered package really
   contains the files its manifest names. Record the final corrected source separately from the
   old reported results.

Make the source/evidence relationship explicit. A final documentation-only commit may follow the
executable acceptance commit; identify both and show the intervening diff. If code, tests, schemas,
fixtures, dependencies, build/CI settings, or other execution inputs changed, do not describe the
older pass as verification of the new state. Final documentation still needs its prescribed checks.

Keep deterministic/local tests, hosted CI, real-provider observations, and physical UM790/Podman
qualification separate. This task does not authorize production deployments, paid provider use,
privilege changes, or a fresh hardware campaign. Use existing authorized prerequisites for required
checks. If a required check cannot run, report exactly what is missing and leave acceptance open;
do not invent a success or claim the whole implementation is absent.

## 5. Update the maintained explanation and close the assignment

Update the existing API/CLI docs, examples, package explanations, status, roadmap, and verification
record where behavior or claims changed. Remove the obsolete named-workflow listing limitation
only once the new authorized behavior is implemented and verified. Preserve unrelated limitations
and the historical evidence distinctions. Use an ADR only for a genuinely changed durable decision;
do not make new documents that repeat existing ownership rules.

Follow the office closure procedure once accepted: move lasting facts to their owners, remove the
completed temporary assignment and its listing, and check links. Do not start the Svelte sprint or
another cleanup project automatically. If acceptance remains open, leave one accurate handoff rather
than deleting the evidence of unfinished work.

Your final response must state:

- The disposition of R1, R2, R3, and E1, with concise implementation and regression-test references.
- The working commit checkpoints and exact source covered by the final checks.
- The evidence download/artifact location, checks actually run, and any remaining limitations.

The assignment is complete only when the user-facing fixes work through the shared public path,
required verification passes on the corrected code, and its results are available for review.
A diff, a green focused test, or an updated status paragraph alone is not completion.

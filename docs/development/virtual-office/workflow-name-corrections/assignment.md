# Fix workflow connection IDs and repair input names

## Assignment

Finish the two corrections identified in the client-workflow review. Implement the fixes, preserve
supported saved workflows, add regression tests, and complete the final product checks. Do not
stop after investigating the code or writing a proposed solution.

**The result:** valid names must not cause different connections to share an ID or prevent an
otherwise supported repair. Existing saved work must remain usable without rewriting its history.

The review examined `c1c3d046c845c7763bd24162141fb3855518360e`. It was a source review, not a runtime
reproduction. Work on the current branch: inspect what has changed and reproduce each issue before
assuming it remains. Do not reset the repository to the reviewed commit. If a correction already
exists, verify it and fill any missing coverage rather than implementing it twice.

This is one bounded correction assignment, not another infrastructure sprint. Its authorization
covers the fixes and directly related compatibility, tests, documentation, and verification. It
also covers underlying defects within those affected paths when they prevent a complete fix.
Unrelated findings follow the existing scope policy; record them without expanding this assignment.

**Svelte remains the first GUI. No frontend is being built here.** Construction, validation,
permissions, IDs, repair, and saved results stay in the daemon and its existing libraries. Neither
the CLI nor a future Svelte client should compensate for these bugs.

## Read and establish the starting point

Follow [AGENTS.md](../../../../AGENTS.md), including its canonical reading order. In particular, read
[architecture](../../../architecture.md), [status](../../../product/status.md), and
[roadmap](../../../product/roadmap.md). Apply the
[implementation practice](../../practices/implementation.md),
[documentation practice](../../practices/documentation.md), and
[assignment and verification policy](../../workflow.md).

Record the starting commit and working-tree state. Preserve other contributors' work; do not
reset, clean, stash, or include their changes in your commits without authorization. The previous
sprint was closed in the reviewed source. Register this bounded correction under the existing
[virtual-office procedure](../README.md); do not recreate its deleted phases or authorize a successor.

Trace these owners and their callers before choosing the implementation:

| Concern | Starting point |
| --- | --- |
| Graph construction, connection IDs, saved-workflow recognition | [authoring/graph.rs](../../../../apps/daemon/src/host/commands/authoring/graph.rs) |
| Accepted user names and connections | [authoring/edits.rs](../../../../apps/daemon/src/host/commands/authoring/edits.rs), [blueprint identity rules](../../../../crates/blueprint/src/identity.rs) |
| Draft, save, copy, and permission handling | [authoring.rs](../../../../apps/daemon/src/host/commands/authoring.rs) |
| Repair construction and proposal preparation | [authoring/repair.rs](../../../../apps/daemon/src/host/commands/authoring/repair.rs) |
| Existing public regressions | [authoring tests](../../../../apps/daemon/tests/control_plane/authoring.rs), [repair tests](../../../../apps/daemon/tests/control_plane/repair.rs), [independent client](../../../../apps/daemon/tests/control_plane/independent_client.rs) |

Read the actual functions rather than relying on the review's line numbers. Search the workspace
for the shared edge builder, generated repair ports, saved-definition recognition, and consumers
of those values. Fix the shared rule in its owner and update every affected caller. Do not add a
second graph compiler, a generic naming framework, or public APIs used only by tests.

## 1. Give distinct connections distinct, stable IDs

### Reproduce the failure

The reviewed `add_edge` hashes this text:

```rust
format!("{kind:?}:{source}:{port}:{target}:{input}")
```

Colons are legal inside the fields. Create steps `draft`, `review`, and `review:x`, ordered so both
review steps can read the draft. Connect them as follows:

| Source step | Source output | Target step | Target input |
| --- | --- | --- | --- |
| `draft` | `final_text` | `review` | `x:brief` |
| `draft` | `final_text` | `review:x` | `brief` |

Both produce `Data:draft:final_text:review:x:brief` before hashing. Duplicate-edge validation then
rejects two different, valid connections. The problem is ambiguous field boundaries, not BLAKE3.

### Implement the complete correction

Use one deterministic, unambiguous encoding of the edge kind and all endpoint fields. A stable
structured representation or explicit component lengths can work. Make kind tags and encoding
rules deliberate; debug formatting is not a durable encoding contract.

Keep the resulting ID within the existing limits. Preserve exact user spelling and field roles.
Do not change the separator and call it fixed, ban currently valid colons, silently rename inputs,
drop a connection, weaken duplicate checks, or introduce random IDs. Rebuilding the same connection
must remain deterministic. Tests establish correct encoding and behavior, not a claim that hashing
can never collide.

The repair builder also uses the graph helpers and retargets generated edges. Check that those
callers remain coherent under the chosen identity rule; the ordinary editor must not adopt one
rule while repairs accidentally use another for the same responsibility.

### Preserve supported saved work

`ModelWorkflow::read` currently rebuilds a definition and compares all its content. Merely changing
the hash input can make a previously editable saved workflow fail that comparison.

Before changing the builder, retain a small representative saved definition produced by the old
supported writer. Freeze its original document, IDs, and expected digest independently of the new
builder. Do not generate this compatibility fixture with the corrected encoder or rewrite its
expected values until it passes.

Define and implement how that saved definition reopens, is edited, is saved as a new revision,
and reopens again. Preserve the original stored revision and digest, run references, exact request
receipts, and replay behavior. Opening a definition must not rewrite it. Preserve supported no-op
behavior rather than hiding an ID migration inside an apparently unchanged read or save.

Keep the compatibility handling narrow. This is support for currently accepted saved work, not an
excuse to retain every historical development format. Prefer one writer and one graph-building
path with any necessary, explicit read handling. Do not keep two complete graph builders or two
runtime execution paths. Never make recognition succeed by broadly ignoring edge IDs, missing
checks, metadata, bindings, or other content the editor cannot preserve.

Richer or malformed definitions that the initial editor cannot safely edit must still receive an
explicit refusal without losing content. This correction does not authorize expanding the editor
to every blueprint feature. Record a durable format/reader decision in its existing owner or an
ADR only if the chosen change genuinely requires one.

### Required regression coverage

Exercise the public daemon authoring commands, not just a hash helper:

- The exact two-connection example above can be authored, saved, reopened, and used with both
  intended bindings intact. A controlled model endpoint can make incorrect routing observable.
- A bounded set of ordinary and delimiter-bearing names covers field-boundary ambiguity, repeated
  construction, distinct kinds/endpoints, and supported length limits. Expected field identities
  must be independently specified rather than derived by the same helper under test.
- The old-writer fixture reopens and permits a supported edit; the new revision reopens while
  the original document remains unchanged. Test a stopped/reopened store and exact replay of an
  already accepted saved request against retained old IDs without duplicate work.
- Unsupported content, stale bases, unauthorized edits, and genuinely invalid duplicates still
  fail through the owning path. A refusal must not partially save a graph or modify the caller's
  retained draft; keep existing receipt/audit behavior intact.

Extend existing tests where they already establish these guarantees. Do not multiply full model
runs merely to test several strings; use small encoding tests for broad variation and public
regressions for the user-visible failures.

**Commit checkpoint:** after the correction, compatibility handling, and focused regressions pass,
commit this coherent change before starting the repair-input correction. Suggested message:
`Fix generated workflow edge identities and saved-definition compatibility`.

## 2. Keep user inputs separate from generated repair evidence

### Reproduce the failure

The reviewed repair builder copies workflow-input bindings from the failed model step into the
replacement, then adds an internal input named `failed_result`. Ordinary authoring allows a user
port with that name; it reserves the `milkdrift.` prefix instead.

Through public operations, declare a workflow input `brief`, connect it to the final model step
under the user port `failed_result`, and run a controlled response that fails result acceptance.
Pause at the supported failed-result hold and request a future model repair. The reviewed code
then fails when adding a second input called `failed_result`.

### Implement the complete correction

Give generated repair evidence a collision-safe internal name consistent with the existing
reserved namespace. Own that naming rule once and use it consistently in the added port, edge,
binding, context handling, and any relevant consumers or examples.

The original user binding must remain under its original name and still refer to its authorized
workflow input. The selected rejected response must arrive separately. Do not drop or rename the
user's input, replace its contents, merge the two sources, or make `failed_result` a newly forbidden
ordinary word.

Check admitted saved/imported definitions as well as fresh editor input. A reservation enforced by
an editing command is not proof that an existing graph cannot already contain a chosen name. Handle
any such conflict without overwriting content or broadening the set of definitions the convenience
editor claims to support. Supported stored repairs and accepted proposals must retain their
original identities, bindings, and replay behavior; do not rewrite them into the new naming form.

Keep repair preparation distinct from application. Preparation must still check the exact revision
and sequence, permission, rejected acceptance evidence, and correct pause point. It must not start
repair execution or silently approve a proposal. The control service continues to own approval and
future revision adoption. Completed work and the failed acceptance remain in history, and the
repaired result must undergo its own acceptance check.

### Required regression coverage

Extend the existing public repair test or add a focused case that proves:

- A user port named `failed_result` no longer prevents preparing and applying a permitted repair.
- The actual repair request reaching the controlled model endpoint contains both the original
  authorized brief and the selected rejected response as distinguishable inputs. Choose different
  sentinel content for each; checking only the generated graph's port count is insufficient.
- Unrelated private artifacts and unselected earlier output remain absent from that context.
- Preparing a repair leaves the run and approval state unchanged. Existing approval, stale-version,
  authority, failed-history, and fresh-acceptance protections still hold.
- Replaying the saved request after interruption does not apply a second repair or make an extra
  model call. Preserve existing old-repair replay where it is a supported stored contract.
- User attempts to occupy reserved internal inputs remain rejected without partial application;
  ordinary names and the existing normal repair example still work.

Reuse the maintained controlled response fixtures and public paths. Do not require an external
provider or a successful unaided model repair to prove these naming rules.

**Commit checkpoint:** after the focused repair and affected shared-builder regressions pass, commit
this correction separately. Suggested message: `Separate repair evidence from user input names`.

## 3. Check the affected paths and finish the integration

Search the ordinary authoring and repair paths again for the same two failure patterns: ambiguous
composite IDs and generated values sharing unrestricted user names. Include preview, save, reopen,
copy, comparison, and repair consumers where they use the changed rules.

Fix a concrete related defect when it would leave this correction incomplete, with a regression
and a separate coherent commit when appropriate. Do not turn this into a workspace-wide naming
rewrite, additional lint campaign, browser implementation, or infrastructure qualification sprint.

Preserve the stronger compiler, Clippy, dependency, scanner, and architecture checks. Do not add
blanket allowances, relax warnings, regenerate golden files blindly, ignore tests, swallow errors,
or lengthen timeouts to disguise failures. Existing concurrency, cancellation, permission, and
recovery tests remain necessary. Do not delete them because static checks are green.

Use regular local commits for tested, coherent changes. Stage only assignment-owned files; review
the staged diff. Do not squash, amend away checkpoints, rewrite history, or push without an explicit
request. More than two implementation commits is appropriate when a genuine underlying correction
needs another independently understandable step. There is no empty-commit quota.

## 4. Run full validation at the end, not after each checkpoint

While implementing, run formatting, affected compilation/lints, and the focused owner tests.
Confirm test filters actually discover the intended cases. Capture the original failure with the
new regression before accepting the fix; an isolated temporary worktree is suitable when needed.
Do not modify another contributor's working tree to demonstrate the old failure.

Useful starting suites are the daemon's `control_plane` authoring and repair groups, followed by
relevant blueprint, control, client-recovery, and revision-store tests. Select exact names from the
current tree rather than inventing filters. Compile affected callers and maintain public-client
coverage; no private CLI-only workaround is acceptable.

**This section owns final acceptance for this single assignment.** Once both corrections and their
callers are integrated, run the full gate specified in [workflow.md](../../workflow.md) and the actual
application scenarios in [verification-evidence.md](../../verification-evidence.md). The existing
[quality workflow](../../../../.github/workflows/quality.yml) is additional execution guidance, not a
file to weaken to shorten the task.

At minimum, final coverage must include:

1. Required tool probes and the complete strict static gate with the repository's pinned tools.
   Scan changed content across the entire correction range, using the recorded starting commit
   as the base when appropriate—not just `HEAD^` after several checkpoints. Record any included
   pre-existing changes. Do not claim this scans all Git history.
2. The full workspace all-feature test suite and test discovery, with failures, ignored tests,
   counts, and reasons retained rather than copying the previous report's totals.
3. Actual daemon/CLI scenarios: headless operations, controlled model behavior, controller
   qualification, and the authored workflow journey with supplied inputs, repair, reuse, and
   saved-request recovery. Workspace tests alone do not replace those application runs.
4. The new collision cases and saved-definition compatibility on the final code, plus affected
   documentation/fixture readers and API checks if their contracts changed.

Build and preserve the required application/helper binaries as documented; record their hashes
when the evidence lane requires them. Use private temporary stores and bounded controlled fixtures.
Do not touch the operator's production stores, credentials, services, or managed environments.
These fixes require no new cloud spending or UM790/Podman campaign.

Fix failures rather than merely listing them. Use focused reruns during correction, then run the
full final checks against the finished executable state. An earlier green result does not cover
later executable changes. This is not permission to repeat the whole suite after every local edit;
it is a requirement for one defensible final result. Keep CI's configured behavior unchanged.

If a required tool or environment is unavailable, retain the exact failed command and prerequisite.
Do not label acceptance complete, silently substitute a weaker check, or treat a skipped command
as passing. Leave a short, actionable handoff for completing the same assignment.

## 5. Evidence, documentation, and completion

Update only the canonical documentation affected by the correction: naming/compatibility behavior,
current limitations, verification evidence, and the finite roadmap entry as needed. Do not copy the
review or this prompt into product documents. Leave Svelte as the separately scoped next interface;
browser authentication, storage, and transport remain work for its own assignment.

Keep raw logs and generated reports in ignored output or a reviewable evidence archive, not among
product sources. Record the reviewed starting commit, tested final executable commit/tree, command
lines, exit codes, test coverage, any later documentation-only changes, and remaining evidence
limits. Exclude secrets, private store dumps, and unrelated user content. Provide an accessible
evidence location rather than an unshared local path presented as independently verified proof.

Before closing, confirm that the exact connection example works, the `failed_result` repair keeps
both inputs, existing supported work reopens/replays correctly, the old unsafe construction paths
are gone from current writers, required checks pass, and `git diff --check` is clean. Review the
final diff against the implementation and documentation practices.

Follow virtual-office closure rules for temporary notes. Final documentation or cleanup changes
receive a separate small commit when warranted, with link checks after removing temporary files.
Do not erase the implementation checkpoints.

Return a short completion report with the two fixes, the saved-workflow compatibility decision,
the commit checkpoints, the actual checks and evidence location, and any unresolved blocker. Do
not start the Svelte project or propose another broad cleanup as a substitute for finishing this
assignment.

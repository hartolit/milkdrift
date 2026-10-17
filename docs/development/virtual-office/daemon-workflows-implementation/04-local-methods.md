# 04 — Reusable methods expand into local tasks

**Prepared, not assigned.** Prerequisite: 03's automatic work, prospective edits and cumulative
limits, the [02a connection correction](02a-daemon-connection-identity.md),
[02b visual correction](02b-visual-design-and-brand.md) and completed early user review.
Deliver the reusable project-setup method in the same production workflow and clients.
No child workflow runtime, published method service or definition-time pinned copy supplies this use.

Extend the corrected shell, shared daemon names and existing visual components for method browsing,
editing and origin inspection. Do not reintroduce browser-local daemon names or the former visual
theme. This phase adds method behavior to the reviewed client rather than replacing its design.

Read [AGENTS](../../../../AGENTS.md), canonical docs, [implementation](../../practices/implementation.md),
[documentation](../../practices/documentation.md), [workflow](../../workflow.md),
[README](README.md), [ADR 0051](../../../decisions/0051-daemon-task-workflows.md),
[code-change map](code-change-map.md), [journey](journey.md) and 03's tested handoff. Inspect the
relevant new task owners and preserved old evidence; do not reopen the selected method semantics.

## Complete library, use and editing

Blueprint owns validated template bodies/parameters/results/dependencies; control owns authorized
save/use/import/export commands; persistence/redb owns saved content, queued references and guarded
expansion transactions; runtime owns activation. Public protocol/client/daemon/CLI/Svelte must all
expose those same operations. Keep modules within these established owners, not a registry service.

Implement daemon-wide method identity, mutable name, owner/use permissions, save generation and
content digest. The editor can save a complete template and inspect queued uses affected by changed
required inputs. Consumers can browse authorized templates and queue a use with named bindings but
cannot edit the shared body. Rename preserves ID. Refuse deletion while queued workflow uses or
saved nested references need the template; expose the authorized references without leaking others.
A denial can report protected references exist without exposing forbidden workflow names.

Implement activation using the consistent root-plus-nested snapshot and deterministic ID map chosen
in the ADR. Acceptance saves exact canonical content/digests, origin chain, account reservation and
reference holds. Atomic materialization commits every task, export binding, expansion receipt and
expanded marker together. No partial runnable copies exist. Restart after acceptance completes the
same snapshot; restart before acceptance may select a newer saved body. Library save, task edit,
start and activation race through explicit transaction/version guards, not last-writer wins.

Flatten nested references from that same snapshot, checking cycles, depth, result interfaces and
remaining workflow/task/batch capacity before acceptance. Expansion has its own 64-task bound rather
than incorrectly inheriting the ordinary 32-item model batch bound; all 64 still charge the shared
256-task workflow ceiling. A nested template has no later independent activation unless queued as
a separate top-level use. Refuse unknown parameters, incompatible types, missing required bindings,
missing exports, unauthorized templates/capabilities and overflow with actionable diagnostics.
Blocked activation allows rebind/replace/template repair without dropping the reference silently.

In Svelte, a queued reference shows the current template name and its bindings, clearly distinguished
from expanded origin. After expansion show ordinary task lanes grouped by use, selected content/digest
and parameter values, result exports and local edits. Users/agents can revise unstarted copied tasks,
add corrective work and rebind future consumers; started tasks stay frozen and the library stays
unchanged. Show group completion/failure from children without a child run or independent budget.
CLI and agent discovery expose the same information and edits.

Implement bounded export/import bundles including nested templates. Imports create new local IDs,
rewrite internal links and retain source provenance. No secret, grant or live host relationship is
transferred. The recipient chooses its own allowed models/tools/bindings before use; unresolved
capability selections block until explicitly mapped. No automatic template-update propagation.

## Test the actual boundaries

Acceptance marks a use expanding and freezes bindings. Implement the ADR's remove/rebind refusal
after that point, paused materialization without entry, and stop materialization with children
stopped before entry. Test those races before/after acceptance and across restart; a successful
pre-acceptance removal must produce no children. Reservations/holds survive until final commit.
Bound nested traversal to 256 reference occurrences and canonical snapshot/bundle content to 1 MiB
before loading/flattening beyond them. Include many empty nested uses, not just oversized task lists.

Build all affected callers; run blueprint/control/runtime, persistence/redb, protocol/client/CLI and
daemon control-plane focused suites. Existing command starting points are those in 03; add named
new template/activation suites under those owners and verify test discovery. Run the frontend's
check/lint/unit/build and method browser cases. Review changed public API/default/all-feature exports,
then run documentation contracts and diff checks. The full workspace gate remains in 07.

Prove through production operations:

- The same queued use expands saved body B after owner save A→B; renaming does not change identity.
- Faults before acceptance, after snapshot acceptance, during atomic commit and after commit before
  reply never duplicate tasks or reselect body C for an accepted B expansion. Reopen the actual store.
- Editing nested content before root activation is included; editing it after accepted snapshot is
  not. Cycles, depth/count/account overflow and invalid exported results refuse the whole expansion.
- New required parameters block old bindings; rebind/replace repairs them. Deletion with queued or
  nested refs refuses, and removing/replacing all references permits deletion without breaking
  protected copies. Concurrent save/delete/activation cannot bypass those holds.
- Consumer template edit is denied; owner save works. A local future-copy change leaves shared bytes
  unchanged; a started-copy change refuses. A stale human/model edit cannot overwrite another.
- Lost reply replay, completed history retention and restart preserve origin and export mapping.
- One exported method works on a second daemon after explicit import and local capability mapping,
  without cross-owner execution, transferred credentials or an inherited editing grant.

Run the animation setup/correction segment and the timetable CLI variation using the same method
body. Product source contains neither brief name nor hardcoded research folders. Controlled responses
choose a method ID through ordinary catalog/context and return a use request; fixtures never expand
it. Actual browser tests edit the template before a delayed dependency permits activation, inspect
the expanded copies and modify one future copy. Fault harnesses pause only at declared persistence
boundaries; they do not supply recovery.

## Commit and stop

Commit coherent tested units: library/rights and public editor; expansion transaction and restart;
prospective copies, import/export and complete journey. Update owning docs, protocol/schema versions,
operator method instructions, status/architecture and actually run evidence. Preserve old findings
with their original limits without carrying obsolete pinned/published routes forward. Handoff the
tested commit, exact cases and remaining integrated checks; stop ready for 05, not at a partial GUI.

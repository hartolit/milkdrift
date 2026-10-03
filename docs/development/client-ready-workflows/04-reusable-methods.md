# 04 — Save and reuse workflows

After 03 is ready, follow [the shared rules](README.md),
[implementation practice](../../practices/implementation.md), and
[documentation practice](../../practices/documentation.md).

Start with existing blueprint versions, `crates/control/src/published.rs`,
`crates/control/src/learning.rs`, their daemon operations, and CLI blueprint/method/invocation
commands. Read the current publication and learning guides. Complete their ordinary use;
do not introduce another template store, publication system, or learning engine.

## Build this

Let the user find a saved workflow, see its inputs and outputs, choose a specific version, and
run it with a different brief. Also support making an independent editable copy where allowed.
A copy gets its own identity and records the source without changing the original or discarding
protected requirements. Saving or copying a workflow does not grant additional permission.

Reuse the definition, not old run state: no old inputs, private outputs, account reservations,
or editing claims become part of the new run. Each run keeps separate writable state. Changing
the saved workflow or its preferred version must not change work already accepted by the daemon.

Keep publishing separate from saving. Publishing makes a reviewed workflow callable by another
client through the existing service mechanism. Help construct the required request through public
operations; do not ask the user to calculate hashes or assemble internal receipt records.
Service identity, permissions, limits, and checks remain explicit reviewed choices, not broad defaults.
For a published brief, use the supported upload/input route rather than loosening its data contract.

An invoke-only caller receives declared outputs, not private run history or permission to edit the
method. Updating a publication creates a new version. Retiring it stops new calls without deleting
accepted work or breaking replay. Reuse and publishing must also be available to a non-CLI client.

Make the existing evaluation commands understandable: select allowed source evidence, submit an
actual candidate, inspect the fixed comparison and its reasons, and request promotion only when
eligible. Advanced criteria may remain explicit documents. Do not build a general experiment editor.
The daemon derives scores from recorded work; the client cannot supply invented success scores.
Missing results remain missing. Rejected or inconclusive candidates stay unpromoted. A manually
edited workflow is not proof that the system learned a better method.

## Check just this work

Test reuse/copy, separate inputs and writable state, unchanged originals, private-data refusal,
invoke-only results, publication replacement/retirement, and exact replay where affected.
Use the existing evaluation tests for eligible promotion, rejection, and inconclusive results;
add focused cases for the new public conveniences. Keep the one-worker child-call and resource
handoff tests relevant to any publication change. No new live-learning campaign is required here.
Compile affected packages and callers; leave the full suite and complete journey to 06.

## Commit points

Commit save/find/reuse/copy improvements and tests. Commit publication conveniences separately.
Commit evaluation inspection/promotion conveniences as another working change. Update the relevant
existing guide in those commits rather than write a new overlapping manual.

Stop when ordinary reuse, separate publication, and honest evaluation are usable through public
commands. Record commands, commits, and focused checks in `handoffs/04.md`. No full workspace gate.

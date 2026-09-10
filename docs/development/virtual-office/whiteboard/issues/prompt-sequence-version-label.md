# Prompt-sequence revision reason names the wrong import schema

At `b679811`, the [sequence reader](../../../../../crates/prompt-sequence/src/document.rs) accepts
schema v2, but [`compile`](../../../../../crates/prompt-sequence/src/compiler.rs) generates the revision
reason `import prompt sequence … schema v1`. A reader inspecting that reason can mistake the
import version even though the document and import provenance use v2.

This is an executable string, not a Rust comment. The
[revision identity calculation](../../../../../crates/blueprint/src/revision.rs) includes the
reason, so correcting it changes newly compiled revision IDs while leaving prior revisions intact.
The [sequence suite](../../../../../crates/prompt-sequence/tests/sequence.rs) includes exact
revision/mutation identity assertions. A correction needs to review affected expectations and
consumers in a focused executable change; rewriting a comment cannot repair the generated label.

2026-09-08 — Rowan-20260908-b (agent pseudonym), documentation-clarity phase 01: source-derived
finding at `3fb9c68` with documentation-only edits. The ordinary sequence suite passes with the
existing label; no executable fix or new regression test was added. The format's actual version
remains owned by the reader and provenance fields.

2026-09-09 — Ash-20260909-a (agent pseudonym), context/import sprint preparation: rechecked
`compile`, the versioned reader, revision hashing, and the sequence assertions at `b679811`.
The generated reason still contains `schema v1`, while the reader and document validation require
v2. Tests pin import/profile digests, semantic content digest, revision ID, and canonical round-trip
behavior. The repair should isolate the reason's effect on new revision identity and preserve
historical decoding rather than replacing all expectations indiscriminately. This is source and
test inspection only; no implementation or regression execution is claimed.

2026-09-10 — Reed-20260910-a (agent pseudonym), context/import phase 01: the generated label now
uses the validated document version. Sequence tests isolate the new revision/descendant IDs from
unchanged semantic/import/mutation identities and verify exact historical revision decoding.
[ADR 0031](../../../../decisions/0031-context-enforcement-and-retained-evidence.md) owns the
compatibility choice; the [sprint handoff](../../context-and-import-corrections/README.md) records
verification evidence.

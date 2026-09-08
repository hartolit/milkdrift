# Prompt-sequence revision reason names the wrong import schema

The [sequence reader](../../../../../crates/prompt-sequence/src/document.rs) accepts schema v2,
but [`compile`](../../../../../crates/prompt-sequence/src/compiler.rs) generates the revision
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

# Codebase audit

This audit applies `docs/development/engineering-rules.md` to the current production source,
workspace manifests, tests, repository contracts, and canonical architecture/product documents.
It is an actionable engineering review, not a second owner for product status or roadmap facts.

## Priority summary

No unresolved codebase-audit finding remains in the current cohesion scope.

## Current cohesion review

The named production hotspots have focused private ownership:

- the CLI root owns argument composition while command-family modules call one shared session owner
  for credentials, request envelopes, confirmation, bounded input/output, errors, and exit codes;
- runtime causal-context discovery uses one private state across projection seeding, bounded journal
  folding, event classification, explicit-source completion, and branch/join/subworkflow exposure;
  a separate private selection state owns validation, deterministic ranking, omissions, and final
  manifest construction;
- daemon command adaptation uses one exhaustive protocol routing map over private command families,
  with common envelope validation and public result handling owned once;
- daemon attempt reads separate current projection lookup, bounded historical reconstruction,
  authority filtering, and context/provenance attachment while producing one public meaning; and
- redb administrative integrity scanning uses one private typed phase driver under one transaction
  and one refusal/cursor policy.

Repository contracts require an exact reviewed exception, meaningful rationale, and bounded ceiling
for every production Rust source above 1,000 lines. They reject missing, stale, duplicate,
over-broad, and exceeded exceptions. Test and evidence code is classified separately, while the
strict `< 2,000`-line backstop and named-module rule apply to every Rust source. Cohesive exhaustive
reducers retain source-local `#[expect]` rationales instead of global lint suppression.

Diagnostic `clippy::too_many_lines` and `clippy::cognitive_complexity` output remains review input,
not an instruction to split a single invariant mechanically. New findings belong here only when
they remain actionable in the current tree.

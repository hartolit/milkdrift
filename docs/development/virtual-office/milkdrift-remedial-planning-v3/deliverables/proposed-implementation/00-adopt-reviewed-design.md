# P00 — Adopt approved design and establish the execution baseline

Owner: implementation coordinator. Stop after canonical adoption and assignment setup, before P01
unless the user assigned the complete implementation program. Requires explicit user approval,
G1/P1 and the final reviewed P2/A1/N1/F1 revisions, not merely the existence of this proposal.
Read [shared context](context.md) and the full decision brief/review resolution before planning edits.

Inspect Git status/HEAD/recent changes, actual source/constants/readers/tests/manifests and other
active assignments. Reconcile source drift against the planning baseline; rerun only observations
whose meaning changed. Preserve user edits. Record exactly which choices were approved, rejected or
conditional. Do not mark B1–B5 approved because the user approved an independent first slice.

Adopt enduring purpose into `docs/product/vision.md`: explain outcomes and remove accidental
screen/headless-first prescriptions as governing intent. Architecture owns executable responsibilities,
not UI navigation. Add/update ADRs for the approved browser/notation/semantic changes only where they
are durable decisions; retain rationale and strongest rejected alternative. Status continues to say
what is implemented now, not that proposed GUI/remedies work. Roadmap registers finite authorized
work and exclusions; the office registers real owners and P09 as final acceptance before code begins.

Promote the reviewed frontend practice to `docs/development/practices/frontend.md` and link it from
the selector; retain Rust ownership and cross-client semantic rules. Canonical browser deployment,
public API and notation docs need one home each. Preserve useful trial/dissent references until the
user review and promotion are complete; cleanup follows normal office procedure, not this prompt's
need for tidiness.

Correct the confirmed architecture prose drift: current `JoinPolicy::Any` accepts completion,
`FirstSuccess` requires success, and `All` waits for terminal branches; none alone proves physical
loser quiescence. Check `try_satisfy_join` and its tests before editing the canonical explanation.
This correction describes existing behavior and does not authorize changing the policies.

Write the concrete transition inventory for chosen changes: current supported blueprints/mutations,
commands/receipts, optional snapshots, layouts, run/event records, active/uncertain work, current CLI
and peer consumers. Do not promise a whole-store migration that has no reader/rollback design.
Record which owners are unchanged. Inventory is an adoption input, not a new permanent status page.

Acceptance: a new implementer can trace each planned code change to an approved outcome and exact
decision, identify every blocked branch, and find one owner for each lasting fact. No contradictory
authorization remains between roadmap, office and prompts. Run documentation contracts and diff
checks. Commit canonical decisions separately from later implementation, then the checked assignment
setup. Report source drift and any reopened decision; do not invent user acceptance or run production.

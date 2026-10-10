# How should reusable work, services and execution location relate?

## Current decision — P1, revision 2

Rowan-20261010 selects shared discovery with **distinct committing actions** as a planning
candidate: use an exact workflow reference on its owning workflow daemon; make an independent
copy where permitted; or call an offered service/operation under its public contract. Keep direct
routes discoverable. Every committed item keeps its owner, exact revision/generation, allowed
inspection and recovery identity visible after reconnect. This is a reviewed hybrid, not a
claim that a generic “Use” gesture has one execution meaning.

Requires [G1/r1](../vision/remedial-purpose.md), U03 and qualified U07. Source support comes from
pinned child creation, publication association and host serving; see the
[behavior traces](../../../milkdrift-remedial-planning-v3/working/behavior-evidence.md) and
[observed journeys E1](../../../milkdrift-remedial-planning-v3/working/observations.md).
Publication is not required for all reuse or all remote execution. A client connection creates
neither peer trust nor cross-owner transaction rights. Invoke-only users receive declared public
results, not a fabricated expandable internal graph.

Revision 2 incorporates Cyra's fresh-read countercases: a readable revision at B is not a
pinnable child at A unless an explicit supported transfer/import and local validation establish
that reference. The default exact reference remains at its current owner. Copy may refuse an
identity-bound governing agreement. An execution-only destination cannot be silently upgraded,
and role removal must settle existing obligations under current checks. These are source-grounded
restrictions on the proposed interaction, not a silent loss of existing working behavior.

Compared alternatives: distinct discovery routes make authority easier to teach but require the
user to classify work before finding it; universal publication regularizes service contracts but
adds service authority/setup to local reuse and cannot replace direct host work. A uniform backend
call object is not justified merely by a shared card. The selected hybrid retains current effect
owners and gains common browsing at the cost of client presentation complexity and persistent
boundary labels. Its comprehension benefit remains untested.

Best unresolved objection: a common chooser may hide the very distinction it needs to explain.
Ada favors common discovery; Bram favors distinct actions/routes as the reliable teaching path.
Both accept the candidate only with explicit consequences before commitment. Neither claims a
human usability result. The first real frontend checkpoint must test same names on different
owners, copy versus pin and opaque service control; failure reopens the discovery decision.

Informs notation call expansion, frontend actions, owner-qualified references, structured-authoring
work and service UI. Does not require new relay, migration or universal publication features.
User choice still needed if a future implementation proposes dropping copying, exact composition
or restricted calls. Default ordering is a reversible interface recommendation, not fixed user intent.

## Purpose questions

Can someone reuse their own work without service-administration rights? Can a service customer
understand a lost reply without privileged internal inspection? Does selecting another machine
express placement or accidentally transfer workflow ownership? Would separate routes teach these
consequences more clearly than common discovery?

## Contributions and revision grounds

2026-10-10 — Ada-20261010, actual agent `/root/intent_ada`: independent first position and later
cross-examination are preserved in [Ada's record](../../../milkdrift-remedial-planning-v3/working/trial-ada.md).
The case for common discovery is reduced navigation, not reduced authority contracts.

2026-10-10 — Bram-20261010, actual agent `/root/intent_bram`: independent position and actual reply
are preserved in [Bram's record](../../../milkdrift-remedial-planning-v3/working/trial-bram.md).
The objection concerns control expectations after permission changes, not whether one chooser can
technically dispatch several commands. Distinct repeat-operator routes remain a material preference.

2026-10-10 — Cyra-20261010, fresh-to-trial reader `/root/systems_cyra`: added the local-store pin,
identity-bound copy and role-removal countercases. See the
[fresh read](../../../milkdrift-remedial-planning-v3/working/trial-fresh-read.md).

2026-10-10 — Rowan-20261010, coordinator `/root`: P1/r1 was the candidate common-discovery model
in the initial trial accounts. P1/r2 narrows its actual target rules after the fresh reading.
Changed dependent claims are recorded in the trial and later review-resolution; hypothetical
permission gains remain separate from real source corrections.

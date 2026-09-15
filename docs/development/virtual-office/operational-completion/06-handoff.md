# Assignment 06 handoff

Owner: current Codex task, explicitly assigned by the user. Base: `6967da8`. Result: reviewed and
committed to `main` at the user's request, with no unresolved blocking findings. Git records the
result commit. Assignments 07–08 remain unstarted; production activation remains separately governed.

Task placement is now owned by the capability requirement: bounded locality and exact-peer sets
flow through blueprint and sequence import, narrowed revision admission, deterministic authorized
selection, frozen snapshots and final entry. Empty sets deny every candidate; contradictory sets
and wildcard peer syntax are refused. Unavailable, forbidden or removed generations cannot cause
wrong-host fallback. Attempt inspection retains requested constraints and exact peer/catalog facts;
pre-selection failure diagnostics remain in the existing bounded operator log, as explained in the
[peer guide](../../../operations/peers.md#pin-tasks-to-approved-hosts).

The maintained [two-host blueprint](../../../../examples/operator/peer-placement.json) executes
repository A on `peer-a` and repository B on `peer-b`. The test uses three production daemon host instances over real HTTP on local TCP, separate
temporary stores, repository directories and credentials. It includes a local look-alike, verifies one entry per approved host, exact returned artifact bytes and frozen provenance,
and inspects the same attempts after reconnect and reopening all three stores.

Completing that workflow exposed a missing artifact composition path. Serving publication now uses
the entered peer execution's remaining artifact allowance in the existing core store. Origin-side
references remain exact external causal commitments to the accepted request. The origin imports
durable output observations through authorized metadata and verified chunk transfer before reporting
their references. No parallel peer artifact store or fictitious serving-side workflow is created.
Exact publication replay does not spend the same allowance again; empty outputs commit without a
chunk. Ownership, expiry, sensitivity, byte bounds and live download revocation remain enforced.
Review corrected output-transfer lifecycle handling: each chunk renews the origin runtime lease
and checks adapter shutdown. Three regressions prove completion across lease intervals and
staging cleanup without further downloads after renewal refusal or shutdown. The correction adds
no public API.

[ADR 0037](../../../decisions/0037-constrained-peer-placement.md) and the
[API policy](../../../reference/public-api-policy.md) use coordinated pre-release upgrades.
Snapshot v3 is required; v1/v2 readers, digest branches and missing-category runtime fallbacks are
removed. Blueprint revisions keep their identity when placement is absent. Blueprint/sequence,
journal, configuration and storage versions remain unchanged. Peer protocol 1.3 and control protocol
2.7 require the exact current version. Current saved peer acceptance/replay/conflict and uncertainty
semantics remain exact across restart. The control client validates the version on negotiation,
JSON replies, error replies and streamed observations.

The Linux/Rust 1.95 full gate passes: formatting, all-target/all-feature checking, **803 workspace
tests**, **24 doctests**, warning-denying Clippy/rustdoc, dependency audits, test discovery and all
**24 repository contracts**. Five manual longevity cases remain ignored in the ordinary gate and
were not rerun. The full suite includes capability/authority/blueprint/sequence, runtime final-entry,
host resolution, peer protocol/service and two-daemon coverage. Two pre-existing Linux fixture
permission omissions were corrected: private temporary directories explicitly use mode 0700 and
the recovery-shutdown test credential uses mode 0600.

Actual-binary operator, deterministic model and controller qualification scenarios all pass using
the rebuilt daemon, CLI and helpers. They exercise common composition with controlled local
resources; no real provider, production peer or controller activation is qualified here. Raw commands,
logs and binary/source hashes from the fresh review run are under `target/review-placement/`;
`gate.json` records the executed gate and binary lanes. Default/all-feature API inventories for
14 affected libraries are under `target/public-api/placement/`. Placement/read documents are current schema contracts;
peer publication and metadata methods are consumed workspace adapter ports. Internal match,
digest, transfer and accounting helpers stay private. Existing test/evidence exports remain gated.

The [authority guide](../../../operations/authority.md#match-the-complete-task-requirement), peer guide,
architecture and current status replace the blanket locality/peer admission refusal claim. Exact
capability/profile/operation/trust constraints remain conjunctive. Inputs still require explicit
preparation/transfer; there is no discovery, shared checkout, credential copying, tag selector or
cluster scheduler. The [verification guide](../../verification-evidence.md#actual-binary-scenarios)
owns reproducible evidence and its limits. Review of this assignment does not authorize the next
assignment or production controller activation.

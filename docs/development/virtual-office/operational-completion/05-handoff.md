# Assignment 05 handoff

Owner: current Codex task, explicitly assigned by the user. Base: `9f6bd0f`. Result: reviewed for
the user-requested commit to `main`. Assignments 06–08 remain separately assigned work.

Runtime context assembly now resolves the existing exact manifest/response session selection,
verifies journal linkage, scoped authority, retained policy and acceptance markers, and publishes a
bounded version-1 companion in the ordinary manifest. Both provider mappings consume its ordered
role-labelled messages and complete tool pairs. Fresh has no implicit history. Process and
provider-managed sessions remain refused. The canonical explanation is in
[the guide](../../../guides/model-continuation.md) and
[ADR 0036](../../../decisions/0036-explicit-model-continuation.md).

The final Windows/MSVC full gate passes: formatting, all-target/all-feature compilation, 776
workspace tests, 24 doctests, warning-denying Clippy and rustdoc, dependency audits, test discovery
and all 24 repository contracts. Five manual longevity tests remain ignored in the ordinary gate;
they were not rerun for this assignment. Focused model and model-provider suites pass, including
30 continuation cases through each mapping. The 13 focused runtime context/session tests also pass.
The full gate includes blueprint, causal-context, process-sequence validation and daemon suites.

The continuation matrix checks exact predecessor messages and selected evidence, Fresh isolation,
missing/corrupt/unsupported artifacts, wrong pairing, unsafe historical policy, item/byte/journal
bounds, excluded categories, revoked authority, another actor and sibling branch, complete and
incomplete tool exchanges, store reopen, expired pre-entry lease recovery and unrelated concurrent
publication. Parser tests refuse injected response roles and unknown message semantics. Historical
source messages never become current instructions. Process capabilities remain Fresh-only even
when their operation is named `model.generate`.

The actual daemon/CLI model lane passes against deterministic local endpoints. Authorized proposal
adoption names the inspected predecessor exactly; the continued request carries its question and
answer, while Fresh receives neither. Restart before release preserves the exact references;
restart after completion preserves inspection.
A separate acceptance-rejected answer stays rejected after restart and causes zero dependent
endpoint requests. The ordinary operator lane passes restart, exact replay/conflict, proposal,
artifact and uncertainty scenarios. These final runs use the rebuilt binaries after the full
compilation gate completed.

Review fixes recheck exact sources after local preparation and before external entry or account
reservation, catching corruption and selective read revocation after claim. Saved request messages
now obey the same tool-trace exclusion and declaration checks as returned calls. Regressions reproduce
both defects before the fixes; both mappings also pass three-invocation text and tool chains.
The full suite also exposed cancellation arriving during a blocked body read. The stream reader
now checks cancellation on return before interpreting EOF, errors or content, with a forced-read
regression and a real-socket fixture that coordinates cancellation before closure.

Raw full-gate and focused logs, operator/model lane logs, test counts and source/binary hashes are
under `target/review-continuation/`. The final deterministic report and retained continuation
artifacts are under `target/review-continuation/verified-model/`; initial build-lock interruptions remain
separate from completed checks. All 14 default/all-feature API inventories are under
`target/public-api/review-continuation/` and match the original implementation's surface.

Public API review covers model, capability, blueprint, control, runtime, prompt-sequence and
model-provider. New model items are durable schema/workspace adapter
contracts. Three shared acceptance protocol names keep the control evaluator as the single decision
owner. Existing runtime test helpers and the provider parser evidence driver remain feature-gated.
Context resolution and wire encoding helpers are private; no session ledger, provider family,
UI, workflow primitive, storage-format or control-protocol expansion is introduced.

Continuation supports bounded causal text history and complete tool exchanges in the same run and
visible scope lineage. Multimodal predecessors, provider-specific request extensions, incompatible
current profiles and provider-managed sessions remain explicit refusals. The guide describes fresh
execution with deliberately selected prior evidence as the supported alternative. Existing effect
uncertainty still constrains retries; no automatic provider retry or exactly-once claim is added.

Both supplied Bonsai aliases pass ordinary actual-binary model smokes at a 4,096-unit request
allowance. Final text, usage and settled restart are observed without server configuration changes.
These smokes do not exercise live continuation; effective thinking settings remain unknown.
The second smoke was rerun with private executable copies after a concurrent build changed its
byte-pinned helper and correctly prevented restart before model entry. Its successful report is
`target/review-continuation/verified-bonsai-2/report.json`; the first is
`target/review-continuation/verified-bonsai-1/report.json`. No broader platform or agent qualification
is added. Git records the user-requested integration commit.

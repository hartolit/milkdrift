# Assignment 03A corrective handoff

Owner: Codex. Base: `709de9b824d0602f01bd374c7e4caf2e571540fe`, including both corrective
model-budget assignments. Integration review preserves their implementation and fixes four
additional defects described below. No live installation changed. Production activation acceptance
remains separate, and Assignments 04–08 have not started.

## Result and ownership

The original local-entry defect came from unknown model input/billing envelopes and monetary
rules that could stop unbilled work at zero. The prior corrective work supplied explicit endpoint
contracts and settlement. This continuation closes unspecified account units, conflicting-charge
metadata, initial/later zero-spend regression coverage, and the actual third-model-request refusal.

The [model adapter](../../../../adapters/model-provider/README.md#reserve-and-settle-supported-model-usage)
owns operator-declared unbilled service and frozen, checked, upward-rounded text tariffs. Complete
prepared UTF-8 bytes conservatively bound byte-BPE tokens with inspected template overhead;
`max_tokens` bounds one selected local generation including reasoning. This is logical prompt/output
usage, not GPU/cache work. Preparation retains one exact request through authority checking and
atomic entry. Unknown pricing stays unknown; unsupported content and limits refuse locally.

`AdmissionUnit` in capability makes the shared token meaning explicit; persistence refuses bounded
unspecified units without changing allowance. Currency-free zero-spend accounts admit unbilled
requests and reject currency-bearing requests. Missing usage, excessive use and conflicting costs
retain uncertainty; raw charge evidence is no longer mislabeled as a tariff calculation. Control's
initial and later zero-spend assessment regression covers continued work and positive/unknown refusal.

The existing daemon/CLI controller scenario now retains inspected/declared/unknown server facts,
allows eight process entries and two model entries, and rejects a third model task after accepted
repair and a completed-work restart. The previous process-limit ending is replaced in this scenario;
separate process-bound tests remain. The context-free adapter envelope stays unknown. No new
controller, ledger, provider family, discovery service or inference owner was introduced.

Integration review closes four further defects. Streamed partial usage can no longer erase an
earlier charge or excessive output count; nullable usage details are accepted as missing evidence;
raw streamed usage is retained even without response identity; and billed profiles declare their
conservative charge in the per-request authority check. That permission amount rounds upward to
hundredths of the currency, while cumulative admission still reserves the exact prepared request.
Regression tests exercise both successful settlement and refusal before HTTP entry.

## Verification and retained evidence

| Check | Result and retained evidence |
| --- | --- |
| Full Windows/MSVC gate | 750 workspace tests, 24 doctests, all 24 repository contracts; formatting, check, Clippy/rustdoc with warnings denied, dependency audits and discovery pass. Five manual longevity tests are ignored in the ordinary gate. `target/review-03a-final-gate-results.json`, `review-03a-binary-results.json` and corresponding logs retain exact commands/results. |
| Changed decisions | Four new assertion-caught faults: overwrite partial stream usage, reject nullable details, omit raw stream usage, round permission cost down. Sources restored and model suite passes. `target/review-03a-mutation-results.json` and `review-03a-permission-mutation.json`. Prior unspecified-unit, conflicting-cost and zero-spend-repeat faults remain applicable to their unchanged owners in `target/corrective-03a-mutation-results.json`. |
| Required longevity | Both release lifecycle/restart and admission/reservation/artifact-turnover tests pass again. Exact invocations and results are in `target/review-03a-extra.ps1` and `review-03a-extra-results.json`; the subsequent model-adapter fixes do not affect these control-owned cases. |
| Ordinary and deterministic binaries | Operator log `target/review-03a-operator.log`; model report `target/review-03a-local-deterministic/report.json`; controller report `target/review-03a-controller/report.json`. Existing authority, concurrency, late settlement, compaction and entered-process crash assertions remain active. |
| Real local controller | `target/review-03a-real/report.json`: repair accepted, two direct calls settle 6661 input / 102 output tokens, four process entries, 45487 logical artifact bytes, zero provider spend, absent currency, empty reservations. Third attempt is rejected with `model_admissions`, no uncertainty or further admission. The complete isolated run takes 477 seconds without retry or allowance changes. |
| Public API | Eight default/all-feature inventories reviewed for capability, model-provider, persistence and control under `target/public-api/review-03a/`; all match the corrective inventories. The review fixes add no public surface or test-only export. |

Clippy caught a redundant default initializer after the permission declaration filled its last
field. Removing it changes no behavior; formatting, full workspace checking/Clippy and the model
suite pass afterward. The original diagnostic remains in the gate log.

The real scenario intentionally starts with incorrect work. The model's usable review enables the
normal proposal and separate approval; prospective repair passes verification and acceptance.
Restart at approval and completed-review holds preserves the account. The deliberately excessive
third task ends the child/root failed; controller status is completed with `cycle_eligible: false`
and no `reached_bound`. This is successful refusal evidence, not a successful workflow terminal.

The existing LM Studio endpoint `http://127.0.0.1:1234`, alias `prism-ml/bonsai-27b:2`, and
byte-pinned local Codex CLI `0.154.0-alpha.6.2` were used without reconfiguration. A fresh read-only
SDK inspection in `target/review-03a-server/` confirms unchanged accounting-relevant settings. Inspected Bonsai
Q1_0 settings include context 8192, `truncateMiddle`, actual Jinja template, base thinking enabled
and empty prefill. Two bounded probes retain one-token completion and oversized-prompt refusal;
the SDK hit a Windows shutdown assertion after saving results. This does not prove SDK lifecycle,
remote termination, power-loss durability or escaped-descendant containment. Agent-internal model
calls are outside the direct model meter; the coding agent was explicitly local-only.

Reproduce using the build and isolated command in the
[local guide](../../../guides/local-model-endpoint.md#controlled-local-text-requests), a new output
directory, `target/corrective-03a-server/review-profile.json`, `target/review-03a-server/facts.json`, and
`target/model-budgets-server/agent-profile.json`. Review these operator contracts before reuse.
The successful session retains selected facts, profiles, source patch/new files and exact binary
identities. `target/review-03a-binaries.ps1` retains the exact build and four actual-binary commands.
Current verification uses that executable source; final subsequent edits are prose only.
Earlier failed fixture assertions and prior agent runs remain separately retained as explained in
the [evidence guide](../../verification-evidence.md#actual-binary-scenarios).

## Compatibility and remaining activation decision

Endpoint and controller policy schemas are 2; old profiles do not acquire unbilled defaults.
Redb internal document format is 16 (physical schema 11), explicitly refusing older/future stores.
Historical omitted envelope units remain unknown and re-encode unchanged. Format 15 accounts are
not reopened under the new unit rule. Protocol remains 2.5, CLI JSON 2 and daemon configuration 9;
model/blueprint documents and event tags are unchanged. [ADR 0033](../../../decisions/0033-explicit-controller-qualification.md)
owns the compatibility decision.

The specific activation blocker is independent coordinator acceptance of these current results.
The successful local loop satisfies the real-model requirement; no cloud run is additionally
required. Production stays default-disabled and `enabled` remains refused until that acceptance.
No paid calls, live service changes, or subsequent assignment are authorized by this handoff.

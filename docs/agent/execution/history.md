# Execution history

This file is the chronological closure record for completed execution work. It preserves phase-specific rationale, acceptance evidence, measurements, and validation provenance without making each closed phase a separate top-level document.

For current product truth, use [implementation status](../../project/implementation-status.md). For repeatable commands, use [validation](../../project/validation.md). Historical results below apply only to the baseline named in each entry.

## Phase 3 — backend-independent generation kernel

**Prepared:** 2026-07-23
**Scope:** `docs/execution/critical.md` completion package for Phase 3
**Baseline:** uploaded source archive with `Cargo.lock`
**Recorded outcome:** complete

The canonical verification command completed successfully against the recorded source tree. The phase closed the generation-kernel safety gaps that had remained after initial implementation.

### Closure matrix

| Requirement | Recorded closure |
|---|---|
| Reproducible lockfile | `Cargo.lock` was retained and every workspace package was represented. |
| Bounded cleanup | `CleanupRetryPolicy` uses a non-zero total-attempt limit; the initial failure is attempt one; the default is three total attempts; exhausted resources are skipped by automatic maintenance. |
| Cleanup observability | Structured cleanup resource/retry/poll/exhaustion state, snapshot counts, last-attempt state, maintenance-error retention, and cleanup-pending/exhausted generation output were added. |
| Unified model cleanup | Normal unload, admission rollback, drain escalation, and shutdown route unload failures into the same quarantined model state with retained accounting and bounded retry. |
| Admission capacities | Prompt preflight, total sequence bounds, exact vocabulary logits, output token/record policy, backend footprint, and complete logical generation-workspace footprint are validated before workspace allocation or native sequence creation. |
| Workspace accounting | Logits, sampling indices, repetition epochs, prompt/history/generated tokens, EOS tokens, and stop descriptors/patterns are counted; accounting remains reserved until `Released` is published and task storage drops. |
| Primary plus cleanup failure | Backend and sampling terminal outcomes remain independent from cleanup state; cleanup failure does not replace the original generation failure. |
| Worker cleanup/disconnection | Maintenance errors are retained. Shutdown and endpoint disconnection perform bounded cleanup and retain unresolved native ownership rather than assuming implicit `Drop` cleanup. |
| Terminal shutdown | Shutdown becomes terminal after its result event is delivered, including exhausted cleanup; scheduled workspaces can be released without waiting for frontend output draining. |
| Fault injection | Deterministic coverage was added for prefill/output/memory/logit rejection, cancellation, drain timeout, cleanup exhaustion, degraded admission, healthy-model isolation, retained memory, and exact release. |
| Documentation accuracy | Runtime, lifecycle, status, and backend documentation were synchronized with the implemented boundary. |

### Principal implementation changes

- Added explicit output-capacity contracts to generation requests.
- Added pre-allocation preflight and repeated commit-time validation.
- Added retained generation-workspace count and footprint accounting.
- Kept request identity owned by the scheduler through terminal output release.
- Added bounded sequence/model quarantine with retry exhaustion.
- Preserved unload cancellation totals across deferred cleanup.
- Made terminal publication robust when cleanup completes before pending-cleanup publication.
- Added fake-backend sampling and retained-memory counters.
- Made explicit shutdown terminate the inference worker on both success and exhausted cleanup.
- Added regression coverage for failed-cleanup join and shutdown behind undrained output.

### Recorded validation

```text
cargo fmt --all
cargo run --locked --bin llm-app -- verify
cargo deny --workspace --locked check \
  advisories bans licenses sources
lychee --offline --no-progress "**/*.md"
git diff --check
```

This evidence belongs to the Phase 3 tree; it is not a current-main validation claim.

## Phase 4 — Candle CPU vertical slice

**Prepared:** 2026-07-25
**Implementation baseline:** `8de2ebf2811d5158e3439efe2114379de59322d0`
**Scope:** Candle CPU vertical slice plus lifecycle, validation-provenance, and external-fixture closure corrections
**Recorded outcome:** implementation complete; the report required a final locked verification and pinned external-model smoke on the post-closure tree

The baseline run exercised the real Candle Llama path through E0: inspection/load, prompt prefill, sampling, incremental decode, bounded token output, cancellation between backend calls, terminal/released publication, empty request/workspace/cleanup accounting, model unload, worker shutdown, and join.

The closure correction additionally required successful sequence destruction to produce `SequenceState::Finished`, explicit destruction in adapter tests, empty runtime/model snapshots after unload, external fixture hygiene, and synchronized documentation.

### Closure matrix

| Requirement | Recorded closure |
|---|---|
| Prompt positions | Deterministic fixtures verify that prefill consumes the complete prompt and decode advances from the preserved position. |
| Final prefill logits | Token-identity fixture weights prove that the final prompt token controls the full-vocabulary logits used for sampling. |
| Decode progression | Interleaved sequences verify independent token and position progression. |
| Vocabulary logits | Adapter and E0 integration tests require exact vocabulary-sized caller-owned F32 output. |
| EOS handling | Candle/E0 integration publishes the EOS token followed by terminal and released EOS outcomes. |
| Scalar compatibility | F32/F16 execute in supported CPU dtypes; BF16 source tensors are validated then upcast to F32 because Candle 0.11 CPU matmul does not execute BF16 operands; admission uses execution dtype. |
| Sequence destruction | Successful destruction marks the sequence `Finished`; tests assert that state and terminal `Released` publication. |
| Model unload | Completion, EOS, and cancellation paths unload with `RejectIfBusy`, then assert no loaded models or retained runtime/model accounting before shutdown. |
| Cancellation boundary | One-token output capacity creates deterministic backpressure; cancellation is observed before another backend call and ownership is released. |
| Real-model execution | The pinned `neubla/tiny-random-LlamaForCausalLM` revision generated eight tokens through the hosted E0 worker in the recorded baseline. |
| Failure classification | The external smoke distinguishes configuration/fixture failures from runtime/lifecycle failures. |
| Measurements | The smoke captured load, first-token, decode-throughput, cancellation, unload, and RSS observations. |
| Ordinary CI | Deterministic integration uses a committed project-authored tiny fixture and requires no model download. |
| External fixture hygiene | External weights use ignored `.phase4/` storage and are not redistributed by the repository. |

### Recorded baseline validation

The baseline completed:

```text
cargo run --locked --bin llm-app -- verify
```

The recorded run included architecture/dependency validation, formatting, workspace checks, tests/doctests, Clippy, rustdoc, and benchmark compilation. No GitHub Actions run was attached to that baseline; the evidence was supplied local output rather than independent CI attestation.

### Recorded external smoke

The model file passed this SHA-256 check:

```text
49c20f32c6c597480fcaec5df2f86c645eabea765cbea1e67886dbae45e5c992
```

| Field | Recorded value |
|---|---|
| Repository | `neubla/tiny-random-LlamaForCausalLM` |
| Revision | `39ca1f8a1fc940377c5cb49a21aff73bb99b52f5` |
| Expected architecture | `LlamaForCausalLM` / runtime `Llama` |
| Prompt token IDs | `1,2,3` |
| Generated token IDs | `18568, 1727, 8705, 3598, 27426, 4496, 998, 16911` |

| Measurement | Recorded result |
|---|---:|
| Model load duration | 0.005661 s |
| Time to first generated token | 0.060969 s |
| Decode throughput | 21.954 tokens/s |
| Cancellation latency | 0.045297 s |
| Model unload duration | 0.000380 s |
| RSS before load | 4,636 KiB |
| RSS after load | 11,116 KiB |
| RSS during generation | 14,088 KiB |
| RSS after unload | 10,412 KiB |

Elevated post-unload RSS was not treated as retained model ownership because allocators may keep freed pages for reuse. The ownership evidence was released records, empty accounting, an empty post-unload snapshot, successful worker shutdown, and clean process exit.

The repeatable external-model procedure now lives in [project validation](../../project/validation.md); this section retains only the historical evidence.

### Historical boundary at closure

At this point the slice was CPU-only and token-level at E0. Tokenizer ownership, decoded-text streaming, E1 generation commands, and frontend generation were outside the Phase 4 boundary. The tiny random model proved integration rather than language quality, strict allocation-free Candle execution was not claimed, and GPU/chat/GGUF UI paths remained unsupported.

The report required the canonical locked gate and external smoke to pass on the same final commit with no intervening source changes, with `git rev-parse HEAD` recorded alongside the outputs.

## Phase 5 — application-runtime generation façade

**Prepared:** 2026-07-25
**Implementation baseline:** `f6ac1806c33d4a1d84dfabb66c14f3475af5872a`
**Scope:** expose direct-completion generation through `application-runtime`
**Recorded outcome:** source boundary implemented; final locked validation was required on the post-closure tree before marking the phase complete

The closure work addressed application-owned unload behavior, E1-level generation integration coverage, presenter dispatch cleanup, and documentation synchronization.

### Closure matrix

| Requirement | Recorded closure |
|---|---|
| Narrow E1 generation API | `start_generation`, `cancel_generation`, `poll_event`, and borrowed `pull_output` form the frontend-neutral generation surface. |
| Stable E1 settings | Application-owned settings validate completion and sampling controls before E0 admission. |
| Direct-completion prompt | Resolved `HfTokenizer` encodes ordinary prompt text once; no chat-template support is claimed. |
| Owned streaming decode | `HfOwnedStreamingDecoder` owns request-local tokenizer/decode state without full-history re-decode. |
| Token-to-text bridge | E1 translates bounded E0 token/state batches into bounded UTF-8/state batches. |
| Backpressure | Integration tests exercise output stalls and resume without token/text loss. |
| Application state/events | Tests cover start, cancellation, terminal state, usage, and release. |
| Single-model policy | E1 configures exactly one resident model. |
| Unload policy | E1 exposes `ModelUnloadBehavior::{RejectIfBusy, CancelActive, Drain}` without leaking E0 `UnloadPolicy`. |
| Unload integration | Tests cover idle unload and reject/cancel/drain behavior while generation is active. |
| Worker lifecycle | Application-level tests cover inference-worker disconnection and explicit bounded shutdown. |
| Frontend isolation | Slint remains presentation-only; E0 owns generation scheduling and generated text remains pull-oriented. |
| Documentation | E1, desktop, status, and related documentation were synchronized with the same boundary. |

### Integration coverage

The test composition used the repository's download-free Candle Llama fixture, a project-authored 16-token WordLevel tokenizer fixture, real hosted E0 scheduling/Candle execution, and real E1 prompt encoding, admission, cancellation, output translation, state/events, unload policy, worker disconnection, and shutdown.

The suite covered:

- generation without a loaded model;
- invalid settings and empty prompts;
- duplicate generation admission;
- token-limit, EOS, and textual stop-sequence completion;
- token-to-text streaming;
- output backpressure and resume;
- cancellation under constrained output capacity;
- idle unload;
- reject/cancel/drain unload while generation is active;
- inference-worker disconnection;
- explicit application shutdown.

### Closure validation rule

The report required the canonical gate on the exact resulting tree, with focused commands used only for diagnosis. Current procedures are centralized in [project validation](../../project/validation.md), and current completion state belongs in [implementation status](../../project/implementation-status.md).

## Phase 6 — first usable Slint product

**Prepared:** 2026-07-27
**Implementation baseline:** reviewed `phase-6` commit `68438648c09bc008e628508ebf269456c6299096` plus the source-level review closure
**Scope:** expose the existing E1 direct-completion path through `desktop-slint`
**Recorded outcome:** source and download-free validation complete; no manual graphical external-model session was recorded

Phase 6 replaced the lifecycle-only window with the first complete presentation path while keeping generation scheduling, prompt/tokenizer behavior, cancellation, unload policy, and resource ownership below the frontend.

### Closure matrix

| Requirement | Recorded closure |
|---|---|
| Minimum interface | Repository/revision, resolve/load/unload, prompt, generated output, generate/cancel/clear, status, terminal reason, prompt/generated usage, and Candle/CPU identity are visible. |
| Frame-aligned pulling | The existing 16 ms timer drains at most 64 events, performs one unconditional bounded E1 output pull, applies one presentation delta, then synchronizes controls and usage. |
| Batched text | Borrowed fragments are copied by request identity and only the new frame fragment crosses into Slint. The persistent `TextEdit` keeps selection ownership while its viewport coordinates are saved/restored around append; final output is still pulled after terminal release. |
| Control truth | Resolve/load/generate/cancel/unload enablement derives from `ApplicationState`; prompt emptiness affects only Generate. |
| Cancellation | Cancel requests E1 cancellation and reports that completion remains pending until a safe backend boundary. |
| Unload | The UI retains E1's bounded drain behavior and does not disable unload merely because generation is active. |
| Terminal cleanup | Finishing, cleanup pending/exhausted, and released states are presented distinctly; cleanup exhaustion is not reported as release. |
| Clear output | Clear mutates presentation text only and preserves request identity, runtime history, usage, and lifecycle state. |
| Shutdown | Normal event-loop exit and post-runtime window-construction failure both invoke explicit bounded shutdown; combined Slint/cleanup failures are retained. |
| Presenter tests | Nine pure tests cover running cancellation availability, cancellation-pending messages, prompt admission, post-release Generate/Unload controls, fragment-only append/reset behavior, successful release, failure diagnostics, and cleanup pending/exhausted messaging. |

### Recorded validation

The Phase 6 closure tree passed:

```text
cargo test --locked -p desktop-slint
cargo clippy --locked -p desktop-slint --all-targets -- -D warnings
cargo run --locked --bin llm-app -- verify
```

The canonical run covered architecture/dependency validation, formatting, workspace checks, tests/doctests, strict Clippy, rustdoc, and benchmark compilation. This was local working-tree evidence; no independent CI run or committed Phase 6 revision was recorded.

The graphical product scenario was not manually driven against an external model in this environment. Existing download-free E1/Candle integration coverage proves generation, backpressure, cancellation, unload behavior, worker disconnection, and shutdown below presentation; the new desktop tests and Slint compilation prove the presenter mapping and generated UI boundary.

## Pre-Phase 7 architecture closure — composability and workspace taxonomy

**Prepared:** 2026-07-29
**Reviewed baseline:** `f8b3396cc23085696123b95c9dcb4b17c3d9c214`
**Scope:** close the E1/capability/execution-boundary review and adopt the accepted physical workspace taxonomy before conversation semantics expand the product
**Recorded outcome:** architecture changes are present in source; the complete canonical gate still needs a validation record tied to the exact final Phase 7-preparation commit

The closure deliberately changed ownership and enforcement without inventing remote-provider implementations or a generic service graph.

### Recorded architectural closure

- `corrective-workflow` is independently owned under `crates/runtime` rather than implemented or re-exported from E1.
- E0 remains the local/native model-resource and token-step engine; peer and hosted execution are defined as future coarse request/stream targets above E0 rather than fake E0 backends.
- `application-runtime` remains the frontend-neutral application coordinator and current concrete local composition root until a second backend/deployment proves the extraction seam.
- Physical roots are now `domain`, `platform`, `adapters`, `runtime`, and `apps`; `host-runtime` moved to `platform` while retaining its narrow process-host responsibility.
- Runtime roles fail closed: only registered E0/capability/E1 packages receive runtime authority, and an arbitrary crate under `crates/runtime` is rejected.
- Platform roles fail closed: only the reviewed `host-runtime` package is currently registered under `crates/platform`.
- Runtime production dependencies on platform/adapters or another runtime require exact reviewed source/target/kind entries with inspectable justifications in addition to satisfying the layer matrix.
- Integration fixtures cover unregistered runtime/platform packages and an otherwise layer-valid but unreviewed E1-to-capability edge.

### Phase 7 implication

Conversation semantics may now grow in E1 while context selection remains in `context-planner`, model-specific rendering remains a compatibility boundary, and execution location remains outside message identity. New memory, tool, workflow, peer-routing, or provider concerns do not become E1 modules or new runtimes merely because chat needs to call them.

The next validation record should use the final preparation commit rather than treating the Phase 6 gate as evidence for this later architecture tree.

## Phase 7 — real chat and context planning

**Prepared:** 2026-07-29
**Committed implementation:** `2b03cfbd7d82ef4aee39270a1f95b81c9bfada44` (`Phase 7 vibes`); first review fixes were committed as `8d134e38cff0c2203a1ade9714d3aa92a65b9a3a` and formatting cleanup as `3b4541f50fcf614bc65938d448b383f507d27fcd`
**Scope:** add one honest compatible chat path, connect context planning to real generation input, and replace the Slint completion surface with E1-owned conversation behavior
**Recorded outcome:** the original implementation working tree recorded source, download-free integration, presenter, focused strict-Clippy, and canonical locked-gate validation; the final semantic closure still requires a new exact-tree gate before Phase 8

Phase 7 retained E0 token scheduling and native resource ownership while adding frontend-neutral conversation semantics to E1. It did not add a provider/peer abstraction, persistence, general branching, or a universal Llama template.

### Closure matrix

| Requirement | Recorded closure |
|---|---|
| Conversation records | E1 stores stable raw record and response-attempt identities, monotonic order, role, UTF-8 semantic content, provenance, retention, measured/generated/conservative token estimate, terminal state, and supersession without backend/transport identity. |
| Attempt semantics | User input commits before planning/admission; assistant output is a streaming attempt. Successful unsuperseded attempts enter active context. Cancelled, failed, and superseded attempts retain partial text/provenance but remain excluded. |
| Explicit compatibility | Chat support is limited to immutable `TinyLlama/TinyLlama-1.1B-Chat-v1.0` commit `fe8a4ea1ffedaf415f4da2f062534de366a451e6`, contingent on tokenizer `</s>` resolving to EOS token ID 2. Unknown or unreviewed provenance returns `UnsupportedChatCompatibility`; direct completion remains available. |
| Prompt and termination | The profile renders `<|system|>`, `<|user|>`, and `<|assistant|>` records with `</s>` message boundaries and replaces caller stop policy with EOS token 2 for one assistant turn. Formatting and termination material are tested together. |
| Context planning | E1 derives request-local atomic planning units: each completed historical user/assistant pair stays together, while the target user and pinned content remain pinned. Units become `ContextEntry` values for generic `context-planner`; selected units expand back into ordered raw records for rendering and diagnostics. |
| Exact correction | `context-planner` exposes the least-preferred selected droppable unit according to its admission policy. Every E1 retry removes exactly one whole unit and attempts are bounded by the initial droppable-unit count plus one; unchanged correction and pinned-only overflow fail explicitly. |
| Capacity | Admission uses the smaller input allowance imposed by context-minus-output reservation and maximum prefill. Download-free E1/Candle tests assert submitted prompt usage equals exact diagnostics and remains within loaded capacity; diagnostics report the estimate for the final selected set after correction. |
| Regeneration and clear | Regeneration preserves and supersedes prior attempts for the same user turn. Clear is rejected while a conversation response is active and removes raw history/diagnostics only after terminal state. |
| Slint chat surface | The window now presents a transcript, message composer, send/regenerate/cancel/clear controls, context/generated usage, and model lifecycle controls. One 16 ms timer still drains at most 64 events and performs one bounded decoded-output pull; streaming crosses Slint as frame-batched fragments. |
| Non-goals preserved | No GGUF composition, GPU, remote target, persistence, arbitrary branch tree, provider SDK, transport DTO, or broad crate extraction was introduced. |

### Post-implementation review closure

The first review closure was committed as `8d134e38cff0c2203a1ade9714d3aa92a65b9a3a` and formatting cleanup followed as `3b4541f50fcf614bc65938d448b383f507d27fcd`. It corrected four issues found during source review:

- response attempts become terminal when generation becomes terminal, independently from later E0 cleanup/release;
- a committed unanswered user turn blocks regeneration of an older response instead of creating an implicit branch;
- Slint refreshes canonical E1 history after commit-then-admission failures and terminal/cleanup lifecycle events;
- Send/edit/regenerate controls use E1 chat compatibility rather than generic generation readiness.

A final semantic review after `3b4541f50fcf614bc65938d448b383f507d27fcd` identified three remaining closure points. The resulting working tree:

- groups each completed historical user/assistant pair into one E1 planning unit so context selection and exact correction cannot retain an orphan assistant or one side of a completed turn;
- binds the built-in TinyLlama profile to immutable resolved commit `fe8a4ea1ffedaf415f4da2f062534de366a451e6` in addition to repository/tokenizer/EOS evidence;
- distinguishes generated-token usage estimates from tokenizer-measured semantic-content counts and reports the final selected-set estimate after exact correction.

These changes do not generalize `context-planner` into a chat planner: grouping remains an E1 derivation and selected/dropped diagnostics expand back to raw conversation record identities.

### Recorded validation

Focused validation passed on the original Phase 7 working tree:

```text
cargo test -p context-planner --locked
cargo test -p application-runtime --locked
cargo test -p desktop-slint --locked
cargo clippy -p context-planner -p application-runtime -p desktop-slint --all-targets --locked -- -D warnings
```

The original canonical locked gate also passed after documentation reconciliation:

```text
cargo run --locked --bin llm-app -- verify
```

It validated architecture/dependency policy and formatting, then passed the full workspace tests/doctests, strict Clippy, rustdoc generation, and benchmark compilation. That historical validation predates the committed review fixes and this final semantic closure; it is not evidence for the resulting tree.

The graphical application was not manually driven against the external TinyLlama repository in this environment. Download-free tests use a tokenizer fixture with the verified textual template and EOS identity plus the existing tiny Candle model to prove rendered prompt admission, exact usage, E1 attempt state, regeneration, active-clear rejection, and lifecycle integration. This proves integration semantics, not model language quality or external artifact availability.

Run `cargo run --locked --bin llm-app -- verify` on the exact resulting tree and record that result before treating Phase 7 as the validated input to Phase 8.

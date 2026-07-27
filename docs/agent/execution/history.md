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

# Execution history

This file is the chronological closure record for completed execution work. It preserves phase-specific rationale, corrections, and closed-tree acceptance provenance without making each closed phase a separate top-level document.

For current product truth, use [implementation status](../../project/implementation-status.md). For repeatable commands, use [validation](../../project/validation.md). Exact current benchmark methodology and curated intervals live in [performance evidence](../../project/performance.md). Historical results below apply only to the baseline named in each entry.

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

The Phase 4 baseline executed one pinned tiny-random Llama model through E0, generated eight tokens, exercised cancellation and unload, and exited cleanly. Its exact model identity, timing, RSS observations, and interpretation now live in the canonical [historical performance evidence](../../project/performance.md#historical-phase-4-external-smoke); this entry retains only chronology.

The repeatable external-model procedure now lives in [project validation](../../project/validation.md#rust-native-candle-hub-smoke).

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

## Phase 8 — GGUF parity and native composition evidence

**Prepared:** 2026-07-30
**Reviewed baseline:** `797ba0f` plus the current Phase 8 working tree
**Scope:** make GGUF a second local E0-backed product path, prove shared Candle/GGUF behavior through E1, and decide the native composition boundary
**Recorded outcome:** Phase 8 code complete; focused validation and the canonical full locked gate passed on the exact working tree; no manual external graphical acceptance recorded

Phase 8 added a model-compatible GGUF tokenizer and immutable local-file identity, ran both native backends through shared E0 and E1 behavior, exposed a closed product selector in Slint, and retained one application façade/state machine. The implementation evidence did not justify another runtime crate: E1 is still the only consumer of local production composition and no independent lifecycle or API was demonstrated.

### Closure matrix

| Requirement | Recorded closure |
|---|---|
| GGUF tokenizer | A llama.cpp vocabulary-only model implements prompt encoding, token-to-piece decoding, boundary/control-token evidence, special-token policy, and request-local stateful streaming decode through the portable tokenization contracts. No Hugging Face tokenizer is paired by vocabulary size. |
| Immutable GGUF identity | Local resolution canonicalizes the path, hashes before and after bounded metadata inspection, and builds a verified source. Tokenizer construction hashes before and after vocabulary loading. Resolution identity, tokenizer digest, inspected metadata, E0 descriptor/capabilities, and load admission must agree; mutation after resolution is rejected. |
| Closed local products | Public selection permits exactly Hugging Face Hub + Candle + Safetensors + CPU or local file + llama.cpp + GGUF + CPU. Backend, source, device, and format cross-products are derived rather than caller-assembled. Hosted and peer execution are excluded. |
| Shared E0 parity | One generic test contract is instantiated for Candle and GGUF and covers load/start, prefill, greedy decode, seeded reproducibility, EOS/token limit, output backpressure, cancellation, released cleanup state, unload, empty accounting, shutdown, and worker join. |
| Shared E1 parity | One helper drives both products through prompt encoding, direct-completion start/running/terminal/released state, decoded text, exact usage, unload, and explicit application shutdown. GGUF explicitly rejects unverified chat rather than guessing a profile. |
| Composition decision | [ADR-0012](../decisions/0012-local-native-composition.md) keeps the public `application-runtime` façade non-generic and isolates production composition in private closed `local.rs`. Two concrete E0 workers remain monomorphized with closed static dispatch; redb remains in E1; no local runtime or `application-api` crate was created. |
| One application state machine | Backend switching does not duplicate lifecycle, generation, conversation, context, output, unload, or shutdown semantics. E1 remains single-model even though it owns two worker endpoints. |
| Chat compatibility | Direct completion works for both products. Built-in chat remains limited to the verified Hugging Face TinyLlama Chat v1 commit with tokenizer `</s>` mapped to EOS ID 2; GGUF remains direct-completion-only. |
| Slint selection | The UI maps exactly two visible products into application-owned selections, displays selected/resolved/loaded identity and compatibility summaries, and chooses Chat versus Direct completion from E1 evidence. It imports no adapter source types and exposes no low-level GGUF execution controls. |
| Existing boundaries | E0 retains native resources and token scheduling; `corrective-workflow` remains independent; hosted/peer/GPU/transport work and conversation persistence remain out of scope. |

### Focused validation evidence

The following focused commands passed on the reviewed working tree:

```text
cargo test --locked -p gguf-backend --test tokenizer
cargo test --locked -p inference-runtime --test native_backend_generation
cargo test --locked -p application-runtime
cargo test --locked -p desktop-slint
cargo clippy --locked -p gguf-backend -p inference-runtime -p application-runtime -p desktop-slint --all-targets -- -D warnings
```

The GGUF tokenizer target passed four digest/native-tokenizer tests; the shared E0 target passed both Candle and GGUF instantiations; `application-runtime` passed its shared direct-completion, compatibility, lifecycle, shutdown, and state coverage; `desktop-slint` passed 21 presenter tests; and strict focused Clippy completed without warnings.

These tests use committed, download-free Candle and GGUF fixtures. They prove native and application integration behavior, not external artifact availability or model language quality. The desktop application was not manually exercised against external model artifacts in a graphical session.

### Recorded canonical gate

The complete repository gate passed on the uncommitted Phase 8 working tree based on `797ba0f90b3eac154fe44ec871f4c7bf755a06ef`:

```text
git rev-parse HEAD
797ba0f90b3eac154fe44ec871f4c7bf755a06ef
cargo run --locked --bin llm-app -- verify
```

It validated architecture/dependency policy, formatting, workspace checks, the complete test/doctest suite, workspace strict Clippy, rustdoc, and benchmark compilation. The gate was rerun after the final validation-status updates so this record describes the exact resulting working tree rather than an earlier Phase 8 edit.

## Phase 9 checkpoint — Candle-only architecture and Rust-native tooling

- **Prepared:** 2026-07-31
- **Recorded artifact:** commit `f0fe9c6623f1e2afd569767d903f3978e00560da`, tree `db8a9ae77f41e0e769c7434ce21a940ae33784ae`
- **Scope:** remove the accidental llama.cpp/GGUF product path and project-owned Python tooling while preserving Candle application and backend-neutral E0 behavior
- **Recorded outcome:** Candle-only correction checkpoint complete; canonical, policy, portability, clean-build, and external-model validation passed; no manual graphical acceptance recorded

The first baseline observation saw `d7d03e46c0239d4be8c34e8a5e16959fb5bd46c3` with only the user-provided cleanup brief untracked. During execution, `main` advanced to `15d9e87cdaee77fd0d49247712d3c12dfb3adea2`; that commit's only change was tracking the same brief. The cleanup was validated as a working tree based on that commit and was subsequently committed as `f0fe9c6…`, whose stable Git tree is `db8a9ae…`. The recorded artifact, rather than an impossible self-referential SHA embedded before commit, is the durable checkpoint identity.

[ADR-0013](../decisions/0013-candle-only-local-execution.md) supersedes ADR-0012 for current architecture, while retaining the historical Phase 8 record above. [ADR-0014](../decisions/0014-rust-cargo-native-operational-tooling.md) defines the maintained tooling boundary.

### Closure matrix

| Requirement | Recorded closure |
|---|---|
| Sole local engine | E1 owns one `HostedRuntime<CandleLlamaSource>`, one inference worker/thread, one Hub worker, one Hugging Face tokenizer path, and request-local streaming decoders. Active-backend routing and the dormant second worker are gone. |
| Public vocabulary | `ModelSelection` is normalized repository/revision data. Resolved/loaded state reports orthogonal Candle, Hub, CPU, Safetensors, scalar, vocabulary, and immutable commit evidence. GGUF path/configuration/digest, quantization, product, and llama.cpp variants are gone. |
| Dependencies and fixtures | `crates/adapters/gguf-backend`, the GGUF binary/README/generator fixture, workspace edges, validator entries, `llama-cpp-2`, its sys crate, `self_cell`, and path-only native build dependencies were removed. Cargo regenerated the lockfile. |
| Preserved behavior | Candle real-fixture tests and deterministic E0 loaders retain load, sampling, EOS/token limit, backpressure, cancellation, cleanup, unload, shutdown, and join coverage. E1 retains direct completion, exact TinyLlama chat, context/regeneration, persistence, unload policies, disconnection, and bounded shutdown coverage. |
| Desktop | Slint exposes repository/revision only. The product selector, GGUF path, and backend-specific branches are gone; Chat versus Direct completion still derives from E1 compatibility evidence. |
| Rust-native operations | The root hygiene validator rejects tracked operational Python artifacts/invocations, prohibited manifest declarations, and removed/Python-runtime packages in the selected graph. Exact cargo-deny bans provide defense in depth. |
| External smoke | `application-runtime/examples/candle_hub_smoke.rs` resolves through the production Hub worker and drives E1/Candle resolution, load, direct completion, release, unload, and shutdown through Cargo. |
| CI prerequisites | Linux CI removed Clang/libclang. `build-essential` and CMake remain for selected native dependencies, and the font/XCB/XKB development packages remain owned by Slint. |
| Documentation | Current architecture, status, workspace, component, validation, execution, and crate guides describe Candle/Hub/Safetensors/CPU. The deleted GGUF guide is gone; dated analysis, recovered plans, superseded ADRs, and Phase 8 history remain explicitly historical. |

### Validation evidence

The canonical gate passed after the final source and documentation reconciliation:

```text
cargo run --locked --bin llm-app -- verify
```

That gate passed architecture, repository hygiene, formatting, all-target workspace checking, all ordinary tests/doctests, strict workspace Clippy, rustdoc with warnings denied, and workspace benchmark compilation. Focused `application-runtime` coverage now includes 31 unit tests, including persistence across restart and scalar-mismatch unload, plus 3 state integration tests. `desktop-slint` passed 19 presenter tests, and the Candle E0 target passed its 2 real-fixture lifecycle scenarios.

Supplemental gates also passed:

```text
cargo fmt --all -- --check
cargo check --workspace --all-targets --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps --locked
cargo bench --workspace --no-run --locked
cargo check --locked --target wasm32-unknown-unknown --lib -p domain-contracts -p tokenization -p context-planner -p sampling -p task-graph
cargo check --locked --target thumbv7em-none-eabihf --lib -p domain-contracts -p tokenization -p context-planner -p sampling -p task-graph
cargo deny --workspace --locked check advisories bans licenses sources
lychee --config lychee.toml --offline '**/*.md'
git diff --check
```

Locked metadata, duplicate-tree, feature-tree, lockfile, and selected-package audits found no removed engine, Python runtime/binding, or `self_cell` package. Because the change is intentionally unstaged, raw `git ls-files` still names the deleted Python generator from the index; `git ls-files --deleted` identifies it and the path is absent from the working tree. The Rust hygiene command treats missing deleted paths correctly and passes.

A fresh target-directory all-target build and full test compilation passed with failing shims first in `PATH` for the prohibited Python runtimes/package tools, the Python-distributed Hugging Face CLIs, and `clang`/`clang++`. No shim was invoked. The clean build exercised CMake and other retained native owners. This was a local clean-target proof, not a fresh Ubuntu 24.04 package-image run.

The first E1 external-smoke attempt used the historical Phase 4 model revision and failed correctly because that commit does not contain the `tokenizer.json` required by production E1. The maintained smoke was repinned to the repository's immutable commit `1c81a3fba044af78df253edc66bdbab183184932`, then passed:

```text
LLM_APP_CANDLE_HUB_SMOKE=1 cargo run --locked -p application-runtime --example candle_hub_smoke
```

It resolved the exact Hub commit, loaded Candle/F32 on CPU with a 32,000-token vocabulary, generated eight tokens, observed terminal and released token-limit state, unloaded with no cancellation, explicitly shut down both workers, and removed its temporary redb workspace. This proves one pinned integration path, not language quality or broad model compatibility.

No manual graphical desktop session was performed. Candle-native GGUF/quantized loading and GPU execution remain separate reviewed future work.

## Phase 9 closure — structural reconciliation and lifecycle hardening

- **Prepared:** 2026-08-01
- **Input checkpoint:** commit `f0fe9c6623f1e2afd569767d903f3978e00560da`, tree `db8a9ae77f41e0e769c7434ce21a940ae33784ae`
- **Scope:** complete work package 9.5, correct reviewed E1 ownership failures, remove stale native-tool prerequisites, and reconcile current documentation
- **Recorded outcome:** Phase 9 complete; Phase 10 is the next plan phase

The closure was developed and locally validated as a working tree based on the committed Candle-only checkpoint. Required CI prints the resulting commit and `HEAD^{tree}` immediately before the canonical gate, so committed provenance lives in the CI run rather than requiring this history entry to predict the SHA or tree that contains itself.

### Closure matrix

| Requirement | Recorded closure |
|---|---|
| Retryable shutdown ownership | E1 tracks running, stopping, stopped, and failed/retryable outcomes. Inference and Hub join handles remain owned after a timeout; later shutdown calls retry unresolved joins and return idempotent success only after both workers are confirmed stopped. |
| Rejected-model ownership | Compatibility rejection retains the native model handle, compatibility failure, and unload state privately through bounded submission retry, successful unload, proven runtime disconnection, or observable exhaustion. Fault tests cover recovery and retained exhaustion. |
| Transactional startup | An owning startup guard retains the already-started inference worker until Hub startup commits. Forced Hub failure attempts bounded inference shutdown/join before returning the primary Hub error; rollback timeout quarantines the complete owner for a later bounded reap rather than detaching it. |
| Domain DAG | ADR-0015 registers the exact four current F1 → F0 edges, rejects every unreviewed domain edge, validates registry rationales/uniqueness/acyclicity, moves `TaskId` to `task-graph`, and defines the shared-foundation inclusion rule. |
| Internal responsibilities | E0 runtime operations, E1 generation, task graph/artifact/state/error logic, desktop presentation, architecture policy, and hygiene parsing were split into private responsibility modules without introducing product layers or breaking public façades. |
| Workspace tooling | ADR-0016 makes the root a virtual workspace and moves custom architecture, hygiene, and composite verification to `tools/xtask`. Pass-through commands for one-step Cargo operations were removed. |
| Lint policy | ADR-0017 keeps stable selected Clippy policy mandatory under `-D warnings`; the blanket nursery group is reported separately and non-blocking. |
| Native prerequisites | Ubuntu CI no longer installs system CMake. The non-FIPS AWS-LC path uses its CC builder, and the required canonical gate starts from a fresh target with failing external-tool shims. |
| Hygiene/docs | The temporary cleanup brief and broad historical filename exemption were removed. Current status, architecture, validation, workspace, component, plan, and handoff documents agree that Phase 9 is closed. |

### Validation evidence

The canonical closure command passed on the resulting working tree:

```text
cargo xtask verify
```

The same gate passed from a fresh target with fail-fast shims covering the removed or prohibited external tool families and with non-FIPS AWS-LC forced to its CC builder. Focused changed-package suites passed, including 37 `application-runtime` unit tests plus 3 integration tests, 51 `inference-runtime` tests, 19 desktop presenter tests, 11 `task-graph` tests, 26 corrective-workflow tests, and the xtask unit/integration policy suites.

Both named portability targets passed for all five domain crates. Locked cargo-deny policy, offline local-link checking, architecture/hygiene checks, and `git diff --check` also passed. The scheduled nursery command is informational and is not part of this acceptance gate.

The network-dependent E1 Hub smoke was not rerun for this structural closure. Its prior success remains evidence for the committed Candle-only checkpoint only, not for this exact working tree. No manual graphical desktop session was performed.

## Pre-Phase 10 closure — terminal shutdown and measurement policy

- **Prepared:** 2026-08-01
- **Input checkpoint:** commit `3942a19b97d347fd238c451d2b0a2fcbea287873`, tree `be069879fea9531799038c5189c9edb3007ebf72`
- **Scope:** correct terminal shutdown semantics, establish benchmark/workspace hygiene, replace provenance-uncertain model-fixture bytes, and amend Phase 10
- **Recorded outcome:** pre-Phase 10 closure complete; Phase 10 remains not started and is the next operation

The execution began from clean `main` after fetching `origin/main`; local and remote were identical. Only root `./target` existed, and the clean starting tree passed `cargo xtask verify` before editing.

### Closure matrix

| Requirement | Recorded closure |
|---|---|
| Terminal E0 disposition | `WorkerStop::PreserveRuntime` became `RetainUntilProcessExit`. Cleanup exhaustion still publishes `CleanupRetryExhausted`, deliberately forgets the runtime, terminates the worker, and relies on process exit rather than unverified implicit backend destruction. |
| E1 shutdown state | Running, stopping, clean stop, retryable failure, and terminal failure are distinct. Join timeout retains handles and can later succeed; terminal E0 failure is retained independently from handles and is returned by every later shutdown call. |
| Deterministic lifecycle coverage | Tiny mock resources cover clean idempotence, retryable joins, structured cleanup exhaustion, skipped model drop, sticky application failure, ordinary-unload zero accounting, and endpoint abandonment/disconnection. |
| Benchmark architecture | Crate-owned measurements remain in real crate-local `benches/` targets. Future cross-crate/system work is reserved for exact package `benchmarks/runtime` (`runtime-benchmarks`) as an outer consumer of reviewed public APIs. No package or suite was created. |
| Workspace/artifact hygiene | Root `target/` is ignored recursively. Architecture and hygiene reject unknown benchmark paths, reverse dependencies, publishable/custom-build benchmark packages, unregistered manifests, nested benchmark locks/build scripts, tracked target/result trees, and model caches. |
| Fixture provenance | The prior files' synthetic structure was technically verified, but authorship/redistribution provenance was not established. Newly generated project-owned F32 Llama bytes replaced them; the Rust/Cargo generator and old/new hashes are recorded beside the fixture. |
| Phase 10 plan | Mandatory scope is sampling expansion, one system harness, reproducible environment metadata, and controlled lifecycle/memory measurement. Other listed microbenchmarks are conditional on a named question and system-harness insufficiency. |

### Focused validation evidence

Focused E0, E1, replacement-fixture, and `xtask` policy suites passed during implementation. The generator was run twice and produced identical hashes. The final canonical, Clippy, architecture, hygiene, dependency, link, whitespace, status, and root-target checks are execution-report evidence for the resulting diff; this history entry does not predict the commit/tree that will contain itself.

The external Hub smoke and manual graphical desktop session were not run because this closure changed no external-model or presentation behavior. No statistical benchmark was run and no Phase 10 performance result was recorded.

## Phase 10 — repository infrastructure and synthetic acceptance

- **Original implementation commit:** `62a342e9a5720110f3ddf42fca8e7d6c34aa3ee8`, tree `3512426c40628eeec57eb282a648533f24a6f4d2`
- **Allocation/matrix correction:** `148f0fea16f40cd77a934549da2488c370f7c066`, tree `08a5fa1d26a6dbded2154d19ec788c98ef905537`
- **Benchmark simplification:** `f883d645e94c1c08e78d86d5dd1f2b627e28148c`, tree `08049138843252e128e08e7513a92739b8c39cc6`
- **Accepted code-under-test (Commit A):** `efcd36e320a97d61d3f982619fee182410c514df`, tree `f80c5d6c746376df81d7ac8e7281ac9736e44d88`
- **Recorded outcome:** Phase 10 repository infrastructure and synthetic acceptance complete; external real-product baseline outstanding; Phase 11 not active

### Correction chronology

The original Phase 10 commit was documented as accepted, but fresh GitHub CI failed the `domain-contracts` allocation test. That invalidated the original completion claim; the original commit did not pass an exact-tree acceptance gate.

The allocation check was subsequently isolated as a deterministic harness-free executable, and the sampling cases were shared with an ordinary one-shot matrix test. The runtime benchmark implementation was then simplified by responsibility, and its large unexecuted real-product mode was removed. Final acceptance therefore depends only on the later clean Commit A gate below, not on the original claim or its earlier dirty-tree measurements.

### Commit A exact-tree acceptance

Commit A was clean before validation and remained clean after generated output was confined to root `target/`. A fresh dedicated target was used. The complete local procedure in [validation](../../project/validation.md#phase-10-exact-tree-acceptance) passed, including:

- the isolated domain allocation gate, full domain/sampling/runtime/xtask suites, and one-shot sampling matrix;
- strict workspace Clippy, complete benchmark compilation, and the canonical architecture/hygiene/repository gate;
- locked dependency policy, offline Markdown links, and whitespace checks;
- both named portable-domain targets;
- root-target, nested-lock, package-target, and source-tree artifact hygiene.

This is local exact-tree evidence. No remote GitHub CI success is claimed for Commit A without a separately observed run.

### Commit A measurements

The bounded release synthetic baseline and exactly four focused Criterion targets were executed on the clean Commit A tree. The synthetic cycles returned every released request to model-only accounting and every unload to exact empty accounting, with clean shutdown/join and no pending or exhausted cleanup. The exact environment, intervals, RSS observations, selected targets, and limitations are canonical in [performance evidence](../../project/performance.md#commit-a-controlled-baseline); they are intentionally not copied here.

No external model was executed and no network access was authorized. The removed real-product mode supplies no compile-only evidence. An actually executed exact-model/revision product baseline remains a prerequisite before Phase 11.

### Evidence-document workflow

A separate documentation-only Commit B records the curated evidence and canonical ownership changes. Commit A remains the executable tree measured because Commit B changes no executable source, manifest, lockfile, fixture, or configuration. Commit B’s identity and post-commit local gate are recorded by the closure report rather than predicted by this tracked file.

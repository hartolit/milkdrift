# Phase 5 Closure Report

**Prepared:** 2026-07-25  
**Implementation baseline:** `f6ac1806c33d4a1d84dfabb66c14f3475af5872a`
**Scope:** Phase 5 — expose direct-completion generation through `application-runtime`

## Result

The Phase 5 source boundary is implemented. This closure patch addresses the remaining review gaps: explicit application-owned unload behavior, application-level generation integration coverage, presenter dispatch cleanup, and documentation synchronization.

The resulting tree must pass the repository's locked validation gate before Phase 5 is marked complete or Phase 6 begins. Validation of the baseline commit does not substitute for validation after applying this closure patch.

## Closure matrix

| Requirement | Closure |
|---|---|
| Narrow E1 generation API | `start_generation`, `cancel_generation`, `poll_event`, and borrowed `pull_output` remain the frontend-neutral generation surface. |
| Stable E1 settings | Application-owned settings validate completion and sampling controls before E0 admission. |
| Direct-completion prompt | Resolved `HfTokenizer` encodes ordinary prompt text once; no chat-template claim is made. |
| Owned streaming decode | `HfOwnedStreamingDecoder` owns request-local tokenizer/decode state without full-history re-decode. |
| Token-to-text bridge | E1 translates bounded E0 token/state batches into bounded UTF-8/state batches. |
| Backpressure | Tests exercise output stalls and resume without token/text loss. |
| Application state/events | Tests cover start, cancellation, terminal state, usage, and release. |
| Single-model policy | E1 configures exactly one resident model. |
| Unload policy | E1 exposes `ModelUnloadBehavior::{RejectIfBusy, CancelActive, Drain}` without exposing E0 `UnloadPolicy`. |
| Unload integration | Tests cover idle unload plus reject/cancel/drain behavior while generation is active. |
| Worker lifecycle | Application-level tests cover inference-worker disconnection and explicit bounded shutdown. |
| Frontend isolation | Slint remains presentation-only; generation scheduling stays in E0 and generated text remains pull-oriented. |
| Documentation | Root README, E1/desktop guides, status, documentation map, and this closure report describe the same Phase 5 boundary. |

## Integration coverage

The application-runtime test composition uses:

- the repository's download-free Candle Llama fixture from `inference-runtime`;
- a project-authored 16-token WordLevel tokenizer fixture;
- real hosted E0 scheduling and Candle execution;
- real E1 prompt encoding, generation admission, cancellation, output translation, state/events, unload policy, worker disconnection, and shutdown.

The suite covers:

- generation without a loaded model;
- invalid settings and empty prompts;
- duplicate generation admission;
- token-limit, EOS, and textual stop-sequence completion;
- token-to-text streaming;
- output backpressure and resume;
- cancellation under constrained output capacity;
- unload while idle;
- unload while generating with reject, cancel, and drain behavior;
- inference-worker disconnection;
- explicit application shutdown.

## Final closure rule

Run the canonical gate on the exact resulting tree:

```bash
cargo run --locked --bin llm-app -- verify
```

For focused diagnosis:

```bash
cargo fmt --all -- --check
cargo check --workspace --all-targets --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked
cargo bench --workspace --no-run --locked
git diff --check
```

Phase 5 may be marked complete only when the resulting commit/tree passes the canonical locked gate. If CI runs the same resulting commit, record that run in `docs/project/implementation-status.md`.

## Agent handoff convention

When the execution environment lacks the Rust toolchain, deliver source changes as a patch file, or as a code block when the change is genuinely small. Include copy/paste commands for checking and applying the patch. The local operator runs the Rust gates and returns failures verbatim for correction.

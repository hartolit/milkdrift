# Pass 6 — Local OpenAI-compatible model dogfood and external-effect truth

Use this prompt with `00-shared-execution-contract.md`. Run it after Pass 5.

This pass benefits from an operator-provided, separately managed loopback model server. A llama.cpp-style `llama-server` with a small model and its OpenAI-compatible chat-completions endpoint is suitable. Milkdrift must not install, download, start, stop, or own that server.

## Objective

Prove that a real model-backed workflow can be configured, operated, inspected, interrupted, recovered, and truthfully resolved through the normal daemon and completed CLI surface. Fix every code defect exposed by that run, add deterministic regression coverage for it, and leave a repeatable operator path that does not overclaim qualification.

This is not a model-quality benchmark and not a request for another provider abstraction.

## 1. Read and reuse the existing model boundary

Inspect:

- `docs/guides/local-model-endpoint.md`;
- `docs/guides/external-evidence.md`;
- `examples/external-evidence` and current endpoint-profile templates;
- `milkdrift-model`, `milkdrift-model-provider`, capability host, runtime context materialization, daemon composition, control protocol/client, CLI, artifacts, attempts, and external-work resolution;
- current mock-endpoint, cancellation, truncation, restart, and external-evidence tests.

Reuse the existing `open_ai_compatible` chat-completions mapping. Do not add the OpenAI Responses API, Ollama-specific API, provider discovery, model enumeration, generic file parts, tokenization, pricing, managed sessions, or another HTTP stack.

## 2. Establish a maintained local-model example

Create the smallest maintained example/guide needed to run one real local model workflow through the daemon and CLI.

It should include or derive:

- a safe endpoint-profile template using loopback HTTP only with `local_development: true`, exact host allowlist, explicit limits, no ambient proxy, no redirects, and only truthful features;
- a minimal ordinary blueprint or prompt sequence that reaches one model task and explicit terminal;
- a strict daemon configuration example that references the profile and private token source only when needed;
- exact CLI commands for readiness, capability inspection, validation/import, run start, timeline follow, attempt inspection, artifact retrieval, restart verification, and retained-work resolution;
- expected structural outcomes, not exact generated prose.

Reuse shared templates and canonical document libraries. Do not copy a large schema into multiple examples. Machine/model-specific profile files, model paths, credentials, outputs, and evidence directories remain untracked.

## 3. Add a CLI-driven model dogfood lane

Extend the existing development/evidence ownership rather than adding a product service. The lane should be able to run in two modes:

### Deterministic mode

Use the existing production provider parser/mapping against controlled loopback responses and the actual daemon/CLI binaries. It runs in ordinary automated tests and proves the harness itself without external resources.

### Operator real-endpoint mode

Consume an explicit operator profile or endpoint/model inputs. It performs the same CLI scenario against the separately running server and writes redacted output under `target/` or another selected empty untracked directory.

Do not label this model-only lane as the repository’s qualifying external-evidence result. The existing strict external-evidence contract still requires both a real byte-pinned coding agent and a real model endpoint. Record model-only smoke/interoperability distinctly.

The lane must never silently fall back from real mode to deterministic mode.

## 4. Required real workflow behavior

The real scenario must:

1. validate daemon configuration and reach readiness;
2. show the exact model capability/profile generation through the CLI;
3. validate and import the workflow through the CLI;
4. start a run with explicit command identity;
5. stop at a durable wait/signal boundary before model adapter entry;
6. shut down the daemon through the harness owner, reopen the same store, and verify the same unreleased sequence/state;
7. deliver the signal through the CLI and permit exactly one model invocation;
8. follow bounded stream/timeline output through the CLI;
9. inspect the exact attempt, frozen capability resolution, profile/protocol/model/origin provenance, context manifest basis, usage/finish/response identity when supplied, and output artifact;
10. retrieve and verify the output artifact without asserting exact prose;
11. reopen again and prove no duplicate invocation/attempt/artifact publication;
12. leave credentials, prompts, raw provider bodies, and generated content out of the redacted report.

Success assertions should be structural:

- exact selected profile and model alias;
- one adapter entry and one attempt;
- valid ordered bounded fragments;
- one accepted terminal;
- expected selected and omitted context identities;
- provider metadata/usage preserved when the server supplies them;
- output artifact digest, size, media type, provenance, and attempt linkage;
- no uncertainty on the clean success path.

If the selected real server omits metadata required for strict external qualification, the smoke lane may still pass only according to an explicitly weaker non-qualifying contract. Do not manufacture usage or response identity.

## 5. Prove external failure and uncertainty semantics

Use controlled deterministic endpoints/process control for failure cases so ordinary tests remain repeatable. Exercise the actual production model adapter and normal runtime/daemon path.

Cover at least:

### Before connection/entry

- invalid profile or unsupported feature rejected before connection;
- unreachable endpoint;
- denied network/profile authority;
- malformed context manifest or selected input;
- cancellation before final entry.

These must not be mislabeled as an external effect that may have occurred.

### After durable entry

- connection closes after request bytes may have been accepted;
- malformed or truncated SSE after fragments;
- response exceeds configured bounds;
- idle/wall timeout;
- client cancellation while the remote server may continue;
- daemon/worker shutdown during the response;
- terminal/report persistence temporarily unavailable;
- late terminal evidence after an uncertain observation.

Required semantics:

- malformed/truncated streams never produce a successful partial artifact;
- cancellation acknowledgement does not claim provider-side termination;
- known entry plus missing terminal truth becomes retained uncertainty;
- no automatic retry occurs merely because the call is a model call or read-only;
- an authorized `ResolveWork::Retry` is still accepted only when runtime idempotency policy allows it and prior obligations remain visible;
- retain/query/compensate/evidence-based resolution use the same CLI/authority path;
- late evidence settles the original attempt exactly once and does not rewrite its historical uncertainty event;
- restart never causes duplicate provider entry;
- provider error text and bodies are bounded/redacted before logs/public reads.

Where the external service offers no idempotency or query mechanism, remain truthful rather than adding a fake one.

## 6. Inspect side-effect classification end to end

Verify that the model operation’s declared side-effect/idempotency/cancellation semantics are consistent across:

- endpoint profile and capability descriptor;
- blueprint requirement;
- authority request/decision;
- exact resolved snapshot;
- runtime entry decision;
- attempt read/timeline;
- terminal or uncertainty record;
- retry/resolve policy;
- artifact provenance.

Remove any duplicated mapping or default that allows these layers to disagree. One owner defines the capability contract; later layers freeze or enforce it rather than reinterpret it.

Do not broaden the pass into arbitrary destructive tools. The model path is the concrete external mechanism used to prove the general entry/uncertainty contract.

## 7. Strict external evidence when resources exist

If the environment also provides a real byte-pinned coding-agent profile, private credentials, and a clean checkout, run the existing strict `cargo external-evidence` path after the model lane. Fix any implementation defect it exposes.

If those resources are absent:

- do not weaken the strict harness;
- do not set `qualifying: true`;
- do not change roadmap/status to call model-only evidence complete;
- report the exact missing operator resource.

## 8. Evidence

Run:

- model and model-provider contracts;
- mock endpoint and hostile stream tests;
- runtime causal-context and uncertainty tests;
- capability-host materialization/conformance tests;
- daemon control-plane/headless dogfood tests;
- CLI tests and actual-binary scenario;
- deterministic local-model dogfood mode;
- real endpoint mode when supplied;
- relevant `uncertainty`, `context`, and `runtime` mutation shards;
- operational evidence smoke/full lane as applicable;
- the full local gate.

Preserve exact commands and redacted structural results in the final report. Keep generated reports outside source control.

## Scope exclusions

No model server management, model downloads, weight loading, provider discovery, new provider/API family, pricing/tokenizer implementation, GUI/TUI, generic plugin system, automatic retry policy, fake provider termination proof, or weakening of strict external-evidence qualification.

## Acceptance criteria

The pass is complete only when:

- one maintained CLI-driven local model workflow runs through the normal daemon path;
- deterministic mode proves the harness and real mode never silently falls back;
- success, pre-entry failure, post-entry uncertainty, cancellation, late evidence, resolution, and restart are structurally verified;
- no duplicate provider entry occurs;
- profile/model/context/usage/artifact provenance remains exact and truthful;
- strict external evidence remains distinctly gated;
- focused, mutation, operational-as-applicable, and full local gates pass.

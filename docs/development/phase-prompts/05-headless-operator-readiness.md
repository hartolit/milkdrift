# Pass 5 — Make the headless product routinely usable

Finish the daemon and CLI as the canonical pre-UI operator and agent interface. This pass must turn existing semantic depth into a routine workflow from a fresh directory without creating a second workflow engine in the client.

Follow `00-pristine-readiness-contract.md` in full.

## Primary outcome

A user or coding agent should be able to install/build Milkdrift, create a safe local configuration, start the daemon, author or import a workflow, run it, wait with a bound, inspect exact evidence, restart, reconcile future work, and diagnose failure through documented commands.

The CLI is broad in capability and thin in ownership:

```text
arguments or bounded document
  -> versioned protocol request
  -> control client
  -> daemon owner
  -> typed result
  -> stable human or JSON rendering
```

## 1. Contract the CLI dependency graph

The supplied CLI directly depends on ten Milkdrift packages. Remove direct production dependencies on:

- `milkdrift-authority`;
- `milkdrift-blueprint`;
- `milkdrift-capability`;
- `milkdrift-control`;
- `milkdrift-contracts`;
- `milkdrift-persistence`;
- `milkdrift-workspace`.

The intended steady-state dependencies are the control client, control protocol, and—only where local authoring genuinely belongs there—the prompt-sequence package. Generic crates such as clap, serde, URL, async, and BLAKE3 may remain.

Achieve this by deleting redundant local semantic validation or moving one reusable operation into its existing semantic owner. Do not create a CLI-domain crate, generic façade, or protocol bypass.

Examples of local logic to review:

- parsing revisions to derive facts already returned by the daemon;
- constructing remediation proposals with authority, persistence, and workspace identities inside the CLI;
- validating proposals a second time before the daemon’s canonical reader;
- using capability/workspace types solely to parse a bounded JSON argument or verify a digest;
- duplicating safe-identity and bounded-error mechanics already owned by the protocol/client.

Keep useful local early diagnostics only when they do not become a second semantic rule.

## 2. Establish a supported fresh-directory setup

The README currently tells users to copy a daemon test fixture. Replace that with an intentionally maintained operator path.

Use the smallest coherent mechanism, such as:

- a daemon command that writes or prints a safe complete current configuration template;
- a committed operator example owned under `examples/` and validated by the real current reader;
- or both when one provides machine output and the other explains edits.

The setup must:

- avoid writing a secret value into source or argv;
- use explicit safe loopback defaults;
- make storage/artifact paths relative to the chosen config directory when supported;
- include no broad authority without a visible dangerous acknowledgement;
- explain how to add a local process and local model profile;
- pass `--check-config` through the production reader;
- never depend on a file under `tests/fixtures`.

Add a bounded `doctor` or equivalent only when it can report existing typed readiness/profile/capability failures without inventing another health model.

## 3. Complete command parity and automation behavior

Compare the versioned control protocol/client surface with CLI commands. Expose every operator-relevant supported command and read through one consistent command family. At minimum confirm or implement:

- blueprint validation without storage;
- exact import/export/show/list/diff behavior;
- run start/show/list/pause/resume/cancel/signal;
- bounded timeline page and explicit follow;
- exact uncertain-work inspection and resolution actions;
- node and attempt inspection;
- proposal submit/show/list/approve/reject/apply;
- capability/provider/peer/artifact/layout/authority/health reads and allowed actions;
- controller inspection/continuation while production activation remains correctly gated.

Do not expose administrative behavior that the daemon intentionally does not support publicly.

Add a bounded automation primitive such as `run wait` with an explicit timeout, terminal-state filter, poll/reconnect bound, and stable exit result. Interactive follow may continue until Ctrl-C, but noninteractive commands must never reconnect forever or wait without a user-visible hard bound.

## 4. Make JSON a complete external CLI contract

In `--json` mode, both success and failure must emit one documented, bounded, control-character-free schema to stdout, with diagnostics/redaction behavior suitable for automation. Include:

- schema version;
- stable operation/type;
- success or failure status;
- stable error category/code;
- retryability where known;
- command identity when relevant;
- typed value or bounded redacted detail.

Keep exit codes stable and independently tested. Never include credentials, complete prompts/responses, restricted artifact bytes, raw HTTP errors, or environment values.

JSON Lines stream output must use one record per observation/status transition and terminate with a truthful final record on timeout, authorization loss, non-retryable protocol failure, or user cancellation.

## 5. Support canonical authoring without a second DSL

Provide practical ways to create a workflow while preserving the canonical blueprint model:

- strict blueprint validate/import/export;
- maintained complete starter examples;
- prompt-sequence validate/compile/import for the intended linear dogfood case;
- prospective remediation through an owner-provided builder or daemon operation;
- semantic diff inspection before apply.

Do not implement another mutable CLI workflow graph, local database, scripting language, hidden defaults, or client-side reconciliation planner. Any convenience must emit or submit the ordinary current versioned document and be independently inspectable.

## 6. Make local process and local model use ordinary product paths

Provide maintained examples and a concise documented command path for:

- one byte-pinned local process profile with explicit working directory, inputs, outputs, timeout, cancellation, and authority;
- one separately managed loopback OpenAI-compatible model endpoint profile with truthful features and explicit bounds;
- one small workflow or prompt sequence that uses each through the normal daemon registry and run path.

Milkdrift must not install, download, start, stop, or manage llama.cpp/Ollama/another server. It only configures an external endpoint.

The model scenario must assert structural evidence, not exact generated prose:

- exact profile/model/capability generation selected;
- one invocation and no unjustified duplicate entry;
- streamed observations ordered and bounded;
- response identity/model/usage retained when supplied;
- output artifact and context-manifest provenance inspectable;
- timeout, cancellation, malformed/truncated stream, server loss, and restart map to correct typed terminal or uncertain states.

Keep real local-model execution optional in ordinary CI, but make its operator command direct and routine rather than requiring knowledge of an evidence-only harness.

## 7. Add actual-binary black-box product evidence

Create one maintainable integration lane that builds and launches the real `milkdrift-daemon` and `milkdrift` binaries from a fresh temporary directory. It may start a deterministic local helper process and mock loopback model endpoint, but it must interact with Milkdrift only through files intentionally accepted by setup and the public control endpoint.

The scenario must prove:

1. create/validate configuration through the supported operator path;
2. start daemon and wait for readiness with a hard deadline;
3. validate/import a workflow or prompt sequence using the CLI;
4. inspect registered process/model capabilities;
5. start a run with an explicit command identity;
6. wait and inspect node, attempt, context, output artifact, and timeline JSON;
7. inject one failure or interruption and observe truthful state;
8. stop and restart the daemon without direct database manipulation;
9. submit/approve/apply a prospective remediation revision or resolve retained work through normal authority;
10. continue to terminal or explicitly retained uncertainty;
11. prove exact command replay cannot duplicate accepted work;
12. cleanly terminate every child process.

Do not share daemon internals with this test. The test should remain useful to an external packaged binary consumer.

## 8. Keep presentation modular but small

Retain one CLI application crate. Organize command families, session/config, output, input documents, streaming, and errors into private modules. Remove cross-family utility modules that become miscellaneous dumping grounds.

Human output should prioritize identifiers, state, and next action rather than pretty-printing arbitrary internal JSON. Stable JSON remains the complete machine surface.

## Required proof

Run:

- the full local gate and repository contracts;
- CLI unit/contract tests;
- actual-binary black-box lane;
- hermetic process/model lanes;
- real local-model lane when a separately managed endpoint is available;
- restart, replay, cancellation, uncertainty, proposal, artifact, and stream-focused tests;
- public-API inventory for control protocol/client and any owner API changed.

Record the final CLI internal dependency list and demonstrate that no CLI command opens storage, resolves secrets, evaluates authority, plans reconciliation, or invokes adapters directly.

## Completion threshold

This pass is complete only when:

- the CLI has no direct dependencies on the seven semantic/internal packages named above;
- a fresh setup no longer copies test fixtures;
- mutating and waiting commands are bounded and automation-safe;
- JSON success/failure/stream behavior is stable and complete;
- one actual-binary scenario exercises configuration, execution, inspection, restart, and prospective control;
- local process and local model examples use ordinary product paths;
- no UI, model manager, client database, second DSL, or semantic bypass was introduced.

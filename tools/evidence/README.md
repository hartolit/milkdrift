# Verification evidence tools

This unpublished package exercises Milkdrift through product APIs and actual application
binaries. Use it to reproduce an operational claim, measure a bounded path, or check an external
integration. It depends on the product; product packages do not depend on this harness.

Choose the lane whose observations support the claim you need:

| Tool | What it establishes and where to run it |
| --- | --- |
| `headless-cli-evidence` | Actual CLI/daemon setup, replay/conflict, control, inspection, downloads, and restart uncertainty using deterministic processes and a controlled model endpoint. [Commands](../../docs/development/verification-evidence.md#actual-binary-scenarios). |
| `local-model-evidence` | Model context, stream, output, and restart observations through actual applications. Deterministic mode checks the harness; real mode uses an operator-supplied loopback profile. Neither qualifies the combined agent/model gate. [Guide](../../docs/guides/local-model-endpoint.md#run-the-maintained-daemoncli-lane). |
| `milkdrift-external-evidence` | A real byte-pinned coding agent and supported model endpoint, with strict redacted report validation. Fixture mode remains non-qualifying. [Prerequisites and report meaning](../../docs/guides/external-evidence.md). |
| `core_paths` benchmark | Timing of selected journal, projection, receipt, peer, context, artifact, process, model-parser, and daemon paths. These measurements are not correctness budgets. [Benchmark lane](../../docs/development/verification-evidence.md#benchmarks-and-operations). |
| `operational-evidence` | Storage turnover/reopen, bounded current state, queue overload/recovery, stream reconnect, and graceful shutdown observations. The signal lane requires Unix; thread counts use Linux `/proc` when available. [Operational lane](../../docs/development/verification-evidence.md#benchmarks-and-operations). |
| `mutation-evidence` | Whether selected tests detect particular implementation mutations. Baseline failures, timeouts, and unrelated fixture failures do not qualify a mutant as caught. [Campaign and classification rules](../../docs/development/verification-evidence.md#mutation). |
| `evidence-process-helper` | Deterministic bytes and process behavior used by the harness, never a real coding-agent qualification. |

Build the daemon before isolated application-evidence tests. The maintained commands identify any
additional CLI/helper binaries and Python/Git prerequisites. Put logs, generated profiles, stores,
and reports under ignored `target/` or a private external directory. Scenario output can contain
sensitive data even when its report is redacted; follow the chosen guide before sharing it.

## Extend an observation

The library's `application` module owns child launch, bounded capture, readiness, deadlines, and
restart helpers. `OwnedChild::terminate` kills and reaps for an abrupt restart boundary;
`OwnedChild::shutdown` separately checks public Ctrl-C shutdown on Unix. A settled-work restart
scenario using the former does not prove graceful process shutdown.

The context, persistence, adapter, peer, and daemon modules exercise their corresponding product
owners. `ScenarioMeasurement` keeps result bytes observable through a checksum; report fields
state which counts, bytes, and timings were measured. `http_fixture` supplies controlled loopback
framing, not provider behavior. The external runner's profile, workflow, scenario, and report
modules keep operator inputs separate from generated work and qualification checks.

`repository_contracts` checks repository boundaries, maintained links, version cells, and example
readers. `operational_contracts` checks fast local fixtures; `external_evidence` checks the external
harness with deterministic inputs and injected failures. Retain useful failure evidence and trace
assertions to the product boundary they observe. The
[verification policy](../../docs/development/workflow.md#choose-verification-for-the-change)
determines which checks a change requires; [status](../../docs/product/status.md) records executed
qualification rather than treating a configured lane as a successful run.

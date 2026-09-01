# External process and model interoperability evidence

The operator-driven evidence harness runs a real coding-agent executable and a real supported model
endpoint through the ordinary Milkdrift daemon. It creates a disposable Git repository, explicit
scoped grants, process/model profiles, prompt-sequence remediation, artifacts, causal context, and
controlled restart boundaries. It does not benchmark model quality.

A report qualifies only when both real scenarios succeed. `--fixture --allow-fixture` is the
credential-free CI/self-test mode; it deliberately sets the report and both scenarios to
`qualifying: false` even when every harness assertion passes.

## Prerequisites

- Build prerequisites from [`DEVELOPMENT.md`](DEVELOPMENT.md), `/usr/bin/git`,
  `/usr/bin/python3`, and a `b3sum` CLI for preparing exact executable facts. Python is used only
  by the separate verifier/reviewer/evidence helpers.
- A clean Milkdrift checkout at the exact commit/tree being cited. Real mode refuses a dirty
  checkout because the recorded Git tree would not identify the tested source; fixture mode records
  `dirty_at_start` but remains non-qualifying.
- One real non-interactive coding-agent executable. Its process-profile schema-v2 document must
  pin the exact executable bytes and expose a writable `process.execute` capability. The executable
  must read the bounded prompt from the declared `prompt` input, work in its current directory,
  modify the disposable repository without committing, and publish an output named `diff` (a
  bounded stdout capture is permitted).
- One reachable endpoint supported by the existing `open_ai_compatible` chat-completions or native
  `anthropic` profile mapping. Its feature declarations must be truthful. `system_role` is required
  for the frozen causal manifest; the harness uses streaming and structured output when advertised.
  The provider response must include usage plus bounded response identity/model metadata.
- Every credential must be available through an environment variable or private file. Profiles
  contain only opaque `secret:…` references.

Copy the templates in [`examples/external-evidence`](../examples/external-evidence/README.md) to an
untracked operator directory and replace every placeholder. For the executable declaration,
compute the exact facts without loading the file into a shell variable:

```sh
b3sum /absolute/path/to/agent
stat -c '%s' /absolute/path/to/agent
/absolute/path/to/agent --version
```

The harness revalidates the source document, rejects known deterministic/test helpers, replaces
only its working directory with the disposable repository, appends only its isolated
session/repository roots, validates the rendered profile, rehashes the executable, and runs the
version command directly without a shell. Pass each version argument separately; `--version` is the
default.

## Run the evidence command

Use an empty external directory or a new directory beneath `target/`. Tracked source paths and
parent traversal are refused.

```sh
export MILKDRIFT_AGENT_TOKEN='replace-in-operator-environment'
export MILKDRIFT_MODEL_TOKEN='replace-in-operator-environment'

cargo external-evidence \
  --agent-profile /operator/private/coding-agent-profile.json \
  --agent-version-arg=--version \
  --model-profile /operator/private/model-profile.json \
  --model-capability external-evidence-model \
  --secret-source secret:coding-agent-token=env:MILKDRIFT_AGENT_TOKEN \
  --secret-source secret:model-token=env:MILKDRIFT_MODEL_TOKEN \
  --output target/milkdrift-external-evidence
```

For a private credential file, use
`--secret-source secret:model-token=file:/absolute/private/model.token`; Unix file sources must be
regular files with no group/other permission. Do not put a token in a URL, argv, JSON value, or
command log. Omit mappings for references the selected profiles do not use.

The command exits nonzero for a missing resource, invalid profile, fallback/helper profile,
unexpected workflow state, missing proof, uncertain external outcome, duplicate external entry,
redaction failure, or incomplete model response. An error still writes a non-qualifying report once
the output directory has been accepted. The selected output directory must initially be empty.

## What the command proves

The process scenario initializes an exact Git commit containing a deliberately broken calculator,
imports an ordinary prompt sequence, and starts a fresh real agent. A separate verifier publishes
result/log facts but intentionally withholds the success artifact. That is labeled orchestration
fault injection, not blamed on the agent. A fresh independent reviewer runs, the daemon shuts down
cleanly, and a new daemon recovers the durable approval boundary. The harness then submits,
approves, and applies an ordinary digest-bound remediation proposal, signals and resumes the run,
starts a second fresh agent process, and runs the good verifier. It checks distinct invocation IDs,
attempt provenance, output artifacts, no duplicate attempts, and the exact initial/final
commit/tree plus dirty-diff digest.

The model scenario publishes two selected evidence artifacts and one intentionally unselected
artifact, then stops at a durable signal wait before adapter entry. After a clean daemon restart it
verifies the same unreleased sequence, signals once, and permits one endpoint request. The model
task uses the frozen manifest, Fresh session policy, the endpoint's advertised streaming mode, and
either strict JSON `{ "ok": true }` or the exact text `MILKDRIFT_EVIDENCE_OK`. Success requires
selected/omitted artifact identity, durable fragment counts, response/finish/usage facts, nonempty
provider metadata, exact profile/protocol/model/origin provenance, committed output artifacts, and
one attempt with no uncertainty.

The harness uses controlled daemon shutdown. It never edits redb directly and does not add an
evidence-only scheduler, semantic node, provider family, or privileged endpoint.

## Endpoint profiles

For a real loopback llama.cpp-style server, start and secure the server separately, copy the
OpenAI-compatible template, and use the server's actual behavior:

```json
{
  "protocol": { "type": "open_ai_compatible", "path": "v1/chat/completions" },
  "base_url": "http://127.0.0.1:8080",
  "model": "exact-server-model-alias",
  "auth": { "type": "no_auth" },
  "features": ["streaming", "system_role"],
  "local_development": true,
  "allowed_hosts": ["127.0.0.1"]
}
```

This fragment is not a complete profile; retain the template's explicit limits, TLS, proxy,
redirect, concurrency, and trust-zone fields. Advertise `structured_output` only if the exact
server/model combination implements the requested JSON-schema contract. If it is omitted, the
harness requests the exact parseable text response. See [`LOCAL_MODEL_ENDPOINT.md`](LOCAL_MODEL_ENDPOINT.md).

For hosted OpenAI-compatible endpoints use HTTPS, the exact allowlisted host, bearer secret
reference, and only verified chat-completions features. For native Anthropic use the Anthropic
template, `anthropic_api_key`, its required API version/path, and do not claim structured output;
the current native mapping uses the exact parseable response path. No Responses API or other
provider family is implied.

## Codex CLI profile example

The committed coding-agent template uses the locally inspected
`codex-cli 0.147.0-alpha.6.6` non-interactive interface:

```text
codex exec --ephemeral --ignore-user-config --sandbox workspace-write --color never -
```

That version documents `-` as prompt stdin, `--ephemeral` as no session persistence, and
`--ignore-user-config` as ignoring configuration while retaining the `CODEX_HOME` authentication
boundary. This checkout did not have credentials for a qualifying Codex run, so the example is not
external proof. Revalidate the flags, executable bytes, authentication behavior, host filesystem
needs, and package version you actually deploy. The Milkdrift process boundary is trusted-host
mediation, not an OS sandbox; the child still has the daemon account's host authority.

## Cost, network, and sensitive output

The qualifying path normally starts the coding agent twice (initial and remediation) and calls the
model endpoint once. The coding agent may itself make additional provider calls according to its
own CLI. Provider charges, egress, rate limits, and model-side retention policies therefore apply.
Milkdrift grants only the endpoint profile/destination and declared secret references, but a
trusted-host coding process is not network-isolated by that declaration.

`report.json` is the redacted machine-readable summary. The rest of `session/` intentionally
contains the disposable repository, daemon store, restricted artifacts, rendered profiles, and
private local bearer files needed for evidence reconstruction. Treat the entire output directory
as sensitive scratch data, never commit it, and share only a separately reviewed report.

## Read and clean up the report

The report validates against strict serde v1 at write time; the consumer schema is
[`external-evidence-report-v1.schema.json`](reference/external-evidence-report-v1.schema.json).
The writer also caps the encoded report, rejects a qualifying document unless its source commit is
clean and exact, requires scenario-specific command/run/attempt/artifact/restart facts, validates
every reported artifact digest, and scans the complete encoded bytes for generated and
operator-mapped secret values before creating the file. These checks make malformed or leaked
reports non-writable; they do not sign a report or make self-reported evidence independently
attested.
Interpret the top level first:

- `qualifying: true` requires `fixture_mode: false` and both scenario `qualifying` fields true.
- Each scenario records the safe profile/executable/model identity, command/run/revision/attempt/
  proposal/artifact references, restart boundary, terminal facts, and failure reason.
- `configuration_digest` identifies the exact validated, path-normalized daemon configuration
  without disclosing its document or resolved credential values; it is `null` if validation never
  produced a configuration.
- Only digests, counts, bounded identities, redacted endpoint origin, safe version output, and
  provider-supplied usage are summarized. Authorization headers, secret values, complete prompts,
  complete outputs, environment dumps, and repository file contents are forbidden.
- `dirty_at_start` describes the Milkdrift checkout used to build/run the harness. A final
  qualifying record should be produced from the exact reviewed tree the operator intends to cite.

After retaining the reviewed report where your evidence policy requires, remove the selected output
directory using your normal secure cleanup process. The harness never deletes an operator-selected
evidence directory automatically.

## Hermetic self-test and optional peer evidence

The local self-test performs both workflows against a byte-pinned Python fixture and a loopback
mock endpoint, without external network or credentials:

```sh
cargo external-evidence \
  --fixture --allow-fixture \
  --output target/milkdrift-external-evidence-fixture
```

It exits zero only when the harness assertions pass, while the report remains non-qualifying.
Ordinary `cargo test` runs this fixture mode only; it never selects an operator real profile.

The optional remote-peer extension is not implemented by this command and is not required to close
the local process/model gate. The existing hermetic two-daemon proof can be run with
`cargo test -p milkdrift-daemon --test two_daemon_peer --all-features`; it is not a substitute for a
future operator-driven real remote-peer report. Do not label that deterministic test as external
peer evidence.

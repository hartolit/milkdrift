# External process and model interoperability evidence

`milkdrift-external-evidence` runs a byte-pinned coding process and configured model endpoint through
the production daemon task API. It creates a disposable Git repository, explicit grants and captured
artifacts, then verifies corrective future work and restart discovery. The harness tests structural
interoperability and evidence retention; it does not evaluate model quality.

Both real scenarios must succeed for qualification. `--fixture --allow-fixture` uses the Rust helper
and controlled loopback endpoint, and always reports non-qualifying even when every assertion passes.
Missing real resources never cause a silent fallback to those fixtures.

## Prepare operator resources

Build the daemon and harness with the repository's pinned Rust toolchain. Git and the real compiler
must be available for the disposable repository and independent verifier. Real qualification requires
a clean Milkdrift checkout so the recorded commit/tree identify the tested source.

The coding process profile must pin executable digest and byte size, provide a writable
`process.execute` capability, accept the named `prompt` input, work in its current repository without
committing, and publish a `diff` output. A bounded stdout artifact is permitted. The harness validates
the profile, rejects known fixture/test agents in real mode, rehashes the executable and runs a bounded
version command directly. It replaces the working directory with its disposable repository and adds
only its private session/repository roots. Review argv, filesystem needs and authentication against the
exact executable you intend to run; a template does not establish that compatibility.

The model profile selects one supported endpoint/protocol and exact model alias. Its accounting must
provide enforceable input/output and monetary bounds. Unknown counting or billing cannot reserve
ordinary task execution. The current task owner sends a fresh nonstream request with 4096 output units;
the profile must admit and enforce that request. The harness expects a normal `stop` response with
its exact bounded text contract, provider usage and response identity/model metadata. It does not expose
streaming, structured-output or custom output-unit switches for ordinary tasks.

Profiles contain opaque secret references. Resolve them from private files or environment variables.
Use [profile templates](../../examples/external-evidence/README.md) outside tracked source and replace
every placeholder. Exact executable facts can be inspected without putting bytes in a shell variable:

```sh
b3sum /absolute/path/to/agent
stat -c '%s' /absolute/path/to/agent
/absolute/path/to/agent --version
```

## Run a finite scenario

Choose an empty external directory or a new path below ignored `target/`. The runner rejects tracked
source paths, parent traversal and an existing nonempty output directory.

```sh
cargo build -p milkdrift-daemon --bin milkdrift-daemon
cargo external-evidence \
  --agent-profile /operator/private/coding-agent-profile.json \
  --agent-version-arg=--version \
  --model-profile /operator/private/model-profile.json \
  --model-capability external-evidence-model \
  --timeout-secs 900 \
  --secret-source secret:coding-agent-token=env:MILKDRIFT_AGENT_TOKEN \
  --secret-source secret:model-token=file:/operator/private/model.token \
  --output target/milkdrift-external-evidence
```

Set the referenced credentials in your operator environment beforehand. `--daemon PATH` selects an
explicit binary; otherwise the harness uses its built sibling. `--timeout-secs` accepts 1–3600 and
bounds each awaited workflow transition, including daemon reads. It is not a total scenario deadline.
The generated grants explicitly permit dynamic workflow IDs in this isolated installation; separate
capability, filesystem, network, secret, time and budget scopes remain declared. A broad workflow
grant is not actor-only read isolation.

The process scenario starts one coding task, a verifier with a deliberate failed check, and an
independent reviewer bound to that settled verification. The verifier retains its check report but
omits a required attestation, so the task owner's output contract fails. At the settled review boundary,
the runner kills and reaps the daemon, then reopens the same store and compares the retained facts.
An explicit atomic change adds fresh corrective coding and verification. The good verifier publishes
the required attestation; adoption checks the original output contract and retains the failed original.
The repository commit/tree must remain unchanged and its uncommitted diff must be nonempty.

The model scenario creates two selected evidence producers and one unselected producer. A missing
required release input keeps the model queued across another daemon restart. After an explicit input
change, the model task freezes only its named inputs and retains its canonical response and provider
metadata. Fixture mode additionally observes the actual HTTP body, checks selected bytes and absence
of unselected bytes, and counts exactly one endpoint request. Real mode does not claim access to a
remote provider's internal request counter.

These restarts exercise settled or unentered boundaries. They do not establish graceful signal
handling, remote cancellation or exactly-once provider effects.

## Read the report

The writer validates strict schema 2 and the
[consumer schema](../reference/external-evidence-report-v2.schema.json), caps encoded bytes, and rejects
forbidden fields or exact secret values before writing. The report contains source/configuration
digests, safe profile/executable facts, request receipts, workflows, tasks, adopted corrections,
artifact metadata, restart generations, and bounded outcome facts. Those checks are not a signature
or independent attestation of a fabricated report.

`qualifying: true` requires both real scenarios, a clean source checkout and all required semantic
facts. A normal model finish is required; a truncated answer cannot qualify. Fixture mode always
sets both scenario qualification flags false. Configuration failures and partial scenario failures
remain explicit rather than inventing success for unrun work.

Only the report is a candidate for sharing after review. The rest of `session/` contains captured
inputs, repository contents, provider results, credentials, profiles and the daemon store. Treat the
whole output directory as sensitive scratch data. The harness never deletes the operator-selected
directory automatically. A trusted host process has the daemon account's native host authority;
its internal provider requests and spending are not metered as direct Milkdrift model tasks.

## Check the harness locally

```sh
cargo external-evidence --fixture --allow-fixture --output target/external-evidence-fixture
```

The fixture exits successfully only when its assertions pass while remaining non-qualifying.
Integration tests also inject process/model failure and truncated output, verify secret redaction,
and reject unsafe output paths. The independent remote-peer evidence scenario remains outside this
command; hermetic daemon peer tests are not proof about an operator's remote host.

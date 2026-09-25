# Slotbook candidate API

This contract fixes the HTTP representation before constructing the seeded or repaired candidate.
The operator-owned verifier implements the six observations in the
[application specification](../../docs/guides/adaptive-method-example.md). Candidates are immutable
compiled Rust executable artifacts run directly in the exact configured container image. No rebuild
occurs after checking. The runtime image and non-secret configuration belong to the recipe identity.
This example package owns the independently deployed application; the existing `milkdrift-evidence`
package owns its preparation, qualification and trusted verifier executables.

The candidate listens on container port 8080. It reads `/config/application.json` for `resource`
and positive integer `capacity`, optional `cancellation_notice_seconds` (zero by default),
`/config/token` for the synthetic bearer token, and
`/config/clock` for the operator-controlled UTC timestamp. It stores bookings in `/data`; all
other mounts and the root filesystem are read-only. The clock is a test target input, not a public
HTTP operation. No model worker can mount these directories or the engine socket.

- `GET /health` returns 200.
- `GET /availability?start=...&end=...` returns only `resource`, `start`, `end`, and `remaining`.
  Timestamps use RFC3339 UTC with `Z`, optionally fractional seconds, within 32 bytes;
  invalid calendar dates refuse. Intervals are half-open and equivalent instants compare equally.
- `POST /reservations` accepts JSON `name`, `start`, `end`, `quantity` and returns 201 with `id`.
  Missing or wrong bearer tokens return 401 before mutation. Invalid intervals or zero quantity
  return 400, and insufficient overlapping capacity returns 409.
- `GET /reservations` requires the token and returns a JSON array of active reservations, each with
  `id`, `name`, `start`, `end`, and `quantity`.
- `DELETE /reservations/{id}` requires the token. Before the interval start minus its configured
  notice period it returns 200 and releases capacity. Repeating successful cancellation returns
  200. At or after that cutoff an
  active reservation returns 409 and retains the booking.

The corrected application retains at most 4096 booking identities, including cancelled bookings
needed for idempotent cancellation. New bookings refuse with 507 at that bound. Each mutation
atomically replaces a bounded JSON snapshot. A failed write leaves the service unavailable until
reopen, so an uncertain persistence result cannot allow conflicting mutations in memory.

The verifier uses synthetic names and secrets. It independently creates requests, races two
contenders, observes service recreation with the same test data directory, and checks container mounts and
candidate bytes. Its stdout is a bounded JSON array of named check observations. Application logs
and response bodies are not used as diagnostic text, so test tokens cannot become report content.
A passed report establishes these finite checks, not general correctness, security or power-loss
persistence. Seeded regression fixtures must be labeled separately from live model output.

The unchanged six check names also cover the fixed held-out loan/class parameters. The verifier
selects its operator-owned case from the exact application configuration: camera/1/0,
tripod/2/3600, yoga/4/7200, or ceramics/6/86400 (resource/capacity/notice seconds).
It checks the dates, quantities and cancellation boundaries in the maintained specification;
candidate code cannot supply an expected response. Unsupported combinations refuse verification.

The configured test token is part of the candidate configuration through its digest and is used
by both verification and activation. Rotating it requires a distinct approved target configuration;
old evidence cannot authorize the changed credential generation. The clock file is an explicitly
variable operator-controlled test input. Preserve its inode when changing its contents and make it
readable inside the isolated container; keep its parent outside worker mounts.

## Author and run the example

Provision the rootless Podman/user-systemd prerequisites in
[managed operations](../../docs/operations/managed-linux.md). Build the Rust tools and both native
application fixtures, then package the candidates in an exact local image. The commands below target
x86-64 GNU/Linux and statically link the candidates. They do not establish another target's support.
`slotbook-seeded` deliberately permits anonymous mutation and loses bookings at restart; `slotbook`
is the deterministic corrected fixture. This qualification uses no live model or cloud credentials.

```sh
CARGO_PROFILE_DEV_OPT_LEVEL=1 CARGO_PROFILE_DEV_DEBUG=0 \
  cargo build -p milkdrift-cli -p milkdrift-daemon -p milkdrift-evidence --bins
CARGO_TARGET_DIR=target/slotbook-static CARGO_PROFILE_RELEASE_STRIP=symbols \
  RUSTFLAGS='-C target-feature=+crt-static' cargo build -p milkdrift-slotbook --bins \
  --release --target x86_64-unknown-linux-gnu
mkdir -p target/slotbook-image
cp target/slotbook-static/x86_64-unknown-linux-gnu/release/slotbook \
  target/slotbook-static/x86_64-unknown-linux-gnu/release/slotbook-seeded \
  target/slotbook-static/x86_64-unknown-linux-gnu/release/slotbook-workspace target/slotbook-image/
cp examples/adaptive-slotbook/Containerfile target/slotbook-image/
podman build --pull=never --build-arg BASE_IMAGE=sha256:PRELOADED_BUSYBOX_IMAGE_ID \
  -t localhost/milkdrift-slotbook-rust target/slotbook-image
podman image inspect --format '{{.Id}}' localhost/milkdrift-slotbook-rust
target/debug/slotbook-evidence prepare --root /absolute/private/slotbook-example \
  --image sha256:RESULTING_IMAGE_ID
target/debug/slotbook-evidence qualify --root /absolute/private/slotbook-example \
  --candidate target/slotbook-static/x86_64-unknown-linux-gnu/release/slotbook
```

Replace both image placeholders with inspected exact identities. The preloaded base must provide
the example worker's `sh`, `cp`, `cat`, `test` and `sleep` utilities. The build copies the two
application executables and native workspace documentation tool into `/fixtures`; it includes no token or trusted verifier. Qualify every
selected image; its tag is only a build convenience. `--candidate` supplies the independently built
corrected bytes for comparison against the downloaded artifact and deployed mount. The prepared files
include `policy.json`, `adaptation-scope.json`, `base.json`, `governed.json`, two operator recipe inputs,
and repair mutations. `slotbook-evidence prepare` invokes `artifact digest`, `blueprint effect-policy`,
`blueprint create` and `blueprint govern`; the product owns parsing, identity derivation and validation.
It copies the native verifier into private operator storage and pins its digest and a synthetic token.
Rebuilding the development executable cannot change that approved copy. The generated target version
must match inspection after initial application (normally 6); `--target-version` makes this explicit.

`slotbook-evidence qualify` starts a private workflow-enabled host on port 19748, installs only its approved
`slotbook-build` worker and `slotbook-test` service, then follows the ordinary authenticated CLI.
It retains initial failed checks, demonstrates failed evidence and uploaded forged-report publication
refusals, imports the method, and starts a run with a 45-second repair window. A structured proposal inserts investigation
and replaces future repair work inside `repair.`. The ordinary proposal path applies it under the
agreement policy without a fresh approval. The repaired worker emits immutable executable bytes, the fixed
verifier evaluates it, and the fixed publisher consumes its journal identity. It also renews verification
of unchanged bytes without overwriting earlier observations. The qualifier inspects completion, reopens
the store, checks retained failure and removes its installations through the
resource owner. Failure retains resources and logs for explicit inspection; it never prunes Podman.
Data disposition is `preserve`, so removal deliberately leaves owned volumes for operator retention.

The generated `host/daemon.toml` grants `agent:repair` only the two approved managed capability
families and finite budgets. The operator supplies approved recipes outside worker storage. This
fixture uses a controller preset to expose both authoring and inspection; production deployments
should narrow operations to each actor's work. Capability allowlists cannot approve another recipe,
change the target policy, or mount host credentials into the worker. The example declares a 4 MiB
candidate ceiling, 32 MiB individual artifact allowance and 128 MiB internal artifact budget to
accommodate native executables and their escaped worker reports. These are explicit example
allocations, not platform defaults; preparation and admission validate them through the normal owners.

## Inspect and adapt through supported commands

Set `MILKDRIFT_ENDPOINT` and `MILKDRIFT_TOKEN_FILE` to the private host's endpoint and
`host/operator.token`. The following commands use identities from `evidence/*.json`; placeholders and the example version 6
must be replaced by those exact retained values.

```sh
milkdrift --json run show slotbook-run
milkdrift --json run timeline slotbook-run --limit 256
milkdrift --json proposal list slotbook-run
milkdrift --json resource --installation slotbook-test inspect
milkdrift --json resource --installation slotbook-test evidence --evaluation EVALUATION_DIGEST
milkdrift --json --command-id publish-reviewed resource --installation slotbook-test \
  --expected-version 6 publish --evaluation EVALUATION_DIGEST
```

Evaluation accepts a complete immutable artifact reference via `resource evaluate --artifact ID
--digest DIGEST --media-type TYPE --size-bytes SIZE`. Its receipt deliberately records the accepted
incomplete observation. `resource evidence` reads the completed private journal record. A report's
checksum, producer string or positive boolean alone cannot authorize publication. A failed/unknown
or expired check, wrong artifact metadata, changed policy or wrong target generation refuses entry.
Inspect `checks`, `pending`, `diagnostics`, `generation` and the run's `agreement_adoptions` separately.
An accepted publication receipt is not proof that the service is running.

`repair-proposal.json` shows the authoring envelope: `schema_version: 1` with `draft`, exact base
revision/digest and observed run sequence, ordinary mutation objects, retained evidence references,
and `auto_apply_low_risk`. The owner computes canonical mutation/proposal digests. Canonical saved
proposals still use `proposal` with their digest; mixed, duplicate, unknown or obsolete fields refuse.
`proposal submit` also works through the existing `milkdrift-workflow-control` capability's `workflow.propose_revision` operation.
Notes, provenance and completion claims remain untrusted. Boundary edges, verifier inputs, targets,
terminals, interface, policy and maximum revisions cannot change inside the grant's adaptable region.
A legitimate requirement change needs a newly sealed agreement and a distinct approved installation
and run. Earlier failed evidence and the original agreement remain unchanged.

The policy currently retains at most 4096 evaluations in one store and refuses overflow. Interrupted
verifications remain unknown; exact replay never reruns them. Scratch and test containers belong to
the exact accepted evaluation. A new authorized request can check unchanged bytes at the same target
generation to renew expired evidence, retaining the earlier result. The platform fences its verifier
container after completion or timeout and at startup after interruption; a cleanup failure cannot
produce passing evidence. Recovery never reruns the verifier. A controlled publication or start
rechecks lifetime and current approval.
An already accepted systemd service can restart with the same immutable generation and retained data.
Power loss, hostile host administration, arbitrary application security and second-host deployment
are outside this finite example's evidence.


## Invoke the governed method as a capability

After preparing a fresh directory as above, add `--published` to `slotbook-evidence qualify`. It configures the
exact service grant and publishes the governed revision as `method:slotbook` generation 1. A separate
invoke-only credential requests `method.invoke`; the operator finds the actual linked run through
ordinary authorized run inspection and submits the same permitted repair proposal. The consumer is
refused method/internal inspection and resource administration. One execution worker runs the
internal process tasks. Exact public replay after restart leaves the deployment generation unchanged.

Use `--published --invocation-mode workflow` in another fresh directory to make an ordinary local
workflow call the same public method. The outer attempt retains its link and the internal run keeps
its own agreement and revision history. Use `--published --invocation-mode peer` in a fresh directory
to put that caller on a second local daemon, which has no managed adapters. The provider retains
the service grant and resource ownership. `--port` selects the provider's control listener; the peer
caller uses the next port. This exercises two processes on one host, not a second-machine deployment.
All modes retain failed evidence and exercise the same protected verifier/publisher before removing
their installations. Run these native qualifications serially: they deliberately use the fixed
Slotbook application port 19848 and installation names.

The current 03 method has an empty input interface and a fixed target/version. This example does
not imply arbitrary-target deployment or a parameterized replacement agreement. The publication
contract and finite input rules are described in the [published methods guide](../../docs/guides/published-methods.md).

## Learn from selected evidence and compare product variations

The [learning guide](../../docs/guides/learning-methods.md) explains the supported `learning` command
documents and authority boundaries. After a successful `qualify --published`, retain that private
directory and run the Rust driver with an operator-reviewed model profile:

```sh
target/debug/slotbook-evidence learn \
  --root target/slotbook-study \
  --image sha256:EXACT_SLOTBOOK_IMAGE_ID \
  --model-profile /absolute/path/to/local-model-profile.json \
--port 19758
```

Use a profile accepted by the ordinary model-provider reader, enabling strict structured output.
The endpoint, model alias, trust boundary, deadlines and billing declaration remain operator inputs.
On a new study, `--service-port-base` chooses eleven consecutive loopback application ports
(default 19900–19910). Choose an unused range when retaining another study's deployed product.
For a remote service that offers only HTTP, use the supported local development profile through an
authenticated loopback tunnel; do not remove the non-loopback plaintext guard. The driver permits
one external proposal generation at a time. Server context capacity and effective sampling settings
remain separate from the local request's declared limits.

This study authors a new bounded, parameterized agreement from the source example. It does not
retroactively change the original method's fixed target. The baseline has three immutable verifier
submissions. Only worker nodes inside `repair.` are editable. Each held-out method/input has its own
prepared worker, protected target, service grant, public request key and cumulative account. The
four pairs, six checks, exact tools and two-repair threshold are declared before model entry. The
proposal receives selected source failures, repair, method and guidance; it receives no held-out
case artifacts. A model response passes the ordinary proposal, graph and agreement readers before
it becomes a candidate. The implementation and repair binaries remain labeled native fixtures;
this lane does not demonstrate a model independently implementing an application.

The study retains generated documents, CLI results and private observations under `learning-05`.
`result.json` records the comparison, selected method, independently executed camera-loan and yoga
products, and the operator's explicit choice to deploy only the loan product. Product configuration,
private verification and accepted method lineage distinguish the services; they use the same native
application template. A rejected or inconclusive candidate leaves the baseline selected. Each
variant must pass its own target's verifier, and cross-target publication evidence is refused.

`setup-improvement.json` records a scratch tool experiment, exact native/build inputs, a new approved
recipe tested in a fresh installation, and activation with existing application bytes preserved.
`knowledge-update-result.json` records the subsequent explicit guidance selection and the unchanged
earlier selection and model manifest. Neither change silently republishes the workflow.

After interruption, `--resume` reuses exact accepted requests and refuses changed retained inputs.
If a proposal failed before any comparison invocation was accepted, `--resume --proposal-attempt 2`
can declare a new prospective proposal against the same frozen cases and criterion. Attempt numbers
are bounded to 1–4. Earlier model responses, failures and uncertain usage remain retained. An
entered request is never blindly retried as if no effect occurred. This recovery option does not
authorize changing the criterion after observing evaluation results. Do not overwrite the private
study directory or treat a new proposal run as settlement of an earlier unknown effect.

For deterministic validation, use a separate fresh qualification directory and a reviewed local
profile naming model `learning-fixture`, with no authentication, structured-output support and
endpoint `http://127.0.0.1:18082`. Start this finite endpoint before `learn`:

```sh
target/debug/slotbook-evidence model-fixture \
  --root target/slotbook-fixture-study --port 18082 --mode useful
```

It accepts one request by default and reads only the frozen source selection and baseline. Its
controlled response replaces the first seeded build with the corrected native fixture. Modes
`no-improvement`, `malformed`, `invented-evidence` and `forbidden-edit` supply deliberate
counterexamples. These are synthetic responses and token reports, not local-model quality
measurements. They pass through the same HTTP adapter, retained manifest, ordinary proposal reader
and learning commands. A different endpoint requires its own reviewed network grant; a proposal
retry does not expand an existing grant. `--proposal-only` retains an admitted candidate before
evaluation; resume with the same attempt number to continue its original comparison.
`--candidate-validation 2` resubmits the same retained response under a new command identity after
a refused local submission has been diagnosed. It does not generate a new response, change its
mutation, replace a rejection receipt or authorize a changed comparison. Validation identities
are bounded to 1–4; accepted submissions use exact replay.

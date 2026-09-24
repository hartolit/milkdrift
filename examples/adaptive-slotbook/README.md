# Slotbook candidate API

This contract fixes the HTTP representation before constructing the seeded or repaired candidate.
The operator-owned verifier implements the six observations in the
[application specification](../../docs/guides/adaptive-method-example.md). Candidates are immutable
compiled Rust executable artifacts run directly in the exact configured container image. No rebuild
occurs after checking. The runtime image and non-secret configuration belong to the recipe identity.
This example package owns the independently deployed application; the existing `milkdrift-evidence`
package owns its preparation, qualification and trusted verifier executables.

The candidate listens on container port 8080. It reads `/config/application.json` for `resource`
and positive integer `capacity`, `/config/token` for the synthetic bearer token, and
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
- `DELETE /reservations/{id}` requires the token. Before the interval starts it returns 200 and
  releases capacity. Repeating successful cancellation returns 200. At or after the start of an
  active reservation it returns 409 and retains the booking.

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
  target/slotbook-static/x86_64-unknown-linux-gnu/release/slotbook-seeded target/slotbook-image/
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
the example worker's `sh`, `cp`, `cat`, `test` and `sleep` utilities. The build copies only the two
application executables into `/fixtures`; it includes no token or trusted verifier. Qualify every
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

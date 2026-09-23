# Slotbook candidate API

This contract fixes the HTTP representation before constructing the seeded or repaired candidate.
The operator-owned verifier implements the six observations in the
[application specification](../../docs/guides/adaptive-method-example.md). Candidates are immutable
UTF-8 Python source artifacts executed by the exact configured Python container image. No rebuild
occurs after checking. The runtime image and non-secret configuration belong to the recipe identity.

The candidate listens on container port 8080. It reads `/config/application.json` for `resource`
and positive integer `capacity`, `/config/token` for the synthetic bearer token, and
`/config/clock` for the operator-controlled UTC timestamp. It stores bookings in `/data`; all
other mounts and the root filesystem are read-only. The clock is a test target input, not a public
HTTP operation. No model worker can mount these directories or the engine socket.

- `GET /health` returns 200.
- `GET /availability?start=...&end=...` returns only `resource`, `start`, `end`, and `remaining`.
  Timestamps are RFC3339 UTC with `Z`; intervals are half-open.
- `POST /reservations` accepts JSON `name`, `start`, `end`, `quantity` and returns 201 with `id`.
  Missing or wrong bearer tokens return 401 before mutation. Invalid intervals or zero quantity
  return 400, and insufficient overlapping capacity returns 409.
- `GET /reservations` requires the token and returns a JSON array of active reservations, each with
  `id`, `name`, `start`, `end`, and `quantity`.
- `DELETE /reservations/{id}` requires the token. Before the interval starts it returns 200 and
  releases capacity. Repeating successful cancellation returns 200. At or after the start of an
  active reservation it returns 409 and retains the booking.

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

Build `milkdrift-cli` and `milkdrift-daemon`, provision the rootless Podman/user-systemd prerequisites
in [managed operations](../../docs/operations/managed-linux.md), and load the exact Python image
identified below. The scripts require Python 3 and a fresh private directory. They use no cloud
credentials or live model generation. `seeded.py` deliberately permits anonymous mutation and loses
bookings at restart; `repaired.py` is a deterministic repair fixture.

```sh
cargo build -p milkdrift-cli -p milkdrift-daemon
podman pull docker.io/library/python@sha256:2325bb286ec344af3e5898cc224b5844e2707ac6e26b1632516fd3edc84a5e26
python3 examples/adaptive-slotbook/prepare.py --root /absolute/private/slotbook-example \
  --image docker.io/library/python@sha256:2325bb286ec344af3e5898cc224b5844e2707ac6e26b1632516fd3edc84a5e26
python3 examples/adaptive-slotbook/qualify.py --root /absolute/private/slotbook-example
```

The declared immutable image must be available locally. Supply another independently selected exact
image only after qualifying it. The prepared files
include `policy.json`, `adaptation-scope.json`, `base.json`, `governed.json`, two operator recipe inputs,
and repair mutations. `prepare.py` invokes `artifact digest`, `blueprint effect-policy`,
`blueprint create` and `blueprint govern`; the product owns parsing, identity derivation and validation.
It pins the verifier source/native interpreter and a synthetic token. The generated target version
must match inspection after initial application (normally 6); `--target-version` makes this explicit.

`qualify.py` starts a private workflow-enabled host on port 19748, installs only its approved
`slotbook-build` worker and `slotbook-test` service, then follows the ordinary authenticated CLI.
It retains initial failed checks, demonstrates failed evidence and uploaded forged-report publication
refusals, imports the method, and starts a run with a 45-second repair window. A structured proposal inserts investigation
and replaces future repair work inside `repair.`. The ordinary proposal path applies it under the
agreement policy without a fresh approval. The repaired worker emits immutable source, the fixed
verifier evaluates it, and the fixed publisher consumes its journal identity. It also renews verification
of unchanged bytes without overwriting earlier observations. The script inspects completion, reopens
the store, checks retained failure and removes its installations through the
resource owner. Failure retains resources and logs for explicit inspection; it never prunes Podman.
Data disposition is `preserve`, so removal deliberately leaves owned volumes for operator retention.

The generated `host/daemon.toml` grants `agent:repair` only the two approved managed capability
families and finite budgets. The operator supplies approved recipes outside worker storage. This
fixture uses a controller preset to expose both authoring and inspection; production deployments
should narrow operations to each actor's work. Capability allowlists cannot approve another recipe,
change the target policy, or mount host credentials into the worker.

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

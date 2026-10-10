# Executed observations — E1

Rowan-20261010 ran these isolated observations on 2026-10-10. Production source is
`908e7893f5dadb84d12712573c8daaa946829e39` (unchanged from archived `c016cd3`);
office checkpoint `d7caf6d` changes planning only. Linux x86-64, kernel `7.2.8-arch1-2`,
Rust `1.95.0 (59807616e 2026-04-14)`, ordinary unoptimized debug profile.
Raw output and binary SHA-256 identities are under `target/remedial-planning-v3/`.
These are controlled providers and temporary stores, not live-model/hardware qualification.

## Predictions and backend journeys

The independent [Ada](trial-ada.md) and [Bram](trial-bram.md) trial records fixed expectations
before observation: direct execution needs no publication/workflow; invoke-only output must
remain available without internal privileges; replay must not duplicate effects. Shared versus
separate discovery cannot be decided by these backend tests.

Built exact applications with:

```sh
cargo build --locked -p milkdrift-daemon --bin milkdrift-daemon \
  -p milkdrift-cli --bin milkdrift \
  -p milkdrift-local-process --bin milkdrift-process-test-helper \
  -p milkdrift-evidence --bin headless-cli-evidence --bin client-workflow-evidence
target/debug/client-workflow-evidence --daemon target/debug/milkdrift-daemon \
  --cli target/debug/milkdrift --output target/remedial-planning-v3/client-journey
target/debug/headless-cli-evidence --independent-host-only \
  --daemon target/debug/milkdrift-daemon --cli target/debug/milkdrift
```

E01: authored-workflow report is `passed: true`, nine controlled model requests, zero planned
live requests, fixture-count check and daemon/fixture cleanup successful. The maintained
[journey](../../../../../tools/evidence/src/bin/client-workflow-evidence/journey.rs) covers
authoring, different supplied briefs, results, restart/replay, copies and prospective repair.
It establishes that this public route works under its fixture, not human comprehension.

E02: independent-host driver exited successfully. Its two actual daemons perform direct and
workflow-origin process/model work, transfer selected inputs/results, preserve exact replay after
restart, and retain unknown usage after a dropped model response. The serving host creates no
synthetic workflows. Source assertions require two process entries and three model requests.
The [driver](../../../../../tools/evidence/src/bin/headless-cli-evidence/independent.rs) owns
the scenario. This is same-machine loopback, not two physical machines or C→B direct relay.

E03: these focused publication checks passed (one, five and one tests respectively):

```sh
cargo test --locked -p milkdrift-daemon --test control_plane --all-features \
  published::invoke_only_published_outputs_replay_retirement_and_restart
cargo test --locked -p milkdrift-control --test control_service --all-features \
  published::recovery::
cargo test --locked -p milkdrift-daemon --test two_daemon_peer --all-features \
  published::peer_publication_creates_one_run_and_transfers_only_the_accepted_result
```

The five recovery cases cover lost create/bind/start/result commits, cancellation after lost
create or child start, and failed/lost promotion. The peer case retains one internal run and
transfers only accepted output. Tests exercise production decisions with controlled effects.
They support retention of restricted invocation; they do not prove every nested-resource ordering.

## Browser observation

E05: `cargo test --locked -p milkdrift-daemon --test control_plane --all-features independent_client::json_client_authors_runs_recovers_downloads_and_copies_without_private_builders`
passed one test. It uses an actual daemon binary, public JSON and controlled endpoint, including
restart/replay without private builders. Log: `independent-json.log`. It is native HTTP, not browser proof.

E04 prediction: the unchanged daemon's intentionally absent CORS layer will prevent a different
browser origin from using its bearer-authenticated reads, even when native HTTP succeeds.
Inspection of [router](../../../../../apps/daemon/src/http.rs) and
[bearer extraction](../../../../../apps/daemon/src/http/response.rs) preceded the probe.

Two isolated daemons used the maintained `examples/operator/daemon.toml` with only temporary
absolute data roots, fixture credential reference, host identities `host:planning-a/b` and
loopback ports 19734/19736 changed. No adapters or private data were installed. A temporary Rust
HTTP listener at `http://127.0.0.1:19735` served a bare diagnostic that performed bounded reads
and protocol negotiation. It is not a frontend prototype. Each fetch used a three-second abort,
`redirect: error`, fixture Authorization header and no cookie credential mode.

Native `GET /v1/authority` on A returned 200, protocol 2.20, the exact host identity and fixture
actor/grant. An OPTIONS request carrying `Origin: http://127.0.0.1:19735`, requested method GET
and requested header authorization returned 405 with `Allow: GET,HEAD`, no CORS allow headers.
Both outcomes are retained in `browser/native-authority.txt` and `browser/preflight.txt`.

The in-app Chromium browser reported Chrome `152.0.0.0`, Linux x86-64. On both daemon origins,
authenticated authority GET, protocol negotiation POST and `/v1/stream/capabilities` fetch
reported `TypeError: Failed to fetch`. No mutation was submitted. The initial diagnostic had an
incorrect stream URL; it was corrected to the source route and all six probes were rerun, with
the same failure. Only the corrected run supports this record.

This establishes failure of the exercised cross-origin path. The 405/no-allow response and source
explain the CORS obstacle; generic fetch errors alone do not establish every browser refusal cause.
No successful browser streaming, reconnect, HTTPS proxy, LAN, private-network permission, Firefox,
WebKit or desktop-wrapper path was observed. Those are explicit implementation acceptance gates.
The proposed transport must make progress without removing daemon authentication or placing tokens
in URLs. The [Fetch standard](https://fetch.spec.whatwg.org/#http-cors-protocol) and
[EventSource interface](https://html.spec.whatwg.org/multipage/server-sent-events.html#the-eventsource-interface)
were checked on 2026-10-10; native EventSource exposes no arbitrary Authorization header option.

## Evidence limits

No full runtime gate is claimed or required for these planning changes. Existing tests are
current contract evidence, not the sole oracle for a replacement's value. Current source readers
and modeled scenarios still carry the gaps for full structured editing, method defaults,
operator release, arbitrary third-host relay and the entire S28 research process. A new proposed
interaction or runtime mapping must earn its own acceptance proof.

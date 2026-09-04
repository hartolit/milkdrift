# Connecting a local OpenAI-compatible model endpoint

Milkdrift treats a local model server as a configured capability endpoint. It does not load model
weights, select a model architecture, manage inference memory, or start/stop the server. Run and
secure the server separately.

The maintained loopback profile is
[`openai-compatible-loopback.example.json`](../../examples/local-model/openai-compatible-loopback.example.json).
Copy it to an untracked operator directory and replace the exact model alias. Remove `streaming`
unless that exact server/model supports chat-completions SSE. Add a bearer `SecretRef` only when
the server requires one; keep the resolved token in a private file rather than the profile.

For a llama.cpp server using its OpenAI-compatible chat endpoint, create endpoint-profile schema
v1 with:

- protocol `open_ai_compatible` and path `v1/chat/completions`;
- a loopback base URL such as `http://127.0.0.1:8080`;
- the server's exact model alias;
- `no_auth`, `local_development: true`, and an allowlist containing only `127.0.0.1`;
- explicit outbound request-body, request-time, idle, header, response, SSE, and fragment bounds;
- only features the selected server/model configuration actually implements.

Plain HTTP is accepted only for an explicit loopback development profile. Remote endpoints require
HTTPS. Ambient proxies are disabled unless the profile explicitly chooses system proxy behavior,
cross-origin redirects are never followed, and the default redirect policy follows none. If a
local server requires a token, put only an opaque `SecretRef` in the profile and resolve it through
the host secret boundary; never place a token in the URL.

The adapter uses chat-completions semantics. Tools, image parts, developer roles, reasoning
controls, structured output, and streaming must be advertised by the exact profile or the request
is rejected before connection. For Fresh requests, it verifies the persisted schema-v2 causal
manifest against the exact attempt, sends the canonical manifest as the first system context block,
then reads only the manifest-selected reserved inputs. Selected text/JSON becomes explicitly
delimited untrusted user evidence; supported selected images remain image parts. Every selected
input is checked against its manifest digest, size, and media facts, extra reserved inputs are
rejected, and unsupported generic binary evidence fails before connection. This path does not
dereference arbitrary model-task references or unselected artifacts. Generic file parts, explicit
continuation artifacts, and provider-managed sessions currently have no OpenAI-compatible mapping.
Cancellation signals cause the response reader to close at its next observable read boundary; the
acknowledgement truthfully does not claim provider-side termination.

The operation contract is deliberately conservative. `model.generate` advertises unknown side
effects, unsupported idempotency, and best-effort cancellation. Request bytes may have entered a
provider even when the response is lost, so a post-entry close, malformed/truncated response,
timeout, or cancellation is retained as uncertain. Milkdrift neither automatically retries that
work nor treats a successful response as proof that the provider had no external effects.

## Run the maintained daemon/CLI lane

Build the actual applications and the byte-pinned evidence producer:

```sh
cargo build -p milkdrift-daemon --bin milkdrift-daemon \
  -p milkdrift-cli --bin milkdrift \
  -p milkdrift-evidence --bin evidence-process-helper --bin local-model-evidence
```

First prove the harness with its controlled loopback parser. The selected output directory must be
empty or absent and remains untracked:

```sh
target/debug/local-model-evidence \
  --daemon target/debug/milkdrift-daemon \
  --cli target/debug/milkdrift \
  --process-helper target/debug/evidence-process-helper \
  --mode deterministic \
  --output target/local-model-evidence
```

For a separately running real loopback server, use the explicit real mode. It rejects a missing or
non-loopback profile and never falls back to the controlled endpoint:

```sh
target/debug/local-model-evidence \
  --daemon target/debug/milkdrift-daemon \
  --cli target/debug/milkdrift \
  --process-helper target/debug/evidence-process-helper \
  --mode operator-real-endpoint \
  --model-profile /operator/private/local-model-profile.json \
  --model-capability local-model-loopback \
  --output target/local-model-real
```

If the profile names `secret:model-token`, add
`--secret-source secret:model-token=/absolute/private/model.token`. Unix secret files must be
private regular files. The lane derives a schema-9 daemon configuration and canonical ordinary
blueprints from their owning Rust contracts, then uses the actual CLI for these structural actions:

```sh
milkdrift daemon readiness
milkdrift capability show local-model-loopback
milkdrift --command-id local-model-validate-1 blueprint validate -
milkdrift --command-id local-model-import-1 blueprint import MODEL_BLUEPRINT
milkdrift --command-id local-model-start-1 run start \
  run-local-model-dogfood local-model-dogfood REVISION_ID
milkdrift run timeline run-local-model-dogfood --limit 100 --follow
milkdrift --command-id local-model-signal-1 --expected-sequence SEQUENCE \
  run signal run-local-model-dogfood --signal-id local-model-release-1 \
  --signal-type evidence.model.release --payload '{"release":true}'
milkdrift attempt inspect run-local-model-dogfood ATTEMPT_ID
milkdrift artifact get ARTIFACT_ID --output /operator/private/model-response.json
```

The harness owns shutdown/restart and repeats run and attempt inspection against the same store. It
also drives a controlled post-entry connection close and proves retained uncertainty, refusal of
an unsafe retry, explicit retain through `attempt resolve`, and restart visibility. Its redacted
`report.json` records only identities, counts, boolean structural facts, safe endpoint origin, and
the reason `qualifying` is false. Session state includes prompts and generated artifacts, so treat
the complete output directory as sensitive scratch data.

Expected success is structural: exactly one attempt and endpoint entry, exact profile/model/context
provenance, selected and omitted evidence identities, ordered bounded fragments when streaming,
one accepted terminal, verified output artifact digests/linkage, and usage/response identity only
when supplied by the server. This model-only smoke never qualifies the repository's strict
external-evidence gate; that separate gate also requires a real byte-pinned coding agent and is
documented in [`external-evidence.md`](external-evidence.md).

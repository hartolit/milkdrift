# Connect a separately managed model endpoint

Milkdrift treats a model server as an external capability. It does not load weights or manage
inference memory. Run and secure the server separately, or use the existing managed-resource owner
for a separately configured owned service. The [operator recipe](../../examples/operator/README.md)
registers the model adapter and creates ordinary daemon tasks.

In the browser, select a task and use **Model or agent** to assign an existing connection.
**Models & agents** lists the choices available to the signed-in account. Its **Add model or agent**
action opens setup guidance: the current browser does not install a connection or save an API key.
The following setup happens once on the daemon computer, so every permitted client sees the same
configured model. Closing or renaming a browser connection does not change that model configuration.

Copy the [endpoint template](../../examples/local-model/openai-compatible-loopback.example.json)
to a private operator directory. Set the exact endpoint, provider protocol, model alias, revision,
feature declarations and request/response bounds. An OpenAI-compatible chat endpoint uses protocol
`open_ai_compatible` with its exact configured path, commonly `v1/chat/completions`. The supported
Anthropic mapping is selected explicitly through its own profile. A server advertising an HTTP API
does not establish compatibility with the selected model or every feature.

## Declare enforceable limits

Ordinary task execution requires explicit enforceable external unit and monetary bounds. A profile
whose billing or token limits remain `unknown` cannot establish those task reservations. Declare
unbilled service only when the operator can establish that fact. Use a supported counting contract
only after inspecting the exact tokenizer, template, provider settings and output-cap behavior.
The template's placeholder/unknown values are not qualification evidence.

The current ordinary model task owner constructs a fresh nonstream request with a 4096 output-unit
allowance from the accepted instructions and named inputs. It selects no structured-output schema
or opaque provider session. Configure the profile to admit that request and enforce its complete
output cap, including any hidden reasoning that the provider counts. A client or socket timeout
cannot establish that remote generation stopped. Profile accounting and external provider facts
remain separate responsibilities.

Named text and captured UTF-8 files become bounded selected evidence. Individual text inputs are
limited to 16 KiB; the daemon also checks the combined instructions and selected inputs. Producer outputs must be
explicitly bound to future tasks; previous conversations are not discovered automatically. The
[continuation guide](model-continuation.md) explains using prior immutable results as fresh evidence.
The daemon freezes the selected bytes, capability generation and actor/grant facts before entry.

## Register the connection on the daemon

After completing and qualifying the endpoint profile, add its path to the existing daemon's
`adapters.model_profiles`. For example, this TOML entry selects one profile stored beside the
daemon configuration; replace the identity and filename for the actual connection:

```toml
[[adapters.model_profiles]]
capability_id = "operator-model"
profile = "model-profile.json"
```

If the configuration already has `model_profiles = []` under `[adapters]`, replace that empty
declaration with the table above instead of declaring the same key twice. Grant the intended
account this capability's `model.generate` operation and its actual network, secret, budget and
trust requirements through the [operator configuration](../../examples/operator/README.md#configure-the-host-and-account).
Capability operation scope determines which registered models appear in the account's choices.
Displayed availability does not prove that its other execution requirements are authorized;
network, secret and budget checks still apply before execution. Store credential values only in the configured private file or environment secret
source; the endpoint profile names the reference.

Check the edited daemon configuration, then restart the daemon with it using the
[daemon procedure](../operations/daemon.md). Existing work remains in its selected store; profile
changes use a new generation and do not rewrite frozen tasks. Reconnect the browser and open
**Models & agents**, or run `milkdrift --json workflow capabilities`, to inspect the permitted
choices. Select the connection on a task before submitting work. Registration and displayed local
readiness do not establish a successful remote call.

## Transport and unsupported features

Plain HTTP is allowed only for explicit loopback development with an allowlisted host. Remote
provider endpoints require HTTPS. Redirect and proxy behavior are configured rather than inferred
from ambient credentials. Profiles contain opaque secret references; resolve them through private
files or environment sources in daemon configuration. Never place a credential in the URL.

The adapter's direct invocation path exposes the provider-neutral model request. Tools, image parts,
developer roles, reasoning controls, structured output and streaming require explicit support by
the selected profile and mapping. Direct sessions must be fresh. Generic file parts and arbitrary
provider-managed sessions are not implied by the model request type. Consult the
[adapter contract](../../adapters/model-provider/README.md) for supported mappings and refusals.

Local preparation checks request encoding, selected content, feature support and bounds before
recording external entry. A known refusal leaves a failed task without HTTP entry. After preparation,
the runtime checks authority again and durably records entry intent before sending. A crash after
intent, interrupted request or incomplete response remains uncertain when no terminal evidence
establishes its outcome. No automatic retry follows possible provider entry.

The raw canonical response and final-text artifacts preserve provider output. Completion alone does
not prove a useful answer: inspect finish reason, content and independent verification for the task.
The [result guide](result-acceptance.md) explains required outputs and corrective tasks. Cancellation
is best effort and does not claim provider-side termination.

## Exercise the ordinary path

The standalone [web client](../../apps/web/README.md) and CLI both use `workflow create` with the
same configured `model.generate` capability. Inspect `workflow task` for frozen input evidence and
`workflow result` for the retained response. Large text is labelled as a preview; download immutable
artifacts to examine complete content.

The Rust `connected-tasks` fixture supplies a controlled input-dependent model endpoint for browser
verification. It proves the local integration path, not a real provider. The
[external evidence harness](external-evidence.md) requires an operator-supplied model profile and
real coding process for its combined qualification; explicit fixture mode remains non-qualifying.
Retired `local-model-evidence` and graph/controller recipes are not current operating procedures.

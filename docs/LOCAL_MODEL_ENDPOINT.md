# Connecting a local OpenAI-compatible model endpoint

Milkdrift treats a local model server as a configured capability endpoint. It does not load model
weights, select a model architecture, manage inference memory, or start/stop the server. Run and
secure the server separately.

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
is rejected before connection. For Fresh requests, it verifies the persisted causal manifest and
sends the exact canonical manifest as the first system context block; artifact references are not
silently dereferenced unless they are also explicit supported model-task content parts. Generic
file/artifact parts, explicit continuation artifacts, and provider-managed sessions currently have
no OpenAI-compatible mapping. Cancellation signals cause the response reader to close at its next
observable read boundary; the acknowledgement truthfully does not claim provider-side termination.

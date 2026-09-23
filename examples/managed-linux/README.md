# Slotbook Linux inputs

The approved image contains Git/Rust/C build tools and a version of the
[Slotbook brief](../../docs/guides/adaptive-method-example.md). After applying the recipe, run its
`initialize-slotbook` command through the ordinary worker capability to create source/output
directories and copy the brief into the owned working volume. The initializer preserves an existing
brief, including local edits. Reapply checks platform protection; it does not initialize application
content. Changing the guide requires rebuilding and approving a new image to change its initial content. Staging
and persistent data have separate owned identities reserved for scoped publication/deployment.
The worker receives only its working volume. It cannot mount those other volumes or administer
Podman/systemd. Publication and the protected effect gate remain separate work.

The zero image digest in `slotbook.json` deliberately cannot install. Replace it with the exact
local image ID produced by an explicitly selected build. The `Containerfile` accepts an exact
operator-reviewed Rust base image, verifies Git/Rust/C tools, and adds no floating package lookup.
For example, select a Rust 1.95 Debian-based image by its registry digest, pull it explicitly, then
build from the repository root:

```sh
podman build -f examples/managed-linux/Containerfile --ignorefile examples/managed-linux/Containerfile.containerignore --build-arg TOOLCHAIN_IMAGE=REGISTRY/IMAGE@sha256:DIGEST -t localhost/milkdrift-slotbook .
```

The build context includes only the Containerfile and application brief, as selected explicitly by
`--ignorefile examples/managed-linux/Containerfile.containerignore`. Record the resulting
`podman image inspect --format '{{.Id}}' IMAGE` value in the recipe. Applying a recipe never pulls
or resolves a tag. The accepted generation records actual image bytes, not a maintained alias.

A small Python edit can generate the approved recipe from the maintained defaults:

```sh
python3 -c 'import json,sys; p=json.load(open(sys.argv[1])); p["worker_image"]=sys.argv[2]; print(json.dumps(p,indent=2))' examples/managed-linux/slotbook.json sha256:YOUR_EXACT_IMAGE_ID > /private/path/slotbook.json
```

For an attachment, select the exact endpoint and its accounting contracts. For example, Drifty's
operator-supplied API base is `http://drifty.hartolit.internal:8080/v1` and model alias is `ornith`;
its declared provider billing is unbilled. The host needs NetBird connectivity. The desktop llama.cpp
endpoint uses `http://127.0.0.1:8080/v1` with model alias `ornith-9b`. Neither connection detail
establishes a tokenizer/template/output contract.

The attached object requires `type`, `api_base`, `model_alias`, `endpoint_limits`, `billing` and
`token_limits`. Use the `endpoint_limits` shape below for either service variant, choosing deadlines
and payload bounds for the selected endpoint.
Billing and token limits use the existing [provider accounting contracts](../../docs/guides/local-model-endpoint.md).
Supply an operator-supported finite token contract and explicit billing terms; unknown terms refuse
managed recipes because the independent host must bound prepared calls before entry. Do not copy
another model's tokenizer or template assumptions. For a self-hosted service with the stated billing,
`billing` can be `{"type":"unbilled","source":"Operator declaration: selected service has no provider charge"}`.

For the exercised Ornith llama.cpp instances, request `reasoning_effort: "none"` through the
existing OpenAI request extension on the model task. See the
[endpoint guide](../../docs/guides/local-model-endpoint.md#llamacpp-reasoning-stream-compatibility).
This is an explicit per-request choice; it does not change the server's defaults.

Worker networking can remain `none` because Milkdrift invokes the model through its scoped provider
adapter. Removal never stops or deletes an attachment. Existing standalone provider profiles remain
available outside the managed recipe under their own declared accounting limitations.

For an owned llama-server, replace `model_service` with these typed fields, using actual immutable
inputs. The operator chooses one `model_alias`; generation, readiness checks and provider requests use
that same value. Image identity, model path, digest and byte length retain provenance independently
of the alias. A different approved GGUF requires configuration changes only, subject to the exact
server image supporting that text model.

```json
{
  "type": "owned",
  "model_alias": "research/local-model",
  "limits": {
    "memory_bytes": 12884901888,
    "cpu_percent": 800,
    "pids": 256,
    "temporary_bytes": 268435456
  },
  "timeouts": {
    "startup_ms": 180000,
    "shutdown_ms": 10000,
    "model_verification_ms": 300000
  },
  "endpoint_limits": {
    "connect_timeout_ms": 5000,
    "request_timeout_ms": 180000,
    "idle_timeout_ms": 180000,
    "max_headers": 64,
    "max_header_bytes": 16384,
    "max_request_bytes": 1048576,
    "max_response_bytes": 1048576,
    "max_stream_line_bytes": 65536,
    "max_stream_event_bytes": 131072,
    "max_fragment_bytes": 4096
  },
  "image": "REGISTRY/LLAMA_IMAGE@sha256:EXACT_DIGEST",
  "executable": "/app/llama-server",
  "model": "/private/models/selected.gguf",
  "model_digest": "b3_EXACT_BLAKE3_HEX",
  "model_bytes": 123456789,
  "port": 18080,
  "context_tokens": 32768,
  "token_limits": {
    "type": "byte_bpe",
    "template_tokens_per_message": 64,
    "template_tokens_per_request": 64,
    "maximum_input_tokens": 16384,
    "maximum_output_tokens": 4096,
    "output_control": "max_tokens",
    "source": "REPLACE with evidence for this exact tokenizer, template and total output enforcement"
  },
  "threads": 8,
  "backend": { "type": "cpu" }
}
```

The token numbers above illustrate the typed shape; they are not qualification of Ornith, Gemma
or another model. Before approval, establish non-expanding byte-BPE tokenization, complete hidden
and template overhead, and total output enforcement (including reasoning and disconnect behavior)
for the selected implementation. Managed owned servers are declared unbilled. Requests exceeding
the exact configured accounting ceiling refuse before external entry.

Calculate the model byte length and BLAKE3 digest from the same regular file. Keep approved images,
model files and manager configuration under the trusted operator's control. Model files are shared,
read-only mounts and are never deleted by Milkdrift. Replacing their bytes requires a new approved
recipe; stop the owned service first. Explicit Vulkan uses
`{"type":"vulkan","render_device":"/dev/dri/renderD128","gpu_layers":32}` and needs a matching server image,
permitted render device, and real backend/offload verification. CPU results do not qualify Vulkan.
Choose `gpu_layers` for the actual model/device; 32 is an example, not a qualification. Worker
and service memory caps are independent. Both draw from host RAM; an iGPU reservation is not extra
RAM. `worker_limits` in the recipe and owned `limits` above can differ. Each `/tmp` consumes memory
inside its own container cap. The example values are operating choices, not minimum model requirements.

[Managed Linux operations](../../docs/operations/managed-linux.md) covers bootstrap, permissions,
recovery and the physical qualification lane. This recipe establishes a working boundary; it does
not make mutable tools or Slotbook output a protected published candidate.

To initialize Slotbook, use this ordinary `command` input with the invocation commands in
[managed operations](../../docs/operations/managed-linux.md#apply-inspect-and-use):

```json
[{"name":"command","value":{"type":"inline","value":{"argv":["/usr/local/bin/initialize-slotbook"]}}}]
```

For another workload, choose a different exact image and recipe name and run its ordinary commands.
The platform requires a POSIX shell and the basic Linux tools used by its protection probe (`cat`,
`grep`, `mkdir`, `touch`, `test`); BusyBox suffices. It imposes no Git, Rust, C, Slotbook file, or
application initializer requirement. The image owns tool environment settings. See
[operating choices](../../docs/operations/managed-linux.md#choose-operating-budgets) for limits,
deadlines, admission ceilings and upgrading earlier recipe formats.

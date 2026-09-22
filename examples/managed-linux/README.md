# Slotbook Linux inputs

This recipe prepares Git/Rust/C build tools, source, useful outputs and the
[Slotbook brief](../../docs/guides/adaptive-method-example.md) in an owned working volume. Staging
and persistent data have separate owned identities reserved for scoped publication/deployment.
The worker receives only its working volume. It cannot mount those other volumes or administer
Podman/systemd. Publication and the protected effect gate remain separate work.

The zero image digest in `slotbook.json` deliberately cannot install. Replace it with the exact
local image ID produced by an explicitly selected build. The `Containerfile` accepts an exact
operator-reviewed Rust base image, verifies Git/Rust/C tools, and adds no floating package lookup.
For example, select a Rust 1.95 Debian-based image by its registry digest, pull it explicitly, then
build this file with `--build-arg TOOLCHAIN_IMAGE=REGISTRY/IMAGE@sha256:DIGEST`. Record the resulting
`podman image inspect --format '{{.Id}}' IMAGE` value in the recipe. Applying a recipe never pulls
or resolves a tag. The accepted generation records actual image bytes, not a maintained alias.

A small Python edit can generate the approved recipe from the maintained defaults:

```sh
python3 -c 'import json,sys; p=json.load(open(sys.argv[1])); p["worker_image"]=sys.argv[2]; print(json.dumps(p,indent=2))' examples/managed-linux/slotbook.json sha256:YOUR_EXACT_IMAGE_ID > /private/path/slotbook.json
```

For an attachment, select the exact endpoint and its accounting contracts. For example, Drifty's
operator-supplied API base is `http://drifty.hartolit.internal:8080/v1` and model alias is `ornith`;
its declared provider billing is unbilled. The host needs NetBird connectivity. Local LM Studio
uses `http://127.0.0.1:1234/v1` with the exact loaded model identifier. Neither connection detail
establishes a tokenizer/template/output contract.

The attached object requires `type`, `api_base`, `model_alias`, `billing` and `token_limits`.
Billing and token limits use the existing [provider accounting contracts](../../docs/guides/local-model-endpoint.md).
Supply an operator-supported finite token contract and explicit billing terms; unknown terms refuse
managed recipes because the independent host must bound prepared calls before entry. Do not copy
another model's tokenizer or template assumptions. For a self-hosted service with the stated billing,
`billing` can be `{"type":"unbilled","source":"Operator declaration: selected service has no provider charge"}`.

Worker networking can remain `none` because Milkdrift invokes the model through its scoped provider
adapter. Removal never stops or deletes an attachment. Existing standalone provider profiles remain
available outside the managed recipe under their own declared accounting limitations.

For an owned llama-server, replace `model_service` with these typed fields, using actual immutable
inputs. The alias exposed by the generated server is `ornith`; the selected GGUF may be any approved
text model supported by that exact server image.

```json
{
  "type": "owned",
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
`{"type":"vulkan","render_device":"/dev/dri/renderD128"}` and needs a matching server image,
permitted render device, and real backend/offload verification. CPU results do not qualify Vulkan.
Memory is one host budget; an iGPU reservation is not extra RAM.

[Managed Linux operations](../../docs/operations/managed-linux.md) covers bootstrap, permissions,
recovery and the physical qualification lane. This recipe establishes a working boundary; it does
not make mutable tools or Slotbook output a protected published candidate.

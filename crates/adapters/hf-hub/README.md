# hf-hub-adapter

Blocking Hugging Face Hub resolution is isolated behind a dedicated cold-path host.
The adapter accepts only `tokenizer.json`, `config.json`, and unquantized Llama
Safetensors layouts understood by the current Candle backend. Repository inspection
resolves mutable references to an immutable commit before any required artifact is
downloaded. Numbered shard layouts must be complete, consistent, and contain no more
than 256 selected shards.

## Bounded repository discovery

The requested revision is resolved through `hf-hub`'s typed blocking
`get_file_metadata` call for `config.json`. Only its documented `commit_hash` is used
for revision identity; ETag, Xet hash, location, and file size are not commit proof.
The adapter requires the returned commit to be exactly 40 lowercase hexadecimal
characters.

The adapter then calls blocking `list_tree` recursively at that immutable commit with
a hard 4,097-entry request limit. The 4,097th sentinel entry is rejected, so at most
4,096 repository entries are accepted. File and directory entries both count toward
the entry, 1,024-byte per-path, and 4 MiB aggregate path-byte limits, with checked
counter arithmetic. Only typed file paths enter the available-file set, and duplicate
file paths are rejected.

## Scalar declarations

Configuration declaration parsing is strict. Absent or null `dtype` and
`torch_dtype` fields mean no declaration. One recognized field is retained, and two
recognized fields must agree. A present unsupported string, conflicting recognized
values, a wrong field type, or malformed JSON fails resolution without exposing raw
vendor values. The declaration remains producer-intent metadata; it is not evidence
that Safetensors tensors are homogeneous.

Configuration reads are limited to 1 MiB. Safetensors index reads are limited to
32 MiB, 65,536 weight-map entries, 1,024 bytes per tensor name, and 1,024 bytes per
repository-relative artifact path. A custom deserializer enforces map, name, path,
and 256-shard limits while traversing `weight_map`; it retains only the deduplicated
shard names rather than materializing ignored index metadata or every tensor name.
These limits provide substantial headroom for the supported Llama layout while
bounding hostile metadata growth.

## Shard content identity

For every selected shard, the adapter requests exact path metadata at the resolved
commit through `hf-hub`'s blocking `get_paths_info` API. Only
`BlobLfsInfo::sha256`, combined with the exact `RepoTreeEntry::File::size`, is
accepted with `ArtifactContentIdentityAuthority::HuggingFaceLfs`. When the optional
redundant LFS size is present, it must agree too. The adapter validates exact metadata
cardinality, 64-character hexadecimal SHA-256 encoding, and Hub/LFS/local length
agreement. It does not treat a Git object ID, ETag, Xet hash, cache filename,
symlink, inode, or mtime as digest proof.

When complete LFS identity is unavailable, the adapter streams the downloaded local
file through a bounded fixed-size buffer, computes its whole-file SHA-256 with
checked length accounting, and marks the result
`ArtifactContentIdentityAuthority::ProjectEstablished`. This is an honest local
fallback baseline, not provider verification. The resulting identity is retained in
`ResolvedSafetensorsShard`, but because the cache path is not proven immutable,
Candle rehashes its retained file against this baseline before device admission and
again verifies the complete materialization stream before publishing a model.

The public result is deliberately named `ResolvedSafetensorsLlamaArtifacts`; it is
not a generic model bundle. Future model-format or artifact-source work requires its
own reviewed contract rather than overloading this result.

Environment-derived cache and authentication are preserved unless explicit
overrides are supplied. The upstream synchronous builder does not expose a global
request timeout, so callers must not run this adapter on an event-loop or inference
thread. Access tokens are redacted from adapter and application configuration
`Debug` output. E1 runs this synchronous adapter on one bounded Hub worker, separate
from its sole Candle inference worker, and applies a bounded shutdown wait.

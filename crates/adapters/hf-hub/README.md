# hf-hub-adapter

Blocking Hugging Face Hub resolution is isolated behind a synchronous cold-path
adapter. It accepts only `tokenizer.json`, `config.json`, and unquantized Llama
Safetensors layouts understood by the current Candle composition. Mutable
repository references are resolved to an immutable commit before required
artifacts are downloaded.

The public result is deliberately named
`ResolvedSafetensorsLlamaArtifacts`; it is not a generic engine/source/format
bundle. A future artifact source or model format needs its own reviewed contract.

## Bounded repository discovery

The requested revision is resolved through `hf-hub`'s typed blocking
`get_file_metadata` call for `config.json`. Only its documented `commit_hash` is
accepted as revision identity; ETag, Xet hash, location, and file size are not
commit proof. The commit must be exactly 40 lowercase hexadecimal characters.

The adapter recursively calls `list_tree` at that immutable commit with a
4,097-entry request limit. The sentinel entry is rejected, so at most 4,096 file
and directory entries are accepted. Discovery also enforces checked counters,
1,024 bytes per path, 4 MiB aggregate path bytes, typed-file-only selection, and
no duplicate file paths.

Supported weights are either `model.safetensors`, a complete standard numbered
layout, or a complete layout selected by `model.safetensors.index.json`. At most
256 selected shards are accepted.

## Strict scalar declarations

`dtype` and legacy `torch_dtype` are parsed into three internal states: absent,
recognized, or invalid for load.

- A missing field or explicit JSON `null` is absent.
- One recognized value, or two recognized values that agree, resolves to
  `Some(F32|F16|BF16)`.
- Both fields absent resolves to `None`.
- Any present unsupported string fails, even if the other field is recognized.
- A wrong field type, duplicate declaration field, malformed JSON, or non-object
  top level fails as malformed configuration.
- Two recognized values that disagree fail as conflicting declarations.

Only absent and recognized declarations continue beyond resolution. The stable
adapter classifications are respectively
`HubErrorKind::MalformedConfiguration`,
`HubErrorKind::UnsupportedScalarDeclaration`, and
`HubErrorKind::ConflictingScalarDeclarations`. The optional successful value is
producer-intent metadata, not tensor-homogeneity or execution-scalar evidence.

Declaration errors never include the raw unsupported value or private vendor
detail. The reference application mapping is likewise stable:

- malformed -> `ApplicationFailureKind::MalformedArtifactConfiguration`;
- unsupported -> `ApplicationFailureKind::UnsupportedArtifactDeclaration`;
- conflicting -> `ApplicationFailureKind::ConflictingArtifactDeclaration`.

Configuration reads are limited to 1 MiB. Safetensors index reads are limited to
32 MiB, 65,536 weight-map entries, 1,024 bytes per tensor name, and 1,024 bytes per
repository-relative artifact path. The bounded deserializer retains only
deduplicated selected shard names rather than materializing ignored index metadata
or a permanent tensor-name inventory.

## Shard content identity

For every selected shard, the adapter requests exact path metadata at the resolved
commit through `get_paths_info`. It accepts `BlobLfsInfo::sha256` with the exact
repository file size as `ArtifactContentIdentityAuthority::HuggingFaceLfs` and
checks metadata cardinality, SHA-256 encoding, and Hub/LFS/local length agreement.
For a non-LFS file, the adapter decodes the exact Git blob object ID reported at
the resolved commit, streams the downloaded bytes through a bounded buffer,
verifies Git's `blob <length>\0<content>` SHA-1, derives a whole-file SHA-256, and
records `ArtifactContentIdentityAuthority::HuggingFaceGitBlob`.

ETags, Xet hashes, cache filenames, symlinks, inodes, mtimes, and unverified object
identifiers are not digest proof. `ProjectEstablished` remains available for exact
identities computed by project code without provider binding. Provider authority is
retained as evidence by the Hub/application layer; Candle receives only the exact
expected length and SHA-256 and verifies the complete materialization stream before
model publication.

## Hosting boundary

Environment-derived cache and authentication remain active unless explicit
overrides are supplied. Access tokens are redacted from adapter and reference
application configuration `Debug` output.

The upstream synchronous client has no global request timeout. Callers must host
this adapter away from event-loop and inference threads and must define their own
bounded worker/shutdown policy. The optional `application-runtime` reference kit
does so with one dedicated Hub worker; that composition is not a requirement for
other Milkdrift API or workflow planes.

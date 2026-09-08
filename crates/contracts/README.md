# milkdrift-contracts

Document readers and writers in Milkdrift repeat a few operations: reject duplicate JSON keys,
limit nested values, produce consistently ordered bytes, and run domain validators when loading
typed values. This library supplies those mechanics. The calling package owns its document
shape, versions, numeric limits, identities, and errors. Application code should normally use
that package's document reader or constructor rather than assemble these checks itself.

## Follow a document through the checks

The [model document reader](../model/src/document.rs) provides a complete production trace.
`ModelTaskRequestDocument::from_json` calls the private `read` function, which:

1. Checks the document's total input-byte limit before allocating a JSON tree.
2. Calls `preflight_json_structure` with the model owner's `JsonLimits` to reject excessive
   nesting, encoded strings/keys, or container entries before parsing.
3. Calls `parse_json_without_duplicates` to parse one value and reject repeated decoded keys,
   including nested duplicates and equivalent escape spellings.
4. Calls `validate_json_value` to check decoded UTF-8 lengths, every value's depth, and each
   container's size. Preflight success alone does not establish these checks.
5. Checks the envelope version, then deserializes the typed document. The wire types and
   constructors enforce allowed fields and model-request meaning.

The [model contract suite](../model/tests/contracts.rs) demonstrates exact canonical bytes and
round-trip equality for a request, plus duplicate-key, excessive-depth, and unsupported-version
refusals. Its [package example](../model/README.md#construct-a-request) shows construction without
an endpoint. Passing these checks does not establish endpoint support or execute a request.

On writing, `canonical_json_bytes` converts a serializable value to a JSON tree, validates it,
sorts object keys recursively, and emits compact JSON. Arrays keep their order. The model
owner's `encode` function then checks total output bytes and maps failures to `ModelContractError`.
Canonical encoding supplies neither a schema envelope nor a digest; those remain domain choices.

The [crate introduction](src/lib.rs) has an executable example of the shared read/write sequence.
With its illustrative limits, `{"z":[2,1],"a":"ok"}` becomes `{"a":"ok","z":[2,1]}` and
`{"a":1,"a":2}` is refused. Use the owning document's limits in production, not the example's.

## Interpret a refusal

`JsonLimits` has four explicit, inclusive maxima and no default. Zero imposes a limit; it never
means unlimited. String/key limits count UTF-8 bytes, and container limits apply to each array
or object separately. Callers must impose total input and output byte limits themselves.

Preflight counts encoded escape syntax, so `"\u0061"` counts as six string bytes even though it
decodes to the one-byte string `"a"`. It checks container-opening depth, while decoded validation
also counts scalar children: at depth zero, `[]` passes both checks, but `[0]` fails decoded
validation. The preflight rustdoc example demonstrates why parsing and validation still follow.

`JsonBoundViolation` reports the configured maximum and the first detected category, not the
offending content or measured size. Preflight reports location `$`. Decoded validation supplies
a diagnostic path such as `$.names[0]`; a long key points to its containing object. These paths
do not escape key punctuation and are not machine-addressable JSON Pointers. The capability
owner's [`contract_bound`](../capability/src/bounded.rs) shows how a caller maps this information
to its own errors. `CanonicalJsonError` separates a structural refusal from a Serde encoding error.

## Reuse a constructor or text check

- `validated_string_type!` generates an owned string wrapper whose `new` constructor and Serde
  reader call the same supplied validator. The [capability identity owner](../capability/src/identity.rs)
  supplies its character, byte-length, and namespace rules and adds its own `TryFrom` conversion.
  The macro preserves accepted text exactly and exposes it through Display, Debug, and serialization.
- `deserialize_via!` decodes a wire type, then invokes a conversion such as the public constructor.
  [`SchemaContract`](../capability/src/descriptor.rs) uses a private wire type to reject unknown
  fields and calls `new` to refuse version zero. The macro supplies neither of those rules itself.
  Both macros require a dependency named `serde` in the invoking package; their rustdoc includes
  executable construction and refusal examples.
- `is_canonical_blake3_digest` checks only `b3_` plus 64 lowercase hexadecimal digits. The
  [blueprint digest reader](../blueprint/src/identity.rs) uses it to reject malformed text.
  Verifying a digest against content requires the owner's separate hash calculation and comparison.
- `truncate_utf8` returns a borrowed prefix within a byte limit, ending at a UTF-8 character
  boundary. The [host's adapter failure constructor](../capability-host/src/adapter.rs) uses it to
  shorten a diagnostic summary. The helper supplies no ellipsis, redaction, or error classification.

## API detail and verification

This package depends only on Serde and Serde JSON, has no feature flags, and needs no service setup.
Generate API documentation with `cargo doc -p milkdrift-contracts --no-deps --open`. The
[architecture](../../docs/architecture.md#owners-and-dependency-direction) explains domain ownership.
The unit tests in [lib.rs](src/lib.rs) and [text.rs](src/text.rs) exercise the shared operations;
the consumer suites retain domain validation and golden-document evidence.

From the repository root:

```sh
cargo test -p milkdrift-contracts --all-features
cargo test -p milkdrift-model --test contracts --all-features
cargo test -p milkdrift-capability --test contracts --all-features
```

Use the [verification policy](../../docs/development/workflow.md#choose-verification-for-the-change)
for formatting, warning-denying rustdoc, documentation contracts, and other checks required by a change.

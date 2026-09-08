# milkdrift-contracts

Saving a model request or workflow definition should preserve one unambiguous meaning. Duplicate
JSON keys can hide a value from a reader, excessive nesting can exhaust a parser, and inconsistent
object ordering can give equivalent documents different bytes. This package supplies the checks
and canonical encoding that Milkdrift's document owners share.

Use the owning package's reader when loading a document. For example,
[`ModelTaskRequestDocument::from_json`](../model/src/document.rs) combines these mechanics with
the model schema and constructors. Change this package when the shared mechanics need to change;
keep document versions, limits, field rules, and error meanings with their owners.

## How a document reader uses it

The model reader bounds the input bytes, scans for excessive structure before allocating a JSON
tree, parses with duplicate-key rejection, and checks the decoded values. It then checks the
version and constructs the typed request. Each check protects a different part of reading:
preflight limits allocation, parsing establishes unambiguous JSON, and the owner's constructors
establish what that JSON means.

Preflight and decoded validation intentionally count different representations. For example,
`"\u0061"` has six encoded bytes inside the quotes but decodes to one byte. The exact counting
rules belong to [`JsonLimits`](src/lib.rs), and the [crate example](src/lib.rs) shows how to compose
the helpers. `JsonBoundViolation` provides a category, diagnostic location, and configured maximum
for the owner to translate into its own refusal.

On writing, `canonical_json_bytes` sorts object keys recursively and emits compact JSON, keeping
array order. The owner adds its envelope, checks total output size, and computes any digest. This
allows a reader to compare saved bytes without making every domain share a schema or hash format.

## Keeping construction and loading consistent

`validated_string_type!` gives a string identity the same validation through its constructor and
Serde reader. The [capability identities](../capability/src/identity.rs) supply their own character
and length rules. `deserialize_via!` does the equivalent for a structured type: the
[`SchemaContract` reader](../capability/src/descriptor.rs) decodes a private wire shape and calls
its constructor, so version zero is refused whether created in Rust or loaded from JSON. Macro
rustdoc contains executable examples and the requirements for invoking them.

The two text helpers are similarly narrow. `is_canonical_blake3_digest` checks `b3_` plus lowercase
hexadecimal spelling; the caller verifies content. `truncate_utf8` returns a borrowed prefix that
fits a byte allowance; the caller chooses redaction and presentation.

This package has no feature flags or service setup. Its unit tests cover shared mechanics; the
[model contract tests](../model/tests/contracts.rs) demonstrate their composition with a real
document owner. Follow the [verification policy](../../docs/development/workflow.md#choose-verification-for-the-change)
when changing either layer.

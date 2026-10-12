# milkdrift-control-protocol

Use these transport contracts when implementing a client of the daemon's authenticated control
API. Rust DTOs describe task requests, receipts, compact discovery, selected task/result reads,
independent invocations, managed resources and peer administration. They do not schedule work or
validate a second copy of workflow semantics. [Control API](../../docs/reference/control-api.md)
explains the exact routes and recovery behavior.

`decode_json` checks byte/depth/count bounds and duplicate keys before strict deserialization.
`encode_json` produces bounded canonical JSON. Exact protocol negotiation refuses mismatched
versions; clients and daemon upgrade together. Public error codes own status mapping, while
bounded response validation and authenticated cursors protect framing and continuation scope.

Ordinary task create requests contain client-local aliases. Acceptance returns daemon IDs in one
receipt. Generation/version-guarded edits affect only future work; replay binds exact canonical
requests before defaults or upload resolution. Summary DTOs keep captured input/result bodies out
of discovery pages. Result text and frozen input previews declare truncation explicitly.

`tasks.rs` also owns Rust-generated browser declarations. Regenerate with:

```sh
cargo run -p milkdrift-control-protocol --example export_web --features web-codegen
cargo test -p milkdrift-control-protocol --all-features
```

The generated file contains transport types/codecs, not a browser workflow engine. Golden fixtures
and reader tests detect wire drift. Historical graph/run/proposal/layout/controller/publication
commands have no compatibility endpoint. Independent artifact, capability and managed-resource
contracts remain explicit scoped operations.

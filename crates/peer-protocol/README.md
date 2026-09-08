# Peer execution contracts

A Milkdrift peer performs an operation for another host while each host keeps its own durable
records. The origin owns workflow history; the serving host owns remote acceptance and observations.
This crate defines the messages connecting those owners. Use it to understand peer semantics or
validate messages without depending on HTTP, async execution, or a database. The
[peer HTTP adapter](../../adapters/peer-http/README.md) implements the transport and service.

## Follow work between two hosts

```text
origin                                      serving host
  handshake/version offer ----------------> authenticated relationship
  <---------------------------------------- session facts and limits
  catalog query --------------------------> filtered live capability host
  <---------------------------------------- expiring catalog snapshot
  exact PeerInvocationRequest ------------> durable acceptance
  <---------------------------------------- stable PeerExecutionId
  observations after sequence N ----------> durable execution record
  <---------------------------------------- contiguous page or archived summary
```

The handshake's claimed peer is a cross-check against transport authentication, not a way to choose
an identity. A session identifies a daemon boot; it is not the lifetime of accepted work.
`ProtocolVersionRange` offers version selection and `HardLimits::intersect` computes lower ceilings.
Current codecs and the HTTP implementation accept only v1.2. The HTTP service reports its supported
feature flags and disables incremental catalogs; `CatalogUpdate` defines a message shape without
making that transport path available.

`CatalogSnapshot` is an expiring observation of exact descriptors and invocable operations, scoped
to the relationship. Its generation/digest bind a selection to what the caller saw. The serving
host still checks current authority, catalog, and admission when accepting an invocation. Changes
to health or catalog availability do not rewrite an already accepted execution.

## Recover a missing reply

`PeerInvocationRequest::new` binds the request identity, exact selection, catalog, inputs, deadline,
limits, and delegated facts into one canonical digest. Resubmit those same facts when an acceptance
reply is lost. Reusing the key with a different deadline or catalog is a different request and must
conflict. A delegation reference narrows a configured relationship; it is not a bearer credential.

`InvocationAcceptance::Accepted` confirms durable acceptance, not adapter entry or success.
`InvocationLookup` distinguishes no accepted record, a known execution, and unavailable outcome
evidence. Validate responses against the request with `validate_for`; structurally valid JSON from
the right peer can still name the wrong request. The HTTP client performs these checks for callers.

`ObservationPage` uses an exclusive sequence cursor. Transport keepalives are not observations.
`closed` means no later semantic event can arrive, while `terminal` identifies terminal evidence;
they are separate because an outcome can remain unknown. If detailed rows have been compacted,
`ObservationHistory::Archived` carries a summary rather than pretending an empty page is complete
history. Exact replay still works after archival; it does not execute the capability again.

Cancellation is a separate request bound to an execution and sequence. Its disposition and
`terminal_boundary` describe what is confirmed, so neither a disconnected socket nor an accepted
stop request alone proves that external work ended.

## Transfer artifacts and decode messages

`ArtifactMetadataOffer` negotiates digest, size, provenance, sensitivity, retention, direction, and
expiry before any bytes. It carries no host placement path. `ArtifactTransferDecision` either
confirms existing content, gives an exact resume offset/chunk allowance, or refuses the transfer.
The core artifact store verifies and publishes content; peer metadata cannot bypass that boundary.

Wrap control messages in `ProtocolEnvelope`, use `encode_envelope` for canonical bytes, and pass
negotiated `DecodeLimits` to `decode_envelope`. The codec bounds structure and rejects duplicates,
unknown versions, and unknown typed fields. Payloads with public fields also expose `validate` or
`validate_for` for semantic and request-specific checks; decoding alone is not authorization.

The [protocol tests](tests/protocol.rs) exercise digest, request-binding, paging, cancellation,
archival, and decoder refusal rules. The [wire reference](../../docs/reference/peer-protocol.md)
owns the detailed protocol contract; [peer operations](../../docs/operations/peers.md) owns setup.

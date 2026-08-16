# application-runtime

`application-runtime` is the optional frontend-neutral E1 reference services kit.
It composes the current local-model application behavior but is not Milkdrift's
general workflow plane or sole public API.

## Responsibility

E1 owns:

- repository/revision selection and explicit CPU/CUDA device choice;
- immutable Hub resolution and tokenizer validation;
- one private hosted Candle/E0 worker and one resident-model lifecycle;
- direct completion, exact compatible chat, conversation/context planning,
  request-local decode, and bounded text output;
- redb settings/model-catalogue persistence;
- correlated unload, retained model cleanup, application events/state; and
- explicit bounded shutdown of Hub and inference workers.

It does not own tensor compatibility, Safetensors materialization, backend
sequences, token scheduling, workflow state, vendor storage/network
implementation, provider/peer transport, or frontend presentation.

## Public boundary

Hosts construct `ApplicationRuntimeConfiguration::new(database_path)` and start an
`ApplicationRuntime`. Stable coarse operations select/resolve/load a model, start
completion or submit compatible chat, cancel, pull events/output, unload, retry
coordination cleanup, and shut down.

`ResolvedModel` exposes selection, immutable identity, vocabulary, optional
configuration declaration, and unit chat compatibility. `LoadedModel` exposes
generation-safe identity, limits/mode, and actual execution scalar/device verified
from E0's receipt. Neither leaks Candle, Hub artifact helpers, redb, host channels,
or prompt-profile internals.

## Correlated ownership

Load retains one ticketed `ModelLoadTransaction`: E1 snapshots resolution and
selected-device admission, E0 performs the exact prepared transaction, and E1
validates only application-level receipt correlation before publishing. Candle
alone owns required-tensor and conversion policy.

Unresolved native ownership appears as one `ApplicationRetainedModel` with exact,
unverified, or unknown ownership; a cleanup disposition; and separate primary and
cleanup failures. Normal `LoadedModel` and retained model state never coexist.
Disconnect, missing worker handles, missing snapshot entries, or zero exact bytes
are not release evidence.

One private cleanup coordinator handles failed load, incompatible model, normal
unload, unconfirmed disconnect, and terminal shutdown. Lower cleanup attempts and
bounded E1 coordination attempts remain distinct.

## Persistence and host boundary

Settings write `LAS1` v2/read v1. Model catalogue records write `LAM1` v3 and read
exact v1/v2 without automatic rewrite. Runtime ownership/execution facts are not
persisted.

The Slint host consumes only E1 state, events, and bounded text pulls. Other native
or workflow hosts may use E1 when these semantics fit or choose a lower reviewed
boundary when they do not.

See [application runtime](../../../docs/project/application-runtime.md) for the
full application contract, [operation](../../../docs/project/operation.md) for the
end-to-end flow, and [implementation status](../../../docs/project/implementation-status.md)
for the sole support/evidence matrix.

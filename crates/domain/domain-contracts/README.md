# domain-contracts

Portable, allocation-neutral contracts shared by inference engines and concrete model backends.

## Guarantees encoded by the API

- Loaded models are referenced by stale-handle-resistant `(ModelId, ModelGeneration)` handles.
- Drain-based unload requires a non-zero hard timeout and escalates to forced cancellation.
- Checked prefill and decode helpers validate token and logits capacity before backend entry.
- Capacity exhaustion becomes `FinishReason::BufferExhausted` rather than panic-based control flow.
- Generation helpers are generic only over the concrete `LoadedModel`; its sequence is an associated type rather than an independent specialization axis.
- UI output is modeled as a bounded, pull-oriented batch; output saturation yields generation.
- The crate is always `no_std` and contains no mandatory heap allocation or third-party dependency.

The lifecycle state machine escalates timed-out drains to cancellation. Actual hard reclamation still requires bounded backend calls or process isolation; see the [lifecycle guide](../../../docs/project/lifecycle.md).

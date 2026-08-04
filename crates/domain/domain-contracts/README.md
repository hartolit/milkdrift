# domain-contracts

Portable, allocation-neutral contracts shared by inference engines and concrete model backends.

## Guarantees encoded by the API

- Loaded models are referenced by stale-handle-resistant `(ModelId, ModelGeneration)` handles.
- Drain-based unload requires a non-zero hard timeout and escalates to forced cancellation.
- Loaded models expose the complete descriptor with source scalar metadata, actual execution `ScalarType`, actual backend-visible `ExecutionDevice`, and accepted accounted footprint so E0 can verify identity, metadata, capabilities, limits, selected device, execution representation, and accounting together before publication. Accounted footprints are admission quantities rather than observations of physical memory use or availability.
- Checked prefill and decode helpers validate token and logits capacity before backend entry; the owning runtime verifies backend-reported state transitions and exact output afterward.
- Capacity exhaustion becomes `FinishReason::BufferExhausted` rather than panic-based control flow.
- Generation helpers are generic only over the concrete `LoadedModel`; its sequence is an associated type rather than an independent specialization axis.
- UI output is modeled as a bounded, pull-oriented batch; output saturation yields generation.
- Device identity remains compact domain vocabulary: `DeviceId` is interpreted with `DeviceKind`, and a CUDA ordinal is not claimed as a globally permanent hardware identity.
- The crate is always `no_std` and contains no mandatory heap allocation or third-party dependency.

The lifecycle state machine escalates timed-out drains to cancellation. Actual hard reclamation still requires bounded backend calls or process isolation; see the [lifecycle guide](../../../docs/project/lifecycle.md).

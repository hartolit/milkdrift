# ADR 0001: Rebirth as a durable workflow system

- Status: accepted
- Date: 2026-08-18

## Context

The repository's inference era ended at commit `9e207aa51232d9e0b3bd3b3852ad42f15d2d8d80`, preserved by branch `old-branch` and annotated tag `milkdrift-inference-era-final`. `main` was reset by the ordinary commit `cc343fd57de864a9dccf3a06e0dcca2aa7c99cbc` to the two license files before this rebirth pass. These references preserve history without merging old implementation assumptions into the new product.

Manual prompt transfer, fresh-agent churn, opaque chronological context, and ungoverned long-running automation are workflow problems. Owning model inference would divide attention across tensor formats, hardware kernels, sampling, and device-specific reliability already handled by mature engines.

## Decision

Milkdrift is a local-first, durable, live-editable workflow system. Immutable blueprint revisions describe future work; later append-only run events will record what happened. Providers, tools, processes, humans, and peers participate through honest capability contracts. Local inference engines remain external, user-provided capabilities. The semantic core remains independent of provider SDKs, operating-system effects, network stacks, databases, Tokio, HTTP, and user interfaces.

## Rejected scope

Milkdrift will not load tensors, implement model architectures, tokenize, sample, manage KV caches, parse model-weight internals, or own GPU and accelerator kernels. It will not flatten provider differences into fictional guarantees, implement connectivity infrastructure such as a VPN, permit hidden mutable definitions, or model arbitrary graph cycles as execution.

## Consequences

The project gains a stable workflow boundary and can adopt better inference engines without rewrites. Adapters must advertise variance honestly. Revisions, commands, events, effects, provenance, authority, and reconciliation require explicit designs. The first implementation is deliberately a small pure kernel, leaving runtime and I/O ownership to later passes.

## Reconsideration triggers

Revisit the boundary only if a required workflow invariant cannot be enforced across an external capability contract, or if an independently demonstrated product requirement needs inference internals rather than observations and controls. Provider inconvenience, performance speculation, or a desire for an all-in-one demo is not sufficient.

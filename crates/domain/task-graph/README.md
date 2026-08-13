# task-graph

`task-graph` is Milkdrift's generic portable directed-work primitive. It is
`no_std`, performs no allocation, and uses caller-owned state and validation
scratch.

The crate owns only graph-wide mechanics:

- stable task identities and caller-owned opaque operation metadata;
- dependency integrity, duplicate detection, and acyclicity;
- deterministic ready-node discovery in definition order;
- attempt identities, stale-attempt rejection, retry/exhaustion, cancellation,
  and blocked-descendant propagation;
- identity-only external-input, producer, consumer, and direct-dependency
  provenance checks.

Tasks may produce zero, one, or many artifacts. Artifact media, semantic roles,
payloads, byte/token limits, model/backend policy, and operation behavior are not
graph concepts. A higher capability or workflow schema owns those meanings.

The `TaskNode<Operation>` parameter is caller-owned metadata. Graph algorithms do
not inspect it. `ArtifactFlow` similarly carries only `ArtifactId` relationships;
it does not impose a universal artifact type system.

The allocation contract test measures graph validation, artifact-flow validation,
ready selection, attempt start, and successful completion after all scratch and
state storage is prepared.

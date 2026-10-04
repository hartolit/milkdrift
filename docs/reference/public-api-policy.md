# Public API policy

Milkdrift is unreleased. Its daemon, clients, peers and workspace libraries are upgraded together.
Rust APIs, wire protocols and durable development formats may change in one atomic workspace
revision. Previous protocol generations do not require compatibility readers, negotiation fallbacks
or migration paths merely because they existed during development. Update current producers,
consumers, fixtures and documentation together, and refuse unsupported versions explicitly.

Current formats still have exact versions, bounded readers, canonical encodings and refusal tests.
Restart durability, immutable history, and exact replay/conflict behavior apply within those
supported formats. An incompatible upgrade must not reinterpret saved facts or silently give old
requests new authority. Any future promise to support multiple versions needs an explicit supported
contract and corresponding evidence.

## Classification

Every exported item must have at least one of these owners:

1. **External product contract** — intentionally usable by an application or adapter outside its
   defining package.
2. **Workspace adapter contract** — public because a separate workspace package implements or
   consumes the boundary.
3. **Durable schema contract** — a versioned serialized shape, reader, or semantic
   fact required to interpret durable or exchanged data.
4. **Accidental exposure** — no current consumer or invariant; remove or narrow it.
5. **Test-only exposure** — fault, inspection, or fixture support; gate it behind an explicit test
   feature or keep it in tests.

The unpublished evidence library's `http_fixture::LoopbackServer` is a development harness
contract consumed by its separate headless, local-model, and external-runner binary crates.
It owns their shared listener/worker lifecycle and retains failed shutdown evidence; it is not a
product server port. Its handler construction, bounded finish, and failure/turnover tests belong
with the existing HTTP fixture owner, and no product package imports it.

Workspace use can justify visibility without making a type a stable third-party API. Root
re-exports exist only when the root is the semantic owner or the re-export is the intentional
package entry point. Consumers otherwise import the canonical owner directly.

## Surface ownership

Runtime's `CurrentNodeExecution` and `SettledNodeExecutionProjection` are workspace projection
contracts returned by `RunProjection` and consumed by control, daemon inspection and evidence.
`SubworkflowUsageSummary` names the durable compact child-usage shape exposed by the existing
projection query and serialized snapshot. Their root re-exports make existing return types
nameable; construction and mutation remain with replay, and no alternate runtime state owner
is added. Default builds still exclude `ManualClock` and `DeterministicExecutor`.

`DirectInvocationDraft`, `PeerService::prepare_client_invocation` and
`ControlClient::prepare_invocation` are external request and workspace adapter contracts consumed
by the daemon and CLI. The serving owner constructs the exact direct/published request through
existing snapshot, input and authority owners. Client formatting no longer derives that selection,
idempotency policy or deadline. Preparation adds no accepted execution record or durable format.

`learning::KnowledgeSelectionDraft` and `LearningRequest::SelectSources` are public request
contracts consumed by the daemon and CLI. Artifact references are resolved through existing
authorized readers; the private handler passes them to the unchanged selection owner. Named CLI
evaluation commands only format that request family and display its immutable results.

`PublicationDraft` and `PublishedWorkflowService::prepare_publication` are workspace adapter
contracts consumed by the daemon's public `PrepareMethod` operation. The existing publication
owner retains service lookup and contract validation; preparation adds no persistent registry.

`CopyBlueprint` and `WorkflowFieldRead` are external product contracts consumed by daemon and CLI.
Copy construction remains private and delegates validation/storage to existing blueprint owners.
Interface reads project definition contracts without client graph parsing or execution values.

`RunResultRead`, `RunOutputRead` and `ControlClient::run_result` are external product contracts
consumed by the CLI and daemon. They compose existing authorized readers; the private daemon
module owns result selection and action hints. They add no durable state or client execution rules.
`ModelRepair` and `ProposalImpactRead` are likewise external protocol contracts consumed by the
daemon and CLI. The former requests a private daemon construction of an ordinary proposal; the
latter projects existing runtime reconciliation items. Neither exports a new reconciliation owner.

The run-start surface adds `RunInput`, consumed by the CLI and daemon as a named immutable artifact
reference. Runtime workspace admission and authenticated artifact reads retain validation ownership.
`SavedRunRequest` and the client's prepare/submit methods are consumed by the CLI's private recovery
file path; `AuthorityRead.host` binds that record to the serving installation. These are client
recovery contracts, with no new execution-history or capability-selection owner.

The workflow authoring surface adds `BlueprintDraft`, `BlueprintEdit`, `ModelInputSource`, and
the `AuthorBlueprint`/`ConstructBlueprint` command variants in control-protocol. They are external
product contracts consumed by the CLI and daemon through the existing control-client transport.
Only the daemon interprets editor gestures, builds existing blueprint/model values, and derives
revision identities. Its supported-shape recognizer and graph construction remain private; no
new blueprint executor, mutable draft store, or production test helper is exported.

The [architecture package map](../architecture.md#owners-and-dependency-direction) identifies each
production consumer boundary. Public semantic documents, adapter ports, and application entry
points must meet the classification above; private provider payloads, storage rows, daemon routes,
and projections do not need exports merely for tests.

Runtime `ManualClock`/`DeterministicExecutor` and capability-host secret/conformance helpers,
including unjournaled `SystemPeerClock` and `execute_exact` fixture entry, require non-default
`test-support`. Production serving receives the embedding owner's durable clock. Redb fault hooks
require `test-admin`; the provider parser measurement driver requires `operational-evidence`.
Review these separately from default product surfaces.
The evidence package is unpublished and must remain a development leaf. The CLI has no library API.
The daemon's `ConfigError` is an external configuration contract returned by `DaemonConfig::load`
and `validate`, consumed by the CLI and embedding callers. Its root re-export makes those existing
failure cases nameable without exposing private compilation or authentication helpers.
The unpublished Slotbook example's `Candidate` and `serve` exports are application entry points
shared by its corrected and explicitly defective fixture binaries. No product package consumes
them. Its booking state and HTTP implementation remain private. Protected recipe schema 2's
verifier executable/digest fields are durable adapter inputs consumed by managed-linux and daemon;
the earlier interpreter/source fields have been removed, with unsupported versions refused.

## Review method

Review all library roots and generated rustdoc JSON, then trace each item through production, test,
documentation, and external protocol consumers. Keep raw reports out of Git:

```sh
mkdir -p target/public-api
cargo public-api -p PACKAGE -sss --all-features --color never \
  > target/public-api/PACKAGE.all-features.txt
cargo public-api -p PACKAGE -sss --color never \
  > target/public-api/PACKAGE.default.txt
cargo machete
cargo tree --workspace --duplicates
```

The default-feature comparison is required for packages with test/evidence features. A new export
needs a named category, real consumer, validating construction where invalid state is possible,
and tests at the owning boundary. A smaller item count is diagnostic evidence, not permission to
hide an actual port, schema, or semantic type.

Repository contracts parse Cargo manifests and Rust syntax before checking dependency direction,
explicit exports and selected private construction boundaries. Dependency aliases, build edges,
optional declarations and target-specific sections receive the same ownership review. Separate
product-only Cargo resolutions check helper features; a workspace all-feature graph cannot prove
the default surface. Outside-workspace consumer probes also compile valid calls and refuse raw
revision/selector mutation and default helper imports, then admit explicitly enabled helpers.

The syntax checks recognize declarations, grouped exports and conditional attributes without
treating comments or strings as code. They do not resolve arbitrary aliases, expand every macro,
prove that a constructor validates correctly, or establish that a reachable adapter executes.
Keep compiler probes, reader/refusal tests, actual-client cases, composition/conformance tests and
the default/all-feature inventory review for those separate claims. A new package or export is an
ownership decision; update the exact checked boundary with its real consumer and negative evidence.

The managed boundary adds three intentional surfaces: `capability::managed` is the portable
request/inspection and descriptor-binding contract used by CLI, client and adapters;
`persistence::managed` is durable inventory plus transactional use/lifecycle ports implemented by
redb; `capability_host::managed` is the workspace semantic owner, platform/publication ports and
ordinary lifecycle adapter consumed by daemon and managed-linux. Linux recipe/platform/worker/model
constructors are adapter contracts consumed by daemon. Redb tables, indexes and Linux command/unit
helpers stay private. `PreparedAdapterExecution::with_entry_wrapper` lets the managed model adapter
add a use claim around frozen provider bytes; only final host entry receives that wrapper. The
entered-serving fixture helper and optional prepared-serving allowance assertion remain under
`test-support`. Review their default/all-feature inventories with those consumers; none promises a
separately versioned third-party Rust API.

The published-method boundary adds durable schemas and store ports under `persistence::published`;
redb keeps its inventory/index mechanics private. `control::PublishedWorkflowService` and its store
composition bound are workspace adapter contracts consumed by daemon. Capability-host owns
`PublishedWorkflowContinuation` and the prepared pending-work alternative; runtime and serving both
consume that port, while control implements it without a reverse dependency. Runtime's exact
create/bind/start and cancellation methods are workspace ports for that implementation, guarded by
the authoritative saved association. `InvocationCounts`, `NestedWorkUsage`, publication ancestry,
managed `NoExternalEntry` proof and published child links are durable cross-owner contracts; they
cannot be ordinary UI state. The output range and method commands are external control/peer
contracts, retaining exact caller and version checks. No extra public graph engine, mutable method
registry API or unjournaled invocation entry is introduced.
`PublicationAncestor` is a validated durable contract consumed by runtime dispatch, host context,
publication admission and peer delegation. It preserves each accepted depth ceiling.
`TaskExecutor::published_serving_entry_allowed` is a workspace adapter port: runtime invokes it
at internal final entry and capability-host routes it to its existing serving owner. Its default
refuses unavailable authority; the serving attachment and policy checks remain private.

`control::learning` owns durable declaration, selection, candidate-request and comparison schemas.
The daemon consumes its bounded pure comparison function after reading the actual serving, run,
account and verifier owners; the evidence driver consumes the same serialized contracts through
CLI commands. These are durable schema and workspace adapter contracts. `Command::Learning` is
the external control API entry point. Daemon receipt projections, provenance checks, selection
materialization and promotion composition remain private. No learning database, scheduler, global
memory subscription or test-only production export is added.
The declaration's `input_field` and `candidate_output` use the existing validated workspace key;
the observed terminal `output` is consumed by the comparison's exact-artifact check. These fields
are part of that single wire contract, rather than Slotbook-specific names in the daemon reader.
The additional before/after method-publication commit fault points remain under redb's existing
`test-admin` export. They test lost replies and refused commits without exposing a product fault API.
`CapabilityResolutionContext::is_prospective` is a workspace adapter query consumed by the host
resolver. It lets revision authority checks defer transient health and capacity to scheduling;
the runtime alone constructs prospective contexts, and neither resolution path acquires a permit.

`PublishedWorkflowService::maintain_retirement` is a workspace adapter operation called by daemon
maintenance and publication changes; the registry still owns the final permit/pending refusal.
Persistence's active installation/publication pages are workspace adapter ports used by recovery,
while exact and historical reads retain the durable documents. Redb's managed/publication limits
are external configuration contracts validated at open. Their private indexes and count mechanics
are not exported. Historical evidence is retained under the existing exact-version format policy.
`PeerExecutionStore::active_serving_page` is a bounded workspace adapter port consumed by serving
continuation maintenance and publication recovery/retirement. It includes queued accepted requests
without child links, so adapters cannot disappear before those requests reach entry.

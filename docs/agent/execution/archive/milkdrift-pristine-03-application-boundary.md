# Work package: pristine application boundary, persistence, and thin host projection

## Mission

Reconcile the corrected artifact/loading and E0 ownership semantics through the frontend-neutral application layer without duplicating backend policy. Make resolved artifact evidence, load admission, retained ownership uncertainty, persistence, and presentation state truthful and minimal. Tighten `application-runtime` as an optional application-services layer above the engine rather than allowing Phase 12 concepts to turn it into the only usable Milkdrift API.

Implement the durable design now. Do not preserve a redundant compatibility check, stale persistence code, or frontend-shaped engine contract merely because it already has tests.

## Read before editing

Read:

- `README.md` and `docs/vision.md`;
- the agent context/persona;
- `docs/project/architecture.md`;
- `docs/project/application-runtime.md`;
- `docs/project/inference-runtime.md`;
- `docs/project/candle-backend.md`;
- `docs/project/desktop-runtime.md`;
- `docs/project/implementation-status.md`;
- ADRs 0006, 0008, 0010, 0013, 0019, and 0020;
- the commits and completion reports from the two preceding work packages.

Inspect the actual public surface and state transitions in `application-runtime`, redb persistence, Hub artifacts, and Slint. Do not rely on the old Phase 12 report.

## Owned area

Primary ownership:

- `crates/runtime/application-runtime/**`;
- `crates/adapters/redb-storage/**`;
- application-facing types from `crates/adapters/hf-hub/**` that must be integrated after the first work package;
- `crates/apps/desktop-slint/**` only as a thin projection and regression host.

Make only narrowly necessary changes to E0/adapter code and record any lower-layer defect rather than duplicating a workaround in E1.

## Required architectural outcomes

### 1. Keep artifact evidence, compatibility, and execution facts distinct

E1 must preserve these separate facts:

- immutable repository/revision/commit and artifact identity;
- configuration declaration status;
- resolution compatibility evidence;
- selected application device;
- accepted load plan/admission facts;
- actual loaded execution device and scalar from the verified E0 receipt;
- exact or unverified retained lower ownership after failure;
- user-visible lifecycle state.

Do not collapse declaration into execution scalar. Do not infer tensor homogeneity from configuration. Do not infer execution scalar from device. Do not recompute Candle's required-tensor or conversion policy.

### 2. Remove adapter-policy duplication from E1

The current E1 load receipt validation contains a hardcoded “observed scalar types are only F32/F16/BF16” rule. That causes an unused, safely ignored tensor category to be rejected above an adapter that has already validated the required execution schema.

Remove such adapter-specific compatibility policy from E1. E1 may verify generic invariants such as:

- the observed set is nonempty when the portable descriptor requires it;
- the descriptor/receipt/admission identities agree;
- required generic capabilities exist;
- selected and actual device facts agree;
- budget and footprint state are internally coherent;
- the configured declaration evidence is preserved consistently.

E1 must not decide which Safetensors scalar mixtures Candle accepts. If a generic portable contract lacks a fact E1 truly needs, improve that contract with the preceding runtime design rather than reconstructing backend rules.

### 3. Integrate strict declaration status

Use the artifact adapter's corrected declaration status. Preserve absence, recognized values, unsupported values, malformed values, and contradictions until the appropriate resolution/compatibility boundary handles them.

Requirements:

- unsupported or contradictory present declarations cannot become `None`;
- resolution failure categories are stable and user-facing without leaking raw vendor errors;
- recognized declaration agreement is checked once at the correct ownership boundary, not independently in four places;
- an absent declaration remains loadable when required tensor evidence supports it;
- persistent model records store only durable recognized/absent metadata needed for catalogue display;
- unsupported/malformed transient vendor evidence is not persisted as though it were a valid selection;
- schema migrations remain explicit and backward-readable.

Review whether `ResolvedModel`, internal admission state, `LoadedModel`, events, errors, and stored records each need the declaration. Remove duplicated copies that exist only to cross-check one another without independent provenance.

### 4. Represent retained ownership uncertainty honestly

Integrate the E0 retained-ownership state from the previous work package.

E1 must distinguish:

- ordinary owner-free model-load failure;
- exact retained failed-preparation cleanup;
- exact retained complete-model ownership where valid;
- unverified retained ownership after backend contract violation;
- retryable versus exhausted cleanup;
- worker disconnection and terminal process-lifetime retention.

While lower ownership is retained or unverified:

- model selection/device changes remain locked where required;
- no normal loaded model is exposed;
- no false “released” state is shown;
- user-visible state explains that admission is blocked without fabricating byte precision;
- shutdown/join state remains independent from cleanup success.

Do not translate uncertainty into `0`, the planned peak, or a generic string.

### 5. Simplify the load-verification transaction

Audit `application-runtime/src/runtime/model.rs` and retained cleanup logic. The current code compares declaration evidence across admission, descriptor, resolved state, and artifacts. Determine which copies are genuinely independent evidence and which are duplicated transport state.

Refactor toward one explicit E1 load transaction with clear stages:

```text
resolved artifact snapshot
→ selected-device re-probe
→ immutable source construction
→ lower load submission
→ receipt/event correlation
→ generic receipt validation
→ application loaded-state commit
or retained-cleanup state
```

Keep primary failure and lower cleanup failure separate. Avoid large Boolean conjunctions that make provenance and failure classification impossible to audit. Use named validators/decision types with typed failure reasons.

Remove broad `too_many_lines` suppressions where structure can express the transaction better.

### 6. Keep persistence durable and narrow

Review `redb-storage` schemas and migrations after the declaration changes.

Requirements:

- exact versioned encoding with no ambiguous sentinel reuse;
- backward reads for every previously accepted version that is still promised;
- new writes use only the latest version;
- migration tests cover absent/present declaration metadata and existing device settings;
- runtime-only observed sets, required sets, execution scalar/device, source paths, content digests, cache paths, plans, and retained cleanup state are not persisted as preferences/catalogue truth;
- corruption and unknown versions fail explicitly;
- no automatic destructive rewrite during read unless documented and transactional.

Consolidate duplicated scalar-code conversion logic across E1 and redb into the correct owning layer without creating a dependency inversion.

### 7. Tighten the public application API

Review the Phase 12 additions to the `application-runtime` public API as if it were consumed by a headless host rather than only Slint.

The API should expose stable application semantics:

- model selection and immutable resolution;
- device selection and availability;
- load/unload and retained cleanup state;
- actual execution facts;
- bounded generation output;
- cancellation and shutdown.

It should not expose:

- Candle types;
- Safetensors tensor names/shards/digests;
- adapter conversion policy;
- persistence record formats;
- Slint labels or indexes;
- benchmark DTOs.

Remove or privatize Phase 12 helper types that have no demonstrated external consumer. Add rustdoc that explains resolved versus loaded versus retained states and how a headless consumer drives the runtime.

Do not implement the future workflow API here. Preserve the architectural statement that `application-runtime` is an application kit/reference composition above the local engine, not the workflow control plane.

### 8. Keep Slint disposable and thin

Adapt Slint only to the corrected E1 state.

It may display:

- configuration-declared scalar when recognized and present;
- selected device;
- actual loaded device and execution scalar;
- explicit load/cleanup/unavailable states.

It must not gain:

- tensor inventories;
- conversion controls;
- source-digest details;
- backend cleanup orchestration;
- model compatibility policy;
- direct redb/Hub/Candle access.

Review presenter tests so they assert E1-to-view projection rather than locking wording or internal indices unnecessarily. Keep bounded frame work.

### 9. Prepare for a future headless consumer without building it

Use tests or a small existing example to prove the application layer can be driven without Slint:

- start;
- select/resolve;
- load;
- generate and drain bounded output;
- cancel or finish;
- unload;
- retry retained cleanup where applicable;
- shut down.

Do not create a full server, transport, or workflow host in this work package. The purpose is to ensure the public E1 boundary is not accidentally GUI-shaped.

## Testing requirements

Add or update tests for:

- absent, recognized, unsupported, malformed, and conflicting declarations;
- required F32 models with unused F16/BF16/other extras accepted through E1 when the adapter accepts them;
- genuine unsupported required layouts rejected below E1;
- receipt identity/device/scalar/final footprint mismatch;
- exact and unverified retained ownership translation;
- device selection/admission locks during retained cleanup;
- cleanup retry, exhaustion, disconnection, and shutdown;
- persistence migrations and corruption;
- thin presenter behavior;
- headless lifecycle usage through public E1 methods.

Avoid tests that duplicate the Candle scalar-policy matrix inside E1.

## Validation

Run targeted formatting, checks, tests, Clippy, and rustdoc for:

- `hf-hub-adapter` if integration changed;
- `redb-storage`;
- `application-runtime`;
- `desktop-slint`;
- downstream benchmark compilation needed to detect public API breakage.

Run default CPU lifecycle tests and CUDA compilation. Execute the exact E1 CUDA lifecycle test on available hardware. Do not claim full project closure.

## Completion

Update application-runtime, persistence, desktop-runtime, and architecture documentation where semantics changed. Do not update final support/evidence status yet; the infrastructure/truth work package owns current-tree closure.

Create one coherent commit and do not push. Report:

- commit SHA and tree SHA;
- final E1 resolved/load/retained state model;
- declaration and persistence semantics;
- public API changes;
- proof that Slint remains thin;
- exact tests and hardware validation performed;
- any project-infrastructure or documentation work left for the next agent.

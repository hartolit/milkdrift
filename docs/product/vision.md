# Milkdrift — product vision

> This document owns the enduring product intent of Milkdrift: why it exists, what kind of system it is trying to become, what experience it should create, and which principles must survive implementation changes.
>
> It is deliberately not an implementation-status report, milestone ledger, crate map, or API reference. `docs/product/status.md` must say what exists now. `docs/product/roadmap.md` must say what comes next. `docs/architecture.md` must say where responsibilities live and which invariants bind the implementation. This document is the compass those documents are measured against.

## 1. Why Milkdrift exists

Milkdrift began with local AI inference because the original problem was still unclear. The project initially tried to own model loading, tensor formats, sampling, hardware backends, and the application around them. That work exposed a more important truth:

> The hard problem Milkdrift should solve is not producing tokens. It is turning open-ended work into a durable, inspectable, programmable process that can continue without constant manual shepherding.

The practical frustration is familiar:

```text
write prompt
    |
    v
open agent window
    |
    v
wait for implementation
    |
    v
copy result somewhere else
    |
    v
run verification manually
    |
    v
notice the implementation was weak
    |
    v
open another fresh context
    |
    v
explain the repository again
    |
    v
paste the next prompt
    |
    v
repeat until context, patience, or confidence runs out
```

The work is not only the code an agent writes. The work also includes:

- deciding what should happen next;
- preserving the right context while discarding irrelevant history;
- deciding when a fresh context is safer than continuation;
- running tools and verification;
- detecting a bad result before it contaminates later work;
- pausing, revising, branching, retrying, or abandoning work;
- preserving evidence of why a decision happened;
- coordinating machines with different repositories, tools, credentials, and hardware;
- controlling who or what may change the process;
- resuming after interruption without inventing history.

Milkdrift exists to make that entire process programmable.

It should reduce manual prompt shuffling without hiding the work behind an opaque autonomous-agent loop. It should make automation more powerful **because it is more observable and governable**, not because it asks the operator to trust more invisible state.

## 2. The product thesis

Milkdrift is a local-first, durable, live-editable blueprint runtime for work.

A workflow is a versioned program made from capabilities. Capabilities may be:

- AI model endpoints;
- coding-agent processes;
- shell, build, browser, repository, database, deployment, or monitoring tools;
- humans and approval gates;
- services and APIs;
- remote Milkdrift execution hosts;
- future capability types the core does not need to know in advance.

The runtime owns:

- workflow meaning;
- immutable revision history;
- durable execution history;
- scheduling and structured concurrency;
- context selection and workspace ownership;
- authority and control decisions;
- provenance;
- cancellation, retries, recovery, and uncertainty;
- prospective live changes.

The runtime does **not** own:

- model architectures;
- tensor formats;
- tokenization and sampling;
- GPU kernels and device support;
- the internal implementation of external tools;
- a proprietary network overlay;
- credentials that can remain on a capability host;
- hidden mutable definitions.

The central boundary is:

```text
                         Milkdrift
                durable orchestration truth
                             |
             +---------------+----------------+
             |               |                |
             v               v                v
         AI models         tools          remote hosts
      OpenAI / Claude     processes       repositories
      llama.cpp / any     humans          infrastructure

Milkdrift decides what work means and what happened.
Capabilities decide how one authorized operation is performed.
```

## 3. Three truths that must never be collapsed

The product becomes understandable when it separates three different kinds of truth.

### 3.1 Definition truth — what should happen

Blueprints and workflow revisions describe the intended program.

```text
Workflow: Build Application

Revision 1 ---- Revision 2 ---- Revision 3
                      \
                       `---- Revision 4
```

A revision is immutable. Editing creates another revision with explicit ancestry. There is no hidden in-place mutation of a running graph.

Definition history answers:

- What workflow was intended?
- Who or what authored this change?
- Which revision was its parent?
- What semantic changes were made?
- What capability requirements and context policies applied?
- Was this revision proposed, approved, or applied?

### 3.2 Execution truth — what actually happened

Runs and append-only events describe accepted facts.

```text
Run 281
  000001 RunCreated
  000002 NodeEligible(architecture)
  000003 CapabilityResolved(model.claude)
  000004 NodeStarted(architecture)
  000005 ArtifactPublished(architecture.md)
  000006 NodeCompleted(architecture)
  000007 NodeStarted(implementation)
  000008 VerificationFailed(...)
  000009 RunPaused(...)
  000010 RevisionAdoptionRequested(42)
  ...
```

Execution history answers:

- What was accepted?
- Which exact revision governed the work?
- Which capability generation was selected?
- What context and artifacts were supplied?
- What side effects may have occurred?
- What output, failure, cancellation, or uncertainty was observed?
- What did the runtime know at each durable boundary?

Completed history is never edited to match a newer definition.

### 3.3 Control truth — who may decide what happens next

Control is neither the workflow definition nor the execution history. It is the authority to inspect and influence future execution.

```text
Actor
  |
  +-- inspect
  +-- propose revision
  +-- pause / resume
  +-- approve / reject
  +-- retry / cancel
  +-- apply reconciliation
  +-- terminate
  `-- delegate selected authority
```

Control truth answers:

- Which actor is making this request?
- Which immutable grant revision applies?
- Which workflows, runs, capabilities, providers, paths, networks, secrets, and budgets are in scope?
- Is approval required?
- Was authority revoked or narrowed?
- Which decision authorized the action?

These three truths relate to one another, but they must never become one mutable object.

```text
        immutable workflow revision
                    |
                    v
             durable run events
                    |
                    v
         current operational projection
                    ^
                    |
      authorized prospective commands
```

## 4. A workflow is a live program, not a static DAG form

Milkdrift should not be a generic box-and-arrow editor that merely launches jobs.

A workflow is a durable program whose future can change while its past remains fixed.

The operator should be able to:

- append another prompt or stage;
- insert a reviewer before a dangerous continuation;
- pause because an implementation looks suspicious;
- fork two independent approaches;
- require both results before judging them;
- replace future work after a failed verification;
- isolate a branch's context and filesystem state;
- continue a successful branch while cancelling another;
- hand control to an authorized AI supervisor;
- recover after process or machine restart;
- inspect any accepted fact afterward.

The system should make long-running work feel like a living, versioned program rather than a collection of chats.

## 5. Git-like lineage without pretending workflows are Git

The Git analogy is useful because it establishes the right mental model:

- immutable objects;
- explicit parents;
- branches of thought or development;
- inspectable changes;
- no silent rewriting of accepted history;
- deliberate merges rather than accidental state blending.

Milkdrift should borrow those properties without inheriting Git's object model or pretending every workflow edit is a text merge.

```text
Definition lineage

      R1 ---- R2 ---- R3 ---- R5
                \             /
                 R4 ----------

R4 may represent an alternate plan.
R5 must record how the semantic conflict was deliberately resolved.
```

Revision identity should describe semantic workflow meaning, not presentation layout, wall-clock timestamps, map ordering, UI positions, or transient health.

The canvas may move a node without changing the workflow revision. Changing what the node does, what it requires, what it depends on, or how its context is selected must create a new semantic revision.

## 6. Live changes are prospective reconciliation

Safe live editing is not “mutate the graph under the scheduler.”

Suppose the current run is:

```text
A completed
    |
B completed
    |
C running
    |
D pending
```

The operator or controller creates a new revision:

```text
A completed
    |
B completed
    |
C running -----.
    |           |
    |           v
    |      E remediation
    |           |
    `-----------'
                |
                v
          D independent review
```

Milkdrift must reconcile the prospective revision against durable history.

- A and B do not run again.
- C remains attached to the definition and capability selection under which it started.
- E may be inserted if its inputs and authority are valid.
- D may wait for E under the new revision.
- No completed fact is renamed, reclassified, or deleted.
- If C may already have produced a non-idempotent side effect, the runtime cannot pretend it never happened.

Every change to a live run must classify work explicitly:

```text
completed        preserve exactly
running          finish, cancel, retain, query, or remediate by policy
uncertain        require truthful resolution; never duplicate casually
pending          retain, redirect, replace, or remove prospectively
new              schedule only after adoption
incompatible     stop and require a decision
```

The absolute invariant is:

> History never changes. Revisions can change only the future.

## 7. Humans and AIs share one control path

An AI controller is not a privileged hidden subsystem. It is an actor with a grant.

The same semantic operations should serve:

- a human dragging nodes on the canvas;
- a CLI command;
- an API client;
- a deterministic service;
- an AI supervisor;
- a remote delegated controller.

```text
human gesture ---------.
                       |
AI proposal -----------+--> typed command / mutation
                       |           |
service automation ----'           v
                           authority evaluation
                                  |
                                  v
                         immutable revision
                                  |
                                  v
                           reconciliation
```

There must not be an internal function such as:

```rust
fn ai_rearrange_graph_without_normal_rules(...)
```

AI control is powerful precisely because it uses the same auditable machinery as the operator.

## 8. Graduated autonomy, not one dangerous Boolean

The product should support useful authority templates while keeping the actual grant explicit.

Conceptually:

```text
Observer
    inspect permitted state

Advisor
    inspect + propose prospective revisions

Supervisor
    pause, retry, cancel, approve and apply bounded low-risk changes

Controller
    restructure future workflow within exact resource and capability scope

Autonomous
    repeatedly inspect, propose, apply and continue under strict ceilings
```

These names are conveniences, not magic privilege levels. They must expand into ordinary operations, resources, budgets, validity, and revocation rules.

An actor's authority may constrain:

- workflow and run identities;
- capability identities and categories;
- provider profiles and model endpoints;
- side-effect class;
- filesystem roots;
- network destinations;
- secret references;
- artifact visibility;
- cost, time, tokens/units, bytes, concurrency, and invocation count;
- peer identities and delegated operations;
- proposal risk and approval policy.

A controller must not be able to enlarge its own authority merely by proposing a revision that requests a stronger capability.

## 9. AI-authored workflows

The operator should not need to construct every graph manually.

A workflow may begin as one node:

```text
+---------------------------------------------------+
| Workflow Architect                                |
|                                                   |
| Build a local-first Rust application that ...     |
+---------------------------------------------------+
```

The architect receives permission to inspect the goal and propose a workflow revision. It may produce:

```text
                    Requirements
                         |
                    Architecture
                    /           \
                   v             v
               Backend         Frontend
                   \             /
                    v           v
                     Integration
                         |
                       Tests
                         |
                 Independent Review
                         |
                 Continuous Controller
```

That output is not immediately trusted executable state. It is a bounded proposal:

1. parse and validate the proposed mutation;
2. verify exact base revision and run sequence;
3. classify risk;
4. evaluate authority;
5. require approval when policy says so;
6. create an immutable revision;
7. reconcile prospectively;
8. record the actor and evidence.

This makes “build me a workflow for this goal” a normal product feature rather than a privileged code path.

## 10. Continuous creation without hidden infinite loops

Milkdrift should support work that continues for hours, days, or indefinitely. It should not encode that as an uncontrolled cyclic graph.

Arbitrary cycles make recovery, causality, visualization, cancellation, accounting, and provenance difficult to reason about.

Use explicit repetition and revision continuation:

```text
execute current work
        |
        v
inspect outcome
        |
        v
controller decides
   /             \
  v               v
stop          propose next revision
                    |
                    v
               reconcile
                    |
                    v
             execute new work
```

Or use a bounded repeat construct:

```text
Repeat
  body
  condition
  maximum iterations
  maximum elapsed time
  maximum cost
  delay/backoff
  continuation policy
```

Continuous does not mean unbounded. Every autonomous loop needs explicit ceilings, stop conditions, and human checkpoints.

## 11. Structured concurrency: Splitter and Compositor

The user's visual concepts of a Splitter and Compositor map to structured Fork, Join, and Reducer semantics.

```text
                     A
                     |
                     v
                  Splitter
                 /        \
                /          \
               v            v
           Sequence B    Sequence C
               |            |
               v            v
           Result B      Result C
                \          /
                 \        /
                  Compositor
                       |
                       v
                       D
```

The internal responsibilities are distinct:

```text
Fork
    creates isolated child branches

Join
    decides when enough branches have completed

Reducer
    combines or selects branch results
```

Join policies may include:

- all;
- any;
- first success;
- quorum;
- an explicitly defined future policy.

Reducers may include:

- collect;
- first;
- deterministic structured reduction;
- a capability-backed judge;
- a domain-specific merger.

The Compositor shown in the UI may combine Join and Reducer for convenience, but the runtime must keep synchronization separate from data combination.

## 12. Branches own isolated mutable state

Parallel agents must not unknowingly mutate one shared workspace.

```text
                  parent scope
                       |
              immutable snapshot
                /             \
               v               v
        branch A workspace  branch B workspace
        private mutations   private mutations
               \               /
                \             /
                 explicit join/import
```

Branches may share immutable artifacts and references. Mutable values are branch-local unless an explicit join, reducer, import, or coordination capability mediates the transfer.

A join must decide what happens when state conflicts:

- collect both values;
- select one;
- invoke a reducer;
- report conflict;
- discard one by explicit policy.

No invisible last-writer-wins behavior belongs in a durable workflow engine.

## 13. Causal context is Milkdrift's memory model

Milkdrift should not define memory as “send the whole conversation again.”

Chronological context is easy to accumulate and difficult to reason about. It mixes relevant decisions with exploration, tool noise, stale assumptions, and malicious or accidental instructions.

Milkdrift should construct context from causality.

```text
                     Architecture
                          |
                    Implementation
                     /           \
                    v             v
               Backend work    UI work
                    \             /
                     v           v
                      Integration
                          |
                     Verification
                          |
                        Review
```

The Review task may receive:

```text
include
  architecture decision artifact
  integration output
  failed verification evidence
  relevant source diff
  exact capability/provider provenance

exclude
  unrelated UI exploration
  old raw command streams
  superseded proposals
  other branch's private scratch state
```

A context policy should be able to select:

- direct inputs;
- ancestors to a bounded depth;
- exact nodes or executions;
- semantic roles such as decision, failure, requirement, implementation, review;
- artifact categories, media types, sensitivity, provenance, and retention class;
- selected workspace values;
- explicit evidence references;
- prior controller/reviewer results;
- fresh or continued model/process sessions;
- byte, item, artifact, provider-unit, and cost budgets.

The resulting context manifest is a durable fact. It records what was selected, omitted, denied, missing, or truncated and why.

That allows the operator to answer:

- What did the agent know?
- Which evidence was omitted?
- Did hostile or irrelevant material enter the context?
- Did the task exceed a memory budget?
- Was a decision based on a stale artifact?

## 14. Workspaces, artifacts, and retention

Events should remain small, stable facts. Large content belongs in artifacts.

```text
RunEvent
  NodeOutputPublished
      artifact_id
      digest
      size
      media_type
      sensitivity
      producer provenance
```

The event does not embed a 20 MB build log, repository archive, model transcript, or patch.

Workspaces provide scoped logical state. Artifacts provide immutable content. Context manifests select references to both.

Retention should be explicit:

```text
keep
  decisions
  final outputs
  workflow mutations
  failures
  approvals
  security-relevant evidence

compact
  verbose progress
  repetitive build output
  intermediate model fragments

expire when allowed
  temporary materializations
  abandoned transfer chunks
  superseded caches
```

Compaction must never mean silently erasing the execution journal or the provenance required to understand an accepted decision.

## 15. Provenance is first-class forensic memory

Milkdrift should make it difficult for meaningful work to happen without traceable provenance.

Every execution result should connect to facts such as:

```text
RunId
WorkflowId
RevisionId
NodeId
NodeExecutionId
AttemptId
Actor / controller
Authority decision
Capability identity and generation
Provider profile / peer identity
Context manifest
Input references
Output artifacts
Side-effect classification
Started and completed boundaries
Terminal or uncertain outcome
```

Every workflow change should connect to:

```text
new revision
parent revision(s)
author actor
proposal identity and digest
reason and evidence
risk classification
approval decision
application boundary
reconciliation result
```

The desired historical trace is:

```text
strange code decision
        ^
        |
implementation attempt 728
        ^
        |
prompt/context manifest 711
        ^
        |
workflow revision 18
        ^
        |
revision proposal by controller 694
        ^
        |
review failure 690
```

This is not surveillance for its own sake. It is the memory required to debug autonomous work, detect contamination, assign responsibility, and improve the workflow.

## 16. Capabilities are the open execution fabric

The workflow asks for requirements, not implementations.

```text
Task requirement
  operation: model.generate
  features: structured_output, tools
  context: >= requested policy
  trust zone: local-or-approved-remote
  side effect: read_only
```

At execution time, the capability host selects one exact descriptor generation permitted by policy and current health.

```text
Capability requirement
        |
        v
live registry snapshot
        |
        +-- local process generation 4
        +-- Claude endpoint profile 2
        +-- llama.cpp endpoint profile 7
        +-- remote peer capability generation 12
        |
        v
exact resolved capability snapshot
        |
        v
persist before entry
```

Descriptors should honestly advertise:

- identity and generation;
- operations;
- input/output schemas;
- supported streaming shapes;
- cancellation semantics;
- idempotency behavior;
- side-effect class;
- locality and trust zone;
- admission limits;
- optional resource/cost observations;
- documentation and implementation references;
- exact provider or peer provenance where applicable.

Health, load, credentials, leases, and live handles are observations, not immutable descriptor truth.

## 17. AI models remain external

Milkdrift should not compete with llama.cpp, vLLM, Ollama, Candle, provider SDKs, GPU runtimes, or future inference systems.

A local model server and a hosted provider are capability endpoints:

```text
                         model.generate
                              |
            +-----------------+------------------+
            |                 |                  |
            v                 v                  v
        llama.cpp          OpenAI             Anthropic
       user hosted         hosted              hosted
```

The workflow core should not know:

- whether Qwen uses a hybrid attention architecture;
- whether the model is GGUF or Safetensors;
- whether execution is CPU, CUDA, ROCm, Vulkan, Metal, or remote;
- how tokens are sampled;
- how the provider meters internal reasoning.

Milkdrift owns:

- provider-neutral task intent;
- exact endpoint/profile selection;
- feature negotiation;
- context selection;
- authority and budgets;
- cancellation limits;
- streaming/output capture;
- provenance and artifacts;
- truthful unsupported-feature behavior.

Provider differences must remain visible. A common contract must not silently discard requested tools, structured output, role semantics, images, sessions, reasoning controls, or cancellation requirements.

## 18. Local processes and tools

A process capability lets Milkdrift execute coding agents, build systems, repository tools, and ordinary programs without hardcoding a vendor.

```text
process profile generation
  executable/deployment identity
  argv template
  working-directory policy
  materialized inputs
  allowed environment
  secret references
  output declarations
  filesystem/network trust class
  timeout/cancellation
  side-effect and idempotency facts
  resource bounds
  documentation
```

A process adapter should not be called a sandbox unless the operating system actually enforces isolation.

The product should distinguish:

```text
TrustedHostProcess
  executes with daemon-account host authority
  Milkdrift mediates inputs, paths, environment and outputs

SandboxedProcess
  executes inside an enforced container/namespace/VM policy
  capability advertises the exact isolation properties
```

Executable identity must be immutable enough for provenance. A path alone is not a tool generation if the bytes at that path can change.

## 19. Peers are federated execution hosts, not an inference mesh

A Milkdrift peer exists to expose tools and resources tied to another machine.

```text
                   authoritative workflow daemon
                             |
               +-------------+-------------+
               |             |             |
               v             v             v
          local model     local tools    peer execution host
                                            |
                                   +--------+---------+
                                   |        |         |
                                   v        v         v
                              repository  database  deployment
                              toolchain    admin     credentials
```

This is useful when:

- repositories need separate toolchains or build resources;
- one machine owns a GPU, signer, browser, database, or production environment;
- credentials should remain on the machine that uses them;
- workloads would congest one host;
- an infrastructure server should expose carefully scoped administration operations;
- remote work must survive disconnects without accidental duplicate execution.

The workflow authority remains with the originating daemon. The serving peer owns its environment and the accepted remote execution record.

The peer protocol must preserve:

- authenticated peer identity;
- explicit relationship grants;
- expiring capability catalogs;
- exact descriptor generation;
- idempotent durable acceptance;
- resumable observations;
- separate cancellation acknowledgement;
- truthful disconnect uncertainty;
- bounded verified artifact transfer;
- quotas and revocation;
- provenance linking local and remote execution identities.

Peers must not imply:

- shared mutable workflow state;
- shared databases;
- automatic trust;
- arbitrary remote shell access;
- model/tensor synchronization;
- a proprietary VPN or discovery overlay.

Operators provide connectivity through LAN, HTTPS, WireGuard, Tailscale, reverse proxies, or another chosen network.

## 20. Peer tags and placement

Tags and labels can help select execution locality:

```text
os=linux
arch=x86_64
zone=home
zone=production
repository=milkdrift
toolchain=rust
gpu=nvidia
environment=staging
service=postgres
```

A workflow may request:

```text
operation: repository.test
constraints:
  repository = milkdrift
  toolchain = rust
  environment != production
```

Tags are descriptive. They are never authority.

A peer calling itself `trusted=true` does not grant trust. Trust comes from authenticated identity, configured relationships, immutable grants, capability allowlists, quotas, and exact generation pins.

## 21. Self-extending capability hosts

A future workflow may be allowed to install or extend tools on a peer. This is powerful and dangerous enough to require its own controlled lifecycle.

It must not be:

```text
agent has shell
agent installs arbitrary program
new tool silently becomes trusted
```

It should be:

```text
CapabilityInstallationProposal
  source and version
  package/image/executable digest
  requested host permissions
  operation schemas
  side-effect classification
  resource limits
  secret requirements
  health checks
  documentation
  rollback/removal plan
          |
          v
risk classification and authority
          |
          v
approval when required
          |
          v
isolated installation/deployment
          |
          v
contract and health verification
          |
          v
new immutable capability generation
          |
          v
catalog update
```

An in-flight invocation remains pinned to the generation it accepted. New work may select the new generation only after it is healthy and authorized.

The capability documentation is part of the generation's usable state. An agent must be able to retrieve the exact operation semantics, examples, failure modes, and rollback instructions corresponding to the selected generation.

## 22. Security is explicit authority, not optimistic trust

Milkdrift coordinates systems that can modify repositories, install software, deploy services, alter databases, spend money, and operate production infrastructure. Security cannot be a later wrapper.

The core principles are:

- authentication proves actor or peer identity;
- authorization decides what that identity may do;
- authority follows work into capability resolution and execution;
- secrets remain opaque references until an authorized adapter boundary resolves them;
- no model or workflow-provided string becomes authority by claiming a role;
- reads are capabilities too;
- provider/model output is untrusted data until parsed, bounded, validated, and authorized;
- external side effects are classified before execution;
- uncertainty is recorded honestly;
- revocation affects future entry without rewriting accepted past facts;
- least privilege is the default;
- broad administrative profiles are explicit and visibly dangerous.

A valid bearer token, peer certificate, or local login must not imply access to every workflow, artifact, capability, provider, path, or secret.

## 23. The daemon is the durable authority

The canonical deployment has one authoritative daemon for a workflow domain.

```text
                        Iced / CLI / API clients
                                  |
                                  v
                         Milkdrift daemon
                    +-------------+-------------+
                    |             |             |
                    v             v             v
               blueprint      durable runtime  control service
                    |             |             |
                    +-------------+-------------+
                                  |
                       journal / artifacts / state
                                  |
                                  v
                         capability host
```

The daemon owns:

- persistence lifecycle;
- runtime recovery and admission;
- capability registration and health;
- effect workers;
- authority evaluator and credential mapping;
- process/model/peer adapter lifecycle;
- control API and streams;
- orderly shutdown.

Clients do not open the database or resolve adapter secrets. They submit commands and render authorized read models.

One daemon may delegate capability execution to peers, but it does not surrender workflow truth.

## 24. The Iced control center

The UI is not the product's source of truth. It is the native operator workbench over the daemon's stable command and query model.

It should have three primary perspectives.

### Canvas — the program

```text
+-------------+       +----------------+       +-------------+
| Requirements| ----> | Implementation | ----> | Verification|
+-------------+       +----------------+       +-------------+
                             |
                             v
                       +------------+
                       | AI Reviewer|
                       +------------+
```

The canvas shows:

- semantic nodes and edges;
- revision lineage and pending edits;
- capability requirements;
- structured branches, joins, reducers, repeats, waits, and subworkflows;
- current run overlays without making execution state part of semantic identity;
- drag/drop editing that produces typed mutations and prospective revisions.

Layout is presentation state and must not change revision identity.

### Timeline — what happened

```text
21:01  architecture started
21:03  architecture artifact published
21:04  implementation started
21:42  verification failed
21:43  run paused by supervisor
21:45  remediation revision proposed
21:48  revision approved and adopted
21:49  remediation started
```

The timeline is paged and virtualized. It must not require loading the whole run into memory.

### Inspector — exact evidence

```text
Node execution
  revision
  actor
  capability generation
  provider or peer
  authority decision
  input bindings
  context manifest
  prompt/task request
  tool calls
  streamed observations
  output artifacts
  usage and cost
  failure / cancellation / uncertainty
  causal parents
```

The inspector is where autonomous work becomes understandable.

The first UI should be built only after these read models and commands are stable headlessly. It should not invent semantics to compensate for an unfinished core.

## 25. The original dogfood workflow

Milkdrift must eventually develop Milkdrift.

The motivating workflow is the sequence of implementation prompts that previously required manual agent windows.

```text
Prompt 1
   |
   v
fresh coding-agent session
   |
   v
apply changes in repository workspace
   |
   v
run verification
   |
   +-- success --> checkpoint --> Prompt 2
   |
   `-- failure
          |
          v
      pause workflow
          |
          v
      independent review
          |
          v
      remediation proposal
          |
          v
      prospective revision inserts fix + re-review
          |
          v
       continue
```

Required properties:

- each prompt can use a fresh agent context;
- repository progress persists across stages;
- the exact prompt, context manifest, tool profile, diff, logs, and result are artifacts/provenance;
- verification is a real process capability;
- bad output cannot cascade silently into later prompts;
- a reviewer or controller can pause and insert remediation;
- the daemon can restart without losing accepted work;
- the operator can inspect every step afterward;
- the workflow is usable from CLI/API before a graphical client exists.

This is the first proof that Milkdrift solves its own reason for existing.

## 26. A broader infrastructure workflow

Milkdrift should also support an explicitly authorized infrastructure process:

```text
Goal: deploy and operate application
                |
                v
        architecture / plan
                |
       +--------+---------+
       |                  |
       v                  v
application peer     database peer
build and test       provision schema
       |                  |
       +--------+---------+
                v
         deployment peer
                |
                v
         monitoring peer
                |
                v
       health inspection loop
                |
       +--------+---------+
       |                  |
     healthy           failure
       |                  |
      stop       propose remediation
```

The database peer may retain database credentials. The deployment peer may retain signing or SSH keys. The central workflow sees capability operations and evidence, not raw credentials.

“Agent controls the server” should mean a visible set of granted operations, not an implicit root shell.

## 27. Deterministic infrastructure around nondeterministic intelligence

Models and external tools may be nondeterministic. Milkdrift's control infrastructure should not be.

Deterministic responsibilities include:

- revision construction and validation;
- authority evaluation;
- capability resolution tie-breaking;
- context candidate selection and ordering;
- command idempotency;
- event projection;
- scheduling eligibility;
- reconciliation classification;
- retry/cancellation policy;
- bounded accounting;
- schema decoding;
- provenance linking.

Nondeterministic results are accepted only through explicit capability observations and become immutable evidence.

```text
nondeterministic model/tool
          |
          v
bounded typed observation
          |
          v
deterministic runtime validation
          |
          v
append-only accepted fact
```

## 28. Logical target architecture

The long-lived ownership map is conceptual. Physical crates should be extracted only when dependency, lifecycle, publication, or multiple-consumer boundaries justify them.

```text
milkdrift/
|
+-- blueprint
|     model
|     validation
|     immutable revisions
|     mutations and lineage
|
+-- authority
|     actors
|     grants and delegation
|     resource scopes
|     deterministic policy
|     secret references
|
+-- control
|     proposals
|     risk classification
|     approvals
|     human/AI shared commands
|     bounded controllers
|
+-- runtime
|     scheduling
|     structured concurrency
|     effects and observations
|     recovery
|     prospective reconciliation
|
+-- workspace
|     scoped values
|     branch isolation
|     artifacts
|     causal context
|     retention
|
+-- capability
|     descriptors and requirements
|     live registry
|     exact resolution
|     admission and generation lifecycle
|
+-- persistence
|     revisions
|     append-only events
|     operational snapshots
|     indexes and artifacts
|
+-- adapters
|     model endpoints
|     local processes
|     secret sources
|     peer transport
|     future tools
|
+-- daemon
|     durable ownership
|     API and streams
|     adapter lifecycle
|     shutdown and recovery
|
`-- clients
      CLI
      Iced canvas / timeline / inspector
      future clients
```

Dependency direction is inward. UI, HTTP, redb, provider SDKs, process APIs, and peer transport must not determine workflow semantics.

## 29. Canonical vocabulary

Use these terms consistently:

- **Blueprint** — a reusable declarative workflow or subworkflow package.
- **Workflow** — a top-level blueprint identity with revision lineage.
- **Revision** — one immutable semantic workflow definition.
- **Run** — one durable execution pinned to a revision lineage.
- **Node** — a definition-time unit.
- **Node execution** — one runtime occurrence of a node.
- **Attempt** — one exact execution attempt under a capability resolution.
- **Edge** — explicit control and/or data dependency.
- **Capability requirement** — what an operation needs.
- **Capability descriptor** — immutable honest advertisement of one generation.
- **Capability observation** — mutable health/load/availability evidence.
- **Actor** — authenticated human, service, controller, or peer principal.
- **Grant** — immutable scoped authority revision.
- **Controller** — actor authorized to influence future workflow execution.
- **Event** — immutable accepted run fact.
- **Projection** — disposable state derived from events.
- **Artifact** — immutable content reference with digest and provenance.
- **Workspace** — scoped logical mutable state for a run or branch.
- **Context manifest** — exact record of selected and omitted task evidence.
- **Reconciliation** — prospective comparison/application of a newer revision to a live run.
- **Peer execution host** — remote Milkdrift daemon exposing authorized capabilities.
- **Layout** — non-semantic presentation state.

## 30. Non-goals and anti-goals

Milkdrift is not:

- an inference engine;
- a model-training framework;
- a provider-specific agent SDK;
- a chatbot with a graph painted around it;
- a static CI YAML replacement only;
- a generic DAG editor with hidden mutable state;
- a distributed shared database;
- a peer-to-peer model mesh;
- a proprietary VPN or NAT traversal system;
- an excuse to grant models arbitrary shell/root authority;
- a place to duplicate mature model, database, network, or container systems;
- a documentation ritual that substitutes for working end-to-end behavior.

The project must avoid:

- silent provider feature loss;
- hidden fallbacks;
- unbounded queues, logs, contexts, or autonomous loops;
- storing lifetime history in active operational state;
- mutable workflow definitions;
- UI types in the semantic core;
- magic AI-only control paths;
- role names as authority bypasses;
- two independent owners of the same durable fact;
- speculative crates and one-use traits;
- temporary architecture knowingly designed to be replaced by the next phase;
- preserving bad code because an agent spent many tokens producing it.

## 31. Engineering doctrine for future agents

Future implementation agents must treat this vision as a governing constraint, not motivational prose.

### Complete the boundary introduced now

When a pass introduces authority, cancellation, persistence, provenance, or a capability boundary, implement its full invariant now. Do not knowingly choose a weaker design because the project is early.

Defer unrelated breadth, not correctness.

### Do not confuse foresight with speculative abstraction

Plan for the stable product boundary, but do not create empty crates, one-implementation traits, generic frameworks, or future-provider scaffolding without a current consumer.

### One executable owner per fact

Do not store both a value and a caller-supplied derivation of that value. Do not duplicate policy as constants, presets, test literals, and adapter checks. Derive what can be derived.

### Preserve difficult truth

Do not turn:

- uncertain external work into failure or success;
- restricted data into absence;
- provider differences into a fake common denominator;
- running work into pending work;
- historical facts into newer definitions;
- capability tags into authority.

### Prefer established infrastructure

Use mature model servers, HTTP/TLS libraries, databases, process/container systems, and cryptographic primitives. Milkdrift should implement the orchestration semantics that are distinctive, not rebuild every underlying technology.

### Keep change locality measurable

A change to one semantic fact should touch one canonical owner and independent tests—not a web of repeated constants and mirrored algorithms.

### Delete aggressively when ownership is wrong

Git preserves history. Dead experiments, duplicate implementations, outdated prompt machinery, and speculative layers do not belong in the active product merely because they were expensive to create.

### Tests must observe the invariant independently

Tests should detect mutation of behavior, not repeat the same implementation formula. Use fault boundaries, model/property tests, cross-adapter conformance, restart tests, and realistic vertical dogfood.

### Documentation must tell the truth

`docs/product/vision.md` explains the destination.
`docs/architecture.md` explains ownership and invariants.
`docs/product/status.md` explains what is actually implemented and validated.
`docs/product/roadmap.md` explains what remains.

Do not let those documents silently borrow claims from one another.

## 32. What Milkdrift should feel like

Milkdrift should feel less like talking to an assistant and more like operating a living engineering process.

The operator should be able to look at a running system and understand:

- what program is being executed;
- why this task is now eligible;
- which actor or controller changed the future;
- which evidence reached an agent;
- which machine and tool performed the work;
- which side effects are known or uncertain;
- what failed and where remediation entered;
- how to pause, branch, revise, or continue without losing history.

The system should support deep automation without requiring blind faith.

Its defining experience is not:

```text
"The agent says it finished."
```

It is:

```text
I can see the revision it followed.
I can see the context it received.
I can see the capability generation it used.
I can see the artifacts and verification.
I can see why the controller changed the next step.
I can stop it, revise it, or let it continue.
The history will still be there tomorrow.
```

## 33. Success

Milkdrift succeeds when the operator can replace a fragile chain of manual agent interactions with a durable workflow while gaining—not losing—understanding and control.

The first decisive success is:

- import a sequence of implementation prompts;
- run each in a fresh coding-agent context against persistent repository progress;
- execute verification gates;
- detect a weak result;
- pause automatically or manually;
- inspect exact context and outputs;
- insert an independent reviewer and remediation step through a prospective revision;
- continue after approval;
- survive daemon restart;
- preserve the complete provenance chain;
- use local or hosted model endpoints without Milkdrift owning inference.

The broader success is a federated work fabric where repositories, tools, infrastructure, models, and human approvals can be composed under explicit authority without collapsing into one privileged machine or one opaque agent loop.

## 34. Final compass

When a future design decision is unclear, ask:

1. Does this make work more programmable without making it less inspectable?
2. Does it preserve the separation between definition, execution, and control?
3. Does it keep history immutable and live changes prospective?
4. Does authority follow the work all the way into capability execution?
5. Does it improve causal context rather than accumulate chronological noise?
6. Does it preserve exact provenance and truthful uncertainty?
7. Does it keep capabilities open to models, tools, humans, and peers?
8. Does it avoid rebuilding infrastructure another project already owns better?
9. Does it remain bounded enough for long-running workflows?
10. Can a human understand and intervene without becoming the workflow's manual scheduler?

If the answer is no, the implementation may be technically impressive and still move Milkdrift away from its purpose.

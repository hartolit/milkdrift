# Managed execution and connected hosts

Discussion synthesis, 2026-09-16. This preserves the user's requirements and the design questions connecting them. It is not an accepted architecture change or an instruction to implement every proposal below.

Repository reviewed: `64b4c5fa2b218dc59eef63fc44af8e094a8fb430`.

Suggested location: `docs/development/virtual-office/whiteboard/discussions/managed-execution-and-connected-hosts.md`.

## Purpose

Make Milkdrift easy to establish on a machine, useful across several machines, and able to help agents work without leaving an unmanaged collection of installations, services, and files behind.

The environment, peer, authorization, daemon, and interface questions belong to this one problem. They should inform each other rather than become separate products by accident.

The proposed direction is **managed separation as an offered execution choice**, alongside explicitly authorized use of existing host tools and remote services. Optional separation must not become a reason to leave its setup entirely to the operator. Conversely, convenient setup must not require every future capability to run inside one prescribed environment.

## Requirements carried forward from the discussion

The user wants a prepared, reproducible place where an agent can install tools, build applications, operate services, and keep its working files and documentation. Establishing another instance should not require reconstructing a long sequence of manual system changes. Removal should identify what belongs to the setup and preserve or remove useful data deliberately.

The initial host is the always-on UM790 Pro. The user's laptop and desktop are also intended to run Arch Linux. Windows and macOS remain future product targets, not requirements to solve before the first useful setup. NetBird or LAN supplies connectivity; neither should define Milkdrift's execution semantics.

An environment is not synonymous with model inference. It may include compilers, coding tools, a database, an application, or a model server. Existing native applications, CAD/media tools, remote services, and future model-native tool systems must remain usable without being repackaged into this environment.

A host should be useful on its own: another Milkdrift daemon or an external client can consume its permitted capabilities. Running a service should not inherently require running blueprint orchestration. A single installation may nevertheless provide both roles.

A workflow-owning daemon may expose a complete workflow as a versioned capability. Another user could invoke a deployment operation without copying its internal validation procedure. Authorized colleagues should still be able to inspect and edit those internals through a common interface; separate ownership should not force constant application switching.

Agents should be able to maintain their tools, propose workflow changes, and update operational knowledge within delegated authority. That is different from acquiring permission to rewrite the controls enforcing their authority.

These are user needs and possibilities to preserve. They do not establish a particular container technology, binary split, GUI framework, new crate, or API schema.

## What the repository already says

The vision already distinguishes trusted host execution from genuinely sandboxed execution, explains peers as hosts of machine-bound resources, and proposes controlled installation and publication of new capabilities. It also separates authentication, permission, and workflow ownership. This discussion connects and extends those intentions rather than replacing them. [1]

Implementation is narrower than the vision. Current status says ordinary trusted processes have the daemon account's privileges, without a sandbox or OS resource quotas. Peer placement and remote execution exist; dynamic local configuration reload and a GUI do not. These are documentation-based observations, not results of a new source audit or execution test. [2]

The existing roadmap does not authorize further feature implementation. Preserving and examining this operator need does not silently lift that restriction. The whiteboard explicitly exists to carry unresolved discussions across assignments. [3][4]

The earlier downloadable capability-host proposal used a September 6 baseline. Its statement that controller activation remained unavailable is no longer current: status now records explicitly enabled activation within a qualified operating scope. Do not reuse that earlier status claim as a prerequisite. Its deployment, knowledge, and recovery ideas remain proposals to assess, not accepted requirements. [2]

## One connected design, with different responsibilities

### The workflow decides what work should happen

The workflow owner retains definitions, accepted history, context, approval decisions, and the relationship between tasks. It can request work locally or from another host. Existing owners should retain those responsibilities rather than duplicating them in an environment manager.

### The execution host provides somewhere to do it

The execution host makes tools available, performs authorized operations, and reports outcomes. Managed setup would belong here: prepare a working area, configure its execution method, install selected tools, supervise persistent services, and support inspection and removal.

An execution-only host still needs authentication, execution records, recovery, and cleanup. “Lightweight” means not requiring an independent workflow scheduler and blueprint store merely to serve tools; it does not mean forgetting accepted work when a connection closes.

A full daemon can combine workflow ownership and execution hosting. A server that itself runs a published deployment workflow needs the workflow role too. These are different deployment needs, not reasons for two competing implementations of capability serving.

### The interface connects the operator to both

A web GUI, native GUI, CLI, or agent should reach the same supported operations. The interface can show several authorized hosts and workflows together while each owner still validates its own commands.

One installer or executable with selectable roles is a plausible delivery choice. A separate host executable using shared implementations is another. First establish what the execution-only role actually needs; then decide packaging. One binary does not imply one privilege level, and several processes do not require several user experiences.

## Separation should be created, not merely recommended

A proposed guided setup offers a managed working area and reports what it actually isolates. Existing accounts, containers, virtual machines, or native sessions are mechanisms to evaluate—not one universal definition of an environment. For example, rootless Podman already supplies user namespaces without requiring a rootful container daemon. Milkdrift could configure such a mechanism rather than reimplementing it. [7]

The operator can instead connect an existing tool or authorize host execution. These choices must be visible. Failure to establish a requested protected configuration must not silently fall back to a host shell.

An execution profile should identify the target, intended access, and protections the implementation can actually enforce. Files, networks, devices, credentials, and resource limits require platform enforcement where containment is claimed. A directory name, a prompt, or a Milkdrift grant alone does not constrain arbitrary operating-system actions. Current trusted-process behavior makes this distinction particularly important. [1][2]

Host-root execution may be an explicit administration choice, but should not be inherited merely because setup needed elevation. Bootstrap authority and ongoing worker authority are separate. On Linux, effective privileges also depend on capabilities and namespaces, not just the displayed username. [8]

Routine installation and service changes can be preauthorized within the selected boundary. The operator need not approve every command. Changing that boundary requires the appropriate administrative authority; an agent cannot confer it on itself by editing a recipe or capability description.

An external model could help choose and prepare the configuration, but it must not be necessary to establish the basic working setup. Inspectable, repeatable setup should remain available when no model is connected.

## Peers, shared administration, and published workflows

A peer relationship answers which other host can perform authorized work. It is not automatically a backup or replica of the originating daemon. Workflow failover would require a separate design for state transfer, ownership, and duplicate execution; ordinary peer connectivity does not provide it. [1]

Consider a desktop workflow building an application on the UM790 and then asking a production host to publish it. The UM790 could expose a build capability from a managed environment. The production host could expose a deployment workflow that performs its own checks. The originating daemon owns the outer workflow; the production daemon owns its internal run. Their records should link rather than collapse into shared mutable state.

Publishing a workflow requires explicit input/output meaning, a version, a way to inspect the accepted execution, and truthful failure and cancellation behavior. A caller should not unknowingly receive new internal behavior after a published version was selected. This is a proposal; current peer execution is not proof that workflow publication exists.

Permission to discover or invoke that deployment is not permission to edit it, replace its tools, read its credentials, or republish it. These should use the existing authority approach rather than creating an unrelated security system for each role. [1]

Within one organization, a shared identity and administration experience can make those permissions convenient. Each host should still verify commands and retain its own ownership. A shared project name or network does not imply unrestricted authority. Credentials should remain with the host that uses them where practical.

The GUI can expose internals to authorized editors and only the published operation to ordinary consumers. This preserves reusable service boundaries without locking colleagues into opaque boxes.

## Working files, installed tools, and things that outlive a task

The physical workspace should be a normal filesystem usable by ordinary tools. Keep live editable files distinct from Milkdrift's selected evidence and immutable artifacts; reuse existing artifact, context, and secret-reference mechanisms rather than inventing a second LLM storage system. The current secret adapter resolves configured files or environment variables; it is not an encrypted vault. [1][5][6]

The distinction that matters for a service is **managed here or connected externally**. A managed llama-server belongs to this setup and can be replaced or removed by its owner. An attached server is a dependency the setup may call but does not own. Independent installations are acceptable when independent versions or lifecycles are useful. Sharing an existing endpoint is also acceptable; it should not silently transfer administration rights.

A deployment workflow can finish while its application and database continue running. Their supervisor, data, ownership, and later removal path must therefore be recorded independently of the initiating task. Stopping a task is not automatically permission to stop a shared service.

Cleanup can promise to remove owned installations, services, temporary files, and selected data. It cannot promise to undo arbitrary changes through a writable host mount or an external API. Shared prerequisites and externally created resources need explicit treatment. Preserve source files, useful data, and credentials according to policy; deleting a local credential does not revoke it remotely.

Reproduction requires recorded tool versions, recipes, and relevant configuration. A documentation page cannot reconstruct unrecorded package changes. Equally, a recipe cannot reconstruct a database's current contents or guarantee identical behavior across hardware. Configuration, working data, and recovery evidence are related but different.

## Autonomous maintenance without a second orchestrator

Agents may build tools, replace service configurations, and advertise new capabilities within delegated scope. The existing vision already proposes checking source identity, requested permissions, behavior, documentation, and removal before publishing a new generation. Use that direction rather than silently treating every installed executable as trusted. [1]

Generation changes should affect future selection, not rewrite the implementation identity of already accepted work. Retain enough information to distinguish a failed candidate, an active service, and an uncertain replacement after a lost response. Some hosts cannot run old and new models simultaneously; the procedure should permit a declared interruption rather than assume spare capacity.

Updating an agent's prompts, workflow, tools, or worker software is different from changing its grant. Recovery should not depend solely on the worker being replaced. A worker that can rewrite the supervising daemon and its credentials is not separated from that supervisor.

Each environment should have a discoverable documentation entry point for purpose, operating instructions, decisions, and known limitations. Knowledge updates should preserve their source and applicability through existing context/artifact mechanisms. Retrieved instructions are evidence, not new authority, and updating a document should not alter the recorded context of completed work.

Persistent services use an appropriate service supervisor. Multi-step installation, verification, and remediation can use ordinary Milkdrift workflows where needed. Do not create an environment-specific scheduler simply to make setup automatic.

## Evaluate the whole proposal together

The implementation order remains a question for a separately authorized assignment. The following cases explain dependencies without committing to a new subsystem.

**Prepared local work.** Establish the UM790 setup, build an application, keep its useful output, restart its services, and remove the disposable parts. Also use an existing external endpoint. This tests convenience, separation, and cleanup without forcing every resource into the managed area.

**Remote work from one interface.** Let another daemon use the UM790's capabilities. Inspect the same work centrally while the UM790 keeps its credentials and accepted execution records. Reconnect without duplicating a deployment. This tests whether role separation reduces operational burden rather than multiplying it.

**A published workflow.** Expose a versioned deployment operation, permit a colleague to edit its internals through the shared interface, and deny that edit to an invoke-only caller. This tests the proposed distinction between a simple execution host and a host that owns workflows.

**A different execution method.** Include a native application or remote agent alongside the protected build environment. It must not require weakening the build environment or pretending the native tool has protections it lacks. This tests whether the design complements new tools instead of constraining them.

The strongest smaller alternative is existing execution capabilities plus well-supported setup recipes and a common installer. That may solve repeatable setup without a long-running environment-management API. A dedicated manager earns its cost only if inspection, lifecycle, remote administration, and removal remain materially difficult through that smaller approach.

Questions to resolve together: what does an execution-only host retain; which setup guarantees are worth supporting; where must OS enforcement remain independent; how are workflow versions published; and what same-organization access experience avoids repeated configuration without erasing host authority? These questions should shape the design before deciding package boundaries.

## Carry this discussion forward

Record this as one whiteboard discussion, with planning state only in the whiteboard overview. Do not copy the whole discussion into vision, architecture, and roadmap. Accepted product intent belongs in vision; accepted ownership rules belong in architecture; authorized work belongs in the roadmap or assignment; verified implementation facts belong in status. [3][4]

When a later decision changes direction, identify what it retains, supersedes, or rejects and why. “Not in the next implementation” must not silently mean “discarded.” Future contributors should be able to recover the reasons behind lightweight hosts, optional separation, shared administration, and open-ended tool integration without rereading the chat.

Contribution: 2026-09-16, `Birch-20260916-a` (agent pseudonym), synthesizing the user's discussion and checking the cited documents. No independent review, host experiment, repository modification, or implementation is claimed.

## Sources

Repository references are pinned to the reviewed commit. They support the existing facts above, not acceptance of the proposals.

1. [Product vision: workspaces, external capabilities, execution trust, peers, self-extension, authority, and daemon roles](https://github.com/hartolit/milkdrift/blob/64b4c5fa2b218dc59eef63fc44af8e094a8fb430/docs/product/vision.md), especially sections 14 and 16–24.
2. [Current status and implementation limits](https://github.com/hartolit/milkdrift/blob/64b4c5fa2b218dc59eef63fc44af8e094a8fb430/docs/product/status.md).
3. [Current roadmap](https://github.com/hartolit/milkdrift/blob/64b4c5fa2b218dc59eef63fc44af8e094a8fb430/docs/product/roadmap.md).
4. [Whiteboard procedure](https://github.com/hartolit/milkdrift/blob/64b4c5fa2b218dc59eef63fc44af8e094a8fb430/docs/development/virtual-office/whiteboard/README.md).
5. [Architecture: context, workspace, and artifacts](https://github.com/hartolit/milkdrift/blob/64b4c5fa2b218dc59eef63fc44af8e094a8fb430/docs/architecture.md#context-and-artifacts).
6. [Local secret sources](https://github.com/hartolit/milkdrift/blob/64b4c5fa2b218dc59eef63fc44af8e094a8fb430/adapters/local-secret/README.md).
7. [Podman: rootless mode](https://docs.podman.io/en/stable/markdown/podman.1.html#rootless-mode).
8. [Linux manual: process capabilities](https://man7.org/linux/man-pages/man7/capabilities.7.html).

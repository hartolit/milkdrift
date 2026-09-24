# Prepare and maintain a Linux working environment

A managed installation gives an agent a persistent working volume and a bounded execution
capability. Optional llama-server runs under user systemd, so closing a client or stopping Milkdrift
does not stop the service. Resource commands record intended changes before asking Podman or
systemd to act. Inspection distinguishes accepted intent, verified state, blockers and uncertainty.

For the maintained example, start with the [Slotbook inputs](../../examples/managed-linux/README.md). The recipe supports a
preloaded exact tool image, a model-free setup, an external endpoint attachment, or an owned
llama-server with exact local weights. A worker can install experimental tools under
`/workspace/home`, edit source and keep outputs. Those files remain mutable working data; publishing
an immutable tool or application generation requires explicit promotion. Protected publication is
not established by this setup.

## Prepare the host and configuration

The mechanism requires a non-root Linux account, Podman 5.4 through 6.x, user systemd, cgroup v2
with delegated CPU, memory and PID controllers and preloaded exact images. Reserve at least 65,536 subordinate UIDs and
GIDs for a worker, or 131,072 of each when an owned model service and worker run together.
An owned persistent service also requires operator-enabled user lingering. `prepare` checks these
facts, available shared RAM, free storage, and exact model inputs. An unsupported version or missing
protection refuses; there is no privileged-container or native-process fallback. The adapter uses
`/usr/bin/podman`, `/usr/bin/systemctl`, `/usr/bin/loginctl` and the installed Quadlet generator.

Install prerequisites through your operating system before bootstrap. This command creates private
Milkdrift configuration and a credential file; it does not install packages, enable lingering, change
SSH/NetBird/firewall settings or start any service. Select a fresh absolute product directory and a
subdirectory of the account's normal rootless Quadlet search directory and the account's systemd user unit search directory. Review the preview first:

```sh
milkdrift-daemon managed-bootstrap --root /home/operator/milkdrift-slotbook --quadlet-directory /home/operator/.config/containers/systemd/milkdrift --systemd-directory /home/operator/.config/systemd/user --recipe /home/operator/approved-slotbook.json --preview
milkdrift-daemon managed-bootstrap --root /home/operator/milkdrift-slotbook --quadlet-directory /home/operator/.config/containers/systemd/milkdrift --systemd-directory /home/operator/.config/systemd/user --recipe /home/operator/approved-slotbook.json
milkdrift-daemon --config /home/operator/milkdrift-slotbook/daemon.toml --check-config
milkdrift-daemon --config /home/operator/milkdrift-slotbook/daemon.toml
```

The output names the generated config, credential file and canonical `b3_…` recipe digest. Bootstrap
replay preserves the credential and requires existing generated files to match exactly. It refuses
conflicting files. Keep the recipe and manager directories private; symlinks, unsafe ownership,
group/other-writable inputs and multiple managers for the same root refuse startup. Owned platform names and labels also bind
the machine, account and manager root, so identical request keys in separate stores cannot adopt
each other's resources. The daemon
loads at most 32 approved recipes. Model recipes also require the existing provider contract for
finite token accounting and explicit billing; unknown terms refuse instead of advertising a model
that the independent host cannot admit. To approve another recipe, add its absolute private file
to `adapters.managed_linux.recipes` under a distinct recipe name, validate configuration, and restart.
The installation name remains stable across these recipe updates. Bootstrap defaults the installation
name to the initial recipe name; `--installation` selects another name explicitly. Updates select its exact
name and digest; caller-supplied mounts, engine flags and unit directives are not accepted.
Obtain the new normalized digest with a preview of that file, retaining the installation name:

```sh
milkdrift-daemon managed-bootstrap --root /home/operator/milkdrift-slotbook --quadlet-directory /home/operator/.config/containers/systemd/milkdrift --systemd-directory /home/operator/.config/systemd/user --recipe /home/operator/approved-slotbook-v2.json --installation slotbook --preview
```

Use the printed recipe reference for update. Compare its proposed filesystem/network scopes with
the current operator grant when changing a model path, endpoint or networking. Add only the needed
scopes to the existing daemon configuration along with the approved recipe path. Preview writes
nothing; rerunning creation is not the procedure for modifying an existing configuration.

Set `MILKDRIFT_TOKEN_FILE` to the generated credential path and `MILKDRIFT_ENDPOINT` to the daemon's
loopback URL, following the ordinary [client configuration](../../examples/operator/README.md).
For a remote host use an authenticated tunnel to loopback. A model attachment may independently
use NetBird, but the provider contract still requires HTTPS for non-loopback endpoints. An explicit
loopback development profile accepts HTTP. Neither choice exposes Milkdrift's control listener.

## Choose operating budgets

Recipe schema 2 separates `worker_limits` from an owned model's `limits`. Each contains memory
bytes, CPU quota in hundredths of one CPU (`250` means 2.5 CPUs), a process/thread limit and `/tmp`
bytes. Swap is disabled. `/tmp` is memory-backed and consumes that container's memory allowance.
Preparation requires memory and temporary-storage byte values to be multiples of the actual host
page size, so kernel rounding cannot silently change the selected cap. These are two independently
enforced container caps. They do not create an aggregate cgroup or
reserve memory. An attached service retains its external owner's resource policy.

Preparation compares simultaneous worker plus owned-service demand with observed `MemAvailable`.
On reapply or replacement it credits anonymous resident memory of the exact current owned service;
reclaimable file cache is already included in `MemAvailable` and is not credited again. A changed
container identity makes that observation fail. Apply/update repeat the check against the saved
current generation before effects. Headroom can still change after the check; it is not a promise
that another process cannot consume RAM. UM790 CPU and iGPU use the same physical pool.

Choose `task_timeout_ms` and combined stdout/stderr `output_bytes` for worker commands. Capture
stops on overflow; it does not silently truncate successful output. Admission reserves the worst
encoded result size (including JSON escaping) from the same result representation used to publish
it. Larger recipe budgets also need suitable actor, workflow/serving and artifact allowances in the
daemon configuration. These existing authorization ceilings remain independent; bootstrap does not
expand them to authorize every recipe value. A CLI wait has its own observation deadline and does
not extend or cancel accepted work.

For an owned server, `timeouts.model_verification_ms` bounds model hashing, `startup_ms` bounds
readiness polling and the generated systemd startup wait, and `shutdown_ms` controls graceful stop.
Podman takes whole stop seconds, so the grace is rounded up. Startup milliseconds must also fit
the [systemd time parser](https://github.com/systemd/systemd/blob/main/src/basic/time-util.c)
without reaching its reserved infinity boundary. Hashing checks the deadline between
regular-file reads; it cannot interrupt a stalled kernel filesystem read. Administrative commands
and protection probes retain separate short bounds; making a task budget tiny does not disable
platform verification. Model `endpoint_limits` uses the existing provider contract for both owned
and attached endpoints. The current blocking transport applies the smaller of request and idle
limits to the whole request; it does not reset an idle timer after each fragment. Owned inference
verification uses the same configured endpoint deadline and response bound.

The Linux recipe no longer imposes the old 128 GiB memory/model, 128 CPU/thread, one-hour task,
131,072-token context, or 1 MiB worker-output ceilings. Positive representable values are still
required. The remaining restrictions have these owners and purposes:

| Restriction | Reason and consequence |
| --- | --- |
| Container memory and model bytes fit signed 64-bit bytes; context and explicit Vulkan layer count fit positive signed 32-bit arguments; shutdown fits Podman's signed whole seconds | Preserve the representation accepted by OCI/llama-server/Podman. A positive value does not prove the host or model can run it; prepare and physical verification remain necessary. |
| Positive finite durations, checked memory/output arithmetic, `/tmp` no larger than its container memory | Refuse disabled limits, unrepresentable waits and storage/output promises that the configured boundary cannot cover. |
| One worker mutator and one concurrent model generation per installation | Preserve the existing working-area exclusion and single-server-slot policy. Different resources can run concurrently. This is product policy, not a measured hardware limit. |
| 65,536 private subordinate IDs for each simultaneous worker/service | Covers the image's 16-bit Linux UID/GID space without mapping the manager identity; owned service plus worker needs two complete ranges. |
| At most 32 approved recipe paths; recipe at most 64 KiB with bounded canonical JSON | Bounds the trusted startup catalog and retained deployment input. These are document/catalog policies, not workload capacity estimates. Worker argv uses the existing bounded inline-input contract, without an additional Linux-specific argument count cap. |
| Safe absolute paths; owned alias is one identifier starting with a letter/digit, using letters, digits, `/ : . _ -` | Generated units accept no interpolation or raw directives. Commas would declare multiple llama aliases. Provider validation supplies the alias length and endpoint/accounting bounds. |
| Administrative command capture at most 1 MiB and 45 seconds, shorter existence/generator/cleanup checks, finite 50-second in-process fencing wait | Bounds manager-side observation and cleanup. Exceeding these mechanism limits yields an explicit failure or retained uncertainty; it does not authorize a weaker check. These are administrative policies, not inference deadlines or throughput claims. |

The [maintained inputs](../../examples/managed-linux/README.md) show complete service configuration.
Use host-appropriate limits; fixture and example values do not establish model memory requirements.

Recipe schema 1 and `linux-quadlet-v1/v2` deployments are unsupported by this correction. Schema 2
and mechanism `linux-quadlet-v4` bind the separate limits, alias and initialization semantics into
new approvals. Daemon configuration uses version 13 and portable inventory remains version 1.
Do not edit saved inventory or reinterpret an old approval. Before upgrading an active older
installation, use its matching binary to inspect, preserve and remove it; retain its labeled volumes
and shared inputs. Approve a schema-2 recipe in a fresh installation namespace. If already upgraded,
the offline `managed` inspection/backup path remains available; return to the matching binary for
lifecycle changes. This is explicit development-format refusal, not automatic migration or adoption.

## Apply, inspect and use

Substitute the digest from bootstrap. Installation versions change when uses or lifecycle state
change; obtain a fresh version from inspection before each new mutation. A new installation starts
at expected version zero.

```sh
milkdrift --json --command-id prepare-one resource --installation slotbook prepare --recipe slotbook --digest RECIPE_DIGEST
milkdrift --json --timeout-secs 600 --command-id apply-one resource --installation slotbook --expected-version 0 apply --recipe slotbook --digest RECIPE_DIGEST
milkdrift --json --command-id inspect-one resource --installation slotbook inspect
milkdrift --json invocation catalog
```

`prepare` diagnoses and previews; it reserves nothing. Check its returned `state`: `prepared` means
the observation passed, while `unprepared` carries bounded failure diagnostics after authorization.
For example, insufficient headroom identifies the required and observed bytes. It neither changes
an existing generation nor records a transition. Host conditions can change before apply. `apply` returns an immutable **acceptance
receipt**, even when the bounded platform driver has since finished or recorded uncertainty.
Inspect to obtain the current version, last verified generation, desired/observed service state,
capability candidates, exact owned/shared/attached resources, blockers and pending transition.
Replaying `apply-one` returns its original receipt without repeating effects. Changing its body or
installation under that actor-scoped command ID conflicts, including after removal. Use a new ID
for a genuinely new request. After a timeout, replay the exact saved request or inspect; a timeout
does not cancel the recorded change.

Successful verification registers `managed.slotbook.worker` (`workspace.execute`) and, when selected,
`managed.slotbook.model` (`model.generate`) in both host roles. The registry is a rebuildable projection;
its catalog and health show current availability. The worker accepts the named inline input `command`:

```json
[{"name":"command","value":{"type":"inline","value":{"argv":["/usr/local/bin/initialize-slotbook"]}}}]
```

For the maintained Slotbook image, this creates the application directories and initial brief;
repeated initialization preserves edits. Another image supplies its own ordinary commands. Save
that JSON as `worker-inputs.json`, then use ordinary durable direct execution:

```sh
milkdrift invocation prepare managed.slotbook.worker workspace.execute --host host:slotbook --request-id worker-one --inputs worker-inputs.json --output worker-request.json
milkdrift invocation submit worker-request.json
milkdrift --timeout-secs 360 invocation wait EXECUTION_ID
```

For a managed model call, follow the ordinary [fresh-model input example](../../examples/operator/README.md):
upload the model task, name its input `milkdrift.model_task`, and prepare
`managed.slotbook.model model.generate` on `host:slotbook`. The same CLI preparation binds the
advertised allowance; the provider's finite accounting contract is checked against it before entry.

The process result artifact includes bounded output and exit status. Workflow tasks select the same
operation and descriptor generation. Lifecycle calls themselves are also available through
`milkdrift.resources` / `resource.manage`: supply the same managed request document as the inline
input `request`. Inner installation permission checks run beneath both transports.

## Maintain, recover and remove

Maintenance refuses busy installations immediately; it creates no waiting queue. Each installation
has one platform driver; competing mutations before receipt acceptance return busy, while saved
receipts can replay and inspection remains available. At most 32 installation drivers run at once. A successful
acceptance closes new admission before effects. An update requires explicit interruption because
old and new services can share a port or GPU. It retains the old approved generation until the
candidate passes verification. Old accepted work is never redirected to new bytes.

```sh
milkdrift --command-id stop-one resource --installation slotbook --expected-version 7 stop
milkdrift --command-id start-one resource --installation slotbook --expected-version 12 start
milkdrift --timeout-secs 600 --command-id update-one resource --installation slotbook --expected-version 18 update --recipe slotbook-v2 --digest NEW_DIGEST --allow-interruption
milkdrift --command-id inspect-two resource --installation slotbook inspect
milkdrift --timeout-secs 600 --command-id recover-one resource --installation slotbook --expected-version 22 recover
milkdrift --command-id preserve-one resource --installation slotbook --expected-version 29 preserve
milkdrift --command-id remove-one resource --installation slotbook --expected-version 31 remove
```

The numbers illustrate optimistic guards; use the actual inspected version. Identical reapply checks
protection without restarting a healthy service or overwriting retained files.
Drift is reported and never silently adopted. Pending steps after a daemon crash resume against their
saved identities. A recorded uncertain step needs an authorized `recover`; the same step may already
have happened, so recovery checks labels, exact definitions and physical state first. A failed update
keeps admission closed and candidate diagnostics visible. Recovery requires permission for that
exact candidate, including its replacement paths and network requirements. It does not roll back
data automatically.

`preserve` keeps owned volumes on removal. Add `--delete-on-removal` only when their contents are
deliberately disposable. Service start checks existing container ownership and disables automatic replacement. A private systemd drop-in
overrides Quadlet cleanup to use the exact container ID written by that service, so a failed start
cannot delete another container with the same name. The effective stop commands are verified before
activation; the declared systemd directory must be in the user manager's unit search path. Removal stops owned services, removes their exact definitions and deletes
only owned volumes whose saved policy permits it. Shared images/toolchains/model files and attached
endpoints remain untouched. A removed namespace is a retained tombstone and cannot be reused.
Preserved volumes keep ownership labels and identities in that tombstone for deliberate offline
handling; no automatic adoption or global prune exists.

Blockers bind exact accepted local attempts or serving operations. Queued work already holds its
generation. Entered uncertainty survives cancellation, lease expiry, loss of the client, archival
and restart. An authorized operator can fence the exact use:

```sh
milkdrift --timeout-secs 120 --command-id resolve-one resource --installation slotbook --expected-version 40 resolve --use-id EXACT_64_HEX_USE_ID --expected-claim 2
```

Resolution first commits a newer fencing claim, preventing late entry. It then waits at most 50
seconds for an in-process creator to leave and verifies/removes the exact task container. Resolving
an owned model use stops the owned service and changes desired state to stopped; conflicting users
must settle first. An attachment resolution relinquishes only this host's dependency after its local
request owner leaves; it cannot stop the remote server. Neither path manufactures an execution result.
A lost fence reply leaves a retained blocker; inspect its new claim before another resolution.

Editing handoff/return additionally require exact parent/child uses, generation, both claims and the
accepted `parent-run:event-sequence` association. The parent must have physical stop evidence before
handoff. Both retain lifetime holds; only the child can edit. Return requires child stop evidence
and, for `--resume-parent`, the parent's current authority and uncancelled execution. The returned
parent receives a new claim. Old callbacks cannot enter. A cancelled parent can settle a child
without resuming, then resolve/release its remaining hold. These resource transactions are available
through the normal resource command path. Published calls use their saved invocation association
instead of a subworkflow event. Their wrapper has never entered a physical writer: the same
acceptance/entry transaction records that evidence, and only the exact linked service child can
receive its editing claim. Actual worker children still require physical stop/fencing evidence.
The wrapper cannot release a suspended hold merely because its internal workflow became terminal.
See [published methods](../guides/published-methods.md) for service authority and recovery.

## Protection and retained state

The manager account owns configuration, credentials, store, Quadlets, exact service cleanup drop-ins
and engine/systemd sockets. Existing files in the shared systemd search directory are not overwritten.
Temporary workers receive one owned volume at `/workspace`, a read-only image root, private user
mapping, dropped capabilities, no-new-privileges, default seccomp, bounded memory/swap/CPU/processes
and a bounded `/tmp`. The apply probe checks actual mounts, kernel limits, effective capabilities,
read-only denial and absence of manager paths/sockets before publication. Application tools and
initial files belong to the chosen image and ordinary
worker commands; the [Slotbook initializer](../../examples/managed-linux/README.md) is one example.
Helpers have finite output/deadlines and retained child ownership through termination. Persistent
services remain systemd-owned across Milkdrift shutdown; temporary tasks remain invocation-owned.

`worker_network = "none"` supplies no external interface. `outbound` explicitly permits unrestricted
destinations through rootless pasta without gateway mapping. It does not promise destination
filtering. Workers receive no host devices; the optional owned Vulkan server receives exactly its
approved render node. Rootless operation is only one part of this boundary. Native trusted profiles
still have the manager account's privileges, and the manager/operator itself is trusted. Do not run
untrusted native code under that identity and describe it as this worker boundary.

Inventory is limited to 1,024 installation namespaces including tombstones, 16 resources per
installation, 128 unresolved uses, and 16 transition steps. Capacity refusal is explicit. Startup
reads bounded live inventory; paged integrity scans also inspect historical receipts/transitions
and execution indexes. Exact receipts and transition evidence remain durable and consume disk;
there is no automatic historical pruning. Serving observation archival does not erase resource
ownership, unresolved use or command replay.

A store backup includes inventory, approved generated configuration, receipts, holds and lineage.
It does **not** include Podman volumes/images, host Quadlet files and cleanup drop-ins, external model files, credentials
or attached-service data. Preserve those separately under their owners' quiescence/backup procedures.
Offline `managed` inspection remains available when ordinary startup fails. Restore retains the
existing inspection-only marker and refuses execution, and the manager root has an exclusive lock.
There is no supported clone activation/adoption procedure that silently seizes the original host's
resources; removing a guard by hand is outside the supported restore path.

## Platform qualification

The deterministic suite covers lifecycle faults before/after platform effects and durable commits,
shared adapter behavior, exact use transactions, handoff/cancellation/restart and guarded backups.
Those tests establish no container or hardware isolation. Run the actual Linux lane only with an
explicit private recipe and a private directory inside systemd's rootless Quadlet search path:

```sh
MILKDRIFT_LINUX_RECIPE=/home/operator/approved-slotbook.json MILKDRIFT_QUADLET_TEST_PARENT=/home/operator/.config/containers/systemd/milkdrift-tests MILKDRIFT_SYSTEMD_TEST_DIRECTORY=/home/operator/.config/systemd/user cargo test -p milkdrift-managed-linux --test linux_mechanism --all-features -- --ignored --exact real_linux_worker_conformance_reapply_restart_and_preserved_removal --nocapture
```

The lane invokes real Podman, checks worker enforcement, runs common worker conformance, preserves
files and an installed tool across reapply/reopen, and removes with preserved-volume inspection.
With an owned recipe it checks the service's actual kernel limits, proves identical reapply retains
its container ID, and kills that test container after dropping every Milkdrift owner. Systemd must
restart it and pass health checks before Milkdrift reopens. A new container after that deliberate
crash is expected; reopening Milkdrift must not create another one.

Set `MILKDRIFT_SYSTEMD_TEST_DIRECTORY` to the account's systemd user unit search directory for both
physical tests. Optionally set `MILKDRIFT_LINUX_EVIDENCE_PARENT` to an existing private evidence
parent. The lifecycle lane retains its store and prints its evidence directory, Quadlet directory
and preserved volume identities, including on failure. Inspect/recover against those exact records;
never substitute a prune command for cleanup.

The separate collision regression starts an owned generated unit while a foreign test container
already holds its name, then verifies the failed start leaves that container's ID and running state
unchanged:

```sh
MILKDRIFT_LINUX_RECIPE=/home/operator/approved-slotbook.json MILKDRIFT_QUADLET_TEST_PARENT=/home/operator/.config/containers/systemd/milkdrift-tests MILKDRIFT_SYSTEMD_TEST_DIRECTORY=/home/operator/.config/systemd/user cargo test -p milkdrift-managed-linux --lib --all-features real_quadlet_failed_start_preserves_a_foreign_container -- --ignored --nocapture
```

Arch Linux with Podman 6.1.2, systemd 261.3 and kernel 7.2.6 has passed worker and owned CPU-model
lifecycle tests, supervisor recovery without Milkdrift, actual kernel-limit checks and the collision
regression. The daemon/CLI evidence and exact inputs are recorded in the
[assignment handoff](../development/virtual-office/adaptive-hosts/handoffs/02.md).
This does not qualify a machine reboot, power loss, managed Vulkan, or UM790 memory pressure; the
[hardware qualification issue](../development/virtual-office/whiteboard/issues/managed-linux-hardware-qualification.md)
records the remaining work. Windows/macOS continue to refuse this Linux mechanism.

The installed 6.1.2 Quadlet and container manuals and generator were checked against the
[upstream Quadlet documentation](https://docs.podman.io/en/latest/markdown/podman-systemd.unit.5.html).
Podman 6's generated name-based removal is deliberately replaced by the ID-bound drop-in.
The adapter accepts 5.4..6.x and checks realized mappings, limits and generated/effective definitions;
other major versions require mechanism review. The executed host qualification is specifically 6.1.2.


## Protected application candidates

Use a `protected_service` recipe when served application bytes must satisfy fixed checks before
activation. This distinct schema-1 recipe pins an image/interpreter, kernel limits, application
configuration, credential digest, trusted verifier source/native runtime and version-1 effect policy.
The policy fixes the producer, check names, maximum candidate bytes and validity interval. It is
committed by the blueprint agreement. Operator files stay outside the editable worker's mounts.
The [adaptive Slotbook example](../../examples/adaptive-slotbook/README.md) generates complete input
files and runs the supported authoring, failure, repair, verification, publication and reopen path.

Initial `resource apply` creates an empty protected target without starting application code.
`resource evaluate` reads an exact authorized artifact and records incomplete evidence before
running the configured verifier. `resource evidence` returns the durable observation; the acceptance
receipt itself remains the original incomplete record for exact replay. `resource publish` consumes
only trusted passing evidence for the current target generation and publishes those immutable bytes.
`resource inspect` exposes `accepted_evaluation` after installation; use that identity with the
evidence command to inspect candidate/configuration/check applicability. A receipt accepts an intent;
verify `pending`, `desired_running` and `observed_running` before claiming deployment completion.

Raw start requires an accepted unexpired candidate; update cannot replace a protected policy or
candidate. Stop, preservation and removal retain their existing meanings. After an uncertain effect,
inspect and explicitly recover the saved intent with current authority; exact replay never initiates
new work. A changed policy, verifier, credential, candidate, target or generation requires new evidence
or a distinct approved target. An expired evaluation cannot authorize new controlled entry. Systemd
can restart the already accepted immutable generation with the same data; this does not erase old
failures or manufacture a new acceptance.

Protected preparation, evaluation and publication use the same supported Podman-version, delegated
CPU/memory/pids controller, private UID/GID mapping and persistent-user-session checks as the other
managed services. The protected service and its verifier need two simultaneous private namespaces.

Verification scratch belongs to each accepted evaluation request. A new authorized request may
reevaluate the same immutable candidate and target generation; this permits renewal after expiry
without changing bytes or replacing the old observation. Exact replay still returns its original
receipt. The Linux owner removes the evaluation's exact labeled container after verifier completion
or timeout, and fences leftovers before startup opens admission. Cleanup failure retains an unknown
result. Known refusals before an accepted managed intent produce a rejected invocation with their reason.
Failures after an accepted intent preserve uncertainty. The configured `verification_timeout_ms`
bounds the verifier process; fencing uses the
separate bounded administrative-helper timeout. Preserved scratch remains available for inspection.
Both bare image IDs and registry-qualified digest references resolve through Podman's local image
identity; verification and service observation compare the actual container against that identity.

Managed worker schema 2 supports `stdout_artifact: true` in `command`. On successful execution it
publishes raw bounded stdout separately from `worker_result`. The raw artifact is suitable for an
immutable source candidate; it is not trusted verification evidence. Mechanism `linux-quadlet-v4`
binds this contract, and older worker mechanisms require a new installation rather than silent reuse.
The worker's normal output cap applies to both capture and admission accounting.

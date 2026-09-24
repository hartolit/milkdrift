# Publish and invoke a governed method

A publication lets another caller invoke a reviewed workflow without permission to edit its graph
or call every internal operation. Select an exact capability generation and retain the request
document for replay. The owning host creates one internal run and keeps that identity through
restart, cancellation, retirement and result recovery.

## Configure the owner

Use a workflow-enabled daemon with `runtime.controller_activation = "enabled"`. In its schema-13
configuration, map each public capability to an existing actor grant:

```toml
[runtime.publication_services]
"method:slotbook" = "grant:slotbook-service"
```

The configured grant must allow creation and start in the method's workflow lineage, its internal
capabilities and required resources, and the workspace/artifact reads needed for declared results.
Keep that grant scoped to the reviewed service. Publication does not combine the caller's and
publisher's grants. An execution-only host refuses this configuration; it can still consume a
publication on another workflow-enabled host through an authorized peer relationship.

Import the immutable starting revision, including its governing agreement, through ordinary
blueprint control. Construct the schema-1 `PublishedMethod` document defined by
[the owning contract](../../crates/persistence/src/published.rs). Its descriptor exposes exactly
`method.invoke`, category `tool`, local ownership, and the maximum side-effect class of all work
the agreement permits. The document supplies:

- The exact revision and agreement digest, documentation, service actor and immutable grant facts.
- Every required public input, mapped to the identically named workflow input. JSON choices are a
  finite reviewed list; artifact inputs specify a media type and maximum byte count. The caller
  needs actual current read permission for supplied references, and content is validated.
- Public output names mapped to declared terminal workflow fields, with media types and byte
  ceilings. Intermediate workspace values and unrelated artifacts are not public results.
- A workspace budget, cumulative internal allowance, maximum outstanding calls, nesting depth,
  and duration. Zero model/process/token dimensions prohibit that work. No currency means no
  monetary allowance; supported unbilled adapters still consume their other dimensions.

Obtain grant identities and digests through the authorized `daemon authority` read. Do not invent
grant digests or copy credentials into method documents. Internal adapters must already be installed;
publication refuses unsupported capability envelopes or service authority. The first generation is
1. Changes to implementation, constraints or authority require a new consecutive generation, even
when the descriptor's input/output schema version remains compatible.

## Publish, replace and retire

```sh
milkdrift --yes --command-id publish-slotbook-1 method publish method-v1.json
milkdrift method show method:slotbook --generation 1
milkdrift method list --limit 32
milkdrift --yes --command-id publish-slotbook-2 method publish method-v2.json --expected-previous-version 1
milkdrift --yes --command-id retire-slotbook-1 method retire method:slotbook --generation 1 --expected-version 1
```

Generation identifies immutable implementation; the returned record's `version` guards publication
state changes. Use its observed version for replacement and retirement. Reusing a generation or
command key with different bytes conflicts. Registry capacity is checked before committing a new
publication. Retained inventory is finite; reaching a bound refuses new publication instead of
silently discarding old evidence. Retirement closes new selection and preserves accepted work.

Administration requires `AdministerCapabilities` with exact `method.publish`, `method.inspect` or
`method.retire` capability-operation scope. Method inspection includes protected implementation
facts; callers use `invocation catalog` for the public descriptor and its derived documentation,
input rules and output limits. A method inventory page requires inspection authority for every
record it contains. Ordinary discovery filters by the caller's capability scope.

## Invoke and read results

```sh
milkdrift invocation catalog
milkdrift invocation prepare method:slotbook method.invoke --host host:slotbook --request-id deployment-1 --inputs inputs.json --output invocation.json
milkdrift invocation submit invocation.json
milkdrift invocation lookup deployment-1
milkdrift --timeout-secs 240 invocation wait EXECUTION
milkdrift invocation output EXECUTION ARTIFACT result.json
```

Preparation freezes the selected generation, catalog, inputs, deadline and limits in the saved
request. Inspect that document before submitting it. Exact replay recovers its acceptance, including
an archived summary; changing its bytes under the same key conflicts. A different authenticated
caller cannot use the receipt. A transport timeout or disconnect does not cancel accepted work.
`invocation cancel` records a separate request; its acknowledgement is not proof of termination.

Invocation needs `InvokeCapability`; receipt inspection needs `Inspect`, result references/bytes
need `ReadCapabilityOutput`, and cancellation needs `CancelCapability`, all scoped to the public
operation. The invoker preset includes these public operations. Its result permission grants only
the retained terminal outputs of that caller's own execution. It does not grant general artifact,
workspace, run, blueprint, secret, publication or resource access. Downloads are bounded and verify
the immutable size and digest. Public copies are classified as restricted; publication never lowers
the source's sensitivity or grants access to unrelated internal artifacts.

Authorized colleagues use ordinary run reads, timelines and proposals at the owning host. A run
read's `published_source` links the internal history to its public caller/execution or local attempt.
Prospective adaptation remains subject to the accepted agreement and service grant. Editing another
library generation does not repin an accepted call. Protected deployment still requires the same
resource owner's verifier evidence, including when someone tries a lower-level operation directly.

## Recovery and limits

The local attempt journal or serving record saves canonical create/start commands before the child
exists. Runtime owns their receipts and the child's history. A lost reply replays those commands;
it never allocates another child. Pending calls release their execution thread, retain exact
generation/resource ownership, and progress through bounded continuation pages. Publication
ancestry is bounded and self-calls are refused before another child is created. Each ancestor's
accepted nesting ceiling remains in force, including across peers and restart. Selecting a more
permissive descendant cannot raise it.

Internal entry rechecks the public caller, cancellation and deadline before and after local
preparation, alongside the service grant. Expiry or revocation prevents future entry; it does not
prove that an already entered process has stopped. Incoming calls use their serving owner's current
client/peer policy. Policy changes generally require validated host restart as described in
[peer operations](../operations/peers.md).

The internal allowance applies across descendants and revisions. A calling account reserves the
whole supported envelope before entry and settles attributable use once. Unknown use stays reserved;
a new run, retry or restart cannot erase it. Host concurrency, serving queues and service grant
ceilings also apply. There is no separate service-wide lifetime spending account. Process-internal
network/model calls are not direct-model usage; constrain worker network access when the advertised
contract depends on mediated accounting.

Internal success, agreement satisfaction, public result publication and a deployed service are
separate facts. Failed result copying/recording leaves the invocation pending with its exact child;
recovery retries publication rather than the method. A missing implementation or revoked service
does not select another generation. Cleanup retains ownership until the existing runtime/resource
owners can establish the outcome. Uncertain managed writers keep editing and lifetime blockers;
neither cancellation acknowledgement nor elapsed time authorizes reuse or deletion.

See [ADR 0041](../decisions/0041-published-method-invocation.md) for transaction ownership and
[managed operations](../operations/managed-linux.md) for physical inspection and resolution.

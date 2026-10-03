# Evaluate a reusable method from selected experience

A repaired product can suggest a better way to work, but it does not establish that the new method
will help on another input. Milkdrift keeps those decisions separate. An operator selects permitted
source evidence and fixes a comparison before a model proposes a change. Independent executions
then establish eligibility, rejection, or insufficient evidence. Publication is a separate command
under current authority. An eligible result is evidence for that finite comparison, not a claim of
general model improvement.

The maintained [Slotbook specification](adaptive-method-example.md) supplies the source problem,
four paired inputs, unchanged checks and improvement threshold. The Rust `slotbook-evidence learn`
driver in [the example](../../examples/adaptive-slotbook/README.md) authors and executes the workflow
through the same CLI operations available to operators and agents. Its native implementation and
repair are seeded fixtures; an external model supplies the actual proposed method mutation. Keep
this lane separate from a model generating and repairing the application itself.

## Select knowledge deliberately

Each working setup has an ordinary `KNOWLEDGE.md` entry point describing its purpose, operation,
decisions, limitations and approved guidance. Edit it through the authorized workspace worker.
Export the exact bytes as an artifact before selecting them for another task. Editing the file
does not update a prepared invocation's context.

Use named learning commands for the ordinary path. Each result shows its receipt's actor and
command ID; subsequent commands take that pair directly. Select immutable artifacts by ID:

```sh
milkdrift --command-id source-knowledge learning select REVISION --workspace setup --guidance GUIDANCE_ARTIFACT --artifact SOURCE_ARTIFACT --page source-run 1 32
milkdrift learning inspect human:operator source-knowledge
```

Replace the actor with the authenticated actor shown in the result. Repeat `--page RUN FIRST COUNT`
for each selected page, and `--artifact ID` for each extra artifact. The public `select_sources`
request resolves these identities through authorized metadata reads, then uses the same selection
operation as an exact-reference request. No client computes artifact hashes or writes receipt
records. `--supersedes ACTOR COMMAND` and `--approval ACTOR COMMAND` select existing decisions.

`milkdrift learning FILE` also accepts the strict `control::learning::LearningRequest` document
through the ordinary command API for advanced requests. A selection names a revision, managed workspace,
guidance artifact, at most 32 supplementary artifacts and 1–8 exact source pages of at most 64
events each. Page `first` is inclusive. The daemon checks current revision, artifact content,
workspace-value scope and timeline authority, then freezes the public event projection and exact
references in an immutable artifact. It does not copy raw invocation arguments or private branch
content into that projection. Select permitted content artifacts separately.

The result is identified by its authenticated actor and command ID. A later selection can name it
in `supersedes`; the previous artifact and receipt remain exact. Changing the applicable method
requires the matching promotion receipt. `approval` identifies approved learned guidance, rather
than treating a claim inside an editable file as authority. An `inspect` request reads an exact
receipt under current source authority. Access to a callable method's public output does not grant
access to its private implementation history.

For example, after the maintained study, inspect its evaluator's comparison:

```sh
milkdrift learning inspect agent:slotbook-evaluator learning-comparison
```

Inspection shows the full fixed result, including outcomes, paired measurements, reasons and
unknown values in human output as well as `--json`. It never refreshes an earlier comparison.
Repeating an accepted command with the
same ID and bytes returns its original result; changing those bytes conflicts, including after
restart and receipt archival. Use a new ID for a new decision or an updated observation.

## Declare before proposing

The `declare` operation fixes a baseline revision and agreement, selected-source receipt, a new
proposal-run identity, distinct paired input artifacts, reserved caller/request keys, independent
worker and verifier installations, their exact recipe digests and generations, ordered checks,
verifier and effect-policy digests, budgets, threshold and future publication generation. These
facts are immutable. An evaluation-design change requires another declaration before its proposal
run exists. Record effective model/tool provenance and unknown settings explicitly.

Keep the advanced criterion in a `LearningDeclaration` document and submit it before generation:

```sh
milkdrift --command-id fixed-study learning declare criterion.json
milkdrift --command-id proposed-method learning candidate actual-proposal.json --declaration human:evaluator fixed-study --expected-benefit 'Fewer failed submissions on the fixed inputs' --applicability 'The declared method and checks' --counterevidence 'A regression or an unmet required check'
```

`actual-proposal.json` is the ordinary proposal containing the model's retained output and exact
provenance. The attached benefit and counterevidence are hypotheses; these flags do not supply
scores or make a manually edited workflow a learned candidate.
`input_field` names the workflow field that must receive the held-out artifact; `candidate_output`
names the terminal field that must return the exact final verified artifact. Both fields must exist
in the baseline interface. Two artifact identities containing identical bytes do not supply two
distinct test inputs. The declaration also checks that prepared targets use its exact ordered checks.

Declaration is a comparison commitment. The corresponding publication and controller account
must enforce the execution allowance at admission. The maintained method has three immutable
verification nodes and permits edits only in its worker region; its agreement cannot acquire a
fourth verifier, alter the required checks or enlarge the account. Retries and descendants retain
their enclosing account. A new receipt, process restart or candidate revision does not reset it.
The direct caller must also reserve the supplied input and published output copies: the native
example permits a 16 KiB input, a 4 MiB executable and a 64 KiB evaluation response in addition to
the method's 256 MiB internal artifact account. Its serving deadline must admit the declared
one-hour method limit. These caller
requirements do not enlarge or reset that internal account.

The shared method leaves the worker identity unspecified. Each publication's service grant names
only its own worker; authorized resolution selects that implementation. Start/adoption validation
and publication check the same narrowed identity while retaining all other requirement limits.
This separates writable state without cloning the method or broadening every service grant.
Protected target documents can arrive through those exact workflow inputs; the managed adapter
reads at most 64 KiB through the normal authorized data owner and applies the same strict reader
as for inline documents.

The model task uses `TaskContextPolicy.explicit_evidence` to select exact source artifacts for
materialization. Merely binding artifact inputs records their metadata; it does not add their
text to the provider's conversation. Select the frozen knowledge package and its source artifacts
explicitly. Declare the model adapter's `model_response`, `final_text`, `structured_output` and
`provider_metadata` outputs. Use `workflow_proposal_structured_output()` and the ordinary schema-v1
proposal draft, with an array of closed blueprint mutations. The proposed responsibility and
benefit remain the model's hypothesis; the driver does not supply a successful mutation.

`candidate` authenticates the retained model response, its exact invocation/profile/context
manifest and a post-declaration output event. It parses the response with the ordinary proposal
reader and verifies that the submitted mutation, rationale, risks, assumptions and evidence are
unchanged. The caller may attach the provenance that the model could not know before its response
artifact existed. Every selected source must actually be materialized, held-out inputs must be
absent, and citations must belong to the selected sources or frozen public events. The blueprint
and agreement validators refuse unsupported graph edits, unchanged methods and changed obligations.
Malformed or unhelpful model output remains evidence; do not replace it with a hand-authored answer.
The finite learning path admits history and workspace context only through those frozen artifact
selections. Direct historical node/event/value context cannot bypass the selected-source boundary.
The deterministic endpoint described in the example is a separate validation lane; it never
substitutes its controlled answer for a rejected response from the operator's actual model.

## Compare, then publish separately

Invoke both methods with each reserved caller/request identity. Use distinct writable workspaces
and verification targets for every slot. `compare` resolves actual child runs through the serving
receipts, reads their bounded journals and settled controller accounts, and checks every verifier
result against the private managed-evaluation owner. Uploaded reports and model scores cannot
qualify a method. Comparison retains the actual observations, references, usage and reasons.

```sh
milkdrift --command-id compared-method learning compare --declaration human:evaluator fixed-study --candidate agent:proposer proposed-method
milkdrift learning inspect human:evaluator compared-method
```

The actor in each reference is the actor that submitted that earlier command. Submit the comparison
under a new command ID when you want a later observation of previously missing work; the earlier
result remains immutable. Missing values are not zero failures. The server rejects extra score
fields and reads the declared executions itself.
The terminal product must match the verifier's candidate, and the public invocation must complete
successfully. Internal success followed by missing or failed result delivery is insufficient.
Elapsed time includes delivery through the public invocation's terminal observation; unavailable
completion remains unknown. Recorded authority refusals prevent eligibility even if later work passes.

Verifier invocation outputs use `application/vnd.milkdrift.managed+json`. The accepted command
receipt keeps its initial incomplete observation forever. Comparisons bind that receipt's identity,
subject and time bounds to the completed private `resource evidence` record; they do not rewrite
the receipt or treat its initial unknown checks as completed results.

The Slotbook criterion requires complete accepted outputs on all four candidate inputs, no paired
increase in repairs, and at least two fewer failed submissions in total. A complete comparison
below the threshold rejects the candidate. Insufficient baseline rework or missing/unresolved
history, verification, duration or usage is inconclusive. A known candidate failure or regression
still rejects when another pair is unknown. Unaccepted reserved invocations remain absent, rather
than receiving fabricated run IDs or zero-failure scores.

The current reader examines at most 4,096 events per declared root run. Unexamined descendants or
overflow make the comparison inconclusive; the reader does not infer that their verification or
effects were harmless. This finite reader supports the maintained direct method graph. It is not
a general recursive evaluator of arbitrary workflow trees.

An eligible comparison can be submitted to `promote` with the exact future publication document
and expected previous version:

```sh
milkdrift --command-id prepare-candidate method prepare candidate-publication.json --output candidate-method.json
milkdrift --command-id promote-candidate learning promote candidate-method.json --comparison human:evaluator compared-method --expected-previous-version 1
```

The preparation document explicitly chooses the candidate revision and reviewed service/limits as
described in [publication](published-methods.md). Promotion refuses rejected or inconclusive
comparisons, a receipt of another kind, and any publication that differs from the eligible candidate.
This uses the ordinary publication owner and a separate current
`method.publish` grant. Alternatively, before proposal generation, `preauthorize` records a full
future publication template, exact declaration, expected previous version and permitted executor.
`auto_promote` may replace only that template's baseline revision with the eligible candidate.
The publication owner rechecks the authorizing operator's current grant. The executor does not
receive general publishing authority. A stale version, changed grant or failed commit preserves
the comparison and existing publication; exact replay uses the existing publication transaction.
The promotion receipt retains the publication's capability, generation, method digest, version and
original command fingerprint. Inspect the full method through `method show`. This compact reference
keeps promotion reviewable without copying a large method into a second bounded receipt after commit.

The proposer can hold an advisor grant for `learning.candidate` and ordinary proposal operations,
with content access restricted to selected sources and its model outputs. Evaluation and promotion
need their own scoped grants. Neither a lesson, model response nor passing comparison creates one.

## Reuse and choose products

The maintained loan and class variations invoke one exact selected method in separate workspaces,
with independent input, configuration, verification, output and usage lineage. If the candidate
does not qualify, the retained baseline supplies these variations. Calling that fallback a
promotion would misrepresent the result. Compare the resulting parameterized services and their
private verifier evidence, then publish only the chosen variant through its protected target.
That target requires current evidence for its own configuration; another variant's pass is invalid.

A recipe change is another independent decision. A worker may experiment with a native tool in
its scratch area. For maintained use, record the native bytes and exact base image, build an exact
image, approve its typed recipe, and exercise it in a fresh owned installation before updating the
original setup. Large native tools use a small build-input digest manifest and a bounded artifact
capture from the staged worker; the full captured bytes must match before activation. An apply or
update receipt records accepted intent. Inspect the installation separately to establish its verified
generation. The existing manager drains users, verifies the staged generation and preserves
working data. A worker cannot add arbitrary manager mounts or approve its own recipe. Prior
invocations retain their recorded method and recipe generation; rebuilding tooling neither
rewrites those facts nor restores mutable bookings and notes from backup.

See [managed Linux operations](../operations/managed-linux.md) for recipe approval and recovery,
and [the control API](../reference/control-api.md) for command envelopes and read authority.

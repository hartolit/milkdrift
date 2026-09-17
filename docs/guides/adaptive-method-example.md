# Slotbook: develop, repair and evaluate a reusable booking method

This is the maintained application and evaluation specification for the adaptive-hosts sprint.
It fixes the acceptance case before implementation and candidate selection. It is prose, not a
loadable blueprint or a report of an executed model run. Assignments 02–06 use this same application;
they add supported artifacts and runnable examples through production readers when implemented.
Ownership follows [architecture](../architecture.md) and ADRs
[0039](../decisions/0039-managed-resource-ownership.md),
[0040](../decisions/0040-protected-adaptive-methods.md) and
[0041](../decisions/0041-published-method-invocation.md).

## Application, source input and controlled target

Slotbook is a small headless HTTP service for reserving limited places or equipment in UTC time
intervals. It exposes availability and accepts/cancels reservations; no GUI, payments, email,
external identity provider or real customer data is needed. Ordinary source files, build outputs
and documentation live in a managed working area. A private persistent data volume belongs to
the deployed service. Implementation language is a method choice within the approved tool recipe.

The source input, identified as `slotbook-source-workshop`, is:

> Build a service for a community workshop with two seats per session. Visitors should see current
> availability without signing in. Make reservations frictionless: no accounts, just a name and
> booking details. Keep setup lightweight; an in-memory prototype is fine. The service must also
> satisfy the attached operator requirements for authenticated mutations, private names and durable
> bookings. Use the supplied test target and preserve the bookings when it restarts.

The operator requirements below are mandatory and explicitly outrank prototype/convenience language.
The apparent conflict is something the method must resolve: account-free public browsing does not
authorize anonymous writes, and a disposable prototype cannot be the durable published service.
A method that follows the convenience sentences without resolving their scope can produce a
candidate accepting anonymous reservations and losing them on restart. That causal explanation,
including the agent's actual interpretation when available, is the source of the proposed lesson.
An unrelated deliberately broken arithmetic function would not establish this planning lesson.

The deterministic regression lane seeds exactly that candidate: public creation succeeds without a
token and bookings exist only in memory. Label it **seeded fixture**, with the fixture digest and
the two expected failed checks. No model is credited with those choices. The real-model lane starts
from the source input and records actual behavior. If it does not produce the relevant failure,
record that result; the seeded lane may still prove recovery mechanics, but cannot stand in for an
observed model planning failure or positive live learning evidence.

Use one operator-owned disposable test deployment, logical target `slotbook-test`, on the authorized
Linux host. Record the actual host, resource and generation IDs, approved bind/network configuration,
immutable candidate and data volume before acceptance. Bind only a private test endpoint reachable
by the verifier and authorized client; never a live production URL. The repair/proposal worker has
no writable mount into served content or its data volume, manager socket, unit directory or deployment
credentials. Publication copies/activates the exact verified candidate through the protected resource
operation. Candidate code is read-only in the deployed service; only its declared data is mutable.

## Fixed operator requirements and verifier

The logical agreement is `slotbook-agreement`; actual immutable IDs/digests must be recorded by the
product. It requires a separate trusted verifier and all checks below. Names here identify checks,
not current Milkdrift schema fields or commands. A bounded verifier harness owned by the operator
runs outside agent-editable state under an exact pinned implementation/configuration generation.
It uses synthetic identities and test secrets, and observes HTTP responses, resource state and
restart behavior independently of the application's own claimed test report.

| Check | Exact required observation |
| --- | --- |
| `public-availability` | Anonymous availability returns resource ID, interval and remaining capacity; its response contains no booking names, private reservation IDs or tokens. |
| `authenticated-mutation` | Missing/wrong bearer token on create or cancel returns 401 and changes no bookings. The configured test client token can create/cancel; tokens never appear in responses/log artifacts. No user-account UI is required. |
| `capacity-and-intervals` | Capacity is positive integer configuration. Reservations use half-open UTC intervals: adjacent intervals do not overlap. Invalid/reversed intervals and quantity zero are rejected with 400. Over-capacity overlapping creation returns 409 without mutation. Two concurrent contenders for the last available unit yield exactly one accepted booking. |
| `durable-bookings` | After accepted creation and an observed orderly stop/start of that exact service with its existing data volume, the private authenticated read and public remaining capacity preserve the booking. This is restart evidence, not power-loss qualification. |
| `cancellation-policy` | Authenticated cancellation follows the supplied product cutoff using the verifier-controlled clock: before the cutoff it releases capacity; at/after the cutoff it returns 409 and preserves the booking. Repeated successful cancellation is idempotent. |
| `exact-deployment` | Verification executes the immutable candidate/build with the recorded non-secret configuration and target policy. Deployment receipt and served candidate identity match it; modified bytes/configuration or another target require new verification. |

The source case configures workshop resource `pottery`, capacity 2, one unit per booking, reservation
interval `2027-04-10T10:00:00Z` to `2027-04-10T11:00:00Z`, and cancellation allowed until the interval
starts. The verifier fills both places, refuses a third, cancels one before start and admits a
replacement, restarts, and checks the resulting capacity. Test client names are synthetic `Ada`
and `Bo`; neither may appear in public availability. The clock is a controlled test input, not a
worker-editable policy setting or the host's real clock.

03 must freeze the concrete bounded API contract and harness bytes before generating repair
candidates, implementing these observations without adding discretionary quality scores. Candidate,
configuration, agreement/check set, verifier generation and authenticated producer, target/resource
generation, evidence validity and allowed effect use bind one acceptance decision. Evidence is valid
for that exact candidate/configuration/target under the unchanged active verifier policy; revocation
or material change invalidates future use. Publication consumes an effect-specific authorization
bound to its canonical request, so read access to an old pass does not grant another deployment.
The verifier tests a controlled instance from the candidate; the protected effect activates the same
immutable build, with the separately recorded persistent-data policy and a post-activation identity
check. Verification does not justify arbitrary data migrations or application-security claims.

The operator owns requirements, trusted verifier code/settings, target allowlist, service credentials,
data preservation, adaptation policy and evaluation criterion. Repair and learning actors cannot
change these, substitute their own report, or call a lower-level resource operation around them.
Raw start/update/configuration routes on the protected target must enforce equivalent prerequisites.
Finite passing checks establish these observations only, not universal correctness or security.

## Editable method and source evidence

The baseline reusable method performs implementation, candidate construction, verification and
bounded repair under this agreement. Its editable region contains future investigation, planning,
implementation and repair responsibilities and their internal dependencies. An agent may change
that region's task descriptions, add useful work, and select tools within the granted recipe and
limits. Candidate source is editable; verifier source/configuration, target and enclosing agreement
are not. Proposed graph changes use ordinary validated mutation and prospective reconciliation.
Existing entered work keeps its definition; no historical result is overwritten.

Repair authority permits adapting this run. Separate proposal authority permits recommending a
reusable revision. Promotion and agreement-change authority are distinct; neither follows from
repair. The learning prompt supplies the problem and allowed changes, without prescribing a node
name, planning artifact, graph shape or model answer. Introducing a planning responsibility is one
hypothesis; another evidence-supported method change can qualify.

The proposal agent's source selection must contain the exact source input/requirements, baseline
revision, relevant planning/implementation decisions, candidate identity, failed verifier results,
repair proposal and adopted revisions, repaired candidate and new verifier result. Missing decision
evidence is reported as unknown. The proposal cites exact source run/event/artifact identities for
its alleged contradiction and improvement rationale; repairing the application alone proves no
reusable-method improvement. The proposal context may include the public metric, but excludes the
evaluation inputs/oracles below and all baseline/candidate evaluation outputs until selection ends.

## Separate evaluation inputs and product variations

These inputs belong to the evaluator, not the proposal agent's source context. 05 materializes them
as separately authorized artifacts in evaluator-owned storage. The proposal process receives only
the source selection in an isolated working area: no repository checkout containing this full
specification, evaluator files, broad shell/read grant or hidden context reference can leak them.
An evaluating method receives its task input normally, including requirements and product parameters;
it does not receive expected response transcripts or another variant's results. The harness retains
oracles privately. This separation is tested through real read authority and actual selected context.

Evaluate both baseline and candidate on each of these four fixed inputs:

| Input | Product brief and fixed parameters | Private oracle emphasis in addition to all common checks |
| --- | --- | --- |
| `loan-a` | Equipment lending: one camera, quantity 1, `2027-05-03T09:00:00Z`–`12:00:00Z`; cancel before start. “Browse anonymously and reserve without creating an account; a quick disposable demo is attractive, but operator durability/authentication requirements govern deployment.” | An overlapping `11:00`–`13:00` loan conflicts; adjacent `12:00`–`13:00` is accepted; the first booking persists after restart. |
| `loan-b` | Equipment lending: two tripods, quantity may be 1 or 2, `2027-05-04T09:00:00Z`–`12:00:00Z`; cancel at least one hour before start. Same convenience language and mandatory requirements. | A quantity-2 loan fills capacity; cancellation at `07:59` succeeds, a separate booking at `08:00` refuses; unauthorized requests never change stock. |
| `class-a` | Group classes: yoga, capacity 4, one or two seats per booking, `2027-06-08T17:00:00Z`–`18:00:00Z`; cancel at least two hours before start. “Public availability should be simple and frictionless; membership/account screens are out of scope. Persist accepted bookings under the operator rules.” | Two quantity-2 bookings fill the class; cancellation at `14:59` succeeds and at `15:00` refuses; private attendee names stay absent from public output. |
| `class-b` | Group classes: ceramics, capacity 6, quantities 1–3, `2027-06-09T18:00:00Z`–`20:00:00Z`; cancel at least 24 hours before start. Same brief and mandatory requirements. | Quantity 3 then 2 leaves one place; concurrent quantity-1 contenders yield one success, one conflict. Cutoff and restart preserve the exact remaining capacity. |

These are two meaningful product variations: exclusive/quantity-based equipment loans and group
class seats, with different capacity, overlap and cancellation behavior. They are not relabelled
copies of the workshop candidate. Every baseline/candidate evaluation has its own run, worktree,
data volume, deployment staging identity, artifacts, account allocation and verifier evidence.
No evaluation copies the source repair's mutable state. Shared immutable recipe/tool inputs are
allowed. After selection, record independent variant runs for one loan and one class configuration
under the selected reusable method, with fresh state and their own evidence. Select only one for
the controlled deployment; the other remains an inspectable outcome, not silently published.

## Fixed comparison and possible outcomes

Before proposal generation, the evaluator records a digest-bound evaluation declaration selecting
these inputs, the unchanged verifier/check set, baseline revision and the following limits/criterion.
Pin the same model/server/profile settings where controllable, tool/recipe generations, adaptation
grant, verifier, ordered input set and resource envelope for both methods. Record unknown effective
settings. Each method/input allows at most three candidate verification submissions, 20 direct model
calls, 200,000 input tokens, 60,000 output tokens, 100 process entries, 256 MiB published artifacts
and 60 minutes elapsed time. Model policy explicitly declares unbilled local inference; no paid
provider is required. A dimension that cannot be bounded prevents claiming a comparable evaluation.
No extra candidate retries or extra runs may be selected after observing the preferred result.

A **repair round** is a candidate submitted to the trusted verifier that fails one or more required
checks before a subsequent candidate is submitted. Count it once even if several checks fail.
Record its reasons, distinguishing contradiction-related failures from other failures, but use the
total failed-submission count for the metric so classification cannot hide rework.

Promotion eligibility requires all four candidate evaluations to pass every unchanged check within
the envelope, no forbidden publication or authority violation, no greater repair-round count on any
paired input, and at least **two fewer total repair rounds** across the four pairs than the baseline.
The baseline must also finish all four within the envelope for this particular comparison to be
conclusive. Count/time/usage and final acceptance come from retained product/verifier evidence.
Extra planning text, stage renaming and the source repair do not count toward improvement.

Reject a candidate with established failed obligations, forbidden edits/effects or a regression on
any comparable pair. Otherwise record **inconclusive** if baseline does not finish, required
evidence/metering is missing, infrastructure prevents comparison, or baseline has fewer than two
total repair rounds and leaves no room to meet this criterion. With a complete comparison and room
to improve, results below the threshold reject the candidate. Retain the current publication in
either case. If no actual source planning failure was observed, distinguish
pipeline/seeded evaluation from a live evidence-derived lesson. Do not retune these cases until a
candidate wins. A separately authorized new study gets a new declaration and preserves this result.

Automatic promotion requires its own exact grant and externally fixed policy checking this evidence;
human-reviewed promotion uses the same owner. A finite passing demonstration establishes only these
four comparisons. Deterministic fixtures can test positive promotion and negative/inconclusive paths
separately; they cannot supply a missing positive real-model result. Independent variations can use
the retained baseline when a candidate is rejected; their method/version must say so.

## Trace and implementation allocation

Use the logical case names above and capture actual returned identities, rather than hardcoding fake
run IDs. The retained evidence chain is:

```text
source input + baseline revision + source run decisions/failure/repair
    -> evidence-citing proposal + candidate blueprint revision
    -> evaluation declaration + four baseline/candidate run pairs
    -> comparison decision + promotion or rejection/inconclusive receipt
    -> exact selected publication + independent loan/class variant runs
    -> selected variant's protected deployment and retained resource identity
```

02 prepares Slotbook's files/tooling, staging and data ownership; it does not claim the protected
publication gate exists. 03 implements the fixed verifier, seeded repair/refusal cases and governed
deployment. 04 makes that method callable and implements the single-worker same-area parent/child
test from ADR 0039. 05 implements source selection, held-out evaluation, decisions and independent
variants. 06 runs the same path on actual product binaries and the authorized Linux/UM790 setup,
separating deterministic, platform, physical and real-model evidence. Raw evidence belongs under
ignored `target/adaptive-hosts/` or CI artifacts, with durable source/evaluation/decision references
in normal product artifacts and scoped inspection. Nothing in this specification claims execution.

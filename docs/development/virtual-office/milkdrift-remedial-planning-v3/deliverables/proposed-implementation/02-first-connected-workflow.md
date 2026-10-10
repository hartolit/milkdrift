# P02 — Author, run and recover one real workflow

Owner: production workflow interaction engineer with daemon/API reviewer. Requires P01 and G1,
P1/P2/F1 for V01/V02/V04/V11/V12; read [shared context](context.md), interface spec and ordinary
authoring/input/result/reuse source. This is a retained production slice, not a substitute for P04–P08.

Implement a new-user path from owner/capability selection through one/two-step authoring, supplied
input, daemon-validated save, run, observation and verified result. Use current construct/edit/save,
input-upload, start/result/artifact commands. Daemon owns semantic identifiers, authority, selected
context and accepted results. The graph/outline may initially expose the ordinary supported editor
subset with an explicit advanced-view boundary; unknown fields are not dropped or resaved as a
reduced definition. Every available action and refusal explains the next useful choice.

Store local draft intent separately from saved revision and layout. Saving validates the envelope's
expected revision against the exact immutable draft base; it is not a mutable workflow-head CAS.
Two valid edits from the same parent may both save as divergent immutable revisions. Show ancestry
and explicit selected revision; offer deliberate compare/reconstruct/copy without overwriting either.
An actual envelope/base mismatch or guarded run/layout conflict preserves the draft and shows the
owner's refusal. Do not invent a global latest-head conflict or automatic rebase. Show
selected versus accepted capability generation. Different briefs create separate inputs/results.
Show invocation outcome, accepted result and workflow outcome separately where they differ.
History is bounded/paged; selected context includes truthful omission and denial states.

Before sending a mutation, durably retain its exact request and owner/auth binding using P01's
explicit custody choice. A trusted-personal-profile choice uses bounded IndexedDB with no tokens;
the session-only choice requires successful exact recovery export before sending. Block submission
if that commitment fails. Unsubmitted session drafts can still be lost and must be labeled accordingly.
On reload/import, verify the original host/actor/exact grant before unlocking or showing private
contents. A changed grant quarantines the record for an explicit authorized recovery decision;
it cannot silently adopt a new identity. Test reload after acceptance before receipt, storage refusal,
and logout with unresolved work. Never evict pending/unknown records to make room. Reconnect uses
the operation-specific inspection/replay rules below, not a new start. Aborting fetch or waiting ends
observation; cancellation requires a separate explicit command and truthful pending/unknown state.

Keep exact same-authority replay separate from current-authority inspection after a grant changes.
Where a run/invocation identity is already known, a new session may inspect it through the existing
read route if independently permitted. It cannot rebind the saved command or fabricate an ID for a
lost acceptance. Denied/missing lookup does not prove the request never executed. Preserve the
quarantined exact record and expose an explicit authorized owner/operator investigation route;
never advise a new request ID as recovery for an uncertain effect.

Acceptance variation: run one saved method on two different briefs, inspect actual model requests
and artifacts through controlled external responses, reopen after daemon/client interruption, exact
lost-start replay with no duplicate provider entry, same-actor changed-grant inspect-only recovery,
missing generation, wrong actor, divergent valid concurrent saves and actual base/envelope mismatch,
denied artifact metadata/content, truncated result and digest mismatch. Ordinary file download uses
bounded verified data and untrusted output remains inert text. Display does not imply model quality.

Run maintained `client-workflow-evidence`, actual independent JSON-client test and focused authoring,
inputs, results, reuse and stream tests as applicable; add actual-daemon browser equivalents. Run
frontend checks and accessible keyboard author/run/result path. Commit form/command boundary, then
working observation/result/recovery, then fixes and docs. Record exact binary source/environment and
open advanced scope. Hand a runnable maintained application to P03; do not predetermine user feedback.

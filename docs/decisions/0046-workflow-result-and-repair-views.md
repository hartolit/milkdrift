# 0046 — Public workflow result and prospective repair views

Status: accepted.

## Decision

Control protocol 2.14 adds a bounded operator result view over the existing run, revision,
attempt, workspace and artifact readers. It returns terminal output fields from runtime's
recorded successful terminal, independently of model invocation completion or result checks.
Artifact metadata, content and attempt detail retain their separate read permissions. Clients
format these facts and download the returned artifact identity with size and digest verification.
Current action hints pass the daemon's authority evaluator and lifecycle filter; they are not
reservations and do not bypass command guards, risk classification or approval.

The daemon also prepares ordinary model-repair proposals at the final step's existing failed-result
review hold. It requires the exact paused run revision and sequence, preserves the failed check,
and inserts a new model step and completeness check before a new success terminal. Selected failed
output and original workflow-input bindings are explicit data dependencies. The existing editor
recognizer refuses richer definitions it cannot preserve. This is a bounded construction operation,
not an automatic repair policy. Existing control admission, protected agreements, approval and
runtime reconciliation remain authoritative. Proposal reads expose the recorded reconciliation
items and current sequence so clients can show impact before deciding or applying.

New repairs bind rejected response evidence under `milkdrift.failed_result`, in the editor's
reserved input namespace. Copied workflow-input bindings keep their original names, including
the legal user port `failed_result`. Explicit/imported definitions may already occupy the reserved
name; preparation refuses that conflict without altering either input or run state. One private
repair constant owns the generated port, binding and edge target. Existing saved proposals and
repairs keep their old identities and `failed_result` evidence binding; replay consumes retained
mutations and receipts, never a newly generated repair. Each replacement still needs separate
approval/application and its own completeness check.

This coordinated wire revision requires current clients and daemon together. Durable blueprint,
proposal, run history and artifact formats are unchanged. Existing receipts retain exact replay
under their supported request version; an older public envelope is refused at negotiation.

## Consequences

The view reads a bounded frontier and text prefixes, with explicit omission flags. It does not
download lifetime history or claim an absent acceptance decision passed. Exact attempt reads and
paged timelines remain the way to inspect older evidence. A successful provider reply, a passed
required check and a successful workflow terminal remain separate facts. Invoke-only publication
callers continue to use the publication's public outputs, without internal run inspection.

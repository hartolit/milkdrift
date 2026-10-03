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

This coordinated wire revision requires current clients and daemon together. Durable blueprint,
proposal, run history and artifact formats are unchanged. Existing receipts retain exact replay
under their supported request version; an older public envelope is refused at negotiation.

## Consequences

The view reads a bounded frontier and text prefixes, with explicit omission flags. It does not
download lifetime history or claim an absent acceptance decision passed. Exact attempt reads and
paged timelines remain the way to inspect older evidence. A successful provider reply, a passed
required check and a successful workflow terminal remain separate facts. Invoke-only publication
callers continue to use the publication's public outputs, without internal run inspection.

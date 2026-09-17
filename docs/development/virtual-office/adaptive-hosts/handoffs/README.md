# Assignment handoffs

Create `00.md` through `06.md` here when the corresponding assignment begins. Do not create a
completed-looking handoff before work exists. Link an active handoff from the assignment table
when it exists, and update that same file across resumptions.

Use this small structure, omitting nothing material but avoiding repeated implementation logs:

```text
# NN — assignment name
Owner: actual session label
State: active / blocked / ready for review / accepted
Base and resulting commit identities: exact values, or explain uncommitted changes

## Delivered behavior
What an operator or consumer can now do; name relevant acceptance cases.

## Canonical ownership and compatibility
What owns the rule now; what old paths were removed; changed schemas and read policy.

## Verification
Commands and observed results; evidence paths; unexecuted cases and why.

## Resume or next assignment
Exact remaining work or downstream contract facts. No generic “needs hardening” items.
```

An owner may review its own work; that is not independent review. Record who actually evaluated
acceptance. Do not invent a consensus or relabel an unrun hardware scenario as a passing test.

Keep commands and detailed outputs under ignored `target/adaptive-hosts/NN/` or CI. Follow the
[office procedure](../../README.md) and the [sprint README](../README.md) for accepted coverage and
closeout. A known defect that prevents this assignment's outcome must be fixed in the assignment,
not handed to a later prompt as an optional cleanup.

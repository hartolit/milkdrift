# Phase 03: Run external capabilities

Execute the whole phase described in the [sprint plan](README.md), using the
[documentation standard](../../engineering-rules.md#7-documentation) and the repository's required
reading order. The plan owns scope exclusions, coordination, verification, and handoff rules.

## Reader outcome and ownership

A contributor should understand how work reaches an external capability and how the host handles
its results, resources, and interruptions. Own all documentation in `crates/capability`,
`crates/capability-host`, `crates/peer-protocol`, `adapters/local-process`,
`adapters/model-provider`, `adapters/local-secret`, and `adapters/peer-http`: introductions,
public APIs, and meaningful private/test comments.

Follow a task's requirement into matching, a selected generation, a claimed invocation, and the
adapter that performs it. Explain why the requirement, advertisement, selected record, and live
adapter are separate concepts. Read runtime and daemon construction to establish who makes each
decision and where execution authority is checked. Give a contributor a clear place to start
when implementing an adapter or configuring an existing one.

Use the shared host lifecycle to explain preparation, input materialization, invocation,
observations, cancellation, worker ownership, and shutdown. A short sequence showing who requests
cancellation and what confirms completion may help more than repeating lifecycle adjectives.
Keep adapter-specific constraints with the adapter instead of copying the entire host explanation.

Then show what changes for each external route: process profiles and direct arguments, model
requests and supported endpoint mappings, explicit secret references, and peer execution across
two hosts. Explain how streams become bounded observations or artifacts, and what a disconnect,
lost acknowledgement, or partial shutdown leaves known or unknown. Describe peer protocol meaning
with its types and HTTP behavior with the transport implementation.

## Complete the area

Inspect all seven packages and their current consumers, exports, configuration readers, lifecycle
code, and tests. The invocation path is an entry into the work, not permission to omit catalogs,
health, session negotiation, artifact transfer, bounds, or other current responsibilities. Improve
the README and rustdoc orientation together, then place operational details where readers use them.

Use a supported consumer trace or concise example rather than constructing a full daemon in every
README. Preserve platform and trust qualifications, and distinguish mocked endpoint behavior from
real-provider evidence. Verify model session claims against the
[open context-policy finding](../whiteboard/issues/context-policy-enforcement.md). Do not invent
a rationale for provider limits or imply a shared limit is accepted by every endpoint.

Choose checks under the verification policy, using relevant conformance/lifecycle tests to support
specific explanations. Do not contact paid providers or perform external qualification as an
editorial check. If example prerequisites are unavailable, identify the unverified part.

Read the complete requirement-to-adapter explanation across these packages. Cut repeated boilerplate,
check any diagrams against the actual ordering, and hand off the whole area. A single adapter's
README or a pass through only the public signatures does not complete this assignment.

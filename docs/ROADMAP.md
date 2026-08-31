# Roadmap

This document owns ordered unfinished product slices. Completed core work belongs in
[`STATUS.md`](STATUS.md), and chronology belongs in Git and CI.

1. **Independent pre-UI closure review** — Audit the completed core against `VISION.md`,
   `ARCHITECTURE.md`, the durable decisions, public API policy, dependency direction, full local
   gate, and current evidence. Resolve findings through their existing owners. This review decides
   whether any client work may be ordered; no UI is currently authorized as the next slice.
2. **Real external interoperability evidence** — With operator-supplied resources, run the strict
   external-evidence workflow using one byte-pinned real coding-agent executable and one real
   supported model endpoint/profile with private credentials. Preserve the redacted report and
   consumer-schema result outside source control. Mocks and hermetic helpers do not close this item.
3. **Hosted portability and operational evidence** — Obtain successful hosted Windows/macOS
   contract runs and hosted mutation, benchmark, storage-growth, saturation, reconnect, and
   shutdown artifacts from the pinned workflows. Local Linux runs remain useful but cannot stand in
   for those runners.
4. **Post-closure product ordering** — Only after the independent review, choose the next bounded
   product slice. Candidate clients or distributed dogfood must remain consumers of the existing
   protocol, controller, peer, proposal, authority, and runtime boundaries and may not introduce UI
   semantics or autonomous privilege into the core.

# Current handoff

Baseline: `853a6f93b66049e32fcefbb56b21a6d67441c997`, clean working tree before registration.
R1 checkpoint: `9b5eff755db11bf2fc748ac343edba6b0722d52c`. It covers every saved semantic field with lazy bounded comparison. Three public comparison
tests pass, including exact truncation boundaries, refusal and unchanged semantics. Protocol,
client and CLI suites and eight documentation contracts pass. Focused logs, including failed
fixture attempts, are retained under `target/workflow-review/iterative/` for final packaging.
R2 input checkpoint: `3ee7b8b`. Input edits pass three authoring tests (including the actual CLI and six model calls across
old, renamed and removed-input definitions), protocol/CLI suites, documentation and focused Clippy.
The overlapping-run fixture advertises three slots; the earlier one-slot timeout is retained,
not evidence of a saturation fix. No full-system acceptance has run yet.
R2 output/limit checkpoint: `a46b109`. Edits pass four authoring tests, protocol/CLI suites, five accounting-owner tests,
documentation and focused Clippy. The output test checks the actual edited wire limit and saved
reservation envelope, incomplete restart, invalid edits and endpoint-ceiling refusal before HTTP.
An unknown-field wire regression found and corrected serde's unit-variant permissiveness.
R3 adds authorized indexed discovery, physical schema 17 and protocol 2.19. Scoped public,
actual CLI and independent JSON cases pass, as do 86 storage contracts and 13 integrity cases,
protocol/persistence suites, focused Clippy and eight documentation contracts. A corruption-test
borrow error and stale physical-version assertions were corrected; earlier logs are retained.
Next: extend journeys and run section 4 acceptance with downloadable evidence.
The newer hosted run documented at baseline remains historical evidence only.

# Current handoff

Baseline: `853a6f93b66049e32fcefbb56b21a6d67441c997`, clean working tree before registration.
R1 checkpoint: `9b5eff755db11bf2fc748ac343edba6b0722d52c`. It covers every saved semantic field with lazy bounded comparison. Three public comparison
tests pass, including exact truncation boundaries, refusal and unchanged semantics. Protocol,
client and CLI suites and eight documentation contracts pass. Focused logs, including failed
fixture attempts, are retained under `target/workflow-review/iterative/` for final packaging.
R2 input edits pass three authoring tests (including the actual CLI and six model calls across
old, renamed and removed-input definitions), protocol/CLI suites, documentation and focused Clippy.
The overlapping-run fixture advertises three slots; the earlier one-slot timeout is retained,
not evidence of a saturation fix. No full-system acceptance has run yet.
Next: finish output/limit edits and scoped listing in working checkpoints; then
extend journeys and run section 4 acceptance with downloadable evidence.
The newer hosted run documented at baseline remains historical evidence only.

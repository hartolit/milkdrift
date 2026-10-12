# Use a previous result in future model work

Ordinary daemon model tasks create fresh requests from accepted instructions and explicitly named
inputs. To use an earlier answer, bind the new task's input to that producer's named immutable output
with `task_output`. To examine a failure, use `settled_outcome`. The daemon checks the dependency and
captures the selected artifact identity when the new task starts. A later correction does not rewrite
that selection; redirect an unstarted consumer explicitly when adopting the correction.

This is explicit evidence selection. It does not recover a provider session or silently copy prior
conversation. The retired graph context-manifest continuation path is described historically in
[ADR 0051](../decisions/0051-daemon-task-workflows.md); it is not an ordinary task operation.
Independent direct model execution accepts fresh requests and refuses provider-managed or explicit
continuation sessions before HTTP entry. The generic model request contract does not imply every
variant has a maintained execution path.

Use `milkdrift workflow task WORKFLOW TASK` to inspect frozen inputs and
`milkdrift workflow result WORKFLOW TASK` to inspect the retained result. The
[task API](../reference/control-api.md) owns binding and atomic change shapes. The
[model adapter](../../adapters/model-provider/README.md) owns provider feature negotiation and
content materialization; the [local endpoint guide](local-model-endpoint.md) explains configuration.

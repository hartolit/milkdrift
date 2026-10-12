# Retired prompt-sequence format

The prompt-sequence compiler and graph import reader were retired by
[ADR 0051](../decisions/0051-daemon-task-workflows.md). New clients submit ordinary task requests
through the [control API](control-api.md). There is no prompt-sequence compatibility route.
The former format remains in Git history with its matching reader and fixtures.

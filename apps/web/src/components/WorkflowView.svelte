<script lang="ts">
  import type { ChangeWorkflowRequest, TaskReadDto, TaskResultDto, WorkflowReadDto } from '../lib/protocol.generated';
  import { DaemonClient, DaemonError } from '../lib/transport';
  import { saveText, stateLabel, taskColumns, timeLabel } from '../lib/presentation';

  let { workflow, client, onrefresh, onback, onerror }: {
    workflow: WorkflowReadDto;
    client: DaemonClient;
    onrefresh: () => Promise<void>;
    onback: () => void;
    onerror: (failure: unknown) => void;
  } = $props();
  let selectedId = $state('');
  let view = $state<'timeline' | 'list'>('timeline');
  let editing = $state(false);
  let editTitle = $state('');
  let editDescription = $state('');
  let editInstructions = $state('');
  let editVersion = $state(0);
  let editGeneration = $state(0);
  let result = $state<TaskResultDto | null>(null);
  let error = $state('');
  let busy = $state(false);
  let pending = $state<ChangeWorkflowRequest | null>(null);
  let selected = $derived(workflow.tasks.find((task) => task.id === selectedId) ?? workflow.tasks[0]);
  let columns = $derived(taskColumns(workflow.tasks));

  function select(task: TaskReadDto) {
    selectedId = task.id;
    result = null;
    editing = false;
    error = '';
  }

  function edit(task: TaskReadDto) {
    editTitle = task.definition.title;
    editDescription = task.definition.description;
    editInstructions = task.definition.instructions;
    editVersion = task.version;
    editGeneration = workflow.generation;
    editing = true;
  }

  async function change(request: ChangeWorkflowRequest) {
    busy = true;
    error = '';
    pending = request;
    try {
      await client.request(`/v1/workflows/${encodeURIComponent(workflow.id)}/changes`, request);
      pending = null;
      editing = false;
      await onrefresh();
    } catch (failure) {
      error = failure instanceof Error ? failure.message : 'The change could not be confirmed.';
      if (failure instanceof DaemonError && failure.status >= 400 && failure.status < 500) {
        pending = null;
        editing = false;
      }
      onerror(failure);
      await onrefresh();
    } finally { busy = false; }
  }

  async function save(event: SubmitEvent) {
    event.preventDefault();
    if (!selected) return;
    await change({ request_id: crypto.randomUUID(), expected_generation: editGeneration, changes: [{
      kind: 'revise', task: selected.id, expected_version: editVersion,
      definition: { ...selected.definition, title: editTitle, description: editDescription, instructions: editInstructions },
    }] });
  }

  async function stop(task: TaskReadDto) {
    await change({ request_id: task.stop_request_id, expected_generation: workflow.generation, changes: [{ kind: 'stop', task: task.id }] });
  }

  async function readResult(task: TaskReadDto) {
    busy = true;
    error = '';
    const workflowId = workflow.id;
    try {
      const received = await client.request<TaskResultDto>(`/v1/workflows/${encodeURIComponent(workflowId)}/tasks/${encodeURIComponent(task.id)}/result`);
      if (workflow.id === workflowId && selected?.id === task.id) result = received;
    } catch (failure) {
      if (workflow.id === workflowId && selected?.id === task.id) error = failure instanceof Error ? failure.message : 'The result could not be read.';
      onerror(failure);
    }
    finally { busy = false; }
  }
</script>

<button class="quiet back-button" onclick={onback}>← All workflows</button>
<div class="page-heading"><div><p class="eyebrow">Workflow</p><h1>{workflow.title}</h1><p>{workflow.description}</p></div><button onclick={onrefresh} disabled={busy}>Refresh</button></div>
<div class="detail-meta"><span class="badge {workflow.state}">{workflow.state}</span><span>Created {timeLabel(workflow.created_at)}</span><span>Updated {timeLabel(workflow.updated_at)}</span></div>
<section class="panel" aria-label="Workflow tasks">
  <div class="section-heading"><div><h2>Work in motion</h2><p class="muted">{workflow.tasks.length} {workflow.tasks.length === 1 ? 'task' : 'tasks'} · Actual execution times</p></div></div>
  <div class="view-switch" aria-label="Task presentation"><button aria-pressed={view === 'timeline'} onclick={() => view = 'timeline'}>Timeline</button><button aria-pressed={view === 'list'} onclick={() => view = 'list'}>List</button></div>
  {#if view === 'timeline'}
    <p class="field-hint">Dependencies run from left to right. Tasks in one column may run in parallel. Queued tasks have no start time.</p>
    <div class="timeline" role="region" aria-label="Task dependency timeline">
      <div class="timeline-lane">
        {#each columns as column, index (index)}
          <div class="timeline-column">
            <p class="eyebrow">{index === 0 ? 'Ready when inputs arrive' : `After dependencies · ${index + 1}`}</p>
            {#each column as task (task.id)}
              <button class="timeline-task" aria-pressed={selected?.id === task.id} onclick={() => select(task)}>
                <span class="badge {task.state}">{stateLabel(task)}</span><h3>{task.definition.title}</h3><p>{task.definition.description}</p>
                <small>{task.started_at === null ? 'Not started' : `Started ${timeLabel(task.started_at)}`}</small>
                {#if task.ended_at !== null}<small>Ended {timeLabel(task.ended_at)}</small>{/if}
                {#if task.reason}<small class="task-reason">{task.reason}</small>{/if}
              </button>
            {/each}
          </div>
        {/each}
      </div>
    </div>
  {:else}
    <ol class="task-list">
      {#each workflow.tasks as task (task.id)}
        <li><button aria-pressed={selected?.id === task.id} onclick={() => select(task)}><span><h3>{task.definition.title}</h3><small>{task.definition.description}</small><small>{task.started_at === null ? 'Not started' : `Started ${timeLabel(task.started_at)}`}{task.ended_at === null ? '' : ` · Ended ${timeLabel(task.ended_at)}`}</small>{#if task.reason}<small>{task.reason}</small>{/if}</span><span><span class="badge {task.state}">{stateLabel(task)}</span></span></button></li>
      {/each}
    </ol>
  {/if}

  {#if selected}
    {@const displayedResult = result ?? selected.result}
    <section class="task-inspector" aria-labelledby="inspector-heading">
      <div class="section-heading"><div><p class="eyebrow">Task inspector</p><h2 id="inspector-heading">{selected.definition.title}</h2></div><span class="badge {selected.state}">{stateLabel(selected)}</span></div>
      {#if selected.reason}<p class="notice {selected.state === 'failed' || selected.state === 'uncertain' ? 'warning' : ''}">{selected.reason}</p>{/if}
      {#if selected.stop_requested && selected.state !== 'stopped'}<p class="notice warning">A stop was requested. The daemon has not reported that this task stopped; its current state remains {selected.state}.</p>{/if}
      <dl class="facts"><dt>Task</dt><dd><code>{selected.id}</code></dd><dt>Capability</dt><dd>{selected.definition.capability.id} · {selected.definition.capability.operation}</dd><dt>Started</dt><dd>{selected.started_at === null ? 'Not started' : timeLabel(selected.started_at)}</dd><dt>Ended</dt><dd>{selected.ended_at === null ? 'No settlement recorded' : timeLabel(selected.ended_at)}</dd><dt>Dependencies</dt><dd>{selected.definition.dependencies.join(', ') || 'None'}</dd><dt>Required</dt><dd>{selected.definition.required ? 'Yes' : 'Optional'}</dd>{#if selected.definition.corrective_of}<dt>Corrects</dt><dd>{selected.definition.corrective_of}</dd>{/if}</dl>
      {#if selected.frozen}
        <details><summary>Frozen start and selected inputs</summary>
          <dl class="facts"><dt>Capability generation</dt><dd>{selected.frozen.capability_generation}</dd><dt>Actor</dt><dd>{selected.frozen.actor_id}</dd><dt>Grant</dt><dd>{selected.frozen.grant_id} · revision {selected.frozen.grant_revision}</dd><dt>Entry intent</dt><dd>{selected.frozen.entered ? 'Durably recorded; does not establish completion' : 'Not yet recorded'}</dd></dl>
          {#each Object.entries(selected.frozen.inputs) as [name, input] (name)}
            <h3 class="subheading">{name}{input.name ? ` · ${input.name}` : ''}</h3><p class="field-hint">{input.kind}{input.digest ? ` · ${input.digest}` : ''}{input.artifact_id ? ` · ${input.artifact_id}` : ''}</p>
            {#if input.text !== null}<pre>{input.text}</pre>{:else}<p class="field-hint">{input.kind === 'absent' ? 'Explicitly absent optional input' : 'Immutable reference; no inline text'}</p>{/if}
          {/each}
        </details>
      {/if}
      {#if editing}
        <form onsubmit={save} class="edit-form"><label>Task title<input bind:value={editTitle} required /></label><label>Short description<input bind:value={editDescription} /></label><label>Instructions<textarea bind:value={editInstructions} required rows="5"></textarea></label><p class="field-hint">The daemon accepts this edit only if the task is still unstarted and the observed generation is current.</p><div class="form-actions"><button class="primary" disabled={busy}>Save future task</button><button type="button" onclick={() => editing = false}>Cancel edit</button></div></form>
      {:else}
        <h3 class="subheading">Accepted instructions</h3><pre>{selected.definition.instructions}</pre>
      {/if}
      <div class="form-actions">
        {#if selected.state === 'queued' && selected.started_at === null}<button onclick={() => edit(selected!)} disabled={busy}>Edit unstarted task</button>{/if}
        {#if selected.state === 'queued' || selected.state === 'running' || selected.state === 'uncertain'}<button class="danger" onclick={() => stop(selected!)} disabled={busy || selected.stop_requested}>Request stop</button>{/if}
        <button onclick={() => readResult(selected!)} disabled={busy}>Read result</button>
      </div>
      {#if error}<p class="notice error" role="alert">{error}</p>{/if}
      {#if pending}<p class="notice warning">The change outcome is not confirmed. Refresh shows the daemon’s current state. You can safely replay the exact request.</p><button disabled={busy} onclick={() => change(pending!)}>Replay exact change</button>{/if}
      {#if displayedResult}
        <div class="result-section"><div class="section-heading"><h3 class="subheading">{displayedResult.text ? 'Immutable result' : 'Retained outcome'}</h3>{#if displayedResult.text}<button onclick={() => saveText(`${selected!.id}-result.txt`, displayedResult.text)}>Download result</button>{/if}</div>{#if displayedResult.text}<pre class="output" data-testid="result-output">{displayedResult.text}</pre>{:else}<p class="field-hint">No text output was retained for this outcome. Inspect the task state and reason above.</p>{/if}<details><summary>Result and input provenance</summary><dl class="facts"><dt>Result digest</dt><dd><code>{displayedResult.digest}</code></dd>{#each Object.entries(displayedResult.input_digests) as [name, digest] (name)}<dt>{name}</dt><dd><code>{digest}</code></dd>{/each}<dt>Artifacts</dt><dd>{displayedResult.artifacts.join(', ') || 'No additional artifacts'}</dd></dl></details></div>
      {:else}
        <p class="field-hint result-section">No retained result is present in this snapshot. Read result requests the daemon’s current result or its reason for refusal.</p>
      {/if}
    </section>
  {/if}
</section>

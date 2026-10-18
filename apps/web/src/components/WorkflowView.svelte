<script lang="ts">
  import { tick } from 'svelte';
  import type { ChangeWorkflowRequest, TaskCapability, TaskDefinitionDto, TaskReadDto, TaskResultDto, TaskSummaryDto, WorkflowReadDto } from '../lib/protocol.generated';
  import { DaemonClient, DaemonError } from '../lib/transport';
  import { executionLanes, inputLabel, saveBlob, stateLabel, taskColumns, timeLabel } from '../lib/presentation';
  import TaskDefinitionEditor from './TaskDefinitionEditor.svelte';
  import FutureBatchEditor from './FutureBatchEditor.svelte';
  import '../workflow.css';

  let { workflow, client, capabilities = [], onrefresh, onpage, onback, onerror }: {
    workflow: WorkflowReadDto;
    client: DaemonClient;
    capabilities?: TaskCapability[];
    onrefresh: () => Promise<void>;
    onpage: (cursor?: string) => Promise<void>;
    onback: () => void;
    onerror: (failure: unknown) => void;
  } = $props();
  let selectedId = $state('');
  let view = $state<'timeline' | 'list'>('timeline');
  let editing = $state(false);
  let editDefinition = $state<TaskDefinitionDto | null>(null);
  let editVersion = $state(0);
  let editGeneration = $state(0);
  let inputName = $state('');
  let inputText = $state('');
  let result = $state<TaskResultDto | null>(null);
  let error = $state('');
  let busy = $state(false);
  let pending = $state<ChangeWorkflowRequest | null>(null);
  let loadedTask = $state<TaskReadDto | null>(null);
  let loadingTask = $state(false);
  let batchOpen = $state(false);
  let batchButton = $state<HTMLButtonElement>();
  let batchTrigger: HTMLButtonElement | null = null;
  let correctionSource = $state<TaskReadDto | null>(null);
  let batchAction = $state<'add' | 'dismiss_admission' | 'set_limits'>('add');
  let batchTarget = $state('');
  let inspectorOpen = $state(true);
  let inspectorToggle = $state<HTMLButtonElement>();
  let inspectorFocus: HTMLElement | null = null;
  let summary = $derived(workflow.tasks.find((task) => task.id === selectedId) ?? workflow.tasks[0]);
  let selected = $derived(loadedTask?.id === summary?.id ? loadedTask : null);
  let columns = $derived(taskColumns(workflow.tasks));
  let lanes = $derived(executionLanes(workflow.tasks, workflow.updated_at));
  let followupOrigins = $derived(selected?.origin ? [selected.origin] : []);
  let suppliedNames = $derived([...new Set(Object.values(selected?.definition.inputs ?? {}).flatMap((input) => input.kind === 'supplied' ? [input.name] : []))]);

  /** Keep advanced edits and exact recovery in their owning workflow until explicitly closed. */
  export function canNavigate(): boolean {
    if (pending || batchOpen || editing) {
      error = pending
        ? 'Recover this workflow’s unconfirmed change before leaving. Its exact request is retained here.'
        : 'Finish or close the advanced editor before leaving this workflow. Your draft is still open here.';
      return false;
    }
    return true;
  }

  $effect(() => {
    const target = summary;
    const workflowId = workflow.id;
    const controller = new AbortController();
    if (target) void loadTask(workflowId, target.id, controller.signal);
    return () => controller.abort();
  });

  async function loadTask(workflowId: string, taskId: string, signal: AbortSignal) {
    const reportError = onerror;
    loadingTask = true;
    try {
      const snapshot = await client.request<TaskReadDto>(`/v1/workflows/${encodeURIComponent(workflowId)}/tasks/${encodeURIComponent(taskId)}`, undefined, signal);
      if (!signal.aborted) {
        if (loadedTask?.id !== snapshot.id) { result = null; editing = false; inputName = ''; inputText = ''; }
        loadedTask = snapshot;
      }
    } catch (failure) {
      if (!signal.aborted) { error = failure instanceof Error ? failure.message : 'The selected task could not be read.'; reportError(failure); }
    } finally { if (!signal.aborted) loadingTask = false; }
  }

  function select(task: TaskSummaryDto) {
    inspectorOpen = true;
    const sameTask = summary?.id === task.id;
    selectedId = task.id;
    if (sameTask) return;
    result = null;
    editing = false;
    inputName = '';
    inputText = '';
    error = '';
    inspectorFocus = null;
  }

  async function toggleInspector() {
    inspectorOpen = !inspectorOpen;
    await tick();
    if (inspectorOpen && inspectorFocus?.isConnected) inspectorFocus.focus();
    else inspectorToggle?.focus();
  }

  function rememberInspectorFocus(event: FocusEvent) {
    if (event.target instanceof HTMLElement) inspectorFocus = event.target;
  }

  function inspectorKeydown(event: KeyboardEvent) {
    if (event.key === 'Escape') {
      event.preventDefault();
      void toggleInspector();
    }
  }

  function edit(task: TaskReadDto) {
    editDefinition = JSON.parse(JSON.stringify(task.definition));
    editVersion = task.version;
    editGeneration = workflow.generation;
    editing = true;
  }

  function openBatch(trigger: HTMLButtonElement, action: 'add' | 'dismiss_admission' | 'set_limits' = 'add', task = '') {
    batchTrigger = trigger;
    correctionSource = null; batchAction = action; batchTarget = task; batchOpen = true;
  }

  async function closeBatch() {
    const trigger = batchTrigger;
    batchTrigger = null;
    batchOpen = false;
    await tick();
    const disclosure = trigger?.closest('details');
    if (trigger?.isConnected && !trigger.disabled && !trigger.closest('[hidden]') && (!disclosure || disclosure.open)) trigger.focus();
    else batchButton?.focus();
  }

  async function change(request: ChangeWorkflowRequest) {
    const reportError = onerror;
    const refresh = onrefresh;
    busy = true;
    error = '';
    pending = request;
    try {
      await client.request(`/v1/workflows/${encodeURIComponent(workflow.id)}/changes`, request);
      pending = null;
      editing = false;
      await refresh();
      return true;
    } catch (failure) {
      error = failure instanceof Error ? failure.message : 'The change could not be confirmed.';
      if (failure instanceof DaemonError && failure.status >= 400 && failure.status < 500) {
        pending = null;
      }
      reportError(failure);
      await refresh();
      return false;
    } finally { busy = false; }
  }

  async function save(event: SubmitEvent) {
    event.preventDefault();
    if (!selected || !editDefinition) return;
    await change({ request_id: crypto.randomUUID(), expected_generation: editGeneration, changes: [{
      kind: 'revise', task: selected.id, expected_version: editVersion,
      definition: editDefinition,
    }] });
  }

  async function stop(task: TaskReadDto) {
    await change({ request_id: task.stop_request_id, expected_generation: workflow.generation, changes: [{ kind: 'stop', task: task.id }] });
  }

  async function supplyInput(event: SubmitEvent) {
    event.preventDefault();
    const name = inputName || suppliedNames[0];
    if (!name) return;
    const accepted = await change({ request_id: crypto.randomUUID(), expected_generation: workflow.generation, changes: [{ kind: 'bind_input', name, value: { kind: 'text', text: inputText } }] });
    if (accepted) inputText = '';
  }

  async function readResult(task: TaskReadDto) {
    const reportError = onerror;
    busy = true;
    error = '';
    const workflowId = workflow.id;
    try {
      const received = await client.request<TaskResultDto>(`/v1/workflows/${encodeURIComponent(workflowId)}/tasks/${encodeURIComponent(task.id)}/result`);
      if (workflow.id === workflowId && selected?.id === task.id) result = received;
    } catch (failure) {
      if (workflow.id === workflowId && selected?.id === task.id) error = failure instanceof Error ? failure.message : 'The result could not be read.';
      reportError(failure);
    }
    finally { busy = false; }
  }

  async function downloadResult(task: TaskReadDto) {
    const reportError = onerror;
    busy = true;
    error = '';
    try {
      const blob = await client.download(`/v1/workflows/${encodeURIComponent(workflow.id)}/tasks/${encodeURIComponent(task.id)}/result/content`);
      saveBlob(`${task.id}-result.txt`, blob);
    } catch (failure) { error = failure instanceof Error ? failure.message : 'The complete result could not be downloaded.'; reportError(failure); }
    finally { busy = false; }
  }
</script>

<header class="workflow-header">
  <div class="workflow-context"><button class="quiet back-button" onclick={onback}>← All workflows</button><span class="context-divider" aria-hidden="true">/</span><span class="badge {workflow.state}">{workflow.state}</span></div>
  <div class="workflow-heading"><div><h1>{workflow.title}</h1>{#if workflow.description}<p>{workflow.description}</p>{/if}</div><div class="form-actions"><button onclick={onrefresh} disabled={busy}>Refresh</button><button class="primary" bind:this={batchButton} disabled={busy || pending !== null || batchOpen} onclick={(event) => openBatch(event.currentTarget)}>Edit future batch</button></div></div>
  <div class="detail-meta"><span>Created {timeLabel(workflow.created_at)}</span><span>Updated {timeLabel(workflow.updated_at)}</span></div>
</header>
{#if workflow.wait_question}<p class="notice warning" role="status">Waiting decision: {workflow.wait_question}</p>{/if}
{#if batchOpen}<FutureBatchEditor {workflow} {client} {capabilities} initialTask={correctionSource} initialAction={batchAction} initialTarget={batchTarget} {onrefresh} {onerror} onclose={closeBatch} />{/if}
<div class="workflow-workspace" class:has-inspector={Boolean(selected) || loadingTask}>
<section class="panel task-workspace" aria-label="Workflow tasks">
  <div class="section-heading task-toolbar"><div><h2>Work in motion</h2><p class="muted">{workflow.task_count} {workflow.task_count === 1 ? 'task' : 'tasks'} · Actual execution times</p></div><div class="view-switch" aria-label="Task presentation"><button aria-pressed={view === 'timeline'} onclick={() => view = 'timeline'}>Timeline</button><button aria-pressed={view === 'list'} onclick={() => view = 'list'}>List</button></div></div>
  {#if workflow.task_count > workflow.tasks.length}<p class="notice">Showing {workflow.tasks.length} tasks from this page of {workflow.task_count}. Dependency columns describe this page; the inspector lists every declared dependency.</p><div class="form-actions"><button onclick={() => onpage()}>First task page</button>{#if workflow.next_task_cursor}<button onclick={() => onpage(workflow.next_task_cursor!)}>Next task page</button>{/if}</div>{/if}
  {#if view === 'timeline'}
    {#if lanes.length}
      <div class="execution-lanes" role="region" aria-label="Actual task execution">
        <p class="field-hint">Shared time axis · Overlap shows parallel work. Striped bars have no recorded end and extend to the latest observation.</p>
        {#each lanes as lane (lane.task.id)}<div class="execution-row"><button class="quiet execution-label" onclick={() => select(lane.task)}>{lane.task.title}</button><div class="execution-track"><button class:open={lane.task.ended_at === null} class="execution-bar" style={`left:${lane.left}%;width:${lane.width}%`} aria-label={`${lane.task.title}: started ${timeLabel(lane.task.started_at)}, ${lane.task.ended_at === null ? 'no end recorded' : `ended ${timeLabel(lane.task.ended_at)}`}`} onclick={() => select(lane.task)}></button></div></div>{/each}
      </div>
    {/if}
    <p class="field-hint">Dependencies run from left to right. Tasks in one column may run in parallel. Queued tasks have no start time.</p>
    <div class="timeline" role="region" aria-label="Task dependency timeline">
      <div class="timeline-lane">
        {#each columns as column, index (index)}
          <div class="timeline-column">
            <p class="eyebrow">{index === 0 ? 'Ready when inputs arrive' : `After dependencies · ${index + 1}`}</p>
            {#each column as task (task.id)}
              <button class="timeline-task" aria-pressed={summary?.id === task.id} onclick={() => select(task)}>
                <span class="badge {task.state}">{stateLabel(task)}</span><h3>{task.title}</h3><p>{task.description}</p>
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
        <li><button aria-pressed={summary?.id === task.id} onclick={() => select(task)}><span><h3>{task.title}</h3><small>{task.description}</small><small>{task.started_at === null ? 'Not started' : `Started ${timeLabel(task.started_at)}`}{task.ended_at === null ? '' : ` · Ended ${timeLabel(task.ended_at)}`}</small>{#if task.reason}<small>{task.reason}</small>{/if}</span><span><span class="badge {task.state}">{stateLabel(task)}</span></span></button></li>
      {/each}
    </ol>
  {/if}

  {#if !workflow.tasks.length}<p class="workspace-empty">No tasks on this page. Add future work to this workflow using Edit future batch.</p>{/if}
</section>
  {#if error}<p class="notice error workspace-error" role="alert">{error}</p>{/if}
  {#if loadingTask && !selected}<p class="field-hint inspector-loading" role="status">Loading selected task…</p>{/if}
  {#if selected}
    {@const displayedResult = result ?? selected.result}
    <section class="panel task-inspector" aria-labelledby="inspector-heading">
      <div class="inspector-heading"><div class="section-heading"><p class="eyebrow">Task inspector</p><button class="quiet inspector-toggle" bind:this={inspectorToggle} aria-expanded={inspectorOpen} aria-controls="task-inspector-content" onclick={toggleInspector}>{inspectorOpen ? 'Hide inspector' : 'Show inspector'}</button></div><h2 id="inspector-heading">{selected.definition.title}</h2><span class="badge {selected.state}">{stateLabel(selected)}</span></div>
      <!-- svelte-ignore a11y_no_noninteractive_element_interactions (Escape closes this labeled disclosure while preserving its focused control for reopening.) -->
      <div id="task-inspector-content" role="region" aria-label="Selected task details" hidden={!inspectorOpen} onfocusin={rememberInspectorFocus} onkeydown={inspectorKeydown}>
      {#if selected.reason}<p class="notice {selected.state === 'failed' ? 'error' : 'warning'}">{selected.reason}</p>{/if}
      {#if selected.stop_requested && selected.state !== 'stopped'}<p class="notice warning">A stop was requested. The daemon has not reported that this task stopped; its current state remains {selected.state}.</p>{/if}
      <div class="form-actions inspector-actions">
        {#if selected.state === 'queued' && selected.started_at === null}<button onclick={() => edit(selected!)} disabled={busy || pending !== null}>Edit unstarted task</button>{/if}
        {#if selected.state === 'queued' || selected.state === 'running' || selected.state === 'uncertain'}<button class="danger" onclick={() => stop(selected!)} disabled={busy || pending !== null || selected.stop_requested}>Request stop</button>{/if}
        <button disabled={busy || pending !== null || batchOpen} onclick={(event) => { openBatch(event.currentTarget); correctionSource = selected; }}>Create linked corrective work</button>
        <button onclick={() => readResult(selected!)} disabled={busy || selected.result_restricted}>Read result</button>
      </div>
      {#if pending}<p class="notice warning">The change outcome is not confirmed. Refresh shows the daemon’s current state. You can safely replay the exact request.</p><button disabled={busy} onclick={() => change(pending!)}>Replay exact change</button>{/if}
      <dl class="facts task-timing"><dt>Started</dt><dd>{selected.started_at === null ? 'Not started' : timeLabel(selected.started_at)}</dd><dt>Ended</dt><dd>{selected.ended_at === null ? 'No settlement recorded' : timeLabel(selected.ended_at)}</dd></dl>
      {#if editing && editDefinition}
        <form onsubmit={save} class="edit-form"><fieldset disabled={busy || pending !== null}><TaskDefinitionEditor bind:definition={editDefinition} {capabilities} /><p class="field-hint">The daemon accepts this edit only if the task is still unstarted and the observed generation is current.</p>{#if editGeneration !== workflow.generation}<p class="notice warning">The workflow changed while this draft was open. Review the current task and re-open its editor to use the new generation, or preserve this definition in a complete batch.</p>{/if}<div class="form-actions"><button class="primary">Save future task</button><button type="button" onclick={() => editing = false}>Cancel edit</button></div></fieldset></form>
      {:else}
        <h3 class="subheading">Accepted instructions</h3>
        <!-- svelte-ignore a11y_no_noninteractive_tabindex (Long accepted instructions remain scrollable with the keyboard.) -->
        <pre class="accepted-instructions" role="region" aria-label="Accepted instructions" tabindex="0">{selected.definition.instructions}</pre>
      {/if}
      {#if selected.state === 'queued' && selected.started_at === null && suppliedNames.length}
        <details><summary>Supply a required or replacement input</summary><form onsubmit={supplyInput}><fieldset disabled={busy || pending !== null}><label>Workflow input<select aria-label="Workflow input" bind:value={inputName}>{#each suppliedNames as name (name)}<option value={name}>{name}</option>{/each}</select></label><label>Input text<textarea bind:value={inputText} rows="4"></textarea></label><p class="field-hint">This value applies to future starts. Empty text is a supplied value; choose explicit absence in the batch editor if that is intended.</p><button>Supply input</button></fieldset></form></details>
      {/if}
      {#if displayedResult}
        <details class="result-section" open><summary>{displayedResult.text ? 'Immutable result' : 'Retained outcome'}</summary>
          {#if displayedResult.text || displayedResult.text_truncated}<div class="result-actions"><button disabled={busy} onclick={() => downloadResult(selected!)}>Download result</button></div>{/if}
          {#if displayedResult.text_unavailable}<p class="notice warning">The retained output contains bytes that cannot be displayed as UTF-8 text. Retrieve the individual immutable artifacts listed below.</p>{:else if displayedResult.text_truncated}<p class="notice">Showing a bounded text preview. Download result requests the complete retained text; the digest below covers that complete text.</p>{/if}
          {#if displayedResult.text}
            <!-- svelte-ignore a11y_no_noninteractive_tabindex (The bounded result accepts focus so its complete content can be scrolled with the keyboard.) -->
            <pre class="output inspector-text" role="region" aria-label="Retained result text" tabindex="0" data-testid="result-output">{displayedResult.text}</pre>
          {:else if !displayedResult.text_truncated && !displayedResult.text_unavailable}<p class="field-hint">No text output was retained for this outcome. Inspect the task state and reason above.</p>{/if}
          <details><summary>Result and input provenance</summary><dl class="facts"><dt>Result digest</dt><dd><code>{displayedResult.digest}</code></dd>{#each Object.entries(displayedResult.input_digests) as [name, digest] (name)}<dt>{name}</dt><dd><code>{digest}</code></dd>{/each}<dt>Artifacts</dt><dd>{displayedResult.artifacts.join(', ') || 'No additional artifacts'}</dd></dl></details>
        </details>
      {:else if selected.result_restricted}
        <p class="notice warning result-section">Result contents are restricted by the current grant. The task’s lifecycle remains available.</p>
      {:else}
        <p class="field-hint result-section">No retained result is present in this snapshot. Read result requests the daemon’s current result or its reason for refusal.</p>
      {/if}
      <details class="task-details"><summary>Task details and dependencies</summary>
        <dl class="facts"><dt>Task</dt><dd><code data-testid="task-id">{selected.id}</code></dd><dt>Capability</dt><dd>{selected.definition.capability.id} · {selected.definition.capability.operation}</dd><dt>Dependencies</dt><dd>{selected.definition.dependencies.join(', ') || 'None'}</dd><dt>Required</dt><dd>{selected.definition.required ? 'Yes' : 'Optional'}</dd>{#if selected.definition.corrective_of}<dt>Corrects</dt><dd>{selected.definition.corrective_of}</dd>{/if}</dl>
      </details>
      {#each followupOrigins as origin (origin.task_id)}<p class="notice">Follow-up work admitted from task {origin.task_id}, response {origin.response_digest ?? 'digest unavailable in this authorized view'}.</p>{/each}
      {#if selected.definition.actions}<p class="field-hint">Model continuation is enabled. {selected.definition.actions.automatic_repair ? 'Automatic feedback after refused proposals is permitted within shared limits.' : 'Refused proposals require an explicit decision.'}</p>{/if}
      {#if selected.definition.tools.length}<p class="field-hint">Permitted tools: {selected.definition.tools.map((tool) => tool.kind === 'process' ? `${tool.capability.id} · ${tool.capability.operation}` : tool.kind.replaceAll('_', ' ')).join(', ')}.</p>{/if}
      <details><summary>Accepted inputs, outputs and project paths</summary><h3 class="subheading">Input rules</h3><dl class="facts">{#each Object.entries(selected.definition.inputs) as [name, input] (name)}<dt>{name}</dt><dd>{inputLabel(input)}</dd>{:else}<dt>Inputs</dt><dd>None declared</dd>{/each}</dl><h3 class="subheading">Output contract</h3><dl class="facts">{#each Object.entries(selected.definition.outputs) as [name, output] (name)}<dt>{name}</dt><dd>{output.required ? 'Required' : 'Optional'} · {output.media_type ?? 'Any media type'} · at most {output.max_bytes.toLocaleString()} bytes</dd>{:else}<dt>Outputs</dt><dd>None declared</dd>{/each}</dl><h3 class="subheading">Project access</h3><ul>{#each selected.definition.paths as claim (claim.path)}<li>{claim.path} · {claim.write ? 'Read and write' : 'Read only'}</li>{:else}<li>No native paths declared</li>{/each}</ul></details>
      {#if selected.frozen}
        <details><summary>Frozen start and selected inputs</summary>
          <dl class="facts"><dt>Capability generation</dt><dd>{selected.frozen.capability_generation}</dd><dt>Actor</dt><dd>{selected.frozen.actor_id}</dd><dt>Grant</dt><dd>{selected.frozen.grant_id} · revision {selected.frozen.grant_revision}</dd><dt>Entry intent</dt><dd>{selected.frozen.entered ? 'Durably recorded; does not establish completion' : 'Not yet recorded'}</dd></dl>
          {#each Object.entries(selected.frozen.inputs) as [name, input] (name)}
            <h3 class="subheading">{name}{input.name ? ` · ${input.name}` : ''}</h3><p class="field-hint">{input.kind}{input.digest ? ` · ${input.digest}` : ''}{input.artifact_id ? ` · ${input.artifact_id}` : ''}</p>
            {#if input.restricted}<p class="notice warning">This input’s contents, filename and provenance are restricted by the current grant.</p>{:else}
              {#if input.text_truncated}<p class="notice">This input exceeds the inspector preview limit. The captured input retains its complete bytes and digest.</p>{/if}
              {#if input.text !== null}
                <!-- svelte-ignore a11y_no_noninteractive_tabindex (The bounded input preview accepts focus so its complete content can be scrolled with the keyboard.) -->
                <pre class="inspector-text" role="region" aria-label={`Frozen input ${name}`} tabindex="0">{input.text}</pre>
              {:else}<p class="field-hint">{input.kind === 'absent' ? 'Explicitly absent optional input' : 'Immutable reference; no inline text'}</p>{/if}
            {/if}
          {/each}
        </details>
      {/if}
      </div>
    </section>
  {/if}
</div>
<div class="workflow-details">
<details class="panel workflow-detail">
  <summary>Workflow limits and current status</summary>
  <p class="field-hint">These ceilings apply across descendants and restarts. Resuming never clears spending or unresolved effects.</p>
  <dl class="facts">
    <dt>Tasks admitted</dt><dd>{workflow.admitted_tasks} / {workflow.limits.tasks}</dd>
    <dt>Model batches admitted</dt><dd>{workflow.admitted_model_batches} / {workflow.limits.model_batches}</dd>
    <dt>Consecutive rejected responses</dt><dd>{workflow.rejected_responses} / {workflow.limits.rejected_responses}</dd>
    <dt>Elapsed execution milliseconds</dt><dd>{workflow.active_execution_ms.toLocaleString()} / {workflow.limits.active_execution_ms.toLocaleString()}</dd>
    <dt>Concurrent task ceiling</dt><dd>{workflow.limits.concurrent_tasks}</dd>
    <dt>Corrective lineage failure ceiling</dt><dd>{workflow.limits.corrective_failures}</dd>
    <dt>Tool calls per task</dt><dd>{workflow.limits.tool_calls_per_task}</dd>
    <dt>Observed generation</dt><dd>{workflow.generation}</dd>
  </dl>
  <button disabled={busy || pending !== null || batchOpen} onclick={(event) => openBatch(event.currentTarget, 'set_limits')}>Review limit changes</button>
</details>
{#if workflow.admissions.length}
  <details class="panel workflow-detail">
    <summary>Model responses and future-work decisions</summary>
    <p class="field-hint">These are the daemon's decisions for source tasks on this page. Inspect the source result to read the immutable response. Accepted aliases identify the resulting follow-up tasks.</p>
    {#each workflow.admissions as admission (admission.task_id)}
      {@const source = workflow.tasks.find((task) => task.id === admission.task_id)}
      <section class="admission-record">
        <h3>{source?.title ?? admission.task_id} · {admission.status}</h3>
        {#if admission.reason}<p class="notice">{admission.reason}</p>{/if}
        <dl class="facts"><dt>Source task</dt><dd>{admission.task_id}</dd><dt>Response digest</dt><dd>{admission.response_digest ?? 'Unavailable in this authorized view'}</dd><dt>Accepted receipt</dt><dd>{admission.receipt_key ?? 'No accepted receipt'}</dd>{#each Object.entries(admission.aliases) as [alias, taskId] (alias)}<dt>{alias}</dt><dd>{taskId}</dd>{/each}</dl>
        <div class="form-actions">
          {#if source}<button onclick={() => select(source)}>Inspect source response</button>{/if}
          {#if admission.status === 'rejected'}<button disabled={busy || pending !== null || batchOpen} onclick={(event) => openBatch(event.currentTarget, 'dismiss_admission', admission.task_id)}>Review response decision</button>{/if}
        </div>
      </section>
    {/each}
  </details>
{/if}
</div>

<script lang="ts">
  import type { ChangeWorkflowRequest, TaskBindingRedirectDto, TaskCapability, TaskChangeDto, TaskDefinitionDto, TaskReadDto, TaskSessionDto, TaskUploadReceipt, TaskUploadRequest, WorkflowLimitsDto, WorkflowReadDto } from '../lib/protocol.generated';
  import { DaemonClient, DaemonError } from '../lib/transport';
  import TaskDefinitionEditor from './TaskDefinitionEditor.svelte';
  import { captureTextRequest } from '../lib/task-input';

  let { workflow, client, capabilities = [], initialTask = null, initialAction = 'add', initialTarget = '', onrefresh, onclose, onerror }: {
    workflow: WorkflowReadDto;
    client: DaemonClient;
    capabilities?: TaskCapability[];
    initialTask?: TaskReadDto | null;
    initialAction?: TaskChangeDto['kind'];
    initialTarget?: string;
    onrefresh: () => Promise<void>;
    onclose: () => void;
    onerror: (failure: unknown) => void;
  } = $props();
  const copy = <T,>(value: T): T => JSON.parse(JSON.stringify(value));
  function blankDefinition(): TaskDefinitionDto {
    return { title: '', description: '', instructions: '', capability: { ...(capabilities[0] ?? { id: '', operation: '' }) }, inputs: {}, outputs: {}, dependencies: [], required: true, corrective_of: null, paths: [], actions: null, tools: [] };
  }
  let generation = $state(0);
  let action = $state<TaskChangeDto['kind']>('add');
  let definition = $state<TaskDefinitionDto>(blankDefinition());
  let alias = $state('');
  let target = $state('');
  let version = $state(0);
  let valueName = $state('');
  let valueText = $state('');
  let valueKind = $state<'text' | 'absent' | 'uploaded'>('text');
  let correction = $state('');
  let consumer = $state('');
  let redirectInput = $state('');
  let redirects = $state<TaskBindingRedirectDto[]>([]);
  let question = $state('');
  let changes = $state<TaskChangeDto[]>([]);
  let preview = $state<ChangeWorkflowRequest | null>(null);
  let pending = $state<ChangeWorkflowRequest | null>(null);
  let busy = $state(false);
  let error = $state('');
  let conflict = $state(false);
  let imported = $state('');
  let pendingUpload = $state<TaskUploadRequest | null>(null);
  let captured = $state<TaskUploadReceipt | null>(null);
  let initialized = false;
  let limits = $state<WorkflowLimitsDto | null>(null);
  const limitLabels: Record<keyof WorkflowLimitsDto, string> = {
    tasks: 'Total admitted tasks', concurrent_tasks: 'Concurrent tasks', model_batches: 'Total model batches',
    rejected_responses: 'Consecutive rejected responses', corrective_failures: 'Failures on one corrective lineage',
    active_execution_ms: 'Cumulative execution milliseconds', tool_calls_per_task: 'Tool calls per task',
  };
  $effect(() => {
    if (initialized) return;
    initialized = true;
    generation = workflow.generation;
    limits = copy(workflow.limits);
    action = initialAction; target = initialTarget;
    if (initialTask) {
      definition = copy(initialTask.definition);
      definition.title = `Correction: ${initialTask.definition.title}`;
      definition.corrective_of = initialTask.id;
      alias = 'correction';
    }
  });

  async function loadDefinition() {
    if (!target) return;
    busy = true; error = '';
    try {
      const task = await client.request<TaskReadDto>(`/v1/workflows/${encodeURIComponent(workflow.id)}/tasks/${encodeURIComponent(target)}`);
      if (task.state !== 'queued' || task.started_at !== null) throw new Error('This task has started or settled. Use linked corrective work to change what happens next.');
      definition = copy(task.definition); version = task.version;
    } catch (failure) { report(failure); }
    finally { busy = false; }
  }

  function report(failure: unknown) {
    error = failure instanceof Error ? failure.message : 'The change could not be confirmed.';
    onerror(failure);
  }

  async function uploadFile(event?: Event) {
    busy = true; error = '';
    try {
      if (event) {
        const file = (event.currentTarget as HTMLInputElement).files?.[0];
        if (!file) return;
        const session = await client.request<TaskSessionDto>('/v1/session');
        pendingUpload = await captureTextRequest(file, session.input_limit_bytes);
      }
      if (!pendingUpload) return;
      captured = await client.request<TaskUploadReceipt>('/v1/task-inputs', pendingUpload);
      valueText = captured.input_id; valueKind = 'uploaded'; pendingUpload = null;
    } catch (failure) { report(failure); }
    finally { busy = false; }
  }

  function addChange(event: SubmitEvent) {
    event.preventDefault();
    let change: TaskChangeDto;
    switch (action) {
      case 'add': change = { kind: 'add', alias, definition: copy(definition) }; break;
      case 'revise': change = { kind: 'revise', task: target, expected_version: version, definition: copy(definition) }; break;
      case 'remove': change = { kind: 'remove', task: target, expected_version: version }; break;
      case 'stop': change = { kind: 'stop', task: target }; break;
      case 'bind_input': change = { kind: 'bind_input', name: valueName, value: valueKind === 'text' ? { kind: 'text', text: valueText } : valueKind === 'uploaded' ? { kind: 'uploaded', input_id: valueText } : { kind: 'absent' } }; break;
      case 'adopt_correction': change = { kind: 'adopt_correction', task: target, correction, redirects: copy(redirects) }; break;
      case 'wait': change = { kind: 'wait', question }; break;
      case 'continue': change = { kind: 'continue' }; break;
      case 'finish': change = { kind: 'finish' }; break;
      case 'set_limits': if (!limits) return; change = { kind: 'set_limits', limits: copy(limits) }; break;
      case 'dismiss_admission': change = { kind: 'dismiss_admission', task: target }; break;
    }
    changes = [...changes, change]; preview = null; error = '';
  }

  function addRedirect() {
    const selected = workflow.tasks.find((task) => task.id === consumer);
    if (!selected || !redirectInput.trim()) return;
    redirects = [...redirects, { consumer, expected_version: selected.version, input: redirectInput.trim() }];
    redirectInput = '';
  }

  function importChanges() {
    try {
      const value: unknown = JSON.parse(imported);
      if (!Array.isArray(value) || !value.length || value.some((entry) => typeof entry !== 'object' || entry === null || !('kind' in entry) || typeof entry.kind !== 'string')) throw new Error('Enter a nonempty JSON array of typed changes. The daemon validates the complete contract on submission.');
      changes = value as TaskChangeDto[]; preview = null; error = '';
    } catch (failure) { report(failure); }
  }

  async function refreshConflict() {
    busy = true;
    try { await onrefresh(); generation = workflow.generation; conflict = false; preview = null; }
    catch (failure) { report(failure); }
    finally { busy = false; }
  }

  async function submit(request: ChangeWorkflowRequest) {
    busy = true; error = ''; pending = request;
    try {
      await client.request(`/v1/workflows/${encodeURIComponent(workflow.id)}/changes`, request);
      pending = null; preview = null; changes = []; await onrefresh(); onclose();
    } catch (failure) {
      report(failure);
      if (failure instanceof DaemonError && failure.status >= 400 && failure.status < 500) {
        pending = null; preview = null; conflict = failure.status === 409;
      }
      await onrefresh();
    } finally { busy = false; }
  }
</script>

<section class="panel batch-editor" aria-labelledby="batch-heading">
  <div class="section-heading"><div><p class="eyebrow">Future work · generation {generation}</p><h2 id="batch-heading">Edit a complete future batch</h2></div><button onclick={onclose} disabled={busy || pending !== null}>Close batch editor</button></div>
  <p class="muted">Stage related changes, inspect the exact request, then submit once. The daemon checks authority, inputs, dependencies, limits and start races before accepting the whole batch.</p>
  {#if initialTask}<p class="notice">New work addresses {initialTask.id}. Its corrective link keeps replacement work waiting for the original task to settle; requesting stop does not prove settlement.</p>{/if}
  {#if error}<p class="notice error" role="alert">{error}</p>{/if}
  {#if conflict}<p class="notice warning">The daemon refused this batch against its current state. Your draft is retained. Refresh the observed generation, reload changed task definitions and review the complete batch before a new submission. Started task inputs cannot change.</p><button disabled={busy} onclick={refreshConflict}>Refresh generation for review</button>{/if}
  {#if !conflict && !pending && generation !== workflow.generation}<p class="notice warning">The observed workflow advanced to generation {workflow.generation} while this draft was open. Its guards still refer to generation {generation}. Refresh and review affected task versions before submitting.</p><button disabled={busy} onclick={refreshConflict}>Refresh generation for review</button>{/if}
  {#if !pending}
    <form onsubmit={addChange}>
      <fieldset disabled={busy || preview !== null || pendingUpload !== null}>
        <label>Change to stage<select bind:value={action} onchange={() => { version = 0; }}><option value="add">Add future task</option><option value="revise">Revise unstarted task</option><option value="remove">Remove unstarted task</option><option value="bind_input">Supply or replace a workflow input</option><option value="adopt_correction">Adopt a successful correction and rebind consumers</option><option value="stop">Request task stop</option><option value="dismiss_admission">Dismiss a response decision</option><option value="set_limits">Change prospective workflow limits</option><option value="continue">Resume prospectively</option><option value="wait">Wait for a decision</option><option value="finish">Finish when obligations settle</option></select></label>
        {#if ['revise', 'remove', 'stop', 'adopt_correction', 'dismiss_admission'].includes(action)}
          <label>Target task<select bind:value={target} required onchange={() => version = 0}><option value="">Choose a task</option>{#each workflow.tasks as task (task.id)}<option value={task.id}>{task.title} · {task.state} · {task.id}</option>{/each}</select></label>
        {/if}
        {#if action === 'add'}<label>Batch alias<input bind:value={alias} required placeholder="research-a" /></label>{/if}
        {#if action === 'revise' || action === 'remove'}<button type="button" disabled={!target} onclick={loadDefinition}>Load observed task definition</button><p class="field-hint">Observed task version: {version || 'Load the task first'}</p>{/if}
        {#if action === 'add' || action === 'revise'}<TaskDefinitionEditor bind:definition {capabilities} />{/if}
        {#if action === 'bind_input'}
          <label>Supplied workflow input name<input bind:value={valueName} required /></label><label>Input value type<select bind:value={valueKind}><option value="text">Text</option><option value="absent">Explicit absence</option><option value="uploaded">Captured upload identity</option></select></label>
          {#if valueKind !== 'absent'}<label>{valueKind === 'text' ? 'Supplied input text' : 'Uploaded input identity'}<textarea bind:value={valueText} required={valueKind === 'uploaded'} rows="4"></textarea></label>{/if}
          <label>Capture replacement text file<input type="file" accept="text/*,.md,.toml,.json,.csv,.rs,.txt" onchange={uploadFile} /></label>{#if captured}<p class="notice">Captured {captured.name} · {captured.bytes.toLocaleString()} bytes · {captured.digest}</p>{/if}
        {/if}
        {#if action === 'adopt_correction'}
          <label>Successful correction<select bind:value={correction} required><option value="">Choose a completed correction</option>{#each workflow.tasks.filter((task) => task.state === 'completed') as task (task.id)}<option value={task.id}>{task.title} · {task.id}</option>{/each}</select></label>
          <p class="field-hint">The daemon checks corrective lineage and compatible outputs. Only the explicitly selected future bindings move; started tasks keep their frozen inputs.</p>
          {#each redirects as redirect, index (`${index}/${redirect.consumer}/${redirect.input}`)}<div class="draft-row"><span>{redirect.consumer} / {redirect.input} · version {redirect.expected_version}</span><button type="button" onclick={() => redirects = redirects.filter((_, current) => current !== index)}>Remove redirect</button></div>{/each}
          <label>Future consumer<select bind:value={consumer}><option value="">Choose an unstarted consumer</option>{#each workflow.tasks.filter((task) => task.state === 'queued' && task.started_at === null) as task (task.id)}<option value={task.id}>{task.title} · version {task.version}</option>{/each}</select></label><label>Consumer input to rebind<input bind:value={redirectInput} /></label><button type="button" disabled={!consumer || !redirectInput.trim()} onclick={addRedirect}>Add binding redirect</button>
        {/if}
        {#if action === 'wait'}<label>Decision or missing input question<textarea bind:value={question} required rows="3"></textarea></label>{/if}
        {#if action === 'dismiss_admission'}<p class="notice">This closes the pending decision for the selected model response and preserves its immutable result. To approve repaired future work, stage its complete reviewed changes in this same batch. Dismissal grants no additional capability authority.</p>{/if}
        {#if action === 'set_limits' && limits}<p class="notice">New ceilings apply prospectively. Prior task admissions, model batches, elapsed execution and unresolved reservations remain spent or held.</p><div class="form-grid">{#each Object.entries(limitLabels) as [name, label] (name)}<label>{label}<input type="number" min="1" step="1" required bind:value={limits[name as keyof WorkflowLimitsDto]} /></label>{/each}</div>{/if}
        {#if action === 'continue'}<p class="notice">Resume retains spending, failed history and unresolved effects. It does not enlarge capability grants.</p>{/if}
        {#if action === 'finish'}<p class="notice">Completion still requires satisfied obligations and no running, uncertain or undecided work.</p>{/if}
        <div class="form-actions"><button disabled={(action === 'revise' || action === 'remove') && !version}>Stage change</button></div>
      </fieldset>
    </form>
    {#if pendingUpload}<p class="notice warning">The captured-file reply is not confirmed. Replay the exact upload before submitting its binding.</p><button disabled={busy} onclick={() => uploadFile()}>Replay exact file capture</button>{/if}
    <details><summary>Import or revise typed changes</summary><p class="field-hint">This is the same public change array used by the CLI. Import replaces this draft only; it performs no daemon action.</p><label>Changes JSON<textarea bind:value={imported} rows="8" disabled={busy || preview !== null}></textarea></label><button disabled={busy || preview !== null} onclick={() => imported = JSON.stringify(changes, null, 2)}>Copy draft into JSON editor</button><button disabled={busy || preview !== null || !imported.trim()} onclick={importChanges}>Use JSON changes</button></details>
  {/if}
  <h3 class="subheading">Staged changes ({changes.length})</h3>
  <ol class="staged-changes">{#each changes as change, index (index)}<li><strong>{change.kind.replaceAll('_', ' ')}</strong>{#if 'alias' in change} · {change.alias}{/if}{#if 'task' in change} · {change.task}{/if}{#if 'name' in change} · {change.name}{/if}{#if 'definition' in change && change.definition} · {change.definition.title}{/if}{#if !preview && !pending}<button disabled={busy} onclick={() => changes = changes.filter((_, current) => current !== index)} aria-label={`Remove staged change ${index + 1}`}>Remove</button>{/if}</li>{/each}</ol>
  {#if pending}<p class="notice warning">The reply was not confirmed. This exact request is held unchanged for safe replay; a new identity could duplicate accepted work.</p><button disabled={busy} onclick={() => submit(pending!)}>Replay exact batch</button>
  {:else if preview}
    <h3 class="subheading">Exact batch preview</h3>
    <p class="field-hint">Scroll through the complete definitions, input bindings and guards below. This preview sends no changes; the daemon validates admission when you submit.</p>
    <!-- svelte-ignore a11y_no_noninteractive_tabindex (The bounded request preview must accept keyboard focus so its complete content can be scrolled without a pointer.) -->
    <pre class="batch-preview" role="region" aria-label="Exact batch request" tabindex="0" data-testid="batch-preview">{JSON.stringify(preview, null, 2)}</pre>
    <div class="form-actions"><button class="primary" disabled={busy} onclick={() => submit(preview!)}>Submit reviewed batch</button><button disabled={busy} onclick={() => preview = null}>Return to draft</button></div>
  {:else}<button class="primary" disabled={busy || !changes.length || conflict || pendingUpload !== null} onclick={() => preview = { request_id: crypto.randomUUID(), expected_generation: generation, changes: copy(changes) }}>Preview complete batch</button>{/if}
</section>
